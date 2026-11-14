//! Round 38c: undo never reports `complete: true` while leaving changes in place.
//!
//! `tee`, `db_kv_set`, `db_sqlite_to_csv` and `dd_copy` journal the file they
//! write (not the one they read); a database a call creates is recorded as
//! absent before, so undo removes it; undoing a `chmod` of a read-only file
//! does not fail; a created file already deleted by hand is reported the same
//! way whichever builtin created it.
//!
//! These drive the real binary, one process per call as the Python SDK does, in
//! a fresh temporary directory, human mode, journal on. The session journal
//! lives in its own temporary directory, never the repository's.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    ws: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-round38c-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            ws: root.join("ws"),
            session: root.join("session"),
            root,
        };
        std::fs::create_dir_all(&d.ws).unwrap();
        std::fs::create_dir_all(&d.session).unwrap();
        d
    }

    fn run(&self, expr: &str) -> Output {
        let out = Command::new(env!("CARGO_BIN_EXE_ish"))
            .current_dir(&self.ws)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env("IRONSHELL_JOURNAL", "on")
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_AGENT")
            .env_remove("IRONSHELL_WORKSPACE")
            .args(["-c", expr])
            .output()
            .expect("run ish");
        assert!(out.status.success(), "{expr}: {}", both(&out));
        out
    }

    /// The printed value of `expr`, whitespace and string quotes removed.
    fn value(&self, expr: &str) -> String {
        let out = self.run(expr);
        String::from_utf8_lossy(&out.stdout)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .trim_matches('"')
            .to_string()
    }

    fn exists(&self, rel: &str) -> bool {
        self.ws.join(rel).exists()
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        // A read-only file would keep the directory.
        if let Ok(rd) = std::fs::read_dir(&self.ws) {
            for e in rd.flatten() {
                if let Ok(md) = e.metadata() {
                    let mut p = md.permissions();
                    #[allow(clippy::permissions_set_readonly_false)]
                    p.set_readonly(false);
                    let _ = std::fs::set_permissions(e.path(), p);
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn both(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn have(tool: &str) -> bool {
    Command::new(tool).arg("--version").output().is_ok()
}

#[test]
fn tee_of_a_new_file_is_one_step_and_undo_removes_it() {
    let d = Dirs::new("tee");
    d.run(r#"tee("t.txt", "hello")"#);
    assert_eq!(d.value("journal().steps"), "1");
    assert_eq!(d.value("undo().complete"), "true");
    assert!(!d.exists("t.txt"), "undo removes the file tee created");
}

#[test]
fn db_kv_set_is_one_step_and_undo_removes_a_new_store() {
    if !have("sqlite3") {
        return;
    }
    let d = Dirs::new("kv");
    d.run(r#"db_kv_set("kv.json", "k", "v")"#);
    assert_eq!(
        d.value("journal().steps"),
        "1",
        "one step, not one per statement"
    );
    assert_eq!(d.value("undo().complete"), "true");
    assert!(
        !d.exists("kv.json"),
        "the store the call created is removed"
    );
}

#[test]
fn db_sqlite_to_csv_journals_the_output_not_the_database() {
    if !have("sqlite3") {
        return;
    }
    let d = Dirs::new("csv");
    d.run(r#"db_sqlite_exec("t.db", "create table t(a int); insert into t values (1)")"#);
    d.run(r#"db_sqlite_to_csv("t.db", "t", "out.csv")"#);
    assert_eq!(d.value("undo().complete"), "true");
    assert!(!d.exists("out.csv"), "undo removes the csv");
    assert!(d.exists("t.db"), "the database was only read");
}

#[test]
fn dd_copy_journals_the_destination_not_the_source() {
    if !have("dd") && !cfg!(windows) {
        return;
    }
    let d = Dirs::new("dd");
    std::fs::write(d.ws.join("a.txt"), "source").unwrap();
    d.run(r#"dd_copy("a.txt", "dd.out")"#);
    assert_eq!(d.value("undo().complete"), "true");
    assert!(!d.exists("dd.out"), "undo removes the copy");
    assert_eq!(
        std::fs::read_to_string(d.ws.join("a.txt")).unwrap(),
        "source"
    );
}

#[test]
fn a_database_a_call_creates_is_removed_by_undo_not_left_empty() {
    if !have("sqlite3") {
        return;
    }
    let d = Dirs::new("open");
    d.run(r#"db_sqlite_open("new.db")"#);
    d.run(r#"db_sqlite_create_table("new.db", "t", "id INTEGER")"#);
    d.run(r#"db_sqlite_insert("new.db", "t", {id: 1})"#);
    assert_eq!(d.value("undo(3).complete"), "true");
    assert!(!d.exists("new.db"), "absent before, so absent after undo");
}

#[test]
fn undoing_a_chmod_of_a_file_made_read_only_succeeds() {
    let d = Dirs::new("chmod");
    std::fs::write(d.ws.join("a.txt"), "x").unwrap();
    d.run(r#"chmod("a.txt", "444")"#);
    assert_eq!(d.value("undo().complete"), "true");
    let md = std::fs::metadata(d.ws.join("a.txt")).unwrap();
    assert!(!md.permissions().readonly(), "writable again, as before");
    assert_eq!(std::fs::read_to_string(d.ws.join("a.txt")).unwrap(), "x");
}

#[test]
fn created_files_already_deleted_by_hand_are_all_reported_alike() {
    let d = Dirs::new("gone");
    d.run(r#"file_write("w.txt", "x")"#);
    d.run(r#"touch("t.txt")"#);
    std::fs::remove_file(d.ws.join("w.txt")).unwrap();
    std::fs::remove_file(d.ws.join("t.txt")).unwrap();
    // Two steps, two rows, both skipped, and `complete` says so.
    assert_eq!(d.value("undo(2).steps | len"), "2");
    // Run again on a fresh journal to read the counts.
    let d2 = Dirs::new("gone2");
    d2.run(r#"file_write("w.txt", "x")"#);
    d2.run(r#"touch("t.txt")"#);
    std::fs::remove_file(d2.ws.join("w.txt")).unwrap();
    std::fs::remove_file(d2.ws.join("t.txt")).unwrap();
    assert_eq!(d2.value("undo(2).skipped"), "2");
}

/// The tester's report: a step that was changed by hand stops the undo, and the
/// older steps it never reached were missing from `steps`, so the counts did not
/// add up to the steps asked for.
#[test]
fn an_undo_that_stops_lists_every_step_it_did_not_reach() {
    let d = Dirs::new("notreached");
    std::fs::write(d.ws.join("k.txt"), "orig").unwrap();
    d.run(r#"file.write("new.txt", "a")"#);
    d.run(r#"file.write("k.txt", "b")"#);
    d.run(r#"mkdir("dd")"#);
    d.run(r#"file.write("big.bin", "x")"#);
    std::fs::remove_file(d.ws.join("new.txt")).unwrap();
    std::fs::remove_file(d.ws.join("k.txt")).unwrap();
    std::fs::remove_dir(d.ws.join("dd")).unwrap();
    // Four steps, four rows: big.bin removed, dd skipped, k.txt skipped (it was
    // removed since), new.txt skipped as not reached.
    assert_eq!(d.value("undo(10).steps | len"), "4");
    let d2 = Dirs::new("notreached2");
    std::fs::write(d2.ws.join("k.txt"), "orig").unwrap();
    d2.run(r#"file.write("new.txt", "a")"#);
    d2.run(r#"file.write("k.txt", "b")"#);
    d2.run(r#"mkdir("dd")"#);
    d2.run(r#"file.write("big.bin", "x")"#);
    std::fs::remove_file(d2.ws.join("new.txt")).unwrap();
    std::fs::remove_file(d2.ws.join("k.txt")).unwrap();
    std::fs::remove_dir(d2.ws.join("dd")).unwrap();
    assert_eq!(d2.value("undo(10).skipped"), "3");
}

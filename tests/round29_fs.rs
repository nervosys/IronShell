//! Round 29: undo covers directories (mkdir and rmdir record steps, and an
//! undo never reaches past a directory removal it cannot reverse), journal
//! paths are normalized, and the filesystem builtins name their failures:
//! `mkdir("")`, a directory moved into itself, `ls` of a file, `cp` into a
//! directory, `fs_realpath` through a missing component, `env` returns Null,
//! and case-insensitive globs on Windows.
//!
//! These drive the real binary, one process per call as the Python SDK does, in
//! a fresh temporary directory. The session journal lives in its own temporary
//! directory, so a test never touches the repository's journal.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    ws: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-round29-{tag}-{}", std::process::id()));
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

    /// `ish -c expr` in the test's directory, in human mode (no approvals), with
    /// the journal on and its session directory in the test's own temp dir.
    fn ish(&self, expr: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-c", expr])
            .current_dir(&self.ws)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env("IRONSHELL_JOURNAL", "on")
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_AGENT")
            .env_remove("IRONSHELL_WORKSPACE")
            .output()
            .expect("run ish")
    }

    /// `ish -c expr`, which must succeed.
    fn run(&self, expr: &str) -> Output {
        let out = self.ish(expr);
        assert!(out.status.success(), "{expr}: {}", both(&out));
        out
    }

    /// The printed value of `expr`, with whitespace and string quotes removed,
    /// so that pretty and compact printing of the same value compare equal.
    fn value(&self, expr: &str) -> String {
        let out = self.run(expr);
        String::from_utf8_lossy(&out.stdout)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .trim_matches('"')
            .to_string()
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Stdout and stderr together, so a test does not care which one carries an error.
fn both(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

// ---------------------------------------------------------------------------
// Undo covers directories
// ---------------------------------------------------------------------------

#[test]
fn undo_after_mkdir_removes_the_directory_not_an_earlier_file() {
    // The reported defect: mkdir recorded no step, so undo(1) reverted the
    // file written before it.
    let d = Dirs::new("mkdir_undo");
    d.run(r#"file_write("keep.txt", "k\n")"#);
    d.run(r#"mkdir("m1")"#);
    assert_eq!(d.value("undo(1).complete"), "true");
    assert!(
        d.ws.join("keep.txt").exists(),
        "undo(1) reverted an earlier step"
    );
    assert!(
        !d.ws.join("m1").exists(),
        "the directory mkdir made is removed"
    );
}

#[test]
fn undo_after_mkdir_removes_every_directory_it_created_innermost_first() {
    let d = Dirs::new("mkdir_chain");
    d.run(r#"mkdir("a/b/c")"#);
    assert_eq!(d.value("undo(1).complete"), "true");
    assert!(
        !d.ws.join("a").exists(),
        "the outermost new directory is removed"
    );
}

#[test]
fn undo_after_mkdir_leaves_a_directory_that_gained_contents() {
    let d = Dirs::new("mkdir_full");
    d.run(r#"mkdir("g")"#);
    // Put something in it without the journal, as another process would.
    std::fs::write(d.ws.join("g").join("x.txt"), "x").unwrap();
    assert_eq!(d.value("undo(1).complete"), "false");
    assert!(
        d.ws.join("g").join("x.txt").exists(),
        "contents are never removed"
    );
    assert!(d.ws.join("g").is_dir());
}

#[test]
fn undo_after_mkdir_with_dot_dot_removes_the_directory_it_made() {
    // `sub/../m2` names `m2`; the journal used to record the typed spelling.
    let d = Dirs::new("mkdir_dotdot");
    d.run(r#"mkdir("sub/../m2")"#);
    assert!(d.ws.join("m2").is_dir());
    assert_eq!(d.value("undo(1).complete"), "true");
    assert!(!d.ws.join("m2").exists());
}

#[test]
fn undo_after_an_empty_rmdir_recreates_the_directory() {
    let d = Dirs::new("rmdir_empty");
    d.run(r#"mkdir("e1")"#);
    d.run(r#"rmdir("e1")"#);
    assert!(!d.ws.join("e1").exists());
    assert_eq!(d.value("undo(1).complete"), "true");
    assert!(
        d.ws.join("e1").is_dir(),
        "undo recreates the empty directory"
    );
}

#[test]
fn undo_after_a_recursive_rmdir_cannot_restore_the_tree_and_does_not_reach_past_it() {
    let d = Dirs::new("rmdir_tree");
    d.run(r#"file_write("keep.txt", "k\n")"#);
    d.run(r#"mkdir("t1")"#);
    d.run(r#"file_write("t1/x.txt", "x\n")"#);
    d.run(r#"rmdir("t1", true)"#);
    assert!(!d.ws.join("t1").exists());

    let undone = d.value("undo(1).complete");
    assert_eq!(undone, "false", "a removed tree is reported, not restored");
    assert!(
        d.ws.join("keep.txt").exists(),
        "undo(1) after rmdir reached past it to an earlier step"
    );
    assert!(!d.ws.join("t1").exists());
}

// ---------------------------------------------------------------------------
// Journal paths
// ---------------------------------------------------------------------------

#[test]
fn a_mkdir_that_creates_nothing_leaves_no_step_for_undo_to_take() {
    // `mkdir(".")` changes nothing, so the last real step is the write before
    // it, and undo(1) is that write's undo.
    let d = Dirs::new("mkdir_noop");
    d.run(r#"file_write("keep.txt", "k\n")"#);
    d.run(r#"mkdir(".")"#);
    assert_eq!(d.value("undo(1).complete"), "true");
    assert!(
        !d.ws.join("keep.txt").exists(),
        "the write is what undo reverts"
    );
}

// ---------------------------------------------------------------------------
// Refusals and named failures
// ---------------------------------------------------------------------------

#[test]
fn mkdir_of_an_empty_path_is_refused() {
    let d = Dirs::new("mkdir_empty_arg");
    assert_eq!(
        d.value(r#"(try { mkdir("") } catch e { e.error.code })"#),
        "E_BAD_ARG"
    );
}

#[test]
fn a_directory_cannot_be_moved_into_itself() {
    let d = Dirs::new("mv_self");
    d.run(r#"mkdir("sub/deep")"#);
    let out = d.ish(r#"mv("sub", "sub/deep/moved")"#);
    let text = both(&out);
    assert!(!out.status.success(), "{text}");
    assert!(
        text.contains("cannot move a directory into itself"),
        "{text}"
    );
    assert!(!text.contains("os error"), "{text}");
    assert!(d.ws.join("sub").join("deep").is_dir(), "nothing moved");
}

#[test]
fn rmdir_of_a_non_empty_directory_names_the_fix() {
    let d = Dirs::new("rmdir_full");
    d.run(r#"mkdir("sub")"#);
    d.run(r#"file_write("sub/x.txt", "x\n")"#);
    let out = d.ish(r#"rmdir("sub")"#);
    let text = both(&out);
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("rmdir: `"), "{text}");
    assert!(text.contains("is not empty"), "{text}");
    assert!(text.contains("removes it and its contents"), "{text}");
    assert!(!text.contains("os error"), "{text}");
    assert!(
        d.ws.join("sub").join("x.txt").exists(),
        "nothing was removed"
    );
}

#[test]
fn mkdir_over_a_file_says_the_file_exists() {
    let d = Dirs::new("mkdir_file");
    d.run(r#"file_write("a.txt", "a\n")"#);
    let out = d.ish(r#"mkdir("a.txt")"#);
    let text = both(&out);
    assert!(!out.status.success(), "{text}");
    assert!(text.contains("mkdir: `"), "{text}");
    assert!(text.contains("exists and is a file"), "{text}");
    assert!(!text.contains("os error"), "{text}");
}

// ---------------------------------------------------------------------------
// ls, cp, fs_realpath, env
// ---------------------------------------------------------------------------

#[test]
fn ls_of_a_file_lists_that_file_as_one_entry() {
    let d = Dirs::new("ls_file");
    d.run(r#"file_write("a.txt", "hi\n")"#);
    assert_eq!(d.value(r#"ls("a.txt") | len"#), "1");
    assert_eq!(
        d.value(r#"ls("a.txt") | map(fn(f) => f.name)"#),
        r#"["a.txt"]"#
    );
    assert_eq!(
        d.value(r#"ls("a.txt") | map(fn(f) => f.is_dir)"#),
        "[false]"
    );
}

#[test]
fn cp_of_a_file_into_a_directory_writes_it_inside_and_undo_removes_it() {
    let d = Dirs::new("cp_into");
    d.run(r#"file_write("a.txt", "hi\n")"#);
    d.run(r#"mkdir("sub")"#);
    assert_eq!(d.value(r#"cp("a.txt", "sub").bytes"#), "3");
    assert!(d.ws.join("sub").join("a.txt").is_file());
    assert!(!d.ws.join("sub").is_file());

    // The copy is journaled as a step of its own, naming the file it wrote.
    assert_eq!(d.value("undo(1).complete"), "true");
    assert!(!d.ws.join("sub").join("a.txt").exists());
    assert!(
        d.ws.join("sub").is_dir(),
        "the directory was not the copy's"
    );
}

#[cfg(unix)]
#[test]
fn cp_refuses_a_symbolic_link_destination_to_a_directory() {
    let d = Dirs::new("cp_link");
    d.run(r#"file_write("a.txt", "hi\n")"#);
    d.run(r#"mkdir("sub")"#);
    std::os::unix::fs::symlink(d.ws.join("sub"), d.ws.join("link")).unwrap();
    let out = d.ish(r#"cp("a.txt", "link")"#);
    assert!(!out.status.success(), "{}", both(&out));
    assert!(!d.ws.join("sub").join("a.txt").exists());
}

#[test]
fn fs_realpath_through_a_missing_component_is_null() {
    let d = Dirs::new("realpath");
    d.run(r#"file_write("a.txt", "a\n")"#);
    // `-c` prints nothing for Null, so the type is what is compared.
    assert_eq!(d.value(r#"typeof(fs_realpath("nope/../a.txt"))"#), "Null");
    assert_eq!(
        d.value(r#"ends_with(fs_realpath("a.txt"), "a.txt")"#),
        "true"
    );
}

#[test]
fn env_of_an_unset_variable_is_null_and_a_default_replaces_it() {
    let d = Dirs::new("env");
    assert_eq!(
        d.value(r#"typeof(env("IRONSHELL_ROUND29_NO_SUCH_VARIABLE"))"#),
        "Null"
    );
    assert_eq!(
        d.value(r#"env("IRONSHELL_ROUND29_NO_SUCH_VARIABLE", "fallback")"#),
        "fallback"
    );
}

#[cfg(windows)]
#[test]
fn a_glob_matches_names_in_either_case_on_windows() {
    let d = Dirs::new("glob_case");
    d.run(r#"file_write("a.txt", "a\n")"#);
    assert_eq!(d.value(r#"glob("*.TXT") | len"#), "1");
    assert_eq!(
        d.value(r#"ls("*.TXT") | map(fn(f) => f.name)"#),
        r#"["a.txt"]"#
    );
}

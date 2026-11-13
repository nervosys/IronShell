//! Round 36: undo never reports `complete: true` while leaving changes in place.
//!
//! Extraction, compression, backup and archive creation journal the files they
//! write (not the files they only read); `ish -b` records in human mode what it
//! records under `--agent --workspace`, `mkdir -p` included; `tar_create` stores
//! relative member names.
//!
//! These drive the real binary, one process per call as the Python SDK does, in
//! a fresh temporary directory, human mode, journal on. The session journal
//! lives in its own temporary directory, never the repository's.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    ws: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-round36-{tag}-{}", std::process::id()));
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

    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ish"));
        c.current_dir(&self.ws)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env("IRONSHELL_JOURNAL", "on")
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_AGENT")
            .env_remove("IRONSHELL_WORKSPACE");
        c
    }

    /// `ish -c expr`, which must succeed.
    fn run(&self, expr: &str) -> Output {
        let out = self.command().args(["-c", expr]).output().expect("run ish");
        assert!(out.status.success(), "{expr}: {}", both(&out));
        out
    }

    /// `ish -b -c script` (the in-process POSIX subset), which must succeed.
    fn sh(&self, script: &str) -> Output {
        let out = self
            .command()
            .args(["-b", "-c", script])
            .output()
            .expect("run ish -b");
        assert!(out.status.success(), "{script}: {}", both(&out));
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

    fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.ws.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn read(&self, rel: &str) -> Vec<u8> {
        std::fs::read(self.ws.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    fn exists(&self, rel: &str) -> bool {
        self.ws.join(rel).exists()
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
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

/// An archive with `a.txt` (new bytes) and `crlf.txt`, made by zip_create from
/// files in `src`.
fn make_zip(d: &Dirs, name: &str) {
    d.write("src/a.txt", b"new\n");
    d.write("src/crlf.txt", b"x\r\ny\r\n");
    d.run(&format!(
        r#"zip_create("{name}", "src/a.txt", "src/crlf.txt")"#
    ));
}

fn slashed(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

// ---------------------------------------------------------------------------
// 1. Extraction journals what it writes
// ---------------------------------------------------------------------------

#[test]
fn undo_of_zip_extract_restores_overwritten_files_and_removes_new_ones() {
    let d = Dirs::new("zip_extract");
    make_zip(&d, "g.zip");
    d.write("ex/a.txt", b"old\r\nbytes\n");
    let archive = d.read("g.zip");

    d.run(r#"zip_extract("g.zip", "ex")"#);
    assert_eq!(d.read("ex/a.txt"), b"new\n");
    assert_eq!(d.read("ex/crlf.txt"), b"x\r\ny\r\n");

    assert_eq!(d.value("undo(1).complete"), "true");
    assert_eq!(d.read("ex/a.txt"), b"old\r\nbytes\n");
    assert!(!d.exists("ex/crlf.txt"), "a file the extraction created");
    assert!(d.exists("ex"), "a directory that was already there");
    assert_eq!(d.read("g.zip"), archive, "the archive is only read");
}

#[test]
fn undo_of_zip_extract_removes_the_destination_it_created() {
    let d = Dirs::new("zip_extract_dest");
    make_zip(&d, "g.zip");
    d.run(r#"zip_extract("g.zip", "fresh/deep")"#);
    assert!(d.exists("fresh/deep/a.txt"));

    assert_eq!(d.value("undo(1).complete"), "true");
    assert!(!d.exists("fresh"), "every directory the call made");
    assert!(d.exists("g.zip"));
}

#[test]
fn undo_of_tar_extract_restores_and_removes() {
    let d = Dirs::new("tar_extract");
    d.write("src/a.txt", b"new\n");
    d.write("src/sub/b.txt", b"b\r\n");
    d.run(r#"tar_create("t.tar", "src")"#);
    d.write("ex/src/a.txt", b"old\n");
    let archive = d.read("t.tar");

    d.run(r#"tar_extract("t.tar", "ex")"#);
    assert_eq!(d.read("ex/src/a.txt"), b"new\n");
    assert_eq!(d.read("ex/src/sub/b.txt"), b"b\r\n");

    assert_eq!(d.value("undo(1).complete"), "true");
    assert_eq!(d.read("ex/src/a.txt"), b"old\n");
    assert!(
        !d.exists("ex/src/sub"),
        "a directory the extraction created"
    );
    assert_eq!(d.read("t.tar"), archive);
}

#[test]
fn an_extraction_over_a_file_too_big_to_keep_is_not_complete() {
    let d = Dirs::new("zip_extract_big");
    d.write("src/big.bin", b"small\n");
    d.run(r#"zip_create("g.zip", "src/big.bin")"#);
    d.write("ex/big.bin", &vec![7u8; 9 * 1024 * 1024]);

    d.run(r#"zip_extract("g.zip", "ex")"#);
    assert_eq!(d.value("undo(1).complete"), "false");
    assert_eq!(
        d.read("ex/big.bin"),
        b"small\n",
        "left as the call wrote it"
    );
}

// ---------------------------------------------------------------------------
// 2. Outputs, not sources
// ---------------------------------------------------------------------------

#[test]
fn undo_of_gzip_compress_removes_the_gz_and_keeps_the_source() {
    let d = Dirs::new("gzip");
    let bom: &[u8] = b"\xEF\xBB\xBFbom\r\n";
    d.write("bom.txt", bom);
    d.run(r#"gzip_compress("bom.txt")"#);
    assert!(d.exists("bom.txt.gz"));

    assert_eq!(d.value("undo(1).complete"), "true");
    assert!(!d.exists("bom.txt.gz"));
    assert_eq!(d.read("bom.txt"), bom);
}

#[test]
fn undo_of_file_backup_removes_the_backup_and_keeps_the_source() {
    let d = Dirs::new("backup");
    d.write("r.txt", b"r\r\n");
    d.run(r#"file_backup("r.txt")"#);
    assert!(d.exists("r.txt.bak"));

    assert_eq!(d.value("undo(1).complete"), "true");
    assert!(!d.exists("r.txt.bak"));
    assert_eq!(d.read("r.txt"), b"r\r\n");
}

#[test]
fn undo_of_file_backup_over_an_old_backup_restores_it() {
    let d = Dirs::new("backup_over");
    d.write("r.txt", b"current\n");
    d.write("r.txt.bak", b"older backup\n");
    d.run(r#"file_backup("r.txt")"#);
    assert_eq!(d.read("r.txt.bak"), b"current\n");

    assert_eq!(d.value("undo(1).complete"), "true");
    assert_eq!(d.read("r.txt.bak"), b"older backup\n");
    assert_eq!(d.read("r.txt"), b"current\n");
}

// ---------------------------------------------------------------------------
// 3. Archive creation is journaled
// ---------------------------------------------------------------------------

#[test]
fn undo_of_tar_create_and_zip_create_removes_the_archive() {
    let d = Dirs::new("create");
    d.write("in.txt", b"in\n");

    d.run(r#"tar_create("a.tar", "in.txt")"#);
    assert_ne!(d.value("journal().steps"), "0");
    assert_eq!(d.value("undo(1).complete"), "true");
    assert!(!d.exists("a.tar"));

    d.run(r#"zip_create("a.zip", "in.txt")"#);
    assert_ne!(d.value("journal().steps"), "0");
    assert_eq!(d.value("undo(1).complete"), "true");
    assert!(!d.exists("a.zip"));
    assert_eq!(d.read("in.txt"), b"in\n");
}

#[test]
fn undo_of_zip_add_restores_the_previous_archive_bytes() {
    let d = Dirs::new("zip_add");
    make_zip(&d, "g.zip");
    let before = d.read("g.zip");
    d.write("src/b.txt", b"b\n");

    d.run(r#"zip_add("g.zip", "src/b.txt")"#);
    assert_ne!(d.read("g.zip"), before);

    assert_eq!(d.value("undo(1).complete"), "true");
    assert_eq!(d.read("g.zip"), before);
}

// ---------------------------------------------------------------------------
// 4 and 5. ish -b records in human mode what agent mode records
// ---------------------------------------------------------------------------

#[test]
fn ish_b_in_human_mode_journals_every_write_and_undo_puts_it_all_back() {
    let d = Dirs::new("posix");
    d.write("sed.txt", b"foo\r\nbar\n");
    d.sh("sed -i.orig s/foo/XXX/ sed.txt; echo hi > e1.txt; tee e2.txt < e1.txt; mkdir -p m1/m2; touch m1/m2/t");
    assert!(d.exists("e1.txt") && d.exists("e2.txt"));
    assert!(d.exists("sed.txt.orig") && d.exists("m1/m2/t"));

    assert_eq!(d.value("undo(journal().steps).complete"), "true");
    assert_eq!(d.read("sed.txt"), b"foo\r\nbar\n");
    for gone in ["sed.txt.orig", "e1.txt", "e2.txt", "m1"] {
        assert!(!d.exists(gone), "{gone} is still there after undo");
    }
}

#[test]
fn undo_of_mkdir_p_in_ish_b_removes_the_directories_it_created() {
    let d = Dirs::new("posix_mkdir");
    d.write("keep.txt", b"k\n");
    d.sh("mkdir -p m1/m2");
    assert!(d.exists("m1/m2"));

    assert_eq!(d.value("undo(1).complete"), "true");
    assert!(!d.exists("m1"), "the outermost directory it created");
    assert_eq!(d.read("keep.txt"), b"k\n");
}

// ---------------------------------------------------------------------------
// 6. tar_create stores relative names
// ---------------------------------------------------------------------------

#[test]
fn tar_create_stores_relative_names_and_extracts_to_them() {
    let d = Dirs::new("tar_names");
    d.write("in.txt", b"in\n");
    d.write("sub/x.txt", b"x\n");
    let outside = d.root.join("other");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("o.txt"), b"o\n").unwrap();

    d.run(&format!(
        r#"tar_create("a.tar", "in.txt", "./sub/../sub", "{}")"#,
        slashed(&outside.join("o.txt"))
    ));
    let listing = both(&d.run(r#"tar_list("a.tar")"#));
    assert!(listing.contains("in.txt"), "{listing}");
    assert!(listing.contains("sub/x.txt"), "{listing}");
    assert!(listing.contains("o.txt"), "{listing}");
    assert!(
        !listing.contains("ae-round36") && !listing.contains("../"),
        "an absolute or climbing member name was stored: {listing}"
    );

    d.run(r#"tar_extract("a.tar", "out")"#);
    assert_eq!(d.read("out/in.txt"), b"in\n");
    assert_eq!(d.read("out/sub/x.txt"), b"x\n");
    assert_eq!(d.read("out/o.txt"), b"o\n");
    assert!(!d.exists("out/Users") && !d.exists("out/tmp") && !d.exists("out/var"));
}

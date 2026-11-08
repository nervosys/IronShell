//! Paths returned to a user are ordinary paths, not the `\\?\C:\...` form that
//! `canonicalize` gives on Windows.
//!
//! `ls` and `find` walk the canonical directory, and `fs_realpath` returns the
//! canonical path, so each reported the verbatim prefix. The prefix is stripped
//! for drive paths. A verbatim UNC path keeps its prefix, since there is no
//! plain spelling for it; this test never builds a UNC path.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ae-verbatim-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cwd").join("sub")).unwrap();
        std::fs::write(root.join("cwd").join("sub").join("f.txt"), "hi\n").unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Scratch { root }
    }

    fn ish(&self, expr: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-c", expr])
            .current_dir(self.root.join("cwd"))
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .output()
            .expect("run ish")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn stdout_of(s: &Scratch, expr: &str) -> String {
    let out = s.ish(expr);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{expr} failed: {err}");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn ls_reports_plain_paths() {
    let s = Scratch::new("ls");
    let text = stdout_of(&s, r#"ls("sub")"#);
    assert!(text.contains("f.txt"), "ls listed nothing: {text}");
    assert!(
        !text.contains(r"\\?\"),
        "ls leaked the verbatim prefix: {text}"
    );
}

#[test]
fn find_reports_plain_paths() {
    let s = Scratch::new("find");
    let text = stdout_of(&s, r#"find("sub")"#);
    assert!(text.contains("f.txt"), "find listed nothing: {text}");
    assert!(
        !text.contains(r"\\?\"),
        "find leaked the verbatim prefix: {text}"
    );
}

#[test]
fn fs_realpath_reports_a_plain_path() {
    let s = Scratch::new("realpath");
    let text = stdout_of(&s, r#"fs_realpath("sub")"#);
    assert!(text.trim_end().ends_with("sub"), "realpath was {text:?}");
    assert!(
        !text.contains(r"\\?\"),
        "fs_realpath leaked the verbatim prefix: {text}"
    );
}

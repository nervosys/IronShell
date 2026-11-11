//! Round 20: in-place writes check the handle they write through, an atomic
//! replace of a hard-linked file says so, undo can take one step out of order,
//! and an audit log without a key says it is unkeyed.
//!
//! These drive the real binary in agent mode, from a fresh directory outside the
//! workspace, with the session journal in its own temporary directory. A test
//! never touches the repository, because the undo journal is keyed by the
//! working directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    workspace: PathBuf,
    outside: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-round20-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            workspace: root.join("ws"),
            outside: root.join("outside"),
            session: root.join("session"),
            root,
        };
        for p in [&d.workspace, &d.outside, &d.session] {
            std::fs::create_dir_all(p).unwrap();
        }
        d
    }

    /// Runs `ish` in agent mode, from the directory outside the workspace.
    fn ish(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(&self.outside)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", &self.workspace)
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env("IRONSHELL_JOURNAL", "on")
            .env("IRONSHELL_ALLOW_SH", "")
            .output()
            .expect("run ish")
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// A file in the workspace with a second name outside it. Returns both paths.
fn linked_pair(d: &Dirs, body: &str) -> (PathBuf, PathBuf) {
    let inside = d.workspace.join("linked.txt");
    let outside = d.outside.join("other.txt");
    std::fs::write(&inside, body).unwrap();
    std::fs::hard_link(&inside, &outside).expect("hard link");
    (inside, outside)
}

#[test]
fn a_redirection_into_a_hard_linked_file_is_refused_and_writes_nothing() {
    let d = Dirs::new("redir");
    let (inside, outside) = linked_pair(&d, "keep\n");
    let out = d.ish(&["-b", "-c", "echo changed > linked.txt"]);
    assert!(!out.status.success(), "{out:?}");
    assert_eq!(read(&outside), "keep\n", "{out:?}");
    assert_eq!(read(&inside), "keep\n", "{out:?}");
}

#[test]
fn an_append_to_a_hard_linked_file_is_refused_and_writes_nothing() {
    let d = Dirs::new("append");
    let (inside, outside) = linked_pair(&d, "keep\n");
    let out = d.ish(&["-c", r#"file_append("linked.txt", "more\n")"#]);
    assert!(!out.status.success(), "{out:?}");
    assert_eq!(read(&outside), "keep\n", "{out:?}");
    assert_eq!(read(&inside), "keep\n", "{out:?}");
}

#[test]
fn a_redirection_to_a_plain_file_still_truncates_then_appends() {
    let d = Dirs::new("plain");
    let target = d.workspace.join("plain.txt");
    std::fs::write(&target, "a much longer earlier body\n").unwrap();
    let out = d.ish(&[
        "-b",
        "-c",
        "echo first > plain.txt; echo second >> plain.txt",
    ]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(read(&target), "first\nsecond\n", "{out:?}");
}

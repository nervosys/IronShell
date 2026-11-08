//! A refused read is written to the audit log, as a refused write is.
//!
//! Denied writes reached `.ironshell/audit.log`, but a read refused by the
//! workspace jail (file.read, cat, ls, glob, a POSIX utility) left no trace, so
//! an operator could not see an agent probing outside its workspace. Each
//! refusal now writes one `deny` entry, and the path in it is redacted.
//!
//! The fake key shapes are in file names inside a temp directory; nothing real
//! is read.

use std::path::PathBuf;
use std::process::{Command, Output};

const AWS_KEY: &str = "AKIAFAKEFAKEFAKE1234";

struct Dirs {
    root: PathBuf,
    workspace: PathBuf,
    outside: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-read-audit-{tag}-{}", std::process::id()));
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
        std::fs::write(d.outside.join("outside.txt"), "outside\n").unwrap();
        d
    }

    fn ish(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(&self.workspace)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", &self.workspace)
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env("IRONSHELL_ALLOW_SH", "")
            .output()
            .expect("run ish")
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.workspace.join(".ironshell").join("audit.log"))
            .unwrap_or_default()
    }

    fn denies(&self) -> usize {
        self.log()
            .lines()
            .filter(|l| l.contains("\"deny\""))
            .count()
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn each_refused_read_is_one_deny_entry() {
    let d = Dirs::new("each");
    let before = d.denies();
    let out = d.ish(&["-c", r#"file.read("../outside/outside.txt")"#]);
    assert!(!out.status.success());
    assert_eq!(d.denies(), before + 1, "log:\n{}", d.log());

    let out = d.ish(&["-b", "-c", "cat ../outside/outside.txt"]);
    assert!(!out.status.success());
    assert_eq!(d.denies(), before + 2, "log:\n{}", d.log());

    let out = d.ish(&["-c", r#"ls("../outside")"#]);
    assert!(!out.status.success());
    assert_eq!(d.denies(), before + 3, "log:\n{}", d.log());
}

#[test]
fn a_refused_glob_and_a_refused_posix_test_are_audited_too() {
    let d = Dirs::new("glob");
    let before = d.denies();
    let out = d.ish(&["-c", r#"glob("../*")"#]);
    assert!(!out.status.success());
    assert_eq!(d.denies(), before + 1, "log:\n{}", d.log());

    let out = d.ish(&["-b", "-c", "test -e ../outside/outside.txt; echo $?"]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).lines().last(),
        Some("2")
    );
    assert_eq!(d.denies(), before + 2, "log:\n{}", d.log());
}

#[test]
fn the_path_in_a_refused_read_is_redacted_in_the_log() {
    let d = Dirs::new("redact");
    let name = format!("../outside/AWS_SECRET_ACCESS_KEY={AWS_KEY}.txt");
    let out = d.ish(&["-c", &format!(r#"file.read("{name}")"#)]);
    assert!(!out.status.success());
    let log = d.log();
    assert!(log.contains("\"deny\""), "no entry was written: {log}");
    assert!(
        !log.contains(AWS_KEY),
        "the key reached the audit log: {log}"
    );
}

#[test]
fn an_allowed_read_is_not_audited_as_a_denial() {
    let d = Dirs::new("allowed");
    std::fs::write(d.workspace.join("inside.txt"), "inside\n").unwrap();
    let before = d.denies();
    let out = d.ish(&["-c", r#"file.read("inside.txt")"#]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(d.denies(), before, "{}", d.log());
}

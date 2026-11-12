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
        self.ish_keyed(args, None)
    }

    /// `ish` with the audit chain key set to `key`, or unset when `None`.
    fn ish_keyed(&self, args: &[&str], key: Option<&str>) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
        cmd.args(args)
            .current_dir(&self.outside)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", &self.workspace)
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env("IRONSHELL_JOURNAL", "on")
            .env("IRONSHELL_ALLOW_SH", "")
            .env_remove("IRONSHELL_AUDIT_KEY_FILE");
        match key {
            Some(k) => cmd.env("IRONSHELL_AUDIT_KEY", k),
            None => cmd.env_remove("IRONSHELL_AUDIT_KEY"),
        };
        cmd.output().expect("run ish")
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

#[test]
fn an_atomic_replace_of_a_hard_linked_file_says_its_other_links_keep_the_old_content() {
    let d = Dirs::new("replace");
    let (inside, outside) = linked_pair(&d, "old\n");
    let out = d.ish(&["-c", r#"file_write("linked.txt", "new\n")"#]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(read(&inside), "new\n", "{out:?}");
    assert_eq!(read(&outside), "old\n", "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("replaced a hard-linked file; its other links keep the old content"),
        "{stdout}"
    );
}

#[test]
fn file_replace_of_a_hard_linked_file_carries_the_same_note() {
    let d = Dirs::new("freplace");
    let (inside, outside) = linked_pair(&d, "old text\n");
    let out = d.ish(&["-c", r#"file_replace("linked.txt", "old", "new")"#]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(read(&inside), "new text\n", "{out:?}");
    assert_eq!(read(&outside), "old text\n", "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("replaced a hard-linked file"),
        "{out:?}"
    );
}

#[test]
fn a_replace_of_a_plain_file_has_no_link_note() {
    let d = Dirs::new("nonote");
    let target = d.workspace.join("solo.txt");
    std::fs::write(&target, "old\n").unwrap();
    let out = d.ish(&["-c", r#"file_write("solo.txt", "new\n")"#]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(read(&target), "new\n", "{out:?}");
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("hard-linked"),
        "{out:?}"
    );
}

#[test]
fn undo_by_id_takes_back_one_step_and_leaves_a_later_step_on_another_file() {
    let d = Dirs::new("undoid");
    let a = d.workspace.join("a.txt");
    let b = d.workspace.join("b.txt");
    std::fs::write(&a, "A1\n").unwrap();
    std::fs::write(&b, "B1\n").unwrap();
    let first = d.ish(&["-c", r#"file_write("a.txt", "A2\n")"#]);
    assert!(first.status.success(), "{first:?}");
    let second = d.ish(&["-c", r#"file_write("b.txt", "B2\n")"#]);
    assert!(second.status.success(), "{second:?}");

    let undone = d.ish(&["-c", "undo({id: 1})"]);
    assert!(undone.status.success(), "{undone:?}");
    assert_eq!(read(&a), "A1\n", "{undone:?}");
    assert_eq!(read(&b), "B2\n", "{undone:?}");
}

#[test]
fn undo_by_id_is_refused_when_a_later_step_changed_the_same_file() {
    let d = Dirs::new("undoid-same");
    let a = d.workspace.join("a.txt");
    std::fs::write(&a, "A0\n").unwrap();
    assert!(d
        .ish(&["-c", r#"file_write("a.txt", "A1\n")"#])
        .status
        .success());
    assert!(d
        .ish(&["-c", r#"file_write("a.txt", "A2\n")"#])
        .status
        .success());

    let refused = d.ish(&["-c", "undo({id: 1})"]);
    assert!(!refused.status.success(), "{refused:?}");
    assert_eq!(read(&a), "A2\n", "{refused:?}");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(text.contains("later entry 2"), "{text}");
    assert!(text.contains("a.txt"), "{text}");

    // The way the hint names: undo the later step, then the earlier one.
    assert!(d.ish(&["-c", "undo({id: 2})"]).status.success());
    assert_eq!(read(&a), "A1\n");
    assert!(d.ish(&["-c", "undo({id: 1})"]).status.success());
    assert_eq!(read(&a), "A0\n");
}

#[test]
fn undo_by_id_with_force_undoes_past_a_later_step_and_an_unknown_id_is_refused() {
    let d = Dirs::new("undoid-force");
    let a = d.workspace.join("a.txt");
    std::fs::write(&a, "A0\n").unwrap();
    assert!(d
        .ish(&["-c", r#"file_write("a.txt", "A1\n")"#])
        .status
        .success());
    assert!(d
        .ish(&["-c", r#"file_write("a.txt", "A2\n")"#])
        .status
        .success());

    let forced = d.ish(&["-c", "undo({id: 1, force: true})"]);
    assert!(forced.status.success(), "{forced:?}");
    assert_eq!(read(&a), "A0\n", "{forced:?}");

    let missing = d.ish(&["-c", "undo({id: 99})"]);
    assert!(!missing.status.success(), "{missing:?}");
}

const UNKEYED_NOTE: &str = "the chain is not keyed";

#[test]
fn an_unkeyed_audit_chain_is_reported_as_unkeyed_with_a_note() {
    let d = Dirs::new("auditkey");
    let log = d.workspace.join("audit.log");
    std::fs::write(&log, "").unwrap();
    let call = format!(
        r#"audit_verify("{}")"#,
        log.display().to_string().replace('\\', "/")
    );

    let unkeyed = d.ish_keyed(&["-c", call.as_str()], None);
    let text = String::from_utf8_lossy(&unkeyed.stdout).to_string();
    assert!(text.contains("keyed"), "{unkeyed:?}");
    assert!(text.contains(UNKEYED_NOTE), "{unkeyed:?}");
    assert!(text.contains("IRONSHELL_AUDIT_LOG"), "{unkeyed:?}");

    let key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let keyed = d.ish_keyed(&["-c", call.as_str()], Some(key));
    let text = String::from_utf8_lossy(&keyed.stdout).to_string();
    assert!(text.contains("keyed"), "{keyed:?}");
    assert!(!text.contains(UNKEYED_NOTE), "{keyed:?}");
}

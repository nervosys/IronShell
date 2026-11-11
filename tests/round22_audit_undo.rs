//! Round 22: a chain key that is set but too short says so, an unkeyed chain's
//! mismatch is not called tampering, agent-mode errors carry no ANSI codes, undo
//! step ids are never reused within a session, and undo's two forms are named.
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
        let root = std::env::temp_dir().join(format!("ae-round22-{tag}-{}", std::process::id()));
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

/// The `audit_verify` call for a log file, with the path in forward slashes.
fn verify_call(log: &Path) -> String {
    format!(
        r#"audit_verify("{}")"#,
        log.display().to_string().replace('\\', "/")
    )
}

const SHORT_KEY_WARNING: &str =
    "IRONSHELL_AUDIT_KEY is set but shorter than 32 characters, so the audit chain is not keyed";

#[test]
fn a_chain_key_set_but_too_short_warns_on_stderr_and_is_named_in_the_verify_note() {
    let d = Dirs::new("shortkey");
    let log = d.root.join("audit.log");
    std::fs::write(&log, "").unwrap();

    // 31 bytes: one short of the minimum.
    let short = "0123456789abcdef0123456789abcde";
    let out = d.ish_keyed(&["-c", verify_call(&log).as_str()], Some(short));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(stderr.matches(SHORT_KEY_WARNING).count(), 1, "{out:?}");

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains("\"keyed\": false") || stdout.contains("\"keyed\":false"),
        "{out:?}"
    );
    assert!(
        stdout.contains(
            "IRONSHELL_AUDIT_KEY is set, but the key it gives is shorter than 32 characters"
        ),
        "{out:?}"
    );
    assert!(
        !stdout.contains(short),
        "the key itself must not be echoed: {out:?}"
    );
}

#[test]
fn a_chain_key_of_the_minimum_length_is_used_without_a_warning() {
    let d = Dirs::new("okkey");
    let log = d.root.join("audit.log");
    std::fs::write(&log, "").unwrap();

    let key = "0123456789abcdef0123456789abcdef";
    let out = d.ish_keyed(&["-c", verify_call(&log).as_str()], Some(key));
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!stderr.contains("shorter than 32"), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!stdout.contains("shorter than 32"), "{out:?}");
    assert!(
        stdout.contains("\"keyed\": true") || stdout.contains("\"keyed\":true"),
        "{out:?}"
    );
}

#[test]
fn an_edited_unkeyed_entry_is_not_called_tampering() {
    let d = Dirs::new("unkeyedmismatch");
    let log = d.root.join("audit.log");
    // One entry whose stored hash cannot match its contents. No `mac` field, so
    // the chain is read as unkeyed.
    std::fs::write(
        &log,
        "{\"entry_hash\":\"00\",\"prev_hash\":\"\",\"seq\":1}\n",
    )
    .unwrap();

    let out = d.ish_keyed(&["-c", verify_call(&log).as_str()], None);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        stdout.contains(
            "entry_hash mismatch (the log was edited or corrupted; an unkeyed chain cannot tell which)"
        ),
        "{out:?}"
    );
    assert!(!stdout.contains("(tampered)"), "{out:?}");
}

#[test]
fn an_edited_keyed_entry_is_still_called_tampering() {
    let d = Dirs::new("keyedmismatch");
    let log = d.root.join("audit.log");
    std::fs::write(
        &log,
        "{\"entry_hash\":\"00\",\"mac\":\"hmac-sha256\",\"prev_hash\":\"\",\"seq\":1}\n",
    )
    .unwrap();

    let key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let out = d.ish_keyed(&["-c", verify_call(&log).as_str()], Some(key));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("entry_hash mismatch (tampered)"), "{out:?}");
}

/// Stdout and stderr together, so a test does not care which one carries an error.
fn both(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn an_undo_id_error_names_the_bad_value_without_ansi_codes() {
    let d = Dirs::new("ansi");
    let out = d.ish(&["-c", "undo({id: 0})"]);
    let text = both(&out);
    assert!(!text.contains('\u{1b}'), "{out:?}");
    assert!(text.contains("got 0"), "{out:?}");
}

#[test]
fn an_unknown_undo_id_says_journal_step_not_entry() {
    let d = Dirs::new("unknownid");
    let out = d.ish(&["-c", "undo({id: 6})"]);
    let text = both(&out);
    assert!(text.contains("no journal step with id 6"), "{out:?}");
    assert!(text.contains("journal steps"), "{out:?}");
    assert!(!text.contains("entrys"), "{out:?}");
    assert!(!text.contains("no journal entry"), "{out:?}");
    assert!(!text.contains('\u{1b}'), "{out:?}");
}

/// The stdout of `ish -c expr` in the session `d`, with whitespace removed, so
/// that pretty and compact printing of the same value compare equal.
fn probe(d: &Dirs, expr: &str) -> String {
    let out = d.ish(&["-c", expr]);
    assert!(out.status.success(), "{out:?}");
    String::from_utf8_lossy(&out.stdout)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Four writes in one session: p (id 1), q (2), p again (3), r (4).
fn four_writes(d: &Dirs) {
    for call in [
        r#"file_write("p.txt", "one\n")"#,
        r#"file_write("q.txt", "two\n")"#,
        r#"file_write("p.txt", "three\n")"#,
        r#"file_write("r.txt", "four\n")"#,
    ] {
        let out = d.ish(&["-c", call]);
        assert!(out.status.success(), "{out:?}");
    }
}

#[test]
fn an_undone_step_keeps_its_id_in_every_listing() {
    let d = Dirs::new("ids");
    four_writes(&d);
    assert_eq!(
        probe(&d, "journal().entries | map(fn(r) => r.seq)"),
        "[1,2,3,4]"
    );

    let undone = d.ish(&["-c", "undo({id: 2})"]);
    assert!(undone.status.success(), "{undone:?}");
    assert!(!d.workspace.join("q.txt").exists(), "{undone:?}");
    assert_eq!(
        probe(&d, "journal().entries | map(fn(r) => r.seq)"),
        "[1,3,4]"
    );
    // The undone step is listed under the id it was recorded with, not the
    // position it had in the undone list (which was 1).
    assert_eq!(
        probe(&d, "session_history().undone | map(fn(r) => r.seq)"),
        "[2]"
    );
    assert_eq!(
        probe(&d, "session_history().done | map(fn(r) => r.seq)"),
        "[1,3,4]"
    );
}

#[test]
fn the_undo_record_names_each_step_by_its_id() {
    let d = Dirs::new("undorecord");
    four_writes(&d);
    assert_eq!(
        probe(&d, "undo({id: 3}).steps | map(fn(r) => r.seq)"),
        "[3]"
    );
    assert_eq!(
        probe(&d, "journal().entries | map(fn(r) => r.seq)"),
        "[1,2,4]"
    );
}

#[test]
fn a_write_after_undoing_the_last_two_steps_gets_the_next_id_not_a_freed_one() {
    let d = Dirs::new("nextid");
    four_writes(&d);
    let undone = d.ish(&["-c", "undo(2)"]);
    assert!(undone.status.success(), "{undone:?}");
    assert_eq!(
        probe(&d, "journal().entries | map(fn(r) => r.seq)"),
        "[1,2]"
    );

    let write = d.ish(&["-c", r#"file_write("s.txt", "five\n")"#]);
    assert!(write.status.success(), "{write:?}");
    assert_eq!(
        probe(&d, "journal().entries | map(fn(r) => r.seq)"),
        "[1,2,5]"
    );
    // A new step ends the undone future, so nothing is left to re-apply.
    assert_eq!(
        probe(&d, "session_history().undone | map(fn(r) => r.seq)"),
        "[]"
    );
}

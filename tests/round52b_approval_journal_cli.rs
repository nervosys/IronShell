//! Round 52b: approval blast radius, approval token text, undo of a step done
//! twice, and CLI argument errors in agent mode.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round52b")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj")).expect("workspace dir");
    std::fs::create_dir_all(dir.join("session")).expect("session dir");
    dir
}

fn ish_agent(dir: &Path, code: &str, approve: Option<&str>) -> Output {
    let ws = dir.join("proj");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["--agent", "--workspace"])
        .arg(&ws)
        .args(["-c", code])
        .current_dir(&ws)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_APPROVE_ALL")
        .env_remove("IRONSHELL_APPROVE")
        .env_remove("IRONSHELL_WORKSPACE");
    if let Some(token) = approve {
        cmd.env("IRONSHELL_APPROVE", token);
    }
    cmd.output().expect("run ish")
}

fn first_error(out: &Output) -> serde_json::Value {
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().next().unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {err:?}"))
}

// ── B1: a recursive delete reports the size of the tree ──

#[test]
fn rmdir_recursive_blast_radius_counts_the_tree() {
    let dir = scratch("b1_rmdir");
    let d = dir.join("proj").join("d");
    std::fs::create_dir_all(d.join("a")).unwrap();
    std::fs::create_dir_all(d.join("b")).unwrap();
    for f in ["x1", "x2", "a/y1", "a/y2", "b/z1", "b/z2"] {
        std::fs::write(d.join(f), "x").unwrap();
    }
    let out = ish_agent(&dir, r#"rmdir("d", true)"#, None);
    let json = first_error(&out);
    assert_eq!(json["error"]["code"], "E_NEEDS_APPROVAL", "{json}");
    let radius = &json["error"]["approval"]["blast_radius"];
    assert_eq!(radius["files"], 6, "{json}");
    assert_eq!(radius["dirs"], 2, "{json}");
    assert!(d.exists(), "a refused delete removes nothing");
}

#[test]
fn the_token_does_not_change_with_the_tree() {
    let dir = scratch("b1_token");
    let d = dir.join("proj").join("d");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("one"), "x").unwrap();
    let first = first_error(&ish_agent(&dir, r#"rmdir("d", true)"#, None));
    std::fs::write(d.join("two"), "x").unwrap();
    let second = first_error(&ish_agent(&dir, r#"rmdir("d", true)"#, None));
    assert_eq!(
        first["error"]["approval"]["token"], second["error"]["approval"]["token"],
        "counts describe the action; the token binds action and target"
    );
}

// ── B2: the hint says what a token approves ──

#[test]
fn the_approval_hint_says_a_token_is_good_for_every_later_run() {
    let dir = scratch("b2_hint");
    std::fs::write(dir.join("proj").join("f.txt"), "x").unwrap();
    let json = first_error(&ish_agent(&dir, r#"rm("f.txt")"#, None));
    let hint = json["error"]["hint"].as_str().unwrap_or("");
    assert!(
        hint.contains("exact action on that exact target every time it is presented"),
        "{hint}"
    );
    assert!(
        hint.contains("until it is removed from IRONSHELL_APPROVE"),
        "{hint}"
    );
}

// ── B3: help("journal") names journal().entries ──

#[test]
fn journal_help_names_the_entries_field() {
    let dir = scratch("b3_help");
    let out = ish_agent(&dir, r#"help("journal")"#, None);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("journal().entries"), "{text}");
}

// ── B4: undoing a step twice says it was already undone ──

#[test]
fn undo_by_step_twice_says_already_undone() {
    let dir = scratch("b4_twice");
    let wrote = ish_agent(&dir, r#"file.write("a.txt", "x")"#, None);
    assert!(wrote.status.success(), "{wrote:?}");
    let first = ish_agent(&dir, "undo({step: 1})", None);
    assert!(first.status.success(), "{first:?}");
    let second = first_error(&ish_agent(&dir, "undo({step: 1})", None));
    assert_eq!(second["error"]["code"], "E_BAD_STATE", "{second}");
    assert_eq!(second["error"]["retryable"], false, "{second}");
    let message = second["error"]["message"].as_str().unwrap_or("");
    assert!(message.contains("step 1 was already undone"), "{message}");
}

#[test]
fn undo_of_a_step_that_never_was_is_still_not_found() {
    let dir = scratch("b4_never");
    let json = first_error(&ish_agent(&dir, "undo({step: 7})", None));
    assert_eq!(json["error"]["code"], "E_NOT_FOUND", "{json}");
    let message = json["error"]["message"].as_str().unwrap_or("");
    assert!(message.contains("no journal step 7"), "{message}");
}

// ── B5: CLI argument errors are JSON in agent and deterministic mode ──

fn ish_raw(args: &[&str], mode_env: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(args)
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_DETERMINISTIC");
    if mode_env {
        cmd.env("IRONSHELL_MODE", "agent");
    }
    cmd.output().expect("run ish")
}

#[test]
fn a_bad_flag_value_is_json_under_agent() {
    for (args, env) in [
        (vec!["--agent", "--memory-limit", "abc", "-c", "1"], false),
        (
            vec!["--deterministic", "--timeout", "abc", "-c", "1"],
            false,
        ),
        (vec!["--memory-limit", "abc", "-c", "1"], true),
    ] {
        let out = ish_raw(&args, env);
        assert_eq!(out.status.code(), Some(64), "{args:?} {out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        let json: serde_json::Value = serde_json::from_str(err.lines().next().unwrap_or(""))
            .unwrap_or_else(|e| panic!("not JSON ({e}): {err:?}"));
        assert_eq!(json["error"]["code"], "E_BAD_ARG", "{json}");
        assert!(
            json["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("--memory-limit")
                || json["error"]["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("--timeout"),
            "{json}"
        );
    }
}

#[test]
fn a_bad_flag_value_stays_clap_text_for_a_person() {
    let out = ish_raw(&["--memory-limit", "abc", "-c", "1"], false);
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with("error:"), "{err:?}");
}

// ── B6: --timeout 0 means no deadline, and --help says so ──

#[test]
fn timeout_zero_is_no_deadline_and_help_says_so() {
    let out = ish_raw(&["--timeout", "0", "-c", "1 + 1"], false);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains('2'),
        "{out:?}"
    );
    let help = ish_raw(&["--help"], false);
    let text = String::from_utf8_lossy(&help.stdout);
    assert!(text.contains("0 for none"), "{text}");
}

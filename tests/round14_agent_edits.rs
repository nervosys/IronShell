//! Agent-mode approval and edit defects (round 14). Each test drives the real
//! binary in agent mode, in a scratch workspace, with its own session directory,
//! as an agent harness would.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh scratch directory: `proj` is the workspace, `session` holds the
/// journal, so no test shares undo history with another or with the repo.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round14_agent_edits")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj")).expect("workspace dir");
    std::fs::create_dir_all(dir.join("session")).expect("session dir");
    dir
}

/// Run `code` in agent mode with `dir/proj` as the workspace, optionally with
/// `approve` as the operator's `IRONSHELL_APPROVE` list.
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

/// The structured refusal on the first stderr line.
fn first_error(out: &Output) -> serde_json::Value {
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().next().unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {err:?}"))
}

/// The approval token a refused call carries in its `approval` payload.
fn token_from_refusal(out: &Output) -> String {
    let json = first_error(out);
    assert_eq!(json["error"]["code"], "E_NEEDS_APPROVAL", "{json}");
    json["error"]["approval"]["token"]
        .as_str()
        .expect("the refusal carries a token")
        .to_string()
}

const TWO_WRITES: &str = r#"apply([{op: "write", path: "a.txt", content: "one"}, {op: "write", path: "b.txt", content: "two"}])"#;

// ── Item 2: an unapproved apply is an error, not a success-shaped record ──

#[test]
fn an_unapproved_apply_exits_75_with_the_plan_token() {
    let dir = scratch("apply_refused");
    let out = ish_agent(&dir, TWO_WRITES, None);
    assert_eq!(
        out.status.code(),
        Some(75),
        "a refused apply must exit with the approval status: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = first_error(&out);
    assert_eq!(json["error"]["code"], "E_NEEDS_APPROVAL", "{json}");
    assert_eq!(json["error"]["approval"]["builtin"], "apply", "{json}");
    let token = token_from_refusal(&out);
    assert!(token.starts_with("apl_"), "{token}");
    let hint = json["error"]["hint"].as_str().unwrap_or("");
    assert!(
        hint.contains(&format!("IRONSHELL_APPROVE={token}")),
        "the hint names the out-of-band route: {hint}"
    );
    assert!(
        !hint.contains("approve(\""),
        "an agent cannot call approve() on its own plan, so the hint must not offer it: {hint}"
    );
    let ws = dir.join("proj");
    assert!(!ws.join("a.txt").exists() && !ws.join("b.txt").exists());
}

// ── Item 3: each file an apply writes is a journal step, so undo reverts it ──

#[test]
fn undo_reverts_every_file_an_approved_apply_wrote() {
    let dir = scratch("apply_undo");
    let token = token_from_refusal(&ish_agent(&dir, TWO_WRITES, None));

    let applied = ish_agent(&dir, TWO_WRITES, Some(&token));
    assert!(
        applied.status.success(),
        "the approved apply must run: {}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let ws = dir.join("proj");
    assert_eq!(std::fs::read_to_string(ws.join("a.txt")).unwrap(), "one");
    assert_eq!(std::fs::read_to_string(ws.join("b.txt")).unwrap(), "two");

    // One journal step per file: two steps, so undo(2) reverts both. Neither
    // file existed before the plan, so undoing removes them.
    let undone = ish_agent(&dir, "undo(2)", None);
    assert!(
        undone.status.success(),
        "undo must run: {}",
        String::from_utf8_lossy(&undone.stderr)
    );
    assert!(
        !ws.join("a.txt").exists() && !ws.join("b.txt").exists(),
        "undo must revert the files the apply wrote"
    );
}

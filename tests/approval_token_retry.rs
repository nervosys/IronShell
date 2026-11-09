//! The approval token from a refusal approves the retry of the same call.
//!
//! `delete_lines` is an alias of `file_delete_lines`. The dispatcher's guard
//! used to judge the call under the alias name (irreversible, unresolved path),
//! and the body then judged it again under `file_delete_lines` (reversible,
//! absolute path), so the token named in the refusal never matched the retry.
//! These drive the real binary in agent mode, in a scratch workspace.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("approval_token_retry")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    std::fs::create_dir_all(dir.join("proj")).expect("project dir");
    std::fs::write(dir.join("proj").join("README.md"), "one\ntwo\nthree\n").expect("seed file");
    dir
}

fn ish_agent(dir: &Path, approve: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["--agent", "-c", "delete_lines(\"proj/README.md\", 2)"])
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_APPROVE_ALL")
        .env_remove("IRONSHELL_APPROVE");
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

#[test]
fn the_token_from_the_refusal_approves_the_retry() {
    let dir = scratch("retry");

    let first = ish_agent(&dir, None);
    assert!(!first.status.success(), "the first call must be refused");
    let json = first_error(&first);
    assert_eq!(json["error"]["code"], "E_NEEDS_APPROVAL", "{json}");
    assert_eq!(json["error"]["builtin"], "file_delete_lines", "{json}");
    let approval = &json["error"]["approval"];
    assert_eq!(approval["builtin"], "file_delete_lines", "{json}");
    assert_eq!(approval["reversible"], true, "{json}");
    let target = approval["targets"][0].as_str().unwrap_or("");
    assert!(
        Path::new(target).is_absolute(),
        "the approval names a relative path: {target:?}"
    );
    let token = approval["token"].as_str().expect("a token").to_string();
    assert!(token.starts_with("apv_"), "{token}");
    assert_eq!(
        std::fs::read_to_string(dir.join("proj").join("README.md")).unwrap(),
        "one\ntwo\nthree\n",
        "a refused call must not delete"
    );

    let retry = ish_agent(&dir, Some(&token));
    assert!(
        retry.status.success(),
        "the retry with the token was refused: {}",
        String::from_utf8_lossy(&retry.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("proj").join("README.md")).unwrap(),
        "one\nthree\n"
    );
}

#[test]
fn a_token_for_another_call_does_not_approve_this_one() {
    let dir = scratch("other_token");
    let out = ish_agent(&dir, Some("apv_0000000000000000"));
    let json = first_error(&out);
    assert_eq!(json["error"]["code"], "E_NEEDS_APPROVAL", "{json}");
    let hint = json["error"]["hint"].as_str().unwrap_or("");
    assert!(
        hint.contains("none of its tokens approve this call"),
        "{hint}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("proj").join("README.md")).unwrap(),
        "one\ntwo\nthree\n"
    );
}

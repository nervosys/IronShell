//! An alias judged under its own name never matches the token its body demands.
//!
//! `mv` is an alias of `file_move`, one implementation that guards itself. The
//! dispatcher used to judge `mv` as `mv`, so the refusal named a token for
//! builtin `mv` with the source alone, and the retry was refused because the
//! body judged `file_move`. The dispatcher now judges an alias under the name
//! its body guards under. An outside target is refused by the jail before any
//! token is issued. These drive the real binary in agent mode, in a scratch
//! workspace.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("approval_alias_token")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    std::fs::write(dir.join("a.txt"), "payload\n").expect("seed file");
    dir
}

fn ish_agent(dir: &Path, code: &str, approve: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["--agent", "-c", code])
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
fn the_token_from_an_mv_refusal_approves_the_retry() {
    let dir = scratch("mv_retry");
    let code = "mv(\"a.txt\", \"b.txt\")";

    let first = ish_agent(&dir, code, None);
    assert!(!first.status.success(), "the first mv must be refused");
    let json = first_error(&first);
    assert_eq!(json["error"]["code"], "E_NEEDS_APPROVAL", "{json}");
    assert_eq!(json["error"]["approval"]["builtin"], "file_move", "{json}");
    let token = json["error"]["approval"]["token"]
        .as_str()
        .expect("a token")
        .to_string();
    assert!(dir.join("a.txt").exists(), "a refused mv must not move");

    let retry = ish_agent(&dir, code, Some(&token));
    assert!(
        retry.status.success(),
        "the retry with the token was refused: {}",
        String::from_utf8_lossy(&retry.stderr)
    );
    assert!(!dir.join("a.txt").exists());
    assert!(dir.join("b.txt").exists());
}

#[test]
fn an_mv_to_a_relative_path_outside_the_workspace_is_refused_without_a_token() {
    let dir = scratch("mv_outside");
    let outside = dir.parent().unwrap().join("ae_alias_outside_x.txt");
    let _ = std::fs::remove_file(&outside);

    let out = ish_agent(&dir, "mv(\"a.txt\", \"../ae_alias_outside_x.txt\")", None);
    let json = first_error(&out);
    assert_eq!(json["error"]["code"], "E_OUTSIDE_WORKSPACE", "{json}");
    assert!(
        json["error"]["approval"].is_null(),
        "an outside target must never yield a token: {json}"
    );
    assert!(dir.join("a.txt").exists(), "the source must not move");
    assert!(
        !outside.exists(),
        "nothing may be written outside the workspace"
    );
}

#[test]
fn a_cp_to_a_relative_path_outside_the_workspace_is_refused() {
    let dir = scratch("cp_outside");
    let outside = dir.parent().unwrap().join("ae_alias_outside_y.txt");
    let _ = std::fs::remove_file(&outside);

    let out = ish_agent(&dir, "cp(\"a.txt\", \"../ae_alias_outside_y.txt\")", None);
    let json = first_error(&out);
    assert_eq!(json["error"]["code"], "E_OUTSIDE_WORKSPACE", "{json}");
    assert!(json["error"]["approval"].is_null(), "{json}");
    assert!(
        !outside.exists(),
        "nothing may be written outside the workspace"
    );
}

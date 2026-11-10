//! A syntax error in a script `ish -b` would run is reported as a syntax error
//! (`E_PARSE`, exit 2), naming the token and where it is, not as a construct
//! outside the subset (`E_POLICY_DENY`, exit 77). A reserved word where no
//! construct of its kind is open, or a construct that is never closed, is one.

use std::process::{Command, Output};

fn run(tag: &str, script: &str, agent: bool) -> Output {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("posix_syntax_errors")
        .join(tag);
    let _ = std::fs::remove_dir_all(&root);
    let session = root.join("session");
    std::fs::create_dir_all(&session).expect("scratch dir");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["-b", "-c", script])
        .current_dir(&root)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", &session)
        .env_remove("IRONSHELL_ALLOW_SH")
        .env_remove("IRONSHELL_MODE");
    if agent {
        cmd.env("IRONSHELL_MODE", "agent");
    }
    cmd.output().expect("run ish")
}

fn assert_syntax_error(out: &Output, message: &str, hint: &str) {
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(out.stdout.is_empty(), "ran something: {:?}", out.stdout);
    assert!(err.contains(&format!("error[E_PARSE]: {message}")), "{err}");
    assert!(err.contains(hint), "{err}");
    assert!(!err.contains("E_POLICY_DENY"), "{err}");
}

#[test]
fn done_after_an_if_names_the_if_it_does_not_close() {
    let out = run(
        "done_after_if",
        "echo start\nif true; then\n  echo a\ndone\n",
        false,
    );
    assert_syntax_error(
        &out,
        "syntax error near unexpected token `done' at line 4, column 1",
        "the `if` at line 2 is closed by `fi`",
    );
}

#[test]
fn a_stray_fi_is_a_syntax_error() {
    let out = run("stray_fi", "echo a\nfi\n", false);
    assert_syntax_error(
        &out,
        "syntax error near unexpected token `fi' at line 2, column 1",
        "there is no open",
    );
}

#[test]
fn an_if_that_is_never_closed_is_a_syntax_error_at_the_end() {
    let out = run("unclosed_if", "if true; then\necho a\n", false);
    assert_syntax_error(
        &out,
        "syntax error: unexpected end of file",
        "the `if` at line 1 needs a closing `fi`",
    );
}

#[test]
fn a_while_without_done_is_a_syntax_error() {
    let out = run(
        "unclosed_while",
        "x=0\nwhile [ $x -lt 2 ]; do\nx=1\n",
        false,
    );
    assert_syntax_error(
        &out,
        "syntax error: unexpected end of file",
        "expected `done` for the `while` at line 2",
    );
}

#[test]
fn a_syntax_error_is_structured_json_in_agent_mode() {
    let out = run("agent_json", "echo one\nfi\n", true);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    let line = err.lines().next().unwrap_or("");
    let json: serde_json::Value =
        serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {err:?}"));
    assert_eq!(json["error"]["code"], "E_PARSE");
    assert_eq!(json["error"]["line"], 2);
    assert_eq!(json["error"]["column"], 1);
}

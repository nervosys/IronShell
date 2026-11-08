//! A script outside the in-process POSIX subset is refused before any of it
//! runs, and the refusal names the construct and where it starts.
//!
//! It used to say "sh() is disabled for security", although the script never
//! called sh(): the transpiler turned the unsupported lines into sh() calls and
//! the first of them refused, after the lines above it had run. Without
//! IRONSHELL_ALLOW_SH the refusal is now made up front, with exit 77
//! (E_POLICY_DENY, EX_NOPERM), and it runs nothing. With IRONSHELL_ALLOW_SH the
//! transpiler path is unchanged.

use std::process::{Command, Output};

fn run(tag: &str, script: &str, agent: bool) -> Output {
    let root = std::env::temp_dir().join(format!("ae-posix-refusal-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("session")).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["-b", "-c", script])
        .current_dir(&root)
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_ALLOW_SH")
        .env("IRONSHELL_SESSION_DIR", root.join("session"));
    if agent {
        cmd.env("IRONSHELL_MODE", "agent");
    }
    let out = cmd.output().expect("run ish");
    let _ = std::fs::remove_dir_all(&root);
    out
}

fn refused(tag: &str, script: &str, want: &str) {
    let out = run(tag, script, false);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(77), "{script}: stderr {err:?}");
    assert!(
        out.stdout.is_empty(),
        "{script}: ran something: {:?}",
        out.stdout
    );
    assert!(
        err.contains(&format!("error[E_POLICY_DENY]: {want} is not in the in-process POSIX subset, so nothing in this script was run")),
        "{script}: stderr {err:?}"
    );
    assert!(err.contains("IRONSHELL_ALLOW_SH=true"), "{err:?}");
    assert!(!err.contains("sh() is disabled"), "{err:?}");
}

#[test]
fn a_function_is_named_with_its_line() {
    refused(
        "fn",
        "echo before\nf() { echo hi; }\nf\n",
        "function definition at line 2, column 2",
    );
}

#[test]
fn case_is_named_with_its_line() {
    refused(
        "case",
        "echo before\ncase x in x) echo y;; esac\n",
        "case ... esac at line 2, column 1",
    );
}

#[test]
fn arithmetic_expansion_is_named_on_a_later_line() {
    refused(
        "arith",
        "echo one\necho two\necho $((1+2))\n",
        "arithmetic expansion $((...)) at line 3, column 6",
    );
}

#[test]
fn a_here_document_is_named_with_its_line() {
    refused(
        "heredoc",
        "echo before\ncat <<EOF\nhi\nEOF\n",
        "here-document (<<) at line 2, column 5",
    );
}

#[test]
fn agent_mode_refuses_with_the_json_form() {
    let out = run("agent", "echo one\necho $((1+2))\n", true);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(77));
    assert!(out.stdout.is_empty());
    let line = err.lines().next().unwrap_or("");
    let json: serde_json::Value =
        serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {err:?}"));
    assert_eq!(json["error"]["code"], "E_POLICY_DENY");
    assert_eq!(json["error"]["builtin"], "posix");
    assert_eq!(
        json["error"]["message"],
        "arithmetic expansion $((...)) at line 2, column 6 is not in the in-process POSIX subset, so nothing in this script was run"
    );
}

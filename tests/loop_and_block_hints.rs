//! IronShell has no loop statements, and a braced block holds one expression.
//! Each wrong form gets an E_PARSE that says so, run through the real binary.
//! Before this, `while (i < 3) { i = i + 1 }` and `for x in [1,2,3] { x * 2 }`
//! said "expected ':' after key", `for ... do` said "unknown builtin: for", and
//! `{ a = 1 \n b = 2 }` said "expected ':' after key" with no reason.
//! Names `for`, `while` and `loop` still work as names. A script echoes its last
//! value, so each one ends in a bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_loophint_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("loop.ish");
    std::fs::write(&script, source).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg(&script)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

const LOOP_HINT: &str =
    "IronShell has no loop statements; iterate with map / where / reduce / each";

fn assert_loop_refused(err: &str, code: i32, keyword: &str) {
    assert_ne!(code, 0, "ran: {err}");
    assert!(err.contains("E_PARSE"), "not E_PARSE: {err}");
    assert!(
        err.contains(&format!("`{keyword}` starts a loop")),
        "no loop message: {err}"
    );
    assert!(err.contains(LOOP_HINT), "no loop hint: {err}");
    assert!(
        !err.contains("unknown builtin"),
        "still a builtin error: {err}"
    );
}

#[test]
fn a_while_loop_is_refused_with_the_iteration_hint() {
    let (_, err, code) = run_script("while", "i = 0\nwhile (i < 3) { i = i + 1 }\ni\n");
    assert_loop_refused(&err, code, "while");
}

#[test]
fn a_for_loop_is_refused_with_the_iteration_hint() {
    let (_, err, code) = run_script("for", "for x in [1, 2, 3] { x * 2 }\n");
    assert_loop_refused(&err, code, "for");
}

#[test]
fn a_for_do_loop_is_refused_not_called_a_builtin() {
    let (_, err, code) = run_script("fordo", "for x in [1, 2, 3] do x * 2\n");
    assert_loop_refused(&err, code, "for");
}

#[test]
fn a_loop_keyword_is_refused() {
    let (_, err, code) = run_script("loop", "loop { 1 }\n");
    assert_loop_refused(&err, code, "loop");
}

#[test]
fn a_block_of_two_statements_says_a_block_holds_one_expression() {
    let (_, err, code) = run_script("block", "{ a = 1\nb = 2 }\n");
    assert_ne!(code, 0);
    assert!(err.contains("E_PARSE"), "{err}");
    assert!(
        err.contains("holds one expression, not statements"),
        "no block hint: {err}"
    );
}

#[test]
fn a_let_inside_braces_gets_the_block_hint() {
    let (_, err, code) = run_script("letblock", "{ let x = 1\nx }\n");
    assert_ne!(code, 0);
    assert!(
        err.contains("holds one expression, not statements"),
        "no block hint: {err}"
    );
}

#[test]
fn a_record_literal_still_parses() {
    let (out, err, code) = run_script("record", "r = {k: 1, for: 2}\nr.k + r.for\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "3");
}

#[test]
fn for_and_while_still_work_as_names() {
    let (out, err, code) = run_script("names", "for = 1\nfor + 1\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "2");
}

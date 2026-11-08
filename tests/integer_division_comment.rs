//! `//` straight after an operand, followed by an operand, is an error rather
//! than a comment, run through the real binary.
//!
//! Reproduced on 14.0.0: `7 // 2` printed 7 with exit 0, `5 // 999` printed 5,
//! and `(7 // 2) * 10` was E_PARSE "expected ')'", because the lexer read `//`
//! as a comment and dropped the rest of the line. IronShell documents `//` as a
//! comment (docs/specs/SPEC.md section 2.0), so that spelling stays a comment
//! where it begins a line, follows an operator, or ends a statement. Only the
//! operand-then-operand form is refused, with a hint that names the division
//! to write instead. A script echoes its last value, so each one ends in a bare
//! expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_intdiv_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("comment.ish");
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

fn assert_prints(tag: &str, source: &str, expected: &str) {
    let (out, err, code) = run_script(tag, source);
    assert_eq!(code, 0, "source {source:?} failed: {err}");
    assert_eq!(out, expected, "source {source:?}");
}

fn assert_refused(tag: &str, source: &str) {
    let (out, err, code) = run_script(tag, source);
    assert_ne!(code, 0, "source {source:?} ran; stdout {out:?}");
    assert!(err.contains("E_PARSE"), "source {source:?}: {err}");
    assert!(
        err.contains("`//` starts a comment in IronShell; there is no integer-division operator: use math.floor(a / b)"),
        "source {source:?} lacks the division hint: {err}"
    );
    assert!(
        out.is_empty(),
        "source {source:?} printed {out:?} before refusing"
    );
}

#[test]
fn a_double_slash_after_a_number_is_refused_not_dropped() {
    assert_refused("num", "7 // 2\n");
}

#[test]
fn a_double_slash_inside_parentheses_is_refused() {
    assert_refused("paren", "(7 // 2) * 10\n");
}

#[test]
fn a_double_slash_with_no_space_before_the_operand_is_refused() {
    assert_refused("tight", "5 //999\n");
}

#[test]
fn a_double_slash_after_a_variable_is_refused() {
    assert_refused("var", "n = 7\nn // 2\n");
}

#[test]
fn a_double_slash_after_a_closing_bracket_is_refused() {
    assert_refused("bracket", "a = [7]\na[0] // 2\n");
}

#[test]
fn a_double_slash_at_the_start_of_a_line_is_still_a_comment() {
    assert_prints("line", "x = 5\n// 2 is not used here\nx\n", "5");
}

#[test]
fn a_double_slash_after_an_operator_is_still_a_comment() {
    assert_prints("op", "x = 5 +\n// 999\n1\n", "6");
}

#[test]
fn a_double_slash_after_a_statement_with_nothing_after_it_is_still_a_comment() {
    assert_prints("trail", "x = 5 //\nx\n", "5");
}

#[test]
fn a_hash_comment_after_an_operand_is_still_a_comment() {
    assert_prints("hash", "x = 5 # 2 is not used\nx\n", "5");
}

#[test]
fn single_slash_division_is_unchanged() {
    assert_prints("div", "7 / 2\n", "3.5");
}

#[test]
fn prose_after_a_number_is_a_comment() {
    assert_prints(
        "prose_num",
        "x = 5 // note words
x
",
        "5",
    );
}

#[test]
fn a_trailing_comment_after_a_call_is_a_comment() {
    assert_prints(
        "prose_call",
        "x = [1, 2]
y = len(x) // TODO fix this
y
",
        "2",
    );
}

#[test]
fn a_name_divided_by_a_name_is_refused() {
    assert_refused(
        "names",
        "a = 7
b = 2
a // b
",
    );
}

#[test]
fn a_call_divided_by_a_number_is_refused() {
    assert_refused(
        "call",
        "x = [1, 2]
len(x) // 2
",
    );
}

#[test]
fn a_name_divided_by_a_parenthesised_sum_is_refused() {
    assert_refused(
        "paren_sum",
        "n = 7
k = 2
n // (k+1)
",
    );
}

//! A `-` written directly after an operand is binary minus, run through the
//! real binary.
//!
//! Reproduced on 14.0.0: `5-1` printed `-1` (the lexer read `-1` as one
//! negative literal after `5`), `x = 5` then `x-1` was E_BAD_ARG "expected a
//! function in `x`, got Int" (the word call `x(-1)`), `(5-1)` and `a[x-1]`
//! were E_PARSE. The forms that must stay as they were (a literal at statement
//! start, after `=`, after a comma, a call argument, the word call `f -1`) are
//! pinned here too. A script echoes its last value, so each one ends in a bare
//! expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_minus_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let script = dir.join("minus.ish");
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

#[test]
fn an_unspaced_minus_after_a_number_subtracts() {
    assert_prints("num", "5-1\n", "4");
}

#[test]
fn an_unspaced_minus_after_a_variable_subtracts_and_is_not_a_call() {
    assert_prints("var", "x = 5\nx-1\n", "4");
}

#[test]
fn an_unspaced_minus_inside_parentheses_subtracts() {
    assert_prints("paren", "(5-1)\n", "4");
}

#[test]
fn an_unspaced_minus_inside_an_index_subtracts() {
    assert_prints("index", "a = [1, 2, 3]\nx = 2\na[x-1]\n", "2");
}

#[test]
fn an_unspaced_minus_after_a_call_subtracts() {
    assert_prints("call", "a = [1, 2, 3]\nlen(a)-1\n", "2");
}

#[test]
fn an_unspaced_minus_inside_a_lambda_index_subtracts() {
    assert_prints(
        "lambda",
        "a = [1, 2, 3]\nmap(a, fn(x) => a[x-1])\n",
        "[1, 2, 3]",
    );
}

#[test]
fn a_negative_literal_at_statement_start_is_still_a_literal() {
    assert_prints("stmt", "-1\n", "-1");
}

#[test]
fn a_negative_literal_after_assignment_is_still_a_literal() {
    assert_prints("assign", "x = -1\nx\n", "-1");
}

#[test]
fn a_negative_literal_in_a_call_argument_is_still_a_literal() {
    assert_prints("callarg", "f = fn(a) => a\nf(-1)\n", "-1");
}

#[test]
fn a_negative_literal_in_an_array_is_still_a_literal() {
    assert_prints("array", "[1, -1]\n", "[1, -1]");
}

#[test]
fn a_spaced_minus_before_a_negative_literal_is_still_subtraction() {
    assert_prints("minusminus", "5 - -1\n", "6");
}

#[test]
fn a_spaced_minus_is_subtraction() {
    assert_prints("spaced", "x = 3\nx - 1\n", "2");
}

#[test]
fn a_word_call_with_a_space_before_a_negative_argument_is_still_a_call() {
    assert_prints("wordcall", "f = fn(a) => a\nf -1\n", "-1");
}

#[test]
fn a_negative_literal_after_a_multiplication_operator_is_still_a_literal() {
    assert_prints("mul", "2*-3\n", "-6");
}

#[test]
fn a_negative_index_is_still_a_literal() {
    assert_prints("negindex", "a = [4, 5]\na[-1]\n", "5");
}

#[test]
fn a_unary_minus_on_a_variable_is_still_negation() {
    // `-x` on its own line after `x = 3` is subtraction in this grammar,
    // before and after this change; the `;` keeps this test about negation.
    assert_prints("neg", "x = 3; -x\n", "-3");
}

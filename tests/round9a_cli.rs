//! Round 9, agent A, through the binary: a value that a print already wrote is
//! not echoed again, and a failing statement names its line.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A scratch directory of this test's own.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_r9a_cli_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn ish(args: &[&str], dir: &PathBuf, agent: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(args)
        .env("IRONSHELL_SESSION_DIR", dir.join("state"))
        .env("IRONSHELL_WORKSPACE", dir)
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_JOURNAL");
    if agent {
        cmd.env("IRONSHELL_MODE", "agent");
    }
    cmd.output().expect("ish runs")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// ---- 3. A print's own value is not echoed a second time ----

#[test]
fn a_print_inside_an_if_block_is_echoed_once() {
    let dir = scratch("print_if");
    let out = ish(&["-c", "if true { print(1) }"], &dir, false);
    assert_eq!(stdout(&out), "1\n");
}

#[test]
fn a_print_inside_a_try_body_is_echoed_once() {
    let dir = scratch("print_try");
    let out = ish(&["-c", "try { print(3) } catch e { 0 }"], &dir, false);
    assert_eq!(stdout(&out), "3\n");
}

#[test]
fn a_lambda_that_prints_and_is_called_last_is_echoed_once() {
    let dir = scratch("print_lambda");
    let out = ish(&["-c", "f = fn(x) => print(x)\nf(3)"], &dir, false);
    assert_eq!(stdout(&out), "3\n");
}

#[test]
fn a_final_value_that_no_print_wrote_is_still_echoed() {
    let dir = scratch("echo_if");
    let out = ish(&["-c", "if true { 5 }"], &dir, false);
    assert_eq!(stdout(&out), "5\n");
}

#[test]
fn an_earlier_print_does_not_suppress_the_final_value() {
    let dir = scratch("echo_after_print");
    let out = ish(&["-c", "print(\"a\"); 5 + 1"], &dir, false);
    assert_eq!(stdout(&out), "a\n6\n");
}

// ---- 7. A runtime error names the line of its statement ----

/// A 30-line script whose line 15 opens a missing file.
fn script_with_missing_file(dir: &std::path::Path) -> PathBuf {
    let mut lines: Vec<String> = (1..=30).map(|n| format!("x_{n} = {n}")).collect();
    lines[14] = "cfg = open(\"missing_r9a.yaml\")".to_string();
    let path = dir.join("script.ish");
    std::fs::write(&path, lines.join("\n") + "\n").expect("script");
    path
}

#[test]
fn a_runtime_error_in_a_script_names_its_line() {
    let dir = scratch("line_text");
    let path = script_with_missing_file(&dir);
    let out = ish(&[path.to_str().unwrap()], &dir, false);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(err.contains("at line 15"), "{err}");
}

#[test]
fn a_runtime_error_in_agent_mode_carries_the_line_in_its_json() {
    let dir = scratch("line_json");
    let path = script_with_missing_file(&dir);
    let out = ish(&[path.to_str().unwrap()], &dir, true);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("\"line\":15") || err.contains("at line 15"),
        "{err}"
    );
}

#[test]
fn an_unclosed_paren_names_the_line_it_was_opened_on() {
    let dir = scratch("unclosed_paren");
    let mut lines: Vec<String> = (1..=30).map(|n| format!("x_{n} = {n}")).collect();
    lines[14] = "z = (1 + ".to_string();
    let path = dir.join("unclosed.ish");
    std::fs::write(&path, lines.join("\n") + "\n").expect("script");
    let out = ish(&[path.to_str().unwrap()], &dir, false);
    assert_eq!(out.status.code(), Some(2));
    let err = stderr(&out);
    assert!(err.contains("'(' opened at line 15 is not closed"), "{err}");
}

//! Round 8 fixes, one test per reported behaviour.
//!
//! In-process where the behaviour is a value or an error (`eval_program`), and
//! through the real binary where it is what reaches stdout or stderr. Scratch
//! files go under the system temp directory.

use iron_shell::value::Value;
use std::path::PathBuf;
use std::process::Command;

fn run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn run_ok(src: &str) -> Value {
    match run(src) {
        Ok(v) => v,
        Err(e) => panic!("`{src}` failed: {e}"),
    }
}

fn run_err(src: &str) -> String {
    match run(src) {
        Ok(v) => panic!("`{src}` should fail, got {v:?}"),
        Err(e) => e.to_string(),
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_round8a_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `ish -c code` and returns (stdout, stderr, exit code).
fn ish_c(tag: &str, code: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", code])
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

// 1. A final print(...) is not echoed a second time; echo still is.

#[test]
fn a_final_print_is_written_once() {
    let (out, err, code) = ish_c("print_once", "print(\"x\")");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "x", "printed twice or not at all");
}

#[test]
fn a_final_echo_is_still_shown() {
    let (out, err, code) = ish_c("echo_shown", "echo(\"x\")");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "x");
}

// 2. error() refuses extra fields named like a standard error key.

#[test]
fn error_refuses_a_reserved_extra_field() {
    let msg = run_err("error(\"s\", {code: 7})");
    assert!(msg.contains("E_BAD_ARG"), "{msg}");
    assert!(msg.contains("`code`"), "reserved key not named: {msg}");
    assert!(
        msg.contains("rename the field, e.g. {user_code: 7}"),
        "{msg}"
    );
}

#[test]
fn error_keeps_an_ordinary_extra_field() {
    let msg = run_err("error(\"s\", {user_code: 7})");
    assert!(msg.contains("E_USER"), "{msg}");
    assert!(msg.contains("\"user_code\":7"), "field dropped: {msg}");
}

// 3. An uncaught error prints its extra fields after the hint.

#[test]
fn an_uncaught_error_prints_its_extra_fields() {
    let (_, err, code) = ish_c("extra_fields", "error(\"too many\", {qty: 3})");
    assert_ne!(code, 0);
    assert!(err.contains("error[E_USER]: too many"), "{err}");
    assert!(
        err.contains("  qty: 3"),
        "extra field missing from report: {err}"
    );
}

// 4. A method call on a non-Record receiver names the working call.

#[test]
fn a_method_call_on_a_string_names_the_builtin_call() {
    let msg = run_err("l = \"abet\"\nl.contains(\"bet\")");
    assert!(msg.contains("a String has no methods"), "{msg}");
    assert!(msg.contains("contains(l, "), "{msg}");
    assert!(msg.contains("l | contains("), "{msg}");
}

#[test]
fn a_method_call_on_a_record_module_still_runs() {
    assert_eq!(run_ok("str.upper(\"abc\")"), Value::Str("ABC".into()));
}

// 5. A wrong-type argument names the builtin's declared types, not the
//    generic hint.

#[test]
fn a_bad_argument_hint_comes_from_the_signature() {
    let msg = run_err("len(5)");
    assert!(
        !msg.contains("check this builtin's required argument count"),
        "{msg}"
    );
    assert!(msg.contains("Number of elements in an array"), "{msg}");
}

// 6. A lambda body that assigns gets the lambda hint.

#[test]
fn a_lambda_body_with_assignment_gets_the_lambda_hint() {
    let err = iron_shell::parser::parse_program("f = fn(x) => y = x * 2")
        .expect_err("assignment in a lambda body must not parse");
    let hint = err
        .downcast_ref::<iron_shell::parser::ParseError>()
        .map(|p| p.hint.clone())
        .unwrap_or_default();
    assert!(
        hint.contains("a lambda body is one expression: fn(x) => x * 2"),
        "hint: {hint}"
    );
}

// 7. Indexing an empty args() names how to pass arguments.

#[test]
fn indexing_empty_args_names_how_to_pass_one() {
    let msg = run_err("args()[0]");
    assert!(msg.contains("index 0 is out of range"), "{msg}");
    assert!(
        msg.contains("the script was run with no arguments: ish script.ish <arg>"),
        "{msg}"
    );
}

#[test]
fn indexing_an_empty_array_says_to_check_its_length() {
    let msg = run_err("[][0]");
    assert!(
        msg.contains("the Array is empty; check len(x) > 0 first"),
        "{msg}"
    );
}

// 8. `|.a.b` reads a field of a piped Record; other subjects are named as the
//    projection, not as map.

#[test]
fn a_projection_stage_reads_a_chained_field_of_a_record() {
    assert_eq!(run_ok("[{a: {b: 5}}] | first | .a.b"), Value::Int(5));
}

#[test]
fn a_projection_stage_over_an_array_still_maps() {
    assert_eq!(run_ok("[{a: 1}, {a: 2}] | .a"), run_ok("[1, 2]"));
}

#[test]
fn a_projection_stage_on_a_non_collection_is_not_called_map() {
    let msg = run_err("5 | .a");
    assert!(msg.contains(".a: expected an Array or a Record"), "{msg}");
    assert!(!msg.contains("map"), "{msg}");
}

// 9. A member a known module lacks is an unknown builtin, named as such.

#[test]
fn a_missing_module_member_is_an_unknown_builtin() {
    let msg = run_err("str.pad_right(\"ab\", 6)");
    assert!(msg.contains("E_UNKNOWN_BUILTIN"), "{msg}");
    assert!(
        msg.contains("module `str` has no member `pad_right`"),
        "{msg}"
    );
}

#[test]
fn a_bound_record_missing_a_field_keeps_the_field_error() {
    let msg = run_err("r = {a: 1}\nr.pad_right");
    assert!(msg.contains("not found in record"), "{msg}");
}

// 10. A module member without parentheses is a pipe stage.

#[test]
fn a_module_member_without_parens_is_a_pipe_stage() {
    assert_eq!(run_ok("[1, 2] | arr.len"), run_ok("[1, 2] | arr.len()"));
    assert_eq!(run_ok("[1, 2] | arr.len"), Value::Int(2));
}

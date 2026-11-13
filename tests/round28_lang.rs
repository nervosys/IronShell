//! Round 28: language refusals that used to pass silently.
//!
//! Each test pins one item: an error inside a `${...}` hole is the error of the
//! string, an unclosed `${` is a parse error, and so on. Run in-process, as
//! `tests/curried_closure_builtins.rs` does.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

/// The error the program fails with, as its message (the JSON error record).
fn run_err(src: &str) -> String {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = match iron_shell::parser::parse_program(src) {
        Ok(stmts) => stmts,
        Err(e) => return e.to_string(),
    };
    iron_shell::eval::eval_program(&stmts, &mut env)
        .expect_err("should fail")
        .to_string()
}

#[test]
fn an_unknown_field_inside_interpolation_is_the_strings_error() {
    let msg = run_err("r = {a: 1}\nprint(\"${r.zz} after\")");
    assert!(msg.contains("E_UNKNOWN_FIELD"), "got: {msg}");
}

#[test]
fn an_interpolation_error_is_caught_by_try() {
    let v = run("r = {a: 1}\ntry { \"${r.zz} after\" } catch e { e.code }");
    assert_eq!(v, Value::Str("E_UNKNOWN_FIELD".into()));
}

#[test]
fn an_interpolation_that_succeeds_is_unchanged() {
    let v = run("r = {a: 1}\n\"v=${r.a} after\"");
    assert_eq!(v, Value::Str("v=1 after".into()));
}

#[test]
fn an_unclosed_interpolation_is_a_parse_error_at_the_dollar() {
    let msg = run_err("x = 1\nprint(\"a ${x\")");
    assert!(msg.contains("E_PARSE"), "got: {msg}");
    assert!(msg.contains("unclosed interpolation `${`"), "got: {msg}");
    assert!(msg.contains("line 2, column 10"), "got: {msg}");
    assert!(msg.contains("write \\\\${ for a literal ${"), "got: {msg}");
}

#[test]
fn an_unclosed_interpolation_in_a_single_quoted_string_is_refused_too() {
    let msg = run_err("x = 1\n'a ${x'");
    assert!(msg.contains("E_PARSE"), "got: {msg}");
    assert!(msg.contains("unclosed interpolation"), "got: {msg}");
}

#[test]
fn an_escaped_dollar_brace_is_still_literal() {
    let v = run("\"a \\${x\"");
    assert_eq!(v, Value::Str("a ${x".into()));
}

#[test]
fn a_one_letter_slip_in_a_short_name_still_gets_its_builtin() {
    let found = iron_shell::builtins::did_you_mean("lenn");
    assert!(found.contains(&"len".to_string()), "{found:?}");
}

#[test]
fn a_four_letter_name_is_not_offered_a_two_letter_slip() {
    let found = iron_shell::builtins::did_you_mean("nope");
    assert!(!found.contains(&"Some".to_string()), "{found:?}");
}

#[test]
fn an_unknown_name_with_no_near_builtin_has_no_suggestion() {
    let msg = run_err("f = fn() => zqxv\nf()");
    assert!(msg.contains("E_UNKNOWN_NAME"), "got: {msg}");
    assert!(!msg.contains("did you mean"), "got: {msg}");
}

//! Round 22 language fixes: `&&` and `||` take Bool operands, `where` and the
//! pipe precedence hint say what to write, assignment errors name the fix, a
//! long append loop is linear, and string literals decode their escapes.

use iron_shell::{
    env::Env,
    eval::{eval_program, eval_program_statement},
    parser::parse_program,
    value::Value,
};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

/// The text of a parse error, for a source the parser must refuse.
fn parse_refusal(src: &str) -> String {
    match parse_program(src) {
        Ok(_) => panic!("expected a parse error for {src:?}"),
        Err(e) => format!("{e}"),
    }
}

/// The text of a run-time error, for a program that must fail.
fn run_refusal(src: &str) -> String {
    match eval(src) {
        Ok(v) => panic!("expected an error for {src:?}, got {v:?}"),
        Err(e) => format!("{e}"),
    }
}

// ---- 1. `&&` and `||` take Bool operands only ----

#[test]
fn and_refuses_a_non_bool_left_operand() {
    let text = run_refusal("1 && 2");
    assert!(
        text.contains("operator And: expected Bool operands, got Int"),
        "{text}"
    );
    assert!(
        text.contains("compare explicitly, e.g. x != 0"),
        "no hint: {text}"
    );
}

#[test]
fn or_refuses_a_non_bool_left_operand() {
    let text = run_refusal("[] || true");
    assert!(
        text.contains("operator Or: expected Bool operands"),
        "{text}"
    );
}

#[test]
fn a_string_is_not_true_under_and() {
    let text = run_refusal("\"false\" && true");
    assert!(
        text.contains("operator And: expected Bool operands"),
        "{text}"
    );
}

#[test]
fn a_null_left_operand_is_refused_too() {
    assert!(run_refusal("null || false").contains("operator Or: expected Bool operands"));
}

#[test]
fn a_non_bool_right_operand_is_refused_when_it_is_evaluated() {
    let text = run_refusal("true && 2");
    assert!(
        text.contains("operator And: expected Bool operands, got Int"),
        "{text}"
    );
    let text = run_refusal("false || 2");
    assert!(
        text.contains("operator Or: expected Bool operands, got Int"),
        "{text}"
    );
}

#[test]
fn a_short_circuited_non_bool_right_operand_is_not_checked() {
    // The right side is never evaluated, so its type does not matter.
    assert!(matches!(eval("false && 2").unwrap(), Value::Bool(false)));
    assert!(matches!(eval("true || 2").unwrap(), Value::Bool(true)));
}

#[test]
fn bool_operands_still_combine() {
    assert!(matches!(eval("true && false").unwrap(), Value::Bool(false)));
    assert!(matches!(eval("false || true").unwrap(), Value::Bool(true)));
    assert!(matches!(
        eval("(1 > 0) && (2 > 1)").unwrap(),
        Value::Bool(true)
    ));
}

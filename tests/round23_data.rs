//! Round 23: the data builtins' error texts and behaviour: grep's options and
//! pattern errors, sql's argument and source messages, sql with no source,
//! to_csv's number and naming, date unit spellings, CSV Bool columns, and
//! csv_columns on a directory. Each test runs in process through the parser
//! and evaluator.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

fn s(v: &str) -> Value {
    Value::Str(v.to_string())
}

// ── item 1: grep's options name the problem ─────────────────────────────

#[test]
fn grep_names_a_line_numbers_value_that_is_not_a_bool() {
    let e = eval(r#"grep("one", "notes.txt", {line_numbers: "yes"})"#).expect_err("not a Bool");
    let text = e.to_string();
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(
        text.contains("line_numbers must be a Bool, got String"),
        "{text}"
    );
}

#[test]
fn grep_names_an_unknown_option_and_the_one_it_takes() {
    let e = eval(r#"grep("one", "notes.txt", {bogus: true})"#).expect_err("unknown option");
    let text = e.to_string();
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(
        text.contains("unknown option bogus; grep takes line_numbers"),
        "{text}"
    );
}

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

// ── item 2: sql's params errors name the params, not the query ──────────

#[test]
fn sql_names_a_non_array_params_value_after_the_query() {
    for src in [
        r#"sql([{id: 1}], "select id from t where id = ?", 5)"#,
        r#"sql([{id: 1}], "select id from t where id = ?", {a: 1})"#,
        r#"[{id: 1}] | sql("select id from t where id = ?", 5)"#,
    ] {
        let text = eval(src).expect_err(src).to_string();
        assert!(text.contains("E_BAD_ARG"), "{src}: {text}");
        assert!(
            text.contains("params must be an Array, e.g. [5], got "),
            "{src}: {text}"
        );
        assert!(!text.contains("expected query"), "{src}: {text}");
    }
    let text = eval(r#"sql([{id: 1}], "select id from t where id = ?", {a: 1})"#)
        .expect_err("a Record")
        .to_string();
    assert!(text.contains("got Record"), "{text}");
}

#[test]
fn sql_with_an_array_of_params_still_binds_them() {
    assert_eq!(
        eval(r#"sql([{id: 1}, {id: 2}], "select id from t where id = ?", [2]) | len"#).unwrap(),
        Value::Int(1)
    );
}

//! Round 24: sql and sql_value take a params Array with no rows and no source
//! (a lone query over the empty database), the (source, query) form is
//! unchanged, and an explicit did-you-mean mapping is the whole suggestion.
//! Each test runs in process through the parser and evaluator.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

fn rec(pairs: &[(&str, Value)]) -> Value {
    Value::Record(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

// ── item 1: a lone query takes a params Array ───────────────────────────

#[test]
fn sql_with_params_and_no_rows_runs_the_query_over_the_empty_database() {
    assert_eq!(
        eval(r#"sql("select ? + 1 as y", [41])"#).unwrap(),
        Value::Array(vec![rec(&[("y", Value::Int(42))])])
    );
}

#[test]
fn sql_value_with_params_and_no_rows_returns_the_one_cell() {
    assert_eq!(
        eval(r#"sql_value("select ? * 2", [21])"#).unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        eval(r#"sql_value("select upper(?)", ["ann"])"#).unwrap(),
        Value::Str("ANN".to_string())
    );
}

#[test]
fn a_lone_query_still_refuses_a_placeholder_count_that_does_not_match() {
    let text = eval(r#"sql_value("select ? * 2", [])"#)
        .expect_err("one placeholder, no values")
        .to_string();
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("1 placeholder"), "{text}");
}

#[test]
fn a_two_argument_call_with_a_string_second_is_still_source_and_query() {
    assert_eq!(
        eval(r#"sql(":memory:", "select 6 * 7 as n") | first | fn(r) => r.n"#).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn source_and_query_with_params_still_reads_the_file() {
    let dir = std::env::temp_dir().join(format!("ae_round24_sql_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("people.csv");
    std::fs::write(&path, "name\nAnn\nBo\n").unwrap();
    let p = path.to_string_lossy().replace('\\', "/");
    let got = eval(&format!(
        r#"sql("{p}", "select name from people where name = ?", ["Ann"]) | len"#
    ));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got.unwrap(), Value::Int(1));
}

// ── item 2: an explicit did-you-mean mapping is the only suggestion ─────

#[test]
fn redo_is_suggested_as_session_redo_and_nothing_else() {
    assert_eq!(
        iron_shell::builtins::did_you_mean("redo"),
        vec!["session_redo".to_string()]
    );
}

#[test]
fn get_is_still_suggested_as_http_get() {
    assert_eq!(
        iron_shell::builtins::did_you_mean("get")
            .first()
            .map(String::as_str),
        Some("http_get")
    );
}

#[test]
fn a_source_with_params_and_no_query_is_named_as_missing_its_query() {
    let e = eval(r#"sql("people.csv", ["Ann"])"#).expect_err("no query");
    let text = e.to_string();
    assert!(text.contains("needs a query after its source"), "{text}");
    let e = eval(r#"sql_value("people.csv", ["Ann"])"#).expect_err("no query");
    assert!(
        e.to_string().contains("needs a query after its source"),
        "{e}"
    );
}

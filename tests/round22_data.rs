//! Round 22: CSV column typing, to_csv's column union, csv_columns' file
//! check, sql's argument messages, enumerate, grep line numbers, JSON indent
//! and json_set, and did-you-mean spellings. Each test runs in process through
//! the parser and evaluator.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

fn s(v: &str) -> Value {
    Value::Str(v.to_string())
}

// ── item 1: whole-column CSV typing ─────────────────────────────────────

#[test]
fn a_leading_zero_code_makes_its_whole_column_text() {
    assert_eq!(
        eval(r#"from_csv("n\n007\nNA\n12\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![s("007"), s("NA"), s("12")])
    );
    assert_eq!(
        eval(r#"from_csv("n\n007\n12\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![s("007"), s("12")])
    );
}

#[test]
fn a_leading_plus_makes_its_column_text() {
    assert_eq!(
        eval(r#"from_csv("n\n+5\n6\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![s("+5"), s("6")])
    );
}

#[test]
fn zero_and_signed_decimals_are_numbers() {
    assert_eq!(
        eval(r#"from_csv("n\n0\n-0.5\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![Value::Float(0.0), Value::Float(-0.5)])
    );
    assert_eq!(
        eval(r#"from_csv("n\n-01\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![s("-01")])
    );
}

#[test]
fn a_column_of_integers_is_int_and_a_mixed_one_is_float() {
    assert_eq!(
        eval(r#"from_csv("n\n1\n2\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        eval(r#"from_csv("n\n1\n2.5\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![Value::Float(1.0), Value::Float(2.5)])
    );
}

#[test]
fn a_blank_in_a_text_column_stays_an_empty_string_and_in_a_numeric_one_is_null() {
    assert_eq!(
        eval(r#"from_csv("n,s\n007,\n8,x\n") | map(fn(r) => r.s)"#).unwrap(),
        Value::Array(vec![s(""), s("x")])
    );
    // The 007 is in another column: the blank and the 2 are a numeric column.
    assert_eq!(
        eval(r#"from_csv("n,m\n007,\n8,2\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![s("007"), s("8")])
    );
    assert_eq!(
        eval(r#"from_csv("n,m\n1,\n8,2\n") | map(fn(r) => r.m)"#).unwrap(),
        Value::Array(vec![Value::Null, Value::Int(2)])
    );
}

// ── item 2: to_csv keeps every key ──────────────────────────────────────

#[test]
fn to_csv_keeps_a_key_that_only_a_later_row_has() {
    assert_eq!(
        eval(r#"[{id: 1}, {id: 2, customer: "Ada"}] | to_csv"#).unwrap(),
        s("customer,id\n,1\nAda,2\n")
    );
    assert_eq!(
        eval(r#"to_csv([{a: 1}, {b: 2}, {a: 3, c: 4}])"#).unwrap(),
        s("a,b,c\n1,,\n,2,\n3,,4\n")
    );
}

// ── item 3: a file name that is not a file ──────────────────────────────

#[test]
fn csv_columns_refuses_a_one_line_file_name_that_is_not_a_file() {
    for name in ["nofile.csv", "nofile.TSV", "nofile.txt"] {
        let e = eval(&format!(r#"csv_columns("{name}")"#)).expect_err(name);
        let text = e.to_string();
        assert!(text.contains("E_NOT_FOUND"), "{text}");
        assert!(text.contains(name), "{text}");
        assert!(
            text.contains("pass CSV text with a newline, or an existing file"),
            "{text}"
        );
    }
}

#[test]
fn csv_columns_still_reads_one_line_text_and_multi_line_text() {
    assert_eq!(
        eval(r#"csv_columns("id,customer")"#).unwrap(),
        Value::Array(vec![s("id"), s("customer")])
    );
    assert_eq!(
        eval(r#"csv_columns("name\nAda\n")"#).unwrap(),
        Value::Array(vec![s("name")])
    );
}

//! Round 20: CSV column order and typing, SQL bind parameters, and the file
//! append contract. Each test runs in process through the parser and evaluator.

use iron_shell::value::Value;
use std::path::{Path, PathBuf};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = iron_shell::parser::parse_program(src).expect("parse");
    iron_shell::eval::eval_program(&stmts, &mut iron_shell::env::Env::new())
}

fn s(v: &str) -> Value {
    Value::Str(v.to_string())
}

fn arr(items: &[&str]) -> Value {
    Value::Array(items.iter().map(|i| s(i)).collect())
}

/// The E_ code in a refusal's message, as the other round tests read it.
fn code_of(e: &anyhow::Error) -> String {
    let text = e.to_string();
    match text.find("E_") {
        Some(i) => text[i..]
            .split(|c: char| !(c.is_ascii_uppercase() || c == '_'))
            .next()
            .unwrap_or("E_?")
            .to_string(),
        None => format!("NO_CODE: {text}"),
    }
}

/// A fresh directory for one test. The tag keeps parallel tests apart.
fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_round20_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A path as a string the source can hold without escapes.
fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

// ── item 1: column order ────────────────────────────────────────────────

#[test]
fn to_csv_writes_the_columns_it_is_given_in_that_order() {
    assert_eq!(
        eval(r#"[{b: 2, a: 1}] | to_csv(["b", "a"])"#).unwrap(),
        s("b,a\n2,1\n")
    );
    assert_eq!(
        eval(r#"to_csv([{b: 2, a: 1}], ["b", "a"])"#).unwrap(),
        s("b,a\n2,1\n")
    );
}

#[test]
fn to_csv_without_columns_keeps_the_sorted_keys() {
    assert_eq!(eval(r#"[{b: 2, a: 1}] | to_csv"#).unwrap(), s("a,b\n1,2\n"));
}

#[test]
fn a_key_a_record_lacks_writes_an_empty_field() {
    assert_eq!(
        eval(r#"[{id: 1}, {id: 2, customer: "Ada"}] | to_csv(["id", "customer"])"#).unwrap(),
        s("id,customer\n1,\n2,Ada\n")
    );
}

#[test]
fn to_csv_refuses_a_columns_argument_that_is_not_names() {
    for src in [
        r#"[{a: 1}] | to_csv([1])"#,
        r#"[{a: 1}] | to_csv([])"#,
        r#"[{a: 1}] | to_csv("a")"#,
    ] {
        let e = eval(src).expect_err(src);
        assert_eq!(code_of(&e), "E_BAD_ARG", "{src}: {e}");
    }
}

#[test]
fn csv_columns_reads_the_header_of_csv_text_in_file_order() {
    assert_eq!(
        eval(r#"csv_columns("id,customer,amount\n1,Ada,3\n")"#).unwrap(),
        arr(&["id", "customer", "amount"])
    );
    assert_eq!(
        eval(r#""zeta,alpha\n1,2\n" | csv_columns"#).unwrap(),
        arr(&["zeta", "alpha"])
    );
}

#[test]
fn csv_columns_reads_a_file_header_and_drops_its_byte_order_mark() {
    let dir = fresh_dir("cols");
    let path = dir.join("rows.csv");
    std::fs::write(&path, "\u{feff}customer,id,amount\nAda,1,3\n").unwrap();
    let p = fwd(&path);
    let got = eval(&format!(r#"csv_columns("{p}")"#));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got.unwrap(), arr(&["customer", "id", "amount"]));
}

#[test]
fn a_csv_file_round_trips_through_open_and_to_csv_in_its_own_column_order() {
    let dir = fresh_dir("roundtrip");
    let path = dir.join("rows.csv");
    let text = "customer,id,amount\nAda,1,3\nGrace,2,\n";
    std::fs::write(&path, text).unwrap();
    let p = fwd(&path);
    let got = eval(&format!(r#"open("{p}") | to_csv(csv_columns("{p}"))"#));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got.unwrap(), s(text));
}

#[test]
fn csv_columns_refuses_a_subject_that_is_not_text() {
    let e = eval("csv_columns(5)").expect_err("an Int is not a path or text");
    assert_eq!(code_of(&e), "E_BAD_ARG", "{e}");
}

//! `count_by(key)`: rows counted by a field name, a `.field` or a lambda, as
//! {key, count} records, most frequent first, ties by key.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn strs(v: &[&str]) -> Value {
    Value::Array(v.iter().map(|s| Value::Str((*s).to_string())).collect())
}

#[test]
fn counts_by_a_field_name_most_frequent_first() {
    let v =
        run(r#"[{u: "a"}, {u: "b"}, {u: "b"}, {u: "c"}] | count_by("u") | map(fn(r) => r.key)"#);
    assert_eq!(v, strs(&["b", "a", "c"]));
    let c = run(r#"[{u: "a"}, {u: "b"}, {u: "b"}] | count_by("u") | map(fn(r) => r.count)"#);
    assert_eq!(c, Value::Array(vec![Value::Int(2), Value::Int(1)]));
}

#[test]
fn ties_are_ordered_by_key() {
    let v = run(r#"[{u: "z"}, {u: "m"}, {u: "a"}] | count_by(.u) | map(fn(r) => r.key)"#);
    assert_eq!(v, strs(&["a", "m", "z"]));
}

#[test]
fn a_lambda_key_and_the_first_argument_subject() {
    let v = run(r#"count_by([1, 2, 1, 1], fn(x) => x) | first | fn(r) => r.count"#);
    assert_eq!(v, Value::Int(3));
}

#[test]
fn a_missing_field_counts_under_null() {
    let v = run(r#"[{a: 1}, {b: 2}] | count_by("a") | map(fn(r) => r.key)"#);
    // Ties are ordered by the key's text: "1" before "<null>".
    assert_eq!(v, Value::Array(vec![Value::Int(1), Value::Null]));
}

#[test]
fn a_bad_key_is_refused() {
    let v = run(r#"(try { [1] | count_by(5) } catch e { e.error.code })"#);
    assert_eq!(v, Value::Str("E_BAD_ARG".to_string()));
}

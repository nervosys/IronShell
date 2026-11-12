//! Round 26: JSON writes refuse a value JSON cannot hold, and parse refusals
//! are not retryable. Each test runs in process through the parser and
//! evaluator.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

fn refusal(src: &str) -> String {
    eval(src).expect_err(src).to_string()
}

// ── item 1: a NaN or an infinity is refused by a JSON write, not written as 0 ──

#[test]
fn to_json_refuses_a_nan_and_names_its_path() {
    // TOML spells NaN as `nan`; from_toml reads it as a Float.
    let text = refusal(r#"from_toml("a = nan") | to_json"#);
    assert!(text.contains("E_BAD_VALUE"), "{text}");
    assert!(
        text.contains("value.a is NaN, which JSON cannot hold"),
        "{text}"
    );
}

#[test]
fn to_json_names_an_infinity_and_a_negative_one() {
    let text = refusal(r#"from_toml("a = inf") | to_json"#);
    assert!(text.contains("value.a is infinity"), "{text}");
    let text = refusal(r#"from_toml("a = -inf") | to_json"#);
    assert!(text.contains("value.a is negative infinity"), "{text}");
}

#[test]
fn to_json_names_the_path_inside_an_array() {
    let text = refusal(r#"from_toml("v = [1.0, nan]") | to_json"#);
    assert!(text.contains("value.v[1] is NaN"), "{text}");
}

#[test]
fn json_stringify_refuses_a_nan_the_same_way() {
    let text = refusal(r#"json_stringify(from_toml("a = nan"))"#);
    assert!(text.contains("E_BAD_VALUE"), "{text}");
    assert!(text.contains("json_stringify: value.a is NaN"), "{text}");
}

#[test]
fn to_json_still_writes_finite_floats_and_ints() {
    assert_eq!(
        eval(r#"to_json([1.5, 2, -0.25])"#).unwrap(),
        Value::Str("[1.5,2,-0.25]".to_string())
    );
}

#[test]
fn from_csv_reads_nan_and_infinity_words_as_text() {
    assert_eq!(
        eval(r#"from_csv("v\nNaN\n1\n") | to_json"#).unwrap(),
        Value::Str(r#"[{"v":"NaN"},{"v":"1"}]"#.to_string())
    );
    assert_eq!(
        eval(r#"from_csv("v\ninf\n-Infinity\n") | to_json"#).unwrap(),
        Value::Str(r#"[{"v":"inf"},{"v":"-Infinity"}]"#.to_string())
    );
    assert_eq!(
        eval(r#"from_csv("v\n1e400\n") | first | fn(r) => r.v"#).unwrap(),
        Value::Str("1e400".to_string())
    );
}

#[test]
fn from_csv_still_types_real_numbers() {
    assert_eq!(
        eval(r#"from_csv("v\n1.5\n2\n") | to_json"#).unwrap(),
        Value::Str(r#"[{"v":1.5},{"v":2.0}]"#.to_string())
    );
}

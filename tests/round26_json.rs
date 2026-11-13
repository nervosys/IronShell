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

/// Evaluate `src` with `v` bound to a record `{a: x}`, `{v: [1.0, x]}`-style
/// values built in Rust: no ish source can make a non-finite Float, since
/// every reader refuses one, so the writers' own refusal is reached this way.
fn refusal_with(name: &str, value: Value, src: &str) -> String {
    let mut env = Env::new();
    env.set_var(name, value).expect("binds");
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut env).expect_err(src).to_string()
}

fn record_a(x: f64) -> Value {
    Value::Record([("a".to_string(), Value::Float(x))].into_iter().collect())
}

// ── item 1: a NaN or an infinity is refused by a JSON write, not written as 0 ──

#[test]
fn to_json_refuses_a_nan_and_names_its_path() {
    let text = refusal_with("r", record_a(f64::NAN), "r | to_json");
    assert!(text.contains("E_BAD_VALUE"), "{text}");
    assert!(
        text.contains("value.a is NaN, which JSON cannot hold"),
        "{text}"
    );
}

#[test]
fn to_json_names_an_infinity_and_a_negative_one() {
    let text = refusal_with("r", record_a(f64::INFINITY), "r | to_json");
    assert!(text.contains("value.a is infinity"), "{text}");
    let text = refusal_with("r", record_a(f64::NEG_INFINITY), "r | to_json");
    assert!(text.contains("value.a is negative infinity"), "{text}");
}

#[test]
fn to_json_names_the_path_inside_an_array() {
    let v = Value::Record(
        [(
            "v".to_string(),
            Value::Array(vec![Value::Float(1.0), Value::Float(f64::NAN)]),
        )]
        .into_iter()
        .collect(),
    );
    let text = refusal_with("r", v, "r | to_json");
    assert!(text.contains("value.v[1] is NaN"), "{text}");
}

#[test]
fn json_stringify_refuses_a_nan_the_same_way() {
    let text = refusal_with("r", record_a(f64::NAN), "json_stringify(r)");
    assert!(text.contains("E_BAD_VALUE"), "{text}");
    assert!(text.contains("json_stringify: value.a is NaN"), "{text}");
}

#[test]
fn from_toml_refuses_inf_and_nan_naming_the_key() {
    for (src, key) in [
        ("a = nan", "a"),
        ("a = inf", "a"),
        ("a = -inf", "a"),
        ("[t]\nv = [1.0, nan]", "t.v[1]"),
    ] {
        let text = refusal(&format!("from_toml({src:?})"));
        assert!(text.contains("not a finite number"), "{src}: {text}");
        assert!(text.contains(key), "{src}: {text}");
        assert!(text.contains("quote it as a string"), "{src}: {text}");
    }
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

// ── item 2: an integer past the 64-bit range is refused, not read as a Float ──

#[test]
fn from_json_refuses_an_integer_past_i64_and_names_its_path() {
    let text = refusal(r#"from_json("9223372036854775808")"#);
    assert!(text.contains("E_BAD_VALUE"), "{text}");
    assert!(
        text.contains(
            "integer overflow at value: 9223372036854775808 does not fit in a 64-bit Int"
        ),
        "{text}"
    );
    assert!(text.contains("quote it as a string"), "{text}");
}

#[test]
fn from_json_names_the_path_of_a_wide_integer() {
    let text = refusal(r#"from_json("[18446744073709551615]")"#);
    assert!(text.contains("at value[0]: 18446744073709551615"), "{text}");
    let text = refusal(r#"from_json("{\"n\":18446744073709551615}")"#);
    assert!(text.contains("at value.n: 18446744073709551615"), "{text}");
    let text = refusal(r#"from_json("-9223372036854775809")"#);
    assert!(text.contains("-9223372036854775809"), "{text}");
}

#[test]
fn to_toml_of_a_wide_integer_is_refused_at_the_read() {
    let text = refusal(r#"to_toml(from_json("{\"n\":18446744073709551615}"))"#);
    assert!(text.contains("E_BAD_VALUE"), "{text}");
}

#[test]
fn json_integers_within_i64_and_real_floats_are_unchanged() {
    assert_eq!(
        eval(r#"from_json("9223372036854775807")"#).unwrap(),
        Value::Int(i64::MAX)
    );
    assert_eq!(
        eval(r#"from_json("-9223372036854775808")"#).unwrap(),
        Value::Int(i64::MIN)
    );
    assert_eq!(eval(r#"from_json("1e300")"#).unwrap(), Value::Float(1e300));
    assert_eq!(eval(r#"from_json("0.1")"#).unwrap(), Value::Float(0.1));
    // A digit run inside a string is text, not a number.
    assert_eq!(
        eval(r#"from_json("{\"a\":\"9223372036854775808\",\"b\":1}").a"#).unwrap(),
        Value::Str("9223372036854775808".to_string())
    );
}

#[test]
fn json_parse_refuses_a_wide_integer_too() {
    let text = refusal(r#"json_parse("[9223372036854775808]")"#);
    assert!(text.contains("E_BAD_VALUE"), "{text}");
    assert!(
        text.contains("json_parse: integer overflow at value[0]"),
        "{text}"
    );
}

#[test]
fn open_json_refuses_a_wide_integer_in_a_file() {
    let path = std::env::temp_dir().join(format!("round26_wide_{}.json", std::process::id()));
    std::fs::write(&path, r#"[{"id": 18446744073709551615}]"#).expect("write");
    let shown = path.to_string_lossy().replace('\\', "/");
    let text = refusal(&format!(r#"open("{shown}")"#));
    let _ = std::fs::remove_file(&path);
    assert!(text.contains("E_BAD_VALUE"), "{text}");
    assert!(
        text.contains("at value[0].id: 18446744073709551615"),
        "{text}"
    );
}

#[test]
fn from_yaml_refuses_a_wide_integer_and_keeps_an_i64() {
    let text = refusal(r#"from_yaml("n: 9223372036854775808")"#);
    assert!(text.contains("E_BAD_VALUE"), "{text}");
    assert!(text.contains("9223372036854775808"), "{text}");
    assert_eq!(
        eval(r#"from_yaml("n: 9223372036854775807").n"#).unwrap(),
        Value::Int(i64::MAX)
    );
}

#[test]
fn from_csv_keeps_a_wide_integer_as_text_and_types_an_i64_as_int() {
    assert_eq!(
        eval(r#"from_csv("v\n9223372036854775808\n1\n") | first | fn(r) => r.v"#).unwrap(),
        Value::Str("9223372036854775808".to_string())
    );
    assert_eq!(
        eval(r#"from_csv("v\n9223372036854775807\n1\n") | first | fn(r) => r.v"#).unwrap(),
        Value::Int(i64::MAX)
    );
}

// ── item 3: a parse error of the given text is not retryable, and says why ──

#[test]
fn a_from_json_parse_error_is_e_bad_arg_and_not_retryable() {
    let err = eval(r#"from_json("{")"#).expect_err("not JSON");
    let coded = err
        .downcast_ref::<iron_shell::safety::SafetyError>()
        .expect("a coded error");
    assert_eq!(coded.code.as_str(), "E_BAD_ARG");
    assert!(!coded.is_retryable(), "{}", coded.message);
    assert!(
        coded.message.contains("the text is not JSON"),
        "{}",
        coded.message
    );
    assert!(coded.hint.contains("line 1"), "{}", coded.hint);
}

#[test]
fn a_from_toml_parse_error_is_not_retryable() {
    let err = eval(r#"from_toml("a = ")"#).expect_err("not TOML");
    let coded = err
        .downcast_ref::<iron_shell::safety::SafetyError>()
        .expect("a coded error");
    assert_eq!(coded.code.as_str(), "E_BAD_VALUE");
    assert!(!coded.is_retryable(), "{}", coded.message);
}

#[test]
fn a_from_toml_missing_value_gives_a_reason() {
    let text = refusal(r#"from_toml("a = ")"#);
    let reason = text
        .split("the text is not TOML: ")
        .nth(1)
        .expect("the refusal says the text is not TOML");
    assert!(!reason.trim().is_empty(), "{text}");
}

#[test]
fn a_from_toml_integer_past_64_bits_is_named_as_out_of_range() {
    let text = refusal(r#"from_toml("n = 9223372036854775808")"#);
    assert!(
        text.contains("the integer 9223372036854775808 at line 1 is out of the 64-bit Int range"),
        "{text}"
    );
    assert!(text.contains("the text is valid TOML"), "{text}");
    assert!(!text.contains("not TOML"), "{text}");
}

#[test]
fn from_csv_still_types_real_numbers() {
    assert_eq!(
        eval(r#"from_csv("v\n1.5\n2\n") | to_json"#).unwrap(),
        Value::Str(r#"[{"v":1.5},{"v":2.0}]"#.to_string())
    );
}

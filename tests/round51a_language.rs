//! Round 51a: language and data fixes (aggregates, group_by, hints).

use iron_shell::safety::{ErrorCode, SafetyError};
use iron_shell::value::Value;

fn try_run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn run(src: &str) -> Value {
    try_run(src).expect("runs")
}

fn refusal(src: &str) -> SafetyError {
    let err = try_run(src).expect_err(src);
    err.downcast_ref::<SafetyError>()
        .unwrap_or_else(|| panic!("{src}: not a SafetyError: {err}"))
        .clone()
}

// A1
#[test]
fn a1_float_aggregates_refuse_a_non_finite_result() {
    for src in [
        "sum([1e308, 1e308])",
        "avg([1e308, 1e308])",
        "mean([1e308, 1e308])",
        "math.sum([1e308, 1e308])",
        "product([1e308, 10.0])",
    ] {
        let se = refusal(src);
        assert_eq!(se.code, ErrorCode::Overflow, "{src}");
        assert_eq!(se.code.as_str(), "E_BAD_VALUE", "{src}");
        assert!(
            se.message.contains("float overflow"),
            "{src}: {}",
            se.message
        );
    }
    assert_eq!(run("sum([1.5, 2.5])"), Value::Float(4.0));
}

// A2
#[test]
fn a2_int_overflow_in_sum_is_e_bad_value() {
    for src in [
        "sum([9223372036854775807, 1])",
        "math.sum([9223372036854775807, 1])",
        "product([9223372036854775807, 2])",
    ] {
        let se = refusal(src);
        assert_eq!(se.code.as_str(), "E_BAD_VALUE", "{src}");
        assert!(se.message.contains("integer overflow"), "{src}");
    }
}

// A3
#[test]
fn a3_group_by_a_missing_field_is_null_in_all_three_forms() {
    for key in [r#""u""#, ".u", "fn(r) => r.u"] {
        let src = format!(
            "[{{u: 1}}, {{x: 2}}, {{u: null}}] | group_by({key}) | map(fn(g) => [g.Name, g.Key, g.Count])"
        );
        let names = run(&format!(
            "[{{u: 1}}, {{x: 2}}, {{u: null}}] | group_by({key}) | map(fn(g) => g.Name)"
        ));
        assert_eq!(
            names,
            Value::Array(vec![Value::Str("1".into()), Value::Str("<null>".into())]),
            "{src}"
        );
        let counts = run(&format!(
            "[{{u: 1}}, {{x: 2}}, {{u: null}}] | group_by({key}) | map(fn(g) => g.Count)"
        ));
        assert_eq!(
            counts,
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
            "{src}"
        );
    }
}

// A4
#[test]
fn a4_group_by_merges_an_int_and_an_equal_float() {
    assert_eq!(
        run("[1, 1.0, 2] | group_by(fn(x) => x) | map(fn(g) => g.Count)"),
        Value::Array(vec![Value::Int(2), Value::Int(1)])
    );
    assert_eq!(
        run("[1.0, 1] | group_by(fn(x) => x) | first | fn(g) => g.Name"),
        Value::Str("1.0".into())
    );
}

// A5
#[test]
fn a5_a_field_no_row_has_is_unknown() {
    for src in [
        r#"[{a: 1}] | count_by("zz")"#,
        r#"[{a: 1}] | count_by(.zz)"#,
        r#"[{a: 1}] | group_by("zz")"#,
        r#"[{abc: 1}] | group_by("abd")"#,
    ] {
        let se = refusal(src);
        assert_eq!(se.code.as_str(), "E_UNKNOWN_FIELD", "{src}");
    }
    let se = refusal(r#"[{abc: 1}] | count_by("abd")"#);
    assert!(se.did_you_mean.contains(&"abc".to_string()));
}

#[test]
fn a5_a_non_record_row_with_a_field_key_is_bad_arg_naming_the_index() {
    for src in ["[1, 2, 3] | count_by(.a)", "[{a: 1}, 5] | group_by(\"a\")"] {
        let se = refusal(src);
        assert_eq!(se.code.as_str(), "E_BAD_ARG", "{src}");
        assert!(se.message.contains("at index"), "{}", se.message);
    }
    // An empty array and a row that merely lacks the field are fine.
    assert_eq!(run(r#"[] | count_by("a")"#), Value::Array(vec![]));
}

// A6
#[test]
fn a6_a_let_over_an_annotated_mut_says_let_not_assign() {
    let se = refusal(r#"mut m: Int = 1; let m = "s""#);
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert!(se.message.starts_with("let m"), "{}", se.message);
    assert!(se.hint.contains("let"), "{}", se.hint);
    // A plain assignment still says assign.
    let se = refusal(r#"mut m: Int = 1; m = "s""#);
    assert!(se.message.starts_with("assign m"), "{}", se.message);
}

// A7
#[test]
fn a7_hints_say_what_is_wrong() {
    let generic = "takes";
    let se = refusal(r#"sum(["a", "b"])"#);
    assert!(se.message.contains("at index 0"), "{}", se.message);
    assert!(!se.hint.contains(generic), "{}", se.hint);
    assert!(se.hint.contains("element 0"), "{}", se.hint);

    let se = refusal("to_csv([1, 2])");
    assert!(se.hint.contains("Record"), "{}", se.hint);
    assert!(!se.hint.starts_with("to_csv takes"), "{}", se.hint);

    let se = refusal(r#"from_yaml("a: 1\na: 2\n")"#);
    assert!(se.hint.contains("repeats a key"), "{}", se.hint);

    let se = refusal(r#"from_yaml("just text")"#);
    assert!(!se.hint.starts_with("from_yaml takes"), "{}", se.hint);

    let se = refusal(r#"[{a: 1}] | sql("select 1; select 2")"#);
    assert!(se.hint.contains("one sql call per query"), "{}", se.hint);
    assert!(!se.hint.starts_with("tables:"), "{}", se.hint);
}

// A8
#[test]
fn a8_a_float_with_no_int_value_prints_in_display_form() {
    for src in ["math.round(1e300)", "math.floor(1e19)", "int(1e300)"] {
        let se = refusal(src);
        assert!(se.message.len() < 80, "{src}: {}", se.message);
        assert!(se.message.contains("has no Int value"), "{}", se.message);
    }
    assert!(refusal("math.round(1e300)").message.contains("1e300"));
}

// A9
#[test]
fn a9_the_power_operator_is_one_token_in_the_word_call_hint() {
    let e = iron_shell::parser::parse_program("print 2 ** 10").expect_err("must not parse");
    let hint = e
        .downcast_ref::<iron_shell::parser::ParseError>()
        .map(|p| p.hint.clone())
        .unwrap_or_default();
    assert!(hint.contains("print(2) ** 10"), "{hint}");
    assert!(hint.contains("print(2 ** 10)"), "{hint}");
    assert!(!hint.contains("* *"), "{hint}");
    assert!(e.to_string().contains("`**`"), "{e}");
}

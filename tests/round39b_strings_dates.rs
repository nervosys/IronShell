//! Round 39b: ISO 8601 forms without seconds and in the basic spelling, the
//! corrected two-pass variance, index_of / last_index_of, regex_split, split's
//! limit, and date_format's refusal naming the specifier.

use iron_shell::value::Value;

fn try_run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn run(src: &str) -> Value {
    try_run(src).unwrap_or_else(|e| panic!("{src}: {e}"))
}

fn refusal(src: &str) -> String {
    try_run(src).expect_err(src).to_string()
}

fn s(text: &str) -> Value {
    Value::Str(text.to_string())
}

fn strs(items: &[&str]) -> Value {
    Value::Array(items.iter().map(|i| s(i)).collect())
}

// 1. date forms

#[test]
fn date_parse_accepts_a_time_without_seconds() {
    for src in [
        r#"date_parse("2024-02-29T13:05")"#,
        r#"date_parse("2024-02-29 13:05")"#,
        r#"date_parse("2024-02-29t13:05Z")"#,
        r#"date_parse("2024-02-29T13:05:00")"#,
        r#"date_parse("20240229T1305")"#,
    ] {
        assert_eq!(run(src), s("2024-02-29T13:05:00Z"), "{src}");
    }
}

#[test]
fn date_parse_accepts_basic_dates_and_basic_offsets() {
    assert_eq!(run(r#"date_parse("20240229")"#), s("2024-02-29"));
    assert_eq!(
        run(r#"date_parse("20240229T130509Z")"#),
        s("2024-02-29T13:05:09Z")
    );
    assert_eq!(
        run(r#"date_parse("2024-02-29T13:05:09+0530")"#),
        s("2024-02-29T07:35:09Z")
    );
    assert_eq!(
        run(r#"date_parse("2024-02-29T13:05-08")"#),
        s("2024-02-29T21:05:00Z")
    );
    assert_eq!(
        run(r#"date_parse("2024-02-29T13:05:09+05:30")"#),
        s("2024-02-29T07:35:09Z")
    );
}

#[test]
fn the_other_date_builtins_share_the_forms() {
    assert_eq!(
        run(r#"date_add("2024-02-29T13:05", 1, "days")"#),
        s("2024-03-01T13:05:00Z")
    );
    assert_eq!(run(r#"date_diff("20240229", "2024-03-02")"#), Value::Int(2));
    assert_eq!(
        run(r#"date_format("2024-02-29 13:05", "%H:%M")"#),
        s("13:05")
    );
}

#[test]
fn a_short_field_is_still_not_zero_padded_and_the_rest_name_their_problem() {
    let text = refusal(r#"date_parse("2024-02-29T1:05")"#);
    assert!(text.contains("is not zero-padded"), "{text}");
    let text = refusal(r#"date_parse("2024-2-29")"#);
    assert!(text.contains("is not zero-padded"), "{text}");
    let text = refusal(r#"date_parse("2024-02-29T13:05:7")"#);
    assert!(text.contains("is not zero-padded"), "{text}");
    // Padded, but not a date: the refusal says so rather than blaming padding.
    let text = refusal(r#"date_parse("2024-02-29T25:05")"#);
    assert!(text.contains("is not a date"), "{text}");
    assert!(!text.contains("zero-padded"), "{text}");
}

// 2. variance

#[test]
fn variance_matches_python_on_large_close_values() {
    assert_eq!(
        run("[100000000.1, 100000000.2, 100000000.3] | variance"),
        Value::Float(0.010000000298023245)
    );
    assert_eq!(run("[1, 2, 3, 4, 5] | variance"), Value::Float(2.5));
}

// 3. index_of

#[test]
fn index_of_counts_characters_and_answers_minus_one() {
    assert_eq!(run(r#"index_of("hello", "l")"#), Value::Int(2));
    assert_eq!(run(r#"index_of("héllo", "l")"#), Value::Int(2));
    assert_eq!(run(r#""hello" | index_of("lo")"#), Value::Int(3));
    assert_eq!(run(r#"index_of("hello", "z")"#), Value::Int(-1));
    assert_eq!(run(r#"str.index_of("hello", "e")"#), Value::Int(1));
    assert_eq!(run(r#"last_index_of("hello", "l")"#), Value::Int(3));
    assert_eq!(run(r#"last_index_of("héllo", "o")"#), Value::Int(4));
    assert_eq!(run(r#"last_index_of("hello", "z")"#), Value::Int(-1));
    assert_eq!(run(r#"str.last_index_of("hello", "h")"#), Value::Int(0));
    assert_eq!(
        run(r#"regex_find("héllo", "l").start"#),
        run(r#"index_of("héllo", "l")"#)
    );
}

#[test]
fn index_of_refuses_a_non_string() {
    assert!(refusal("index_of(\"abc\", 1)").contains("E_BAD_ARG"));
}

// 4. regex_split and split's limit

#[test]
fn regex_split_splits_at_each_match() {
    assert_eq!(
        run(r#"regex_split("a1b22c", "[0-9]+")"#),
        strs(&["a", "b", "c"])
    );
    assert_eq!(
        run(r#""a, b;c" | regex_split("[,;] *")"#),
        strs(&["a", "b", "c"])
    );
    assert_eq!(run(r#"regex_split("abc", "z")"#), strs(&["abc"]));
    assert_eq!(run(r#"str.regex_split("a b", " ")"#), strs(&["a", "b"]));
    assert!(refusal(r#"regex_split("abc", "a{")"#).contains("E_BAD_ARG"));
}

#[test]
fn split_takes_a_limit_of_pieces() {
    assert_eq!(run(r#"split("a,b,c", ",", 2)"#), strs(&["a", "b,c"]));
    assert_eq!(run(r#""a,b,c" | split(",", 2)"#), strs(&["a", "b,c"]));
    assert_eq!(run(r#"str.split("a,b,c", ",", 1)"#), strs(&["a,b,c"]));
    assert_eq!(run(r#"split("a,b,c", ",", 10)"#), strs(&["a", "b", "c"]));
    assert_eq!(run(r#"split("abc", "", 2)"#), strs(&["a", "bc"]));
    assert_eq!(run(r#"split("a,b,c", ",")"#), strs(&["a", "b", "c"]));
    assert!(refusal(r#"split("a,b", ",", 0)"#).contains("E_BAD_ARG"));
    assert!(refusal(r#"split("a,b", ",", "x")"#).contains("E_BAD_ARG"));
}

// 5. date_format names the specifier

#[test]
fn date_format_names_the_first_unsupported_specifier() {
    for (layout, spec) in [("%Y %L", "%L"), ("%d %^a", "%^a"), ("%#b", "%#b")] {
        let text = refusal(&format!(r#"date_format("2024-02-29", "{layout}")"#));
        assert!(text.contains("E_BAD_ARG"), "{layout}: {text}");
        assert!(text.contains(&format!("{spec:?}")), "{layout}: {text}");
    }
    assert_eq!(
        run(r#"date_format("2024-02-29T10:00:00.123456789Z", "%6f")"#),
        s("123456")
    );
    assert_eq!(
        run(r#"date_format("2024-02-29T10:00:00.123456789Z", "%f")"#),
        s("123456789")
    );
}

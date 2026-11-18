//! Round 52a: numbers, text, dates and hints.

use iron_shell::safety::SafetyError;
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

fn s(text: &str) -> Value {
    Value::Str(text.to_string())
}

// A1
#[test]
fn a1_int_takes_a_base_and_to_base_writes_one() {
    assert_eq!(run(r#"int("ff", 16)"#), Value::Int(255));
    assert_eq!(run(r#"int("0x1F", 16)"#), Value::Int(31));
    assert_eq!(run(r#"int("-0b101", 2)"#), Value::Int(-5));
    assert_eq!(run(r#"int("0o17", 8)"#), Value::Int(15));
    assert_eq!(run(r#"int("Z", 36)"#), Value::Int(35));
    assert_eq!(run(r#""101" | int(2)"#), Value::Int(5));
    assert_eq!(run(r#"int("42")"#), Value::Int(42));
    assert_eq!(run(r#"to_base(255, 16)"#), s("ff"));
    assert_eq!(run(r#"to_base(-255, 2)"#), s("-11111111"));
    assert_eq!(run(r#"to_base(0, 7)"#), s("0"));
    assert_eq!(
        run(r#"to_base(-9223372036854775807 - 1, 16)"#),
        s("-8000000000000000")
    );
    for src in [
        r#"int("1_0", 10)"#,
        r#"int("12", 2)"#,
        r#"int("0x", 16)"#,
        r#"int("0x-5", 16)"#,
        r#"int("ff", 37)"#,
        r#"int("ff", 1)"#,
        r#"to_base(1, 1)"#,
    ] {
        assert!(try_run(src).is_err(), "{src}");
    }
    let se = refusal(r#"int("0x1F")"#);
    assert!(se.hint.contains(r#"int("0x1F", 16)"#), "{}", se.hint);
}

// A2
#[test]
fn a2_radix_literals_say_so() {
    for (src, word, base) in [
        ("0xff", "hexadecimal", "16"),
        ("0b101", "binary", "2"),
        ("0o17", "octal", "8"),
    ] {
        let err = iron_shell::parser::parse_program(src).expect_err(src);
        let msg = err.to_string();
        assert!(msg.contains(word), "{src}: {msg}");
        assert!(msg.contains(&format!("int(\"ff\", {base})")), "{src}: {msg}");
    }
    let err = iron_shell::parser::parse_program("1x").expect_err("1x");
    assert!(err.to_string().contains("valid exponent"));
}

// A3
#[test]
fn a3_date_parse_trims_and_reads_week_and_ordinal_dates() {
    for src in [
        r#"date_parse("2024-02-29 ")"#,
        r#"date_parse("2024-02-29\n")"#,
        r#"date_parse("  2024-02-29")"#,
        r#"date_parse("2024-W09-4")"#,
        r#"date_parse("2024-060")"#,
    ] {
        assert_eq!(run(src), s("2024-02-29"), "{src}");
    }
    assert_eq!(run(r#"date_parse("2024-060T10:00:00Z")"#), s("2024-02-29T10:00:00Z"));
    assert_eq!(run(r#"date_parse("2024-W01-1")"#), s("2024-01-01"));
    assert_eq!(run(r#"date_parse("2020-W53-7")"#), s("2021-01-03"));
    for src in [
        r#"date_parse("2023-366")"#,
        r#"date_parse("2023-W53-1")"#,
        r#"date_parse("2024-W09-8")"#,
        r#"date_parse("2024-2-9")"#,
    ] {
        assert!(try_run(src).is_err(), "{src}");
    }
}

// A4
#[test]
fn a4_index_of_doc_does_not_call_regex_find_a_byte_offset() {
    let text = include_str!("../src/signature.rs");
    assert!(!text.contains("as regex_find's start"));
    assert_eq!(run(r#"index_of("héllo", "l")"#), Value::Int(2));
    assert_eq!(run(r#"regex_find("héllo", "l").start"#), Value::Int(2));
}

// A5: behaviour is unchanged and the docs state the tie rule.
#[test]
fn a5_ties_round_away_from_zero_and_the_docs_say_so() {
    assert_eq!(run("format_number(0.125, 2)"), s("0.13"));
    assert_eq!(run("format_number(2.5, 0)"), s("3"));
    assert_eq!(run("math.round(2.5)"), Value::Int(3));
    let sig = include_str!("../src/signature.rs");
    assert!(sig.matches("unlike Python").count() >= 2);
}

// A6
#[test]
fn a6_other_languages_names_point_at_the_real_builtin() {
    for (name, want) in [
        ("from_timestamp", "from_epoch"),
        ("epoch", "from_epoch"),
        ("strip", "trim"),
        ("substr", "slice"),
        ("upcase", "upper"),
        ("downcase", "lower"),
        ("hex", "to_base"),
        ("to_hex", "hex_encode"),
        ("sprintf", "format_number"),
        ("sprintf", "pad_left"),
    ] {
        let se = refusal(&format!("{name}(1)"));
        assert!(
            se.did_you_mean.iter().any(|n| n == want),
            "{name}: {:?}",
            se.did_you_mean
        );
    }
    for name in ["ord", "chr"] {
        let se = refusal(&format!("{name}(\"a\")"));
        assert!(se.hint.contains("no ord or chr"), "{name}: {}", se.hint);
    }
    let se = refusal(r#"str.bytes("a")"#);
    assert!(
        se.did_you_mean.iter().any(|n| n == "hex_encode"),
        "{:?}",
        se.did_you_mean
    );
    assert!(!se.did_you_mean.iter().any(|n| n.contains("random")));
}

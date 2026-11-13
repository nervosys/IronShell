//! Text and number edges, one test per round-28 item: the `\U` escape, the
//! rounding of format_number, crypto.hash's algorithm refusal, a lone UTF-16
//! surrogate in JSON, math.pow's overflow, the int and float refusals, the
//! substring hints of the str module, and how `ish -c` renders strings inside
//! an array.

use iron_shell::safety::SafetyError;
use iron_shell::value::Value;
use std::path::PathBuf;
use std::process::Command;

fn run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn text(src: &str) -> String {
    match run(src).unwrap_or_else(|e| panic!("{src}: {e:#}")) {
        Value::Str(s) => s,
        other => panic!("{src}: expected a String, got {other:?}"),
    }
}

fn refusal(src: &str) -> SafetyError {
    let err = run(src).expect_err("must be refused");
    err.downcast_ref::<SafetyError>()
        .unwrap_or_else(|| panic!("not a structured error: {err:#}"))
        .clone()
}

// 1. `\U` takes exactly eight hex digits; other backslashes are kept.

#[test]
fn a_long_unicode_escape_decodes_to_its_code_point() {
    assert_eq!(run(r#"len("\U0001F600")"#).unwrap(), Value::Int(1));
    assert_eq!(text(r#""\U0001F600""#), "\u{1F600}");
    assert_eq!(text(r#""a\U00000041b""#), "aAb");
}

#[test]
fn a_backslash_u_without_eight_digits_keeps_its_backslash() {
    // A Windows path must survive: `\Users` has no eight hex digits after `\U`.
    assert_eq!(text(r#""C:\Users""#), "C:\\Users");
    assert_eq!(run(r#"len("C:\Users")"#).unwrap(), Value::Int(8));
}

#[test]
fn a_long_unicode_escape_out_of_range_or_a_surrogate_is_e_parse() {
    for src in [r#""\U00110000""#, r#""\U0000D800""#] {
        let err = run(src).expect_err("must be refused");
        let msg = format!("{err:#}");
        assert!(msg.contains("E_PARSE"), "{src}: {msg}");
        // The error renders as JSON, so its backslash is escaped there.
        assert!(msg.contains("invalid unicode escape `"), "{src}: {msg}");
        assert!(msg.contains(&src[3..11]), "{src}: {msg}");
    }
}

// 2. format_number rounds the exact binary value; an exact tie goes away from zero.

#[test]
fn format_number_rounds_the_exact_binary_value() {
    assert_eq!(text("format_number(2.675, 2)"), "2.67");
    assert_eq!(text("format_number(1.005, 2)"), "1.00");
    assert_eq!(text("format_number(9.995, 2)"), "9.99");
    assert_eq!(text("format_number(-2.675, 2)"), "-2.67");
}

#[test]
fn format_number_rounds_an_exact_tie_away_from_zero() {
    assert_eq!(text("format_number(2.5, 0)"), "3");
    assert_eq!(text("format_number(0.5, 0)"), "1");
    assert_eq!(text("format_number(0.125, 2)"), "0.13");
    assert_eq!(text("format_number(-2.5, 0)"), "-3");
}

#[test]
fn format_number_carries_into_the_integer_part() {
    assert_eq!(text("format_number(9.5, 0)"), "10");
    assert_eq!(text("format_number(99.5, 0)"), "100");
    assert_eq!(text("format_number(0.5, 0)"), "1");
}

// 3. crypto.hash names the value it took as the algorithm.

#[test]
fn crypto_hash_names_the_algorithm_it_was_given() {
    let e = refusal(r#"crypto.hash("abc", "nope")"#);
    // Neither argument is an algorithm, so both are named; "nope" among them.
    assert!(e.message.contains("\"nope\""), "{}", e.message);
}

#[test]
fn crypto_hash_names_both_values_when_neither_is_an_algorithm() {
    let e = refusal(r#"crypto.hash("nope", "abc")"#);
    assert!(
        e.message.contains("neither \"nope\" nor \"abc\""),
        "{}",
        e.message
    );
}

#[test]
fn crypto_hash_still_takes_the_algorithm_first() {
    assert_eq!(
        text(r#"crypto.hash("sha256", "abc")"#),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

// 4. A lone surrogate escape in JSON is named as such.

#[test]
fn json_parse_names_a_lone_high_surrogate() {
    let e = refusal(r#"json.parse("\"\\ud83d\"")"#);
    assert!(
        e.message
            .contains("a lone UTF-16 surrogate \\ud83d is not a character"),
        "{}",
        e.message
    );
    assert!(e.message.contains("(\\uD83D\\uDE00)"), "{}", e.message);
}

#[test]
fn json_parse_still_reads_a_surrogate_pair() {
    assert_eq!(text(r#"json.parse("\"\\ud83d\\ude00\"")"#), "\u{1F600}");
}

// 5. Non-finite results and inputs are refused.

#[test]
fn math_pow_refuses_an_overflow() {
    // Two Int operands stay Int (as the `**` operator does), so 10 ** 400 is
    // refused as an Int overflow rather than as a Float overflow.
    let e = refusal("math.pow(10, 400)");
    assert_eq!(e.code.as_str(), "E_BAD_VALUE");
    assert!(
        e.message.contains("10 ** 400 does not fit in a 64-bit Int"),
        "{}",
        e.message
    );
    let f = refusal("math.pow(10.0, 400)");
    assert!(
        f.message.contains("math_pow: 10 ** 400 overflows a Float"),
        "{}",
        f.message
    );
    assert!(!e.hint.is_empty());
}

#[test]
fn math_pow_still_returns_finite_results() {
    assert_eq!(run("math.pow(2, 10)").unwrap(), Value::Int(1024));
}

#[test]
fn float_names_the_finite_rule_for_infinity_and_nan() {
    for src in [r#"float("inf")"#, r#"float("nan")"#] {
        let e = refusal(src);
        assert!(
            e.message.contains("IronShell numbers are finite: "),
            "{src}: {}",
            e.message
        );
        assert!(
            !e.message.contains("is not a number"),
            "{src}: {}",
            e.message
        );
    }
}

// 6. int names a string that is not a decimal integer.

#[test]
fn int_says_a_string_is_not_a_decimal_integer() {
    let e = refusal(r#"int("1_000")"#);
    assert!(
        e.message
            .contains("\"1_000\" is not a decimal integer (digits with an optional sign)"),
        "{}",
        e.message
    );
    // The hint still points at float for decimals.
    let e = refusal(r#"int("1.5")"#);
    assert!(e.hint.contains("float("), "{}", e.hint);
}

// 7. substring spellings name slice, and nothing else.

#[test]
fn a_substring_member_names_slice_only() {
    for src in ["str.substring(\"abc\", 0, 1)", "str.substr(\"abc\", 0, 1)"] {
        let e = refusal(src);
        assert_eq!(e.did_you_mean, vec!["slice".to_string()], "{src}");
    }
}

// 8. `ish -c` renders strings inside an array quoted, as print does.

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_round28_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn ish_c(tag: &str, src: &str) -> String {
    let dir = scratch(tag);
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg("-c")
        .arg(src)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn the_c_result_quotes_strings_inside_an_array() {
    assert_eq!(
        ish_c("split", r#"str.split("a,,b", ",")"#),
        r#"["a", "", "b"]"#
    );
    assert_eq!(ish_c("empty", r#"str.split("", ",")"#), r#"[""]"#);
}

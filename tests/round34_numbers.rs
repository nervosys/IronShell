//! Round 34: finite numbers and ISO dates.
//!
//! Each test names a wrong answer the shell used to give and asserts it is gone:
//! `abs` of the smallest Int, a Float that overflowed to inf, Float `%`,
//! `i64::MIN % -1`, `pow(0, -1)`, `round` against `format_number`, the
//! `format_number` size refusal, and dates outside 0001 to 9999 or written
//! loosely.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

/// The message of the error `src` raises, caught inside the script.
fn message(src: &str) -> String {
    match run(&format!("try {{ {src} }} catch e {{ e.message }}")) {
        Value::Str(s) => s,
        other => panic!("{src} did not raise an error, it gave {other:?}"),
    }
}

fn code(src: &str) -> String {
    match run(&format!("try {{ {src} }} catch e {{ e.code }}")) {
        Value::Str(s) => s,
        other => panic!("{src} did not raise an error, it gave {other:?}"),
    }
}

const INT_MIN: &str = "(-9223372036854775807 - 1)";

#[test]
fn abs_of_the_smallest_int_is_refused() {
    assert_eq!(
        message(&format!("abs({INT_MIN})")),
        "integer overflow in operator Abs"
    );
    assert_eq!(run("abs(-5)"), Value::Int(5));
    assert_eq!(run("abs(-9223372036854775807)"), Value::Int(i64::MAX));
}

#[test]
fn float_overflow_is_refused_not_inf() {
    for src in [
        "float(\"1e308\") * 10",
        "float(\"1e308\") + float(\"1e308\")",
        "0 - float(\"1e308\") - float(\"1e308\")",
    ] {
        let m = message(src);
        assert!(m.contains("float overflow in operator"), "{src}: {m}");
        assert!(m.contains("not a finite number"), "{src}: {m}");
    }
    assert_eq!(
        message("float(\"1e308\") * 10"),
        "float overflow in operator Mul: the result is not a finite number"
    );
    assert_eq!(code("float(\"1e308\") * 10"), "E_BAD_VALUE");
    assert_eq!(run("1.5 * 2"), Value::Float(3.0));
}

#[test]
fn division_by_zero_is_still_refused() {
    assert!(message("1.0 / 0").contains("division by zero"));
}

#[test]
fn float_remainder_follows_the_dividend() {
    assert_eq!(run("7.5 % 2.0"), Value::Float(1.5));
    assert_eq!(run("-7.5 % 2"), Value::Float(-1.5));
    assert_eq!(run("7 % 2.5"), Value::Float(2.0));
    assert_eq!(run("-7 % 3"), Value::Int(-1));
    assert!(message("7.5 % 0.0").contains("remainder by zero"));
    assert!(message("7 % 0.0").contains("remainder by zero"));
}

#[test]
fn int_min_remainder_minus_one_is_zero() {
    assert_eq!(run(&format!("{INT_MIN} % -1")), Value::Int(0));
}

#[test]
fn pow_of_zero_to_a_negative_power_is_a_division_by_zero() {
    assert_eq!(message("pow(0, -1)"), "division by zero: 0 ** -1");
    assert!(message("0 ^ (0 - 1)").contains("division by zero"));
    assert_eq!(run("pow(2, -1)"), Value::Float(0.5));
}

#[test]
fn round_uses_the_rule_format_number_uses() {
    assert_eq!(run("round(2.675, 2)"), Value::Float(2.67));
    assert_eq!(run("round(0.125, 2)"), Value::Float(0.13));
    assert_eq!(run("round(-2.675, 2)"), Value::Float(-2.67));
    assert_eq!(run("round(2.5)"), Value::Int(3));
    assert_eq!(run("round(-2.5)"), Value::Int(-3));
    assert_eq!(run("round(4.966, 2)"), Value::Float(4.97));
    assert_eq!(
        run("format_number(2.675, 2)"),
        Value::Str("2.67".to_string())
    );
}

#[test]
fn format_number_size_refusal_names_the_cap() {
    let m = message("format_number(float(\"1e300\"), 0)");
    assert!(m.contains("1e38"), "{m}");
    assert!(!m.contains("fewer decimals"), "{m}");
    assert!(!m.contains("with 0 decimals"), "{m}");
}

#[test]
fn dates_stay_in_years_0001_to_9999() {
    for src in [
        "date_add(\"9999-12-31\", 1, \"days\")",
        "date_add(\"1970-01-01T00:00:00Z\", 253402300800, \"seconds\")",
        "date_add(\"0001-01-01\", -1, \"days\")",
        "date_add(\"9999-12-01\", 1, \"months\")",
        "date_add(\"9999-01-01\", 1, \"years\")",
    ] {
        let m = message(src);
        assert!(m.contains("out of range"), "{src}: {m}");
    }
    assert_eq!(code("date_add(\"9999-12-31\", 1, \"days\")"), "E_BAD_VALUE");
    assert_eq!(
        run("date_add(\"9999-12-30\", 1, \"days\")"),
        Value::Str("9999-12-31".to_string())
    );
    assert_eq!(
        run("date_add(\"0001-01-02\", -1, \"days\")"),
        Value::Str("0001-01-01".to_string())
    );
    for src in [
        "date_parse(\"+10000-01-01\")",
        "date_parse(\"10000-01-01\")",
        "date_parse(\"-0001-01-01\")",
        "date_parse(\"0000-12-31\")",
        "date_diff(\"+10000-01-01\", \"2024-01-01\")",
        "date_format(\"+10000-01-01\", \"%Y\")",
    ] {
        let m = message(src);
        assert!(m.contains("out of range"), "{src}: {m}");
    }
}

#[test]
fn date_parse_wants_zero_padded_fields_and_no_leap_second() {
    let m = message("date_parse(\"2024-2-9\")");
    assert!(m.contains("YYYY-MM-DD"), "{m}");
    assert_eq!(code("date_parse(\"2024-2-9\")"), "E_BAD_ARG");
    assert_eq!(code("date_parse(\"2024-02-9\")"), "E_BAD_ARG");
    assert_eq!(code("date_parse(\"24-02-09\")"), "E_BAD_ARG");
    assert!(
        message("date_parse(\"2024-02-29T23:59:60Z\")").contains("leap seconds are not supported")
    );
    assert_eq!(
        run("date_parse(\"2024-02-09\")"),
        Value::Str("2024-02-09".to_string())
    );
    assert_eq!(
        run("date_parse(\"2024-02-29T23:59:59Z\")"),
        Value::Str("2024-02-29T23:59:59Z".to_string())
    );
    // Text that is not date-shaped is still "not a date".
    assert_eq!(code("date_parse(\"someday\")"), "E_BAD_VALUE");
}

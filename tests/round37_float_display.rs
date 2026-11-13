//! Round 37: a Float always displays as a Float.
//!
//! `print(100.0)` showed `100`, `sqrt(16)` showed `4` and `-0.0` showed `-0`, so
//! an agent could not tell a Float from an Int by reading the text. A finite
//! integral Float now carries `.0`; a magnitude of 1e16 or more, or under 1e-5,
//! uses the exponent form (as the JSON writer does). Int display is unchanged.

use iron_shell::value::{float_text, plain_text, Value};

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

/// `Display` writes ANSI colour, so strip it before comparing.
fn plain_display(v: &Value) -> String {
    let s = format!("{v}");
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The text of `src`'s result through every renderer, which must all agree.
fn shown(src: &str) -> String {
    let v = run(src);
    let text = plain_text(&v, false);
    assert_eq!(plain_display(&v), text, "{src}: Display and plain_text");
    text
}

fn string(src: &str) -> String {
    match run(src) {
        Value::Str(s) => s,
        other => panic!("{src} did not give a String: {other:?}"),
    }
}

#[test]
fn an_integral_float_prints_with_a_point() {
    assert_eq!(shown("100.0"), "100.0");
    assert_eq!(shown("4.0"), "4.0");
    assert_eq!(shown("-0.0"), "-0.0");
    assert_eq!(shown("0.0"), "0.0");
    assert_eq!(shown("-2.0"), "-2.0");
}

#[test]
fn a_float_result_of_a_builtin_or_division_prints_with_a_point() {
    assert_eq!(shown("sqrt(16)"), "4.0");
    assert_eq!(shown("6 / 2"), "3.0");
    assert_eq!(shown("log(1)"), "0.0");
    assert_eq!(shown("exp(0)"), "1.0");
    assert_eq!(shown("float(5)"), "5.0");
}

#[test]
fn a_non_integral_float_prints_as_before() {
    assert_eq!(shown("0.1 + 0.2"), "0.30000000000000004");
    assert_eq!(shown("1.5"), "1.5");
    assert_eq!(shown("0.5"), "0.5");
    assert_eq!(shown("7 / 2"), "3.5");
}

#[test]
fn a_huge_or_tiny_float_uses_the_exponent_form() {
    assert_eq!(shown("1e21"), "1e21");
    assert_eq!(shown("1e-7"), "1e-7");
    assert_eq!(shown("-1e21"), "-1e21");
    assert_eq!(
        float_text(1.797_693_134_862_315_7e308),
        "1.7976931348623157e308"
    );
    // Below the threshold the digits are written in full, with the point.
    assert_eq!(shown("1e15"), "1000000000000000.0");
    assert_eq!(shown("0.00001"), "0.00001");
    // The exponent form is what the JSON writer produces for the same value.
    let json = serde_json::to_string(&Value::Float(1e21).to_json()).unwrap();
    assert_eq!(json, "1e21");
}

#[test]
fn an_int_prints_as_before() {
    assert_eq!(shown("100"), "100");
    assert_eq!(shown("-0"), "0");
    assert_eq!(shown("9223372036854775807"), "9223372036854775807");
}

#[test]
fn interpolation_and_str_show_a_float_as_a_float() {
    assert_eq!(string(r#""${2.0}""#), "2.0");
    assert_eq!(
        string(
            r#"x = 2.0
"v=${x}""#
        ),
        "v=2.0"
    );
    assert_eq!(string("str(2.0)"), "2.0");
    assert_eq!(string("to_string(2.0)"), "2.0");
    assert_eq!(string("str(2)"), "2");
    assert_eq!(string(r#""${2}""#), "2");
}

#[test]
fn join_shows_a_float_as_a_float_and_an_int_as_an_int() {
    assert_eq!(string(r#"join([1.0, 2], ",")"#), "1.0,2");
    assert_eq!(string(r#"[1.5, 2.0] | join("-")"#), "1.5-2.0");
}

#[test]
fn a_float_inside_a_container_keeps_its_point() {
    assert_eq!(shown("[1.0, 2]"), "[1.0, 2]");
    assert_eq!(shown("{a: 1.0, b: 2}"), "{a: 1.0, b: 2}");
    assert_eq!(string("str([1.0, 2])"), "[1.0, 2]");
}

#[test]
fn data_writers_still_agree_with_the_text() {
    assert_eq!(string("to_json(100.0)"), "100.0");
    assert_eq!(string(r#"to_json({a: 1.0})"#), r#"{"a":1.0}"#);
    // format_number takes explicit decimals and is unchanged.
    assert_eq!(string("format_number(100.0, 0)"), "100");
    assert_eq!(string("format_number(1234.5, 2)"), "1234.50");
}

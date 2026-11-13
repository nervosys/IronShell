//! Round 29: value-level fixes, tested in-process.

use iron_shell::value::Value;
use std::cmp::Ordering;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn text(s: &str) -> Value {
    Value::Str(s.to_string())
}

// ---- Int against Float is exact, not rounded through f64 ----

#[test]
fn int_float_equality_is_exact_past_2_to_the_53() {
    assert_eq!(
        run("9007199254740993 == 9007199254740992.0"),
        Value::Bool(false)
    );
    assert_eq!(
        run("9007199254740992 == 9007199254740992.0"),
        Value::Bool(true)
    );
    assert_eq!(
        run("9007199254740992.0 == 9007199254740993"),
        Value::Bool(false)
    );
}

#[test]
fn int_float_ordering_is_exact_past_2_to_the_53() {
    assert_eq!(
        run("9007199254740993 < 9007199254740992.0"),
        Value::Bool(false)
    );
    assert_eq!(
        run("9007199254740993 > 9007199254740992.0"),
        Value::Bool(true)
    );
    assert_eq!(
        run("9007199254740993 <= 9007199254740992.0"),
        Value::Bool(false)
    );
    assert_eq!(
        run("9007199254740992.0 < 9007199254740993"),
        Value::Bool(true)
    );
    assert_eq!(
        run("9007199254740992.0 >= 9007199254740993"),
        Value::Bool(false)
    );
    assert_eq!(
        run("9007199254740992 < 9007199254740994.0"),
        Value::Bool(true)
    );
}

#[test]
fn int_float_ordering_handles_fractions_and_signs() {
    assert_eq!(run("3 < 3.5"), Value::Bool(true));
    assert_eq!(run("3 >= 3.5"), Value::Bool(false));
    assert_eq!(run("3.5 > 3"), Value::Bool(true));
    assert_eq!(run("3 == 3.5"), Value::Bool(false));
    assert_eq!(run("1 == 1.0"), Value::Bool(true));
    assert_eq!(run("-3 < -2.5"), Value::Bool(true));
    assert_eq!(run("-3 > -3.5"), Value::Bool(true));
}

#[test]
fn int_max_is_below_two_to_the_63_float() {
    assert_eq!(
        run("9223372036854775807 < 9223372036854775808.0"),
        Value::Bool(true)
    );
    assert_eq!(
        run("9223372036854775807 == 9223372036854775808.0"),
        Value::Bool(false)
    );
}

#[test]
fn match_on_an_int_does_not_take_a_rounded_float_literal() {
    assert_eq!(
        run(r#"match 9007199254740993 { 9007199254740992.0 => "eq", _ => "ne" }"#),
        text("ne")
    );
    assert_eq!(run(r#"match 3 { 3.0 => "eq", _ => "ne" }"#), text("eq"));
}

#[test]
fn cmp_int_float_covers_infinities_nan_and_range_edges() {
    use iron_shell::value::cmp_int_float;
    assert_eq!(cmp_int_float(i64::MAX, f64::INFINITY), Some(Ordering::Less));
    assert_eq!(
        cmp_int_float(i64::MIN, f64::NEG_INFINITY),
        Some(Ordering::Greater)
    );
    assert_eq!(cmp_int_float(0, f64::NAN), None);
    assert_eq!(
        cmp_int_float(i64::MIN, -9_223_372_036_854_775_808.0),
        Some(Ordering::Equal)
    );
    assert_eq!(
        cmp_int_float(i64::MIN, -9_223_372_036_854_777_856.0),
        Some(Ordering::Greater)
    );
    assert_eq!(
        cmp_int_float(i64::MAX, 9_223_372_036_854_775_808.0),
        Some(Ordering::Less)
    );
    assert_eq!(cmp_int_float(-1, -0.5), Some(Ordering::Less));
    assert_eq!(cmp_int_float(-1, -1.5), Some(Ordering::Greater));
}

// ---- `\U` with four to seven hex digits is refused, not kept as text ----

fn try_run(src: &str) -> Result<Value, String> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).map_err(|e| format!("{e:#}"))?;
    iron_shell::eval::eval_program(&stmts, &mut env).map_err(|e| format!("{e:#}"))
}

#[test]
fn a_long_unicode_escape_with_four_to_seven_digits_is_e_parse() {
    for src in [
        r#""x\U0001F60 y""#,
        r#""\U000041""#,
        r#""\U0000041""#,
        r#""\U0001F60""#,
    ] {
        let msg = try_run(src).expect_err("must be refused");
        assert!(msg.contains("E_PARSE"), "{src}: {msg}");
        assert!(msg.contains("invalid unicode escape"), "{src}: {msg}");
        assert!(msg.contains("UHHHHHHHH"), "{src}: {msg}");
    }
}

#[test]
fn a_backslash_u_with_fewer_than_four_digits_stays_literal() {
    assert_eq!(try_run(r#""C:\UEFI""#), Ok(text("C:\\UEFI")));
    assert_eq!(try_run(r#""C:\Users""#), Ok(text("C:\\Users")));
    assert_eq!(try_run(r#""\U041""#), Ok(text("\\U041")));
    assert_eq!(try_run(r#""\U""#), Ok(text("\\U")));
}

// ---- math_pow's refusals name the operands compactly ----

#[test]
fn math_pow_overflow_message_uses_the_exponent_form_for_huge_operands() {
    let msg = try_run("math.pow(-2, 1e300)").expect_err("must be refused");
    assert!(
        msg.contains("math_pow: -2 ** 1e300 overflows a Float"),
        "{msg}"
    );
    assert!(msg.len() < 300, "{msg}");
    // Small operands keep their plain form, which is the one round 28 pins.
    let msg = try_run("math.pow(10.0, 400)").expect_err("must be refused");
    assert!(
        msg.contains("math_pow: 10 ** 400 overflows a Float"),
        "{msg}"
    );
}

#[test]
fn math_pow_nan_message_names_its_operands() {
    let msg = try_run("math.pow(-2, 0.5)").expect_err("must be refused");
    assert!(
        msg.contains("math_pow: -2 ** 0.5 is not a real number"),
        "{msg}"
    );
}

// ---- YAML 1.1 readers would read these as numbers or dates: they are quoted ----

#[test]
fn to_yaml_quotes_strings_that_yaml_11_reads_as_numbers_or_dates() {
    let out = run(r#"["1_000", "2026-01-02", "0b101", "+12", ".5", "1:20"] | to_yaml"#);
    let Value::Str(text) = out else {
        panic!("to_yaml gives a String")
    };
    for quoted in [
        "\"1_000\"",
        "\"2026-01-02\"",
        "\"0b101\"",
        "\"+12\"",
        "\".5\"",
        "\"1:20\"",
    ] {
        assert!(text.contains(quoted), "{quoted} not in {text}");
    }
}

#[test]
fn yaml_strings_that_look_like_numbers_round_trip_as_strings() {
    let v = run(r#"["1_000", "2026-01-02", "0b101", "+12", ".5", "1:20"] | to_yaml | from_yaml"#);
    assert_eq!(
        v,
        Value::Array(
            ["1_000", "2026-01-02", "0b101", "+12", ".5", "1:20"]
                .iter()
                .map(|s| text(s))
                .collect()
        )
    );
}

// ---- a stray backslash names the likely earlier `\"` ----

#[test]
fn a_stray_backslash_hints_at_an_earlier_string_that_ended_early() {
    // `"a\""` is one string, `a"`, five characters long, so the backslash at column 7
    // is outside any string.
    let msg = try_run(r#""a\"" \ "b""#).expect_err("must be refused");
    assert!(msg.contains("E_PARSE"), "{msg}");
    assert!(msg.contains("at line 1, column 7"), "{msg}");
    assert!(
        msg.contains("an earlier string may have ended early"),
        "{msg}"
    );
}

// ---- to_csv keeps a row whose only field is empty ----

#[test]
fn to_csv_keeps_an_empty_single_field_row_through_from_csv() {
    let v = run(r#"[{a: "x"}, {a: ""}, {a: "y"}] | to_csv | from_csv | len"#);
    assert_eq!(v, Value::Int(3));
    let v = run(r#"[{a: "x"}, {a: ""}, {a: "y"}] | to_csv"#);
    assert_eq!(v, text("a\nx\n\"\"\ny\n"));
}

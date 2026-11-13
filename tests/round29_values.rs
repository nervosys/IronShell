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

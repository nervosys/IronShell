//! Round 19: date_add's weeks, months and years, the sign date_diff answers,
//! and the hint for a number mixed with an empty String.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

fn text(s: &str) -> Value {
    Value::Str(s.to_string())
}

// --- date_add: weeks, months and years ----------------------------------------

#[test]
fn date_add_moves_by_weeks() {
    assert_eq!(
        eval(r#"date_add("2024-02-28", 1, "weeks")"#).unwrap(),
        text("2024-03-06")
    );
    assert_eq!(
        eval(r#"date_add("2024-03-06", -2, "weeks")"#).unwrap(),
        text("2024-02-21")
    );
}

#[test]
fn date_add_clamps_a_month_to_the_last_day_of_the_target_month() {
    assert_eq!(
        eval(r#"date_add("2024-01-31", 1, "months")"#).unwrap(),
        text("2024-02-29")
    );
    assert_eq!(
        eval(r#"date_add("2024-03-31", -1, "months")"#).unwrap(),
        text("2024-02-29")
    );
    assert_eq!(
        eval(r#"date_add("2023-01-31", 1, "months")"#).unwrap(),
        text("2023-02-28")
    );
    assert_eq!(
        eval(r#"date_add("2024-01-15", 1, "months")"#).unwrap(),
        text("2024-02-15")
    );
}

#[test]
fn date_add_moves_by_months_across_a_year_boundary() {
    assert_eq!(
        eval(r#"date_add("2024-11-30", 3, "months")"#).unwrap(),
        text("2025-02-28")
    );
    assert_eq!(
        eval(r#"date_add("2024-01-10", -1, "months")"#).unwrap(),
        text("2023-12-10")
    );
}

#[test]
fn date_add_moves_by_years_and_clamps_leap_days() {
    assert_eq!(
        eval(r#"date_add("2024-02-29", 1, "years")"#).unwrap(),
        text("2025-02-28")
    );
    assert_eq!(
        eval(r#"date_add("2024-02-29", 4, "years")"#).unwrap(),
        text("2028-02-29")
    );
    assert_eq!(
        eval(r#"date_add("2024-06-01", -2, "years")"#).unwrap(),
        text("2022-06-01")
    );
}

#[test]
fn date_add_keeps_the_time_of_day_for_months_and_years() {
    assert_eq!(
        eval(r#"date_add("2024-01-31T10:30:00Z", 1, "months")"#).unwrap(),
        text("2024-02-29T10:30:00Z")
    );
    assert_eq!(
        eval(r#"date_add("2024-02-29T23:59:59Z", 1, "years")"#).unwrap(),
        text("2025-02-28T23:59:59Z")
    );
}

#[test]
fn date_add_answers_a_date_for_calendar_units_and_refuses_unknown_units() {
    let answer = eval(r#"date_add("2024-01-31", 1, "months")"#).unwrap();
    assert!(
        matches!(&answer, Value::Str(s) if !s.contains('T')),
        "{answer}"
    );
    let err = eval(r#"date_add("2024-01-31", 1, "fortnights")"#).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("weeks, days, months, years"), "{msg}");
}

// --- date_diff: b minus a ----------------------------------------------------

#[test]
fn date_diff_answers_b_minus_a_so_a_reversed_pair_is_negative() {
    assert_eq!(
        eval(r#"date_diff("2024-02-29", "2026-11-09")"#).unwrap(),
        Value::Int(984)
    );
    assert_eq!(
        eval(r#"date_diff("2026-11-09", "2024-02-29")"#).unwrap(),
        Value::Int(-984)
    );
}

// --- a number and an empty String ---------------------------------------------

fn refused(src: &str) -> iron_shell::safety::SafetyError {
    let err = eval(src).expect_err("a number and a String must not combine");
    err.downcast_ref::<iron_shell::safety::SafetyError>()
        .expect("a structured refusal")
        .clone()
}

#[test]
fn an_empty_string_in_arithmetic_is_named_with_a_filter_hint() {
    let se = refused("1.5 + \"\"");
    assert_eq!(se.message, "operator Add: cannot combine Float and String");
    assert!(se.hint.contains("the String is empty"), "hint: {}", se.hint);
    assert!(se.hint.contains("where(.x != \"\")"), "hint: {}", se.hint);
    let se = refused("\"\" + 2");
    assert!(se.hint.contains("the String is empty"), "hint: {}", se.hint);
}

#[test]
fn an_empty_string_in_a_comparison_is_named_with_a_filter_hint() {
    let se = refused("1.5 >= \"\"");
    assert!(se.hint.contains("the String is empty"), "hint: {}", se.hint);
    let se = refused("3 < \"\"");
    assert!(se.hint.contains("the String is empty"), "hint: {}", se.hint);
    let se = refused("\"\" > 2.5");
    assert!(se.hint.contains("the String is empty"), "hint: {}", se.hint);
}

#[test]
fn a_non_empty_string_keeps_the_conversion_hint() {
    let se = refused("1 < \"a\"");
    assert!(!se.hint.contains("empty"), "hint: {}", se.hint);
    assert!(se.hint.contains("int(x)"), "hint: {}", se.hint);
    let se = refused("1 + \"a\"");
    assert!(!se.hint.contains("empty"), "hint: {}", se.hint);
}

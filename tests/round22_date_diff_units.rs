//! Round 22: date_diff counts weeks, and calendar months and years as the
//! inverse of date_add.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

fn int(src: &str) -> i64 {
    match eval(src).unwrap_or_else(|e| panic!("{src}: {e}")) {
        Value::Int(n) => n,
        other => panic!("{src}: {other:?}"),
    }
}

#[test]
fn weeks_are_whole_seven_day_periods() {
    assert_eq!(int(r#"date_diff("2024-01-01", "2024-01-15", "weeks")"#), 2);
    assert_eq!(int(r#"date_diff("2024-01-01", "2024-01-14", "weeks")"#), 1);
    assert_eq!(int(r#"date_diff("2024-01-15", "2024-01-01", "weeks")"#), -2);
}

#[test]
fn months_are_the_inverse_of_date_add() {
    assert_eq!(int(r#"date_diff("2024-01-31", "2024-02-29", "months")"#), 1);
    assert_eq!(int(r#"date_diff("2024-01-31", "2024-02-28", "months")"#), 0);
    assert_eq!(int(r#"date_diff("2024-01-15", "2024-03-14", "months")"#), 1);
    assert_eq!(int(r#"date_diff("2024-01-15", "2024-03-15", "months")"#), 2);
    assert_eq!(
        int(r#"date_diff("2024-03-15", "2024-01-20", "months")"#),
        -1
    );
    assert_eq!(
        int(r#"date_diff("2024-03-15", "2024-01-15", "months")"#),
        -2
    );
}

#[test]
fn years_are_twelve_calendar_months() {
    assert_eq!(int(r#"date_diff("2024-02-29", "2025-02-28", "years")"#), 1);
    assert_eq!(int(r#"date_diff("2020-06-01", "2026-05-31", "years")"#), 5);
    assert_eq!(int(r#"date_diff("2026-06-01", "2020-06-01", "years")"#), -6);
}

#[test]
fn a_bad_unit_names_the_calendar_units() {
    let e = eval(r#"date_diff("2024-01-01", "2024-02-01", "fortnights")"#).expect_err("bad unit");
    let text = e.to_string();
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("months"), "{text}");
}

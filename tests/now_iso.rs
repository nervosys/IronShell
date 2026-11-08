//! `now_iso()` is the current time as an ISO 8601 / RFC 3339 String, to the
//! second: `2026-11-08T14:03:05Z` in UTC, or with "local" the local time and
//! its offset from UTC.
//!
//! The shape is checked, not the value: the clock is different on every run.

use iron_shell::env::Env;
use iron_shell::safety::{ErrorCode, SafetyError};
use iron_shell::value::Value;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn run(src: &str) -> anyhow::Result<Value> {
    let mut env = Env::new();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn text(src: &str) -> String {
    match run(src).unwrap_or_else(|e| panic!("{src}: {e}")) {
        Value::Str(s) => s,
        other => panic!("{src} did not return a String: {other:?}"),
    }
}

/// `YYYY-MM-DDTHH:MM:SS` with digits where digits go.
fn has_the_date_and_time_shape(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 19
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && b[10] == b'T'
        && b[11..13].iter().all(u8::is_ascii_digit)
        && b[13] == b':'
        && b[14..16].iter().all(u8::is_ascii_digit)
        && b[16] == b':'
        && b[17..19].iter().all(u8::is_ascii_digit)
}

#[test]
fn the_default_is_utc_to_the_second_with_z() {
    let _g = lock();
    let s = text("now_iso()");
    assert!(has_the_date_and_time_shape(&s), "{s}");
    assert_eq!(s.len(), 20, "no fractional seconds: {s}");
    assert!(s.ends_with('Z'), "{s}");
}

#[test]
fn utc_named_is_the_same_form_as_the_default() {
    let _g = lock();
    let s = text("now_iso(\"utc\")");
    assert!(has_the_date_and_time_shape(&s), "{s}");
    assert!(s.ends_with('Z'), "{s}");
}

#[test]
fn local_carries_the_offset_from_utc() {
    let _g = lock();
    let s = text("now_iso(\"local\")");
    assert!(has_the_date_and_time_shape(&s), "{s}");
    assert_eq!(s.len(), 25, "{s}");
    let offset = &s[19..];
    assert!(offset.starts_with('+') || offset.starts_with('-'), "{s}");
    assert_eq!(offset.as_bytes()[3], b':', "{s}");
}

#[test]
fn the_year_is_a_plausible_clock_reading() {
    let _g = lock();
    let year: i64 = text("now_iso()")[0..4].parse().expect("a year");
    assert!((2020..=2200).contains(&year), "{year}");
}

#[test]
fn an_unknown_zone_is_refused() {
    let _g = lock();
    let err = run("now_iso(\"mars\")").expect_err("refused");
    let se = err
        .downcast_ref::<SafetyError>()
        .expect("a structured refusal");
    assert_eq!(se.code, ErrorCode::BadArg);
}

#[test]
fn piped_input_is_refused() {
    let _g = lock();
    assert!(run("\"x\" | now_iso()").is_err());
}

#[test]
fn the_date_names_other_tools_use_point_at_now_iso() {
    for name in ["date_iso", "iso_date", "today"] {
        let got = iron_shell::builtins::did_you_mean(name);
        assert!(got.contains(&"now_iso".to_string()), "{name}: {got:?}");
    }
}

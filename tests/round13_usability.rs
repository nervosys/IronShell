//! Four usability defects an agent hit, pinned in-process.
//!
//! - String operands order with `<`, `<=`, `>` and `>=`, by byte.
//! - A missing record field's hint lists the record's keys.
//! - `head(n)` and `tail(n)` take a piped Array as well as a String.
//! - `group_by` takes a lambda key as well as a field name.

use iron_shell::env::Env;
use iron_shell::safety::{ErrorCode, SafetyError};
use iron_shell::value::Value;

fn run(src: &str) -> anyhow::Result<Value> {
    let mut env = Env::new();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn ok(src: &str) -> Value {
    match run(src) {
        Ok(v) => v,
        Err(e) => panic!("{src:?} failed: {e:#}"),
    }
}

fn coded(src: &str) -> SafetyError {
    match run(src) {
        Ok(v) => panic!("{src:?} should fail, produced {v:?}"),
        Err(e) => match e.downcast_ref::<SafetyError>() {
            Some(se) => se.clone(),
            None => panic!("{src:?} failed without a structured error: {e:#}"),
        },
    }
}

fn ints(v: &[i64]) -> Value {
    Value::Array(v.iter().map(|i| Value::Int(*i)).collect())
}

// ---- 1. String ordering ----

#[test]
fn strings_compare_lexicographically_with_each_ordering_operator() {
    assert_eq!(ok(r#""b" >= "a""#), Value::Bool(true));
    assert_eq!(ok(r#""a" < "b""#), Value::Bool(true));
    assert_eq!(ok(r#""b" > "a""#), Value::Bool(true));
    assert_eq!(ok(r#""a" <= "a""#), Value::Bool(true));
    assert_eq!(ok(r#""b" <= "a""#), Value::Bool(false));
    assert_eq!(ok(r#""abc" < "abd""#), Value::Bool(true));
}

#[test]
fn strings_order_by_byte_so_uppercase_sorts_before_lowercase() {
    // 'B' is 0x42 and 'a' is 0x61, so "B" < "a" as Rust's str Ord says.
    assert_eq!(ok(r#""B" < "a""#), Value::Bool(true));
    // A shorter prefix sorts first.
    assert_eq!(ok(r#""ab" < "abc""#), Value::Bool(true));
}

#[test]
fn a_string_and_a_number_still_do_not_order_and_the_hint_names_the_fix() {
    let e = coded(r#""a" < 1"#);
    assert_eq!(e.code.as_str(), ErrorCode::BadArg.as_str());
    assert!(
        e.hint.contains("compare two numbers or two strings"),
        "{}",
        e.hint
    );
    assert!(!e.hint.contains("operands it applies to"), "{}", e.hint);
}

// ---- 2. Unknown field hint lists the record's keys ----

#[test]
fn a_missing_field_hint_lists_the_record_keys_sorted() {
    let e = coded(r#"r = {Name: "a", Count: 1, Group: 2}; r.missing_field"#);
    assert_eq!(e.code.as_str(), ErrorCode::UnknownField.as_str());
    assert!(
        e.hint.ends_with("the record has: Count, Group, Name"),
        "{}",
        e.hint
    );
}

#[test]
fn a_missing_field_hint_stops_at_twelve_keys_then_an_ellipsis() {
    let fields: Vec<String> = (1..=13).map(|i| format!("k{i:02}: {i}")).collect();
    let src = format!("r = {{{}}}; r.missing_field", fields.join(", "));
    let e = coded(&src);
    assert!(
        e.hint.ends_with(
            "the record has: k01, k02, k03, k04, k05, k06, k07, k08, k09, k10, k11, k12, …"
        ),
        "{}",
        e.hint
    );
}

// ---- 3. head and tail on a piped Array ----

#[test]
fn head_of_a_piped_array_is_its_first_n_elements() {
    assert_eq!(ok("[1, 2, 3, 4] | head(3)"), ints(&[1, 2, 3]));
    assert_eq!(ok("[1, 2, 3] | head"), ints(&[1, 2, 3]));
}

#[test]
fn tail_of_a_piped_array_is_its_last_n_elements() {
    assert_eq!(ok("[1, 2, 3, 4] | tail(2)"), ints(&[3, 4]));
    assert_eq!(ok("[1, 2, 3] | tail"), ints(&[1, 2, 3]));
}

#[test]
fn head_and_tail_still_take_a_piped_string_as_lines() {
    assert_eq!(ok(r#""a\nb\nc" | head(2)"#), Value::Str("a\nb".into()));
    assert_eq!(ok(r#""a\nb\nc" | tail(2)"#), Value::Str("b\nc".into()));
}

// ---- 4. group_by with a lambda key ----

fn rows() -> &'static str {
    r#"[{region: "north", v: 1}, {region: "south", v: 2}, {region: "north", v: 3}]"#
}

#[test]
fn group_by_a_dot_field_matches_the_field_name_form() {
    let by_name = ok(&format!(r#"{} | group_by("region")"#, rows()));
    let by_dot = ok(&format!("{} | group_by(.region)", rows()));
    assert_eq!(by_dot, by_name);
}

#[test]
fn group_by_a_lambda_matches_the_field_name_form() {
    let by_name = ok(&format!(r#"{} | group_by("region")"#, rows()));
    let by_lambda = ok(&format!("{} | group_by(fn(r) => r.region)", rows()));
    assert_eq!(by_lambda, by_name);
}

#[test]
fn group_by_a_lambda_makes_one_group_per_distinct_key() {
    assert_eq!(
        ok(&format!("{} | group_by(fn(r) => r.region) | len", rows())),
        Value::Int(2)
    );
    // Groups come out in key order, so the first is "north", with two rows.
    assert_eq!(
        ok(&format!(
            "{} | group_by(fn(r) => r.region) | first | fn(g) => g.Count",
            rows()
        )),
        Value::Int(2)
    );
}

#[test]
fn group_by_still_refuses_a_key_that_is_neither_a_name_nor_a_lambda() {
    let e = coded(&format!("{} | group_by(42)", rows()));
    assert_eq!(e.code.as_str(), ErrorCode::BadArg.as_str());
}

//! Round 26: to_yaml and from_yaml. Each test runs in process, either through
//! the yaml module or through the builtins by evaluating ish source.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value, yaml};
use std::collections::BTreeMap;

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

fn rec(pairs: &[(&str, Value)]) -> Value {
    Value::Record(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    )
}

fn empty_record() -> Value {
    Value::Record(BTreeMap::new())
}

// ── item 1: empty collections ───────────────────────────────────────────

#[test]
fn an_empty_record_or_array_field_is_written_in_flow_style() {
    assert_eq!(
        eval(r#"to_yaml({a: null, b: {}, c: []})"#).unwrap(),
        Value::Str("a: null\nb: {}\nc: []\n".to_string())
    );
}

#[test]
fn empty_collections_inside_a_list_are_flow_style_and_read_back() {
    let v = Value::Array(vec![empty_record(), Value::Array(vec![])]);
    let text = yaml::emit(&v);
    assert_eq!(text, "- {}\n- []\n");
    assert_eq!(yaml::parse(&text).unwrap(), v);
}

#[test]
fn empty_collections_round_trip_at_every_level() {
    let v = rec(&[
        ("a", Value::Null),
        ("b", empty_record()),
        ("c", Value::Array(vec![])),
        ("d", Value::Array(vec![empty_record()])),
    ]);
    assert_eq!(yaml::parse(&yaml::emit(&v)).unwrap(), v);
    assert_eq!(
        yaml::parse(&yaml::emit(&empty_record())).unwrap(),
        empty_record()
    );
    assert_eq!(
        yaml::parse(&yaml::emit(&Value::Array(vec![]))).unwrap(),
        Value::Array(vec![])
    );
}

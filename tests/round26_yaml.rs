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

// ── item 2: floats ──────────────────────────────────────────────────────

#[test]
fn a_whole_float_keeps_its_point_and_reads_back_as_a_float() {
    assert_eq!(
        eval(r#"to_yaml({f: 1.0})"#).unwrap(),
        Value::Str("f: 1.0\n".to_string())
    );
    let v = rec(&[("f", Value::Float(1.0))]);
    assert_eq!(yaml::parse(&yaml::emit(&v)).unwrap(), v);
}

#[test]
fn negative_zero_keeps_its_sign_and_its_float_type() {
    assert_eq!(yaml::emit(&rec(&[("z", Value::Float(-0.0))])), "z: -0.0\n");
    let Value::Record(r) = yaml::parse("z: -0.0\n").unwrap() else {
        panic!("expected a record")
    };
    let Some(Value::Float(z)) = r.get("z") else {
        panic!("expected a Float, got {:?}", r.get("z"))
    };
    assert!(z.is_sign_negative() && *z == 0.0);
}

#[test]
fn exponent_form_floats_keep_a_point() {
    let text = yaml::emit(&rec(&[("big", Value::Float(1e20))]));
    assert_eq!(text, "big: 1.0e20\n");
    assert_eq!(
        yaml::parse(&text).unwrap(),
        rec(&[("big", Value::Float(1e20))])
    );
}

#[test]
fn special_floats_read_as_floats_in_every_yaml_1_2_spelling() {
    for (text, expect) in [
        (".inf", f64::INFINITY),
        (".Inf", f64::INFINITY),
        (".INF", f64::INFINITY),
        ("+.inf", f64::INFINITY),
        ("-.inf", f64::NEG_INFINITY),
        ("-.Inf", f64::NEG_INFINITY),
        ("-.INF", f64::NEG_INFINITY),
    ] {
        let v = yaml::parse(&format!("x: {text}\n")).unwrap();
        assert_eq!(v, rec(&[("x", Value::Float(expect))]), "reading {text}");
    }
    for text in [".nan", ".NaN", ".NAN"] {
        let Value::Record(r) = yaml::parse(&format!("x: {text}\n")).unwrap() else {
            panic!("expected a record")
        };
        let Some(Value::Float(x)) = r.get("x") else {
            panic!("{text} should read as a Float")
        };
        assert!(x.is_nan(), "{text}");
    }
}

#[test]
fn infinity_written_by_to_yaml_reads_back_as_a_float() {
    let v = rec(&[
        ("pos", Value::Float(f64::INFINITY)),
        ("neg", Value::Float(f64::NEG_INFINITY)),
    ]);
    let text = yaml::emit(&v);
    assert_eq!(text, "pos: .inf\nneg: -.inf\n");
    assert_eq!(yaml::parse(&text).unwrap(), v);
}

#[test]
fn a_quoted_float_look_alike_stays_a_string() {
    assert_eq!(
        eval(r#"to_yaml({v: "1.0", s: ".inf"})"#).unwrap(),
        Value::Str("s: \".inf\"\nv: \"1.0\"\n".to_string())
    );
    let v = rec(&[
        ("v", Value::Str("1.0".into())),
        ("s", Value::Str(".inf".into())),
    ]);
    assert_eq!(yaml::parse(&yaml::emit(&v)).unwrap(), v);
}

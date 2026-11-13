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

// ── item 3: YAML 1.1 boolean words are quoted ───────────────────────────

#[test]
fn yaml_1_1_boolean_words_are_quoted_on_output() {
    assert_eq!(
        eval(r#"to_yaml({x: "no", z: "yes"})"#).unwrap(),
        Value::Str("x: \"no\"\nz: \"yes\"\n".to_string())
    );
}

#[test]
fn every_yaml_1_1_and_1_2_boolean_spelling_is_quoted_and_reads_back() {
    let words = [
        "y", "Y", "yes", "Yes", "YES", "n", "N", "no", "No", "NO", "on", "On", "ON", "off", "Off",
        "OFF", "true", "True", "TRUE", "false", "False", "FALSE",
    ];
    for w in words {
        let v = rec(&[(w, Value::Str(w.into())), ("v", Value::Str(w.into()))]);
        let text = yaml::emit(&v);
        assert!(text.contains(&format!("v: \"{w}\"\n")), "{w}: {text}");
        assert_eq!(yaml::parse(&text).unwrap(), v, "{w}");
    }
}

#[test]
fn an_unquoted_boolean_word_still_reads_as_a_string() {
    // ish's own reader is YAML 1.2 core: only true/false are booleans.
    assert_eq!(
        yaml::parse("x: yes\nz: off\n").unwrap(),
        rec(&[
            ("x", Value::Str("yes".into())),
            ("z", Value::Str("off".into())),
        ])
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
fn special_floats_are_refused_in_every_yaml_1_2_spelling() {
    // IronShell numbers are finite, so `.inf` and `.nan` are not read as Floats.
    for text in [
        ".inf", ".Inf", ".INF", "+.inf", "-.inf", "-.Inf", "-.INF", ".nan", ".NaN", ".NAN",
    ] {
        let e = yaml::parse(&format!("x: {text}\n")).expect_err(text);
        let message = e.to_string();
        assert!(message.contains("line 1"), "{text}: {message}");
        assert!(message.contains("not a finite number"), "{text}: {message}");
        assert!(
            message.contains("quote it as a string"),
            "{text}: {message}"
        );
    }
}

#[test]
fn a_quoted_special_float_stays_a_string() {
    assert_eq!(
        yaml::parse("x: \".inf\"\n").unwrap(),
        rec(&[("x", Value::Str(".inf".into()))])
    );
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

// ── item 4: double-quoted escapes are decoded ───────────────────────────

#[test]
fn unicode_escapes_in_a_double_quoted_scalar_are_decoded() {
    assert_eq!(
        yaml::parse(r##"s: "\u00e9 \U0001F600""##).unwrap(),
        rec(&[("s", Value::Str("é 😀".into()))])
    );
}

#[test]
fn every_yaml_1_2_escape_is_decoded() {
    let text = r##"s: "\\ \" \/ \n \t \r \0 \b \a \e \x41 \u0042 \U00000043""##;
    assert_eq!(
        yaml::parse(text).unwrap(),
        rec(&[(
            "s",
            Value::Str("\\ \" / \n \t \r \0 \u{8} \u{7} \u{1b} A B C".into())
        )])
    );
}

#[test]
fn a_bad_escape_is_an_error_naming_the_line() {
    for (text, expect) in [
        ("a: 1\nb: \"\\q\"\n", "line 2: unknown escape"),
        ("b: \"\\x4\"\n", "line 1: \\x needs 2 hex digits"),
        ("b: \"\\uD800\"\n", "\\uD800 is not a Unicode code point"),
        ("b: \"\\UFFFFFFFF\"\n", "is not a Unicode code point"),
        ("b: \"ends\\\"\n", "lone backslash"),
    ] {
        let err = yaml::parse(text).unwrap_err().to_string();
        assert!(err.contains(expect), "parsing {text:?}: {err}");
    }
}

#[test]
fn single_quoted_scalars_take_backslashes_literally() {
    assert_eq!(
        yaml::parse(r#"s: 'a\nb\\c''d'"#).unwrap(),
        rec(&[("s", Value::Str("a\\nb\\\\c'd".into()))])
    );
}

#[test]
fn a_backslash_and_a_quote_are_written_as_escapes_and_read_back() {
    let v = rec(&[("q", Value::Str("a\\b\"c".into()))]);
    assert_eq!(yaml::emit(&v), "q: \"a\\\\b\\\"c\"\n");
    assert_eq!(yaml::parse(&yaml::emit(&v)).unwrap(), v);
}

#[test]
fn keys_with_quotes_backslashes_and_newlines_round_trip() {
    let v = rec(&[("k\"e\\y\nx", Value::Int(1))]);
    assert_eq!(yaml::parse(&yaml::emit(&v)).unwrap(), v);
}

#[test]
fn the_whole_round_trip_survives_the_new_decoder() {
    let v = rec(&[
        ("a", Value::Null),
        ("b", empty_record()),
        ("c", Value::Array(vec![])),
        ("f", Value::Float(1.0)),
        ("z", Value::Float(-0.0)),
        ("w", Value::Str("no".into())),
        ("q", Value::Str("a\\b\"c".into())),
        ("u", Value::Str("é😀".into())),
    ]);
    assert_eq!(yaml::parse(&yaml::emit(&v)).unwrap(), v);
}

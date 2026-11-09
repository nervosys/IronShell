//! Postfix indexing: `expr[index]` on Arrays, Strings and Records.
//!
//! Before this, `a[0]` after an identifier parsed as a word call with an array
//! argument, so it failed with "expected a function in `a`", and `[5,6][1]` read
//! as the array literal `[1]`, which returned `[1]` with no error at all.

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

#[test]
fn an_identifier_indexes_an_array() {
    assert_eq!(ok("a = [5, 6]; b = a[1]; b"), Value::Int(6));
    assert_eq!(ok("a = [1, 2, 3]; a[0]"), Value::Int(1));
}

#[test]
fn an_array_literal_indexes_directly() {
    assert_eq!(ok("[5, 6][1]"), Value::Int(6));
}

#[test]
fn a_field_chain_indexes_its_array() {
    assert_eq!(ok("x = {items: [5, 6]}; x.items[1]"), Value::Int(6));
}

#[test]
fn indexing_chains_through_nested_arrays() {
    assert_eq!(ok("a = [[1, 2], [3]]; a[0][1]"), Value::Int(2));
    assert_eq!(ok("[[1, 2], [3]][1][0]"), Value::Int(3));
}

#[test]
fn a_negative_index_counts_from_the_end() {
    assert_eq!(ok("a = [1, 2, 3]; a[-1]"), Value::Int(3));
    assert_eq!(ok("a = [1, 2, 3]; a[-3]"), Value::Int(1));
}

#[test]
fn a_string_index_yields_one_character() {
    assert_eq!(ok("s = \"abc\"; s[1]"), Value::Str("b".to_string()));
    assert_eq!(ok("s = \"abc\"; s[-1]"), Value::Str("c".to_string()));
}

#[test]
fn a_record_index_takes_a_key() {
    assert_eq!(ok("r = {k: 7}; r[\"k\"]"), Value::Int(7));
}

#[test]
fn an_out_of_range_index_is_bad_arg_and_names_the_length() {
    for src in [
        "a = [1, 2, 3]; a[5]",
        "a = [1, 2, 3]; a[-4]",
        "a = []; a[0]",
    ] {
        let e = coded(src);
        assert_eq!(e.code, ErrorCode::BadValue, "{src}");
        assert_eq!(e.code.as_str(), "E_BAD_ARG", "{src}");
        assert!(e.message.contains("out of range"), "{src}: {}", e.message);
        assert!(
            e.hint.contains("length") || e.hint.contains("len(x) > 0"),
            "{src}: {}",
            e.hint
        );
    }
    let e = coded("a = [1, 2, 3]; a[5]");
    assert!(e.message.contains("length 3"), "{}", e.message);
    assert_eq!(e.got, "5");
}

#[test]
fn a_missing_record_key_is_unknown_field() {
    let e = coded("r = {k: 7}; r[\"zz\"]");
    assert_eq!(e.code.as_str(), "E_UNKNOWN_FIELD");
}

#[test]
fn an_index_of_the_wrong_type_is_bad_arg() {
    let e = coded("a = [1, 2]; a[\"x\"]");
    assert_eq!(e.code.as_str(), "E_BAD_ARG");
    assert!(e.got.contains("String"), "{}", e.got);
}

#[test]
fn a_space_before_the_bracket_is_still_a_word_call_argument() {
    // `f [1, 2]` passes an array; only `f[1, 2]` (no space) is an index.
    assert_eq!(
        ok("f = fn(x) => x; f [1, 2]"),
        Value::Array(vec![Value::Int(1), Value::Int(2)])
    );
}

#[test]
fn an_array_on_the_next_line_is_a_new_statement() {
    assert_eq!(
        ok("a = 1\n[2, 3]"),
        Value::Array(vec![Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn nested_array_literals_still_parse() {
    assert_eq!(
        ok("[[1, 2], [3]]"),
        Value::Array(vec![
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
            Value::Array(vec![Value::Int(3)]),
        ])
    );
}

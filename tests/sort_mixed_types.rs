//! `sort` refuses an array whose elements have no order between them.
//!
//! `[3, "a", 1] | sort` used to return `[3, "a", 1]` with exit 0: the comparator
//! called a number and a string equal, so the input order stayed. Such an array
//! has no sorted form, so it is an `E_BAD_ARG` that names the types. Arrays of
//! one orderable type sort as before, and so does a mix of ints and floats.

use iron_shell::env::Env;
use iron_shell::safety::{ErrorCode, SafetyError};
use iron_shell::value::Value;
use iron_shell::{eval, parser};

fn run(src: &str) -> anyhow::Result<Value> {
    let mut env = Env::new();
    let stmts = parser::parse_program(src)?;
    eval::eval_program(&stmts, &mut env)
}

#[test]
fn a_mix_of_numbers_and_strings_is_refused_and_names_both() {
    let e = run("[3, \"a\", 1] | sort").expect_err("a mixed array has no sorted form");
    let se = e
        .downcast_ref::<SafetyError>()
        .unwrap_or_else(|| panic!("structured error expected: {e:#}"));
    assert_eq!(se.code, ErrorCode::BadArg);
    assert!(
        se.got.contains("number") && se.got.contains("String"),
        "{}",
        se.got
    );
    assert!(se.message.contains("a mix of"), "{}", se.message);
}

#[test]
fn a_mix_of_kinds_other_than_numbers_and_strings_is_refused() {
    assert!(run("[1, null] | sort").is_err());
    assert!(run("[\"x\", true] | sort").is_err());
}

#[test]
fn strings_sort_as_before() {
    assert_eq!(
        run("[\"b\", \"c\", \"a\"] | sort").unwrap(),
        Value::Array(vec![
            Value::Str("a".into()),
            Value::Str("b".into()),
            Value::Str("c".into()),
        ])
    );
}

#[test]
fn ints_and_floats_together_still_sort_numerically() {
    assert_eq!(
        run("[3, 1.5, 2] | sort").unwrap(),
        Value::Array(vec![Value::Float(1.5), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn an_empty_or_single_element_array_is_sorted_as_before() {
    assert_eq!(run("[] | sort").unwrap(), Value::Array(vec![]));
    assert_eq!(
        run("[\"a\"] | sort").unwrap(),
        Value::Array(vec![Value::Str("a".into())])
    );
}

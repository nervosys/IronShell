//! `int(x)` and `float(x)` convert between text and numbers, and refuse what
//! has no value of the target type rather than guessing.
//!
//! `int` takes a whole number: a Float is truncated toward zero, a Bool is 1 or
//! 0, and text must be a whole number. `float` takes a decimal. A refusal is
//! `E_BAD_VALUE`, names the text, and says which conversion parses it.

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

fn refusal(src: &str) -> SafetyError {
    let err = run(src).expect_err("the conversion must be refused");
    err.downcast_ref::<SafetyError>()
        .expect("a structured refusal")
        .clone()
}

fn is_true(src: &str) -> bool {
    run(src).unwrap_or_else(|e| panic!("{src}: {e}")) == Value::Bool(true)
}

#[test]
fn int_of_text_parses_a_whole_number() {
    let _g = lock();
    assert_eq!(run("int(\"42\")").unwrap(), Value::Int(42));
    assert_eq!(run("int(\"  -7 \")").unwrap(), Value::Int(-7));
    assert_eq!(run("int(\"+5\")").unwrap(), Value::Int(5));
}

#[test]
fn int_of_a_float_truncates_toward_zero() {
    let _g = lock();
    assert_eq!(run("int(3.9)").unwrap(), Value::Int(3));
    assert_eq!(run("int(-3.9)").unwrap(), Value::Int(-3));
}

#[test]
fn int_of_a_bool_is_one_or_zero() {
    let _g = lock();
    assert_eq!(run("int(true)").unwrap(), Value::Int(1));
    assert_eq!(run("int(false)").unwrap(), Value::Int(0));
}

#[test]
fn int_of_an_int_is_itself() {
    let _g = lock();
    assert_eq!(run("int(7)").unwrap(), Value::Int(7));
}

#[test]
fn int_takes_its_operand_from_the_pipeline() {
    let _g = lock();
    assert_eq!(run("\"12\" | int()").unwrap(), Value::Int(12));
}

#[test]
fn int_refuses_text_that_is_not_a_whole_number() {
    let _g = lock();
    for text in ["1.5", "abc", ""] {
        let src = format!("int(\"{text}\")");
        let se = refusal(&src);
        assert_eq!(se.code, ErrorCode::Overflow, "{src}");
        assert_eq!(se.code.as_str(), "E_BAD_VALUE", "{src}");
        assert!(
            se.message.contains(&format!("{text:?}")),
            "{src}: {}",
            se.message
        );
        assert!(
            se.hint.contains("float(\"1.5\") parses decimals"),
            "{src}: {}",
            se.hint
        );
        assert!(!se.is_retryable(), "{src}");
    }
}

#[test]
fn int_refuses_a_float_with_no_int_value() {
    let _g = lock();
    // 1e19 is past the 64-bit Int range; a truncation would wrap or saturate.
    let se = refusal("int(float(\"1e19\"))");
    assert_eq!(se.code.as_str(), "E_BAD_VALUE");
    assert_eq!(
        refusal("int(float(\"-1e19\"))").code.as_str(),
        "E_BAD_VALUE"
    );
}

#[test]
fn float_of_text_parses_a_decimal_and_an_exponent() {
    let _g = lock();
    assert!(is_true("float(\"1e3\") == 1000.0"));
    assert!(is_true("float(\"-2.5\") == -2.5"));
    assert!(is_true("float(\" 0.25 \") == 0.25"));
}

#[test]
fn float_widens_an_int() {
    let _g = lock();
    assert!(is_true("float(2) == 2.0"));
}

#[test]
fn float_refuses_text_that_is_not_a_finite_number() {
    let _g = lock();
    for text in ["abc", "", "inf", "NaN"] {
        let src = format!("float(\"{text}\")");
        let se = refusal(&src);
        assert_eq!(se.code.as_str(), "E_BAD_VALUE", "{src}");
        assert!(
            se.message.contains(&format!("{text:?}")),
            "{src}: {}",
            se.message
        );
    }
}

#[test]
fn a_conversion_takes_one_value() {
    let _g = lock();
    assert_eq!(refusal("int(1, 2)").code, ErrorCode::BadArg);
    assert_eq!(refusal("float(\"1\", 2)").code, ErrorCode::BadArg);
}

#[test]
fn int_refuses_a_value_that_is_neither_text_nor_a_number() {
    let _g = lock();
    assert_eq!(refusal("int([1, 2])").code, ErrorCode::BadArg);
}

#[test]
fn the_names_other_languages_use_point_at_int_and_float() {
    assert!(iron_shell::builtins::did_you_mean("to_int").contains(&"int".to_string()));
    assert!(iron_shell::builtins::did_you_mean("parse_int").contains(&"int".to_string()));
    assert!(iron_shell::builtins::did_you_mean("to_float").contains(&"float".to_string()));
    let number = iron_shell::builtins::did_you_mean("number");
    assert!(number.contains(&"int".to_string()), "{number:?}");
    assert!(number.contains(&"float".to_string()), "{number:?}");
    assert!(iron_shell::builtins::did_you_mean("to_number").contains(&"float".to_string()));
}

#[test]
fn mixing_text_and_a_number_names_the_conversions() {
    let _g = lock();
    let se = refusal("\"a\" + 1");
    assert!(se.hint.contains("int(x)"), "{}", se.hint);
    assert!(se.hint.contains("float(x)"), "{}", se.hint);
}

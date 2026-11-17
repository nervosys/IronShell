//! Round 51a: language and data fixes (aggregates, group_by, hints).

use iron_shell::safety::{ErrorCode, SafetyError};
use iron_shell::value::Value;

fn try_run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn run(src: &str) -> Value {
    try_run(src).expect("runs")
}

fn refusal(src: &str) -> SafetyError {
    let err = try_run(src).expect_err(src);
    err.downcast_ref::<SafetyError>()
        .unwrap_or_else(|| panic!("{src}: not a SafetyError: {err}"))
        .clone()
}

// A1
#[test]
fn a1_float_aggregates_refuse_a_non_finite_result() {
    for src in [
        "sum([1e308, 1e308])",
        "avg([1e308, 1e308])",
        "mean([1e308, 1e308])",
        "math.sum([1e308, 1e308])",
        "product([1e308, 10.0])",
    ] {
        let se = refusal(src);
        assert_eq!(se.code, ErrorCode::Overflow, "{src}");
        assert_eq!(se.code.as_str(), "E_BAD_VALUE", "{src}");
        assert!(
            se.message.contains("float overflow"),
            "{src}: {}",
            se.message
        );
    }
    assert_eq!(run("sum([1.5, 2.5])"), Value::Float(4.0));
}

// A2
#[test]
fn a2_int_overflow_in_sum_is_e_bad_value() {
    for src in [
        "sum([9223372036854775807, 1])",
        "math.sum([9223372036854775807, 1])",
        "product([9223372036854775807, 2])",
    ] {
        let se = refusal(src);
        assert_eq!(se.code.as_str(), "E_BAD_VALUE", "{src}");
        assert!(se.message.contains("integer overflow"), "{src}");
    }
}

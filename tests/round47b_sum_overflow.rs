//! `sum` and `math.sum` raise on Int overflow, as `product` and `+` do; they
//! wrapped silently before.

use iron_shell::value::Value;

fn try_run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn run(src: &str) -> Value {
    try_run(src).expect("runs")
}

#[test]
fn sum_refuses_an_int_overflow() {
    for src in [
        "m = 9223372036854775807
sum([m, 1])",
        "m = 9223372036854775807
sum([m, m])",
        "m = 9223372036854775807
[m, 1] | sum",
        "m = 9223372036854775807
math.sum([m, 1])",
    ] {
        let err = try_run(src).expect_err(src);
        let text = err.to_string();
        assert!(text.contains("sum within Int range"), "{src}: {text}");
        assert!(text.contains("integer overflow"), "{src}: {text}");
    }
}

#[test]
fn sum_still_adds_in_range() {
    assert_eq!(run("sum([1, 2, 3])"), Value::Int(6));
    assert_eq!(
        run("m = 9223372036854775807
sum([m, -1, 1])"),
        Value::Int(i64::MAX)
    );
}

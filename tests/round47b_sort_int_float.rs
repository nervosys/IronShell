//! `sort` and `sort_by` order an Int against a Float exactly, as `>` does,
//! not through a lossy f64 cast.

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
fn sort_orders_a_large_int_against_an_equal_looking_float() {
    assert_eq!(
        run("9007199254740993 > 9007199254740992.0"),
        Value::Bool(true)
    );
    let want = Value::Array(vec![
        Value::Float(9007199254740992.0),
        Value::Int(9007199254740993),
    ]);
    assert_eq!(run("sort([9007199254740993, 9007199254740992.0])"), want);
    assert_eq!(run("sort([9007199254740992.0, 9007199254740993])"), want);
}

#[test]
fn sort_by_orders_a_large_int_against_an_equal_looking_float() {
    let want = Value::Array(vec![
        Value::Float(9007199254740992.0),
        Value::Int(9007199254740993),
    ]);
    assert_eq!(
        run("[9007199254740993, 9007199254740992.0] | sort_by(fn(x) => x)"),
        want
    );
    assert_eq!(
        run("[9007199254740992.0, 9007199254740993] | sort_by(fn(x) => x, \"desc\")"),
        Value::Array(vec![
            Value::Int(9007199254740993),
            Value::Float(9007199254740992.0)
        ])
    );
}

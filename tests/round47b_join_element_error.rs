//! `join` names the index and type of the element it cannot join.

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
fn join_names_the_offending_element() {
    let err = try_run("join([\"a\", 1, null], \",\")").expect_err("refused");
    let text = err.to_string();
    assert!(text.contains("Null at index 2"), "{text}");
}

#[test]
fn join_still_joins_scalars() {
    assert_eq!(
        run("join([\"a\", 1, true], \",\")"),
        Value::Str("a,1,true".to_string())
    );
}

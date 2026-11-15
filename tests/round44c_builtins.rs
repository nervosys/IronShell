//! Round 44c: piped forms, declarations that match the run, and error names.

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
fn pow_takes_its_base_from_the_pipe() {
    assert_eq!(run("2 | pow(3)"), Value::Int(8));
    assert_eq!(run("2 | math.pow(3)"), Value::Int(8));
    assert_eq!(run("pow(2, 3)"), Value::Int(8));
}

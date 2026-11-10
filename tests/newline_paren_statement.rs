//! A `(` at the start of a line starts a new statement. It used to continue
//! the line before as a call: `x = 5` then `(x + 1) * 2` parsed as
//! `x = 5(x + 1) * 2` and failed with "unknown name `x`".

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

#[test]
fn a_paren_on_a_new_line_is_a_new_statement() {
    assert_eq!(run("x = 5\n(x + 1) * 2"), Value::Int(12));
    assert_eq!(run("f = fn(a) => a * 10\n(f(3))"), Value::Int(30));
    assert_eq!(run("r = {a: 1}\n(r | merge({b: 2})).b"), Value::Int(2));
}

#[test]
fn a_call_on_one_line_is_still_a_call() {
    assert_eq!(run("f = fn(a) => a + 1\nf(1)"), Value::Int(2));
    assert_eq!(run("f = fn(a, b) => a + b\nf(1,\n  2)"), Value::Int(3));
    assert_eq!(run("(fn(a) => a * 2)(4)"), Value::Int(8));
}

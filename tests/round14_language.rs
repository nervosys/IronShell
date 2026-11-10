//! Round-14 language fixes, one group per item: mixed Int/Float ordering,
//! split on the empty separator, `else if`, parser hints for `?:` and `?.`,
//! ontology_describe on an unknown name, and the merge and format_number
//! builtins.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn run_err(src: &str) -> String {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = match iron_shell::parser::parse_program(src) {
        Ok(s) => s,
        Err(e) => return format!("{e:?}"),
    };
    match iron_shell::eval::eval_program(&stmts, &mut env) {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => format!("{e:?}"),
    }
}

// ---- 1. Mixed Int/Float ordering -------------------------------------------

#[test]
fn an_int_and_a_float_order_against_each_other() {
    assert_eq!(run("1.5 < 0"), Value::Bool(false));
    assert_eq!(run("1 < 1.5"), Value::Bool(true));
    assert_eq!(run("1.5 > 1"), Value::Bool(true));
    assert_eq!(run("2 >= 1.5"), Value::Bool(true));
    assert_eq!(run("1.0 <= 1"), Value::Bool(true));
    assert_eq!(run("1 > 1.0"), Value::Bool(false));
}

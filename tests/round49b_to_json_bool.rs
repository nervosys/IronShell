//! `to_json(x, true)` indents by 2 and `to_json(x, false)` is compact; it used
//! to fail "expected indent: Int".

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

#[test]
fn true_indents_by_two() {
    assert_eq!(
        run(r#"to_json({a: 1}, true)"#),
        Value::Str("{\n  \"a\": 1\n}".to_string())
    );
}

#[test]
fn false_is_compact() {
    assert_eq!(
        run(r#"to_json({a: 1}, false)"#),
        Value::Str("{\"a\":1}".to_string())
    );
}

#[test]
fn piped_value_takes_the_bool_too() {
    assert_eq!(
        run(r#"[1, 2] | to_json(true) | lines | len"#),
        Value::Int(4)
    );
}

#[test]
fn an_int_indent_still_works_and_is_range_checked() {
    assert_eq!(run(r#"to_json([1], 0)"#), Value::Str("[1]".to_string()));
    assert_eq!(
        run(r#"(try { to_json([1], 9) } catch e { e.error.code })"#),
        Value::Str("E_BAD_ARG".to_string())
    );
}

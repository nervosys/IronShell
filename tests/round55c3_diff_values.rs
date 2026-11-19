//! diff_values(a, b): the structural difference of two values as
//! {path, op, old, new} rows.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn strs(v: &[&str]) -> Value {
    Value::Array(v.iter().map(|s| Value::Str((*s).to_string())).collect())
}

#[test]
fn rows_name_the_path_and_the_operation() {
    let v = run(
        r#"diff_values({a: 1, b: {c: 2, d: 5}, items: [1, 2]}, {a: 1, b: {c: 3}, items: [1, 2, 3], z: 0}) | map(fn(r) => r.path + ":" + r.op)"#,
    );
    assert_eq!(
        v,
        strs(&["b.c:changed", "b.d:removed", "items[2]:added", "z:added"])
    );
}

#[test]
fn old_and_new_carry_the_values_and_null_for_the_missing_side() {
    let v = run(r#"diff_values({a: 1}, {b: 2}) | map(fn(r) => [r.path, r.old, r.new])"#);
    assert_eq!(
        v,
        Value::Array(vec![
            Value::Array(vec![Value::Str("a".into()), Value::Int(1), Value::Null]),
            Value::Array(vec![Value::Str("b".into()), Value::Null, Value::Int(2)]),
        ])
    );
}

#[test]
fn an_int_and_the_equal_float_do_not_differ_and_a_pipe_works() {
    assert_eq!(
        run(r#"[{a: 1}] | first | diff_values({a: 1.0}) | len"#),
        Value::Int(0)
    );
    assert_eq!(
        run(r#"diff_values(1, 2) | map(fn(r) => r.path)"#),
        strs(&[""])
    );
}

#[test]
fn the_guessed_names_suggest_it() {
    for name in ["json_diff", "deep_diff", "record_diff"] {
        let mut env = iron_shell::modules::env_with_modules();
        let stmts = iron_shell::parser::parse_program(&format!("{name}({{a: 1}}, {{a: 2}})"))
            .expect("parses");
        let err = iron_shell::eval::eval_program(&stmts, &mut env).expect_err("not a builtin");
        let se = err
            .downcast_ref::<iron_shell::safety::SafetyError>()
            .expect("a structured refusal");
        assert!(
            se.did_you_mean.iter().any(|s| s == "diff_values"),
            "{name}: {:?}",
            se.did_you_mean
        );
    }
}

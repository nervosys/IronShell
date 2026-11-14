//! `math.pi` and `math.e` are Float constants on the math module, read as
//! fields with no call. They were missing, and the error suggested unrelated
//! members (ps, pick, ping).

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

#[test]
fn math_pi_is_the_float_constant() {
    assert_eq!(run("math.pi"), Value::Float(std::f64::consts::PI));
}

#[test]
fn math_e_is_the_float_constant() {
    assert_eq!(run("math.e"), Value::Float(std::f64::consts::E));
}

#[test]
fn math_pi_takes_part_in_arithmetic() {
    assert_eq!(run("math.pi * 2"), Value::Float(std::f64::consts::TAU));
}

#[test]
fn the_constants_are_not_builtins_and_not_callable_as_functions() {
    // A constant has no builtin behind it, so it is not in the module's
    // function list, and the module still lists its real functions.
    assert!(iron_shell::modules::builtin_for_member("math", "pi").is_none());
    assert!(iron_shell::modules::builtin_for_member("math", "e").is_none());
    assert_eq!(
        iron_shell::modules::builtin_for_member("math", "sqrt").as_deref(),
        Some("sqrt")
    );
}

#[test]
fn a_bare_e_is_still_free_for_a_variable() {
    // `e` is a member of math, not a global name: `catch e` and `e = 2` are unaffected.
    assert_eq!(run("e = 2\ne + 1"), Value::Int(3));
}

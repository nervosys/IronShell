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

// ---- 2. split on the empty separator ---------------------------------------

fn strs(v: &[&str]) -> Value {
    Value::Array(v.iter().map(|s| Value::Str((*s).into())).collect())
}

#[test]
fn split_on_the_empty_separator_gives_the_characters() {
    assert_eq!(run(r#""abc" | split("")"#), strs(&["a", "b", "c"]));
    assert_eq!(run(r#"split("abc", "")"#), strs(&["a", "b", "c"]));
}

#[test]
fn split_on_the_empty_separator_counts_unicode_characters() {
    assert_eq!(run(r#""café" | split("") | len"#), Value::Int(4));
    assert_eq!(run(r#""café" | split("")"#), strs(&["c", "a", "f", "é"]));
}

#[test]
fn split_of_an_empty_string_on_the_empty_separator_is_empty() {
    assert_eq!(run(r#""" | split("")"#), Value::Array(vec![]));
}

#[test]
fn split_on_a_real_separator_is_unchanged() {
    assert_eq!(run(r#""a,b,c" | split(",")"#), strs(&["a", "b", "c"]));
}

//! Parser round 45b: a word-call argument is one atom, and an operator right
//! after the arguments is E_PARSE with both readings in the hint.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn parse_err(src: &str) -> (String, String) {
    let e = iron_shell::parser::parse_program(src).expect_err("must not parse");
    let hint = e
        .downcast_ref::<iron_shell::parser::ParseError>()
        .map(|p| p.hint.clone())
        .unwrap_or_default();
    (e.to_string(), hint)
}

#[test]
fn an_operator_after_word_call_arguments_is_ambiguous() {
    let (msg, hint) = parse_err("len [1,2,3] == 3");
    assert!(msg.contains("ambiguous word-call"), "{msg}");
    assert_eq!(
        hint,
        "write len([1,2,3]) == 3 to use the result, or len([1,2,3] == 3) to pass the whole expression"
    );
    let (msg, _) = parse_err("x=[1,2,3]; len x - 1");
    assert!(msg.contains("ambiguous word-call"), "{msg}");
    let (msg, hint) = parse_err("path_join \"a\" \"b\" == \"a/b\"");
    assert!(msg.contains("ambiguous word-call"), "{msg}");
    assert!(
        hint.contains("path_join(\"a\", \"b\") == \"a/b\""),
        "{hint}"
    );
    assert!(
        hint.contains("path_join(\"a\", \"b\" == \"a/b\")"),
        "{hint}"
    );
    let (msg, _) = parse_err("print (1+2) * 3");
    assert!(msg.contains("ambiguous word-call"), "{msg}");
    let (msg, _) = parse_err("x = 1\nprint x && true");
    assert!(msg.contains("ambiguous word-call"), "{msg}");
}

#[test]
fn the_ambiguity_is_named_in_an_if_condition_too() {
    let (msg, hint) = parse_err("if len [1,2,3] == 3 { 1 }");
    assert!(msg.contains("ambiguous word-call"), "{msg}");
    assert!(hint.contains("len([1,2,3]) == 3"), "{hint}");
}

#[test]
fn parenthesised_calls_and_glued_calls_are_unambiguous() {
    assert_eq!(run("x = (len [1,2,3])\nx"), Value::Int(3));
    assert_eq!(run("(len [1,2,3]) == 3"), Value::Bool(true));
    assert_eq!(run("len([1,2,3]) == 3"), Value::Bool(true));
    assert_eq!(run("abs len([1,2])"), Value::Int(2));
    assert_eq!(run("len upper(\"ab\")"), Value::Int(2));
    assert_eq!(run("x = 3\nabs -x"), Value::Int(3));
    assert_eq!(run("abs -3"), Value::Int(3));
    assert_eq!(run("len [1,2,3] | abs"), Value::Int(3));
    assert_eq!(run("x = [1,2,3]\nlen x"), Value::Int(3));
}

#[test]
fn a_negated_or_not_argument_is_one_argument() {
    assert!(iron_shell::parser::parse_program("print !true").is_ok());
    assert!(iron_shell::parser::parse_program("print -1").is_ok());
    assert!(iron_shell::parser::parse_program("x = 1\nprint -x").is_ok());
}

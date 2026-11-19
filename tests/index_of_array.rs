//! index_of takes an Array subject: the index of the first equal element, -1
//! when none is, as it answers -1 for text without the substring.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};

fn eval(src: &str) -> Value {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new()).expect("eval")
}

#[test]
fn a_piped_array_finds_the_first_equal_element() {
    assert!(matches!(eval("[1, 2, 3, 2] | index_of(2)"), Value::Int(1)));
    assert!(matches!(eval("[1, 2, 3] | index_of(9)"), Value::Int(-1)));
}

#[test]
fn a_direct_array_and_other_element_types_work() {
    assert!(matches!(
        eval(r#"index_of(["a", "b"], "b")"#),
        Value::Int(1)
    ));
    assert!(matches!(eval("index_of([[1], [2]], [2])"), Value::Int(1)));
    assert!(matches!(eval("index_of([], 1)"), Value::Int(-1)));
}

#[test]
fn text_still_answers_in_characters() {
    assert!(matches!(eval(r#"index_of("héllo", "l")"#), Value::Int(2)));
    assert!(matches!(eval(r#""hello" | index_of("z")"#), Value::Int(-1)));
}

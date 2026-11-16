//! Parser round 46b: word calls are statements only, one error for a bad
//! word call inside a block, and a hint for a comma after word-call arguments.

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
fn a_word_call_in_a_nested_position_is_refused_with_the_parenthesised_form() {
    let (msg, hint) = parse_err("f = fn(x) => print -x\nf(3)");
    assert!(msg.contains("word call outside a statement"), "{msg}");
    assert_eq!(hint, "word calls work only as statements; write print(-x)");
    let (_, hint) = parse_err("{a: abs -3}");
    assert_eq!(hint, "word calls work only as statements; write abs(-3)");
    let (_, hint) = parse_err("xs = [1, 2]\nxs | map(fn(v) => abs -v)");
    assert_eq!(hint, "word calls work only as statements; write abs(-v)");
    let (_, hint) = parse_err("r = {n: len [1,2]}");
    assert_eq!(hint, "word calls work only as statements; write len([1,2])");
    let (_, hint) = parse_err("f = fn(a) => len a");
    assert_eq!(hint, "word calls work only as statements; write len(a)");
}

#[test]
fn real_subtraction_and_parenthesised_calls_are_unchanged() {
    assert_eq!(run("cnt = 5\nr = {v: cnt - 3}\nr.v"), Value::Int(2));
    assert_eq!(run("cnt = 5\nr = {v: cnt -3}\nr.v"), Value::Int(2));
    assert_eq!(run("f = fn(n) => n -1\nf(4)"), Value::Int(3));
    assert_eq!(run("{v: abs(-3)}.v"), Value::Int(3));
    assert_eq!(run("[1, -2] | map(fn(v) => abs(v))"), run("[1, 2]"));
    assert_eq!(run("abs -3"), Value::Int(3));
    assert_eq!(run("x = 5\nabs -x"), Value::Int(5));
}

#[test]
fn a_bad_word_call_inside_a_block_reports_one_error() {
    let (msg, hint) = parse_err("if true { print 1 + 1 } else { 0 }");
    assert!(msg.contains("ambiguous word-call"), "{msg}");
    assert!(msg.contains("found 1 error(s)"), "{msg}");
    assert!(!msg.contains("found 2 error(s)"), "{msg}");
    assert!(!msg.contains("expected key in record"), "{msg}");
    assert!(hint.contains("print(1) + 1"), "{hint}");
}

#[test]
fn a_comma_after_word_call_arguments_names_the_parenthesised_call() {
    let (msg, hint) = parse_err("x = 1\nprint x, x + 1");
    assert!(msg.contains("unexpected ','"), "{msg}");
    assert!(hint.contains("one argument per atom"), "{hint}");
    assert!(hint.contains("print(x, x + 1)"), "{hint}");
}

//! Parser round 44b: BOM, word-call arguments, projection suffixes, hints.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn parse_err(src: &str) -> String {
    let e = iron_shell::parser::parse_program(src).expect_err("must not parse");
    let hint = e
        .downcast_ref::<iron_shell::parser::ParseError>()
        .map(|p| p.hint.clone())
        .unwrap_or_default();
    format!("{e} || {hint}")
}

#[test]
fn a_leading_bom_is_skipped_and_a_later_one_is_named() {
    assert_eq!(run("\u{FEFF}1 + 2"), Value::Int(3));
    let e = parse_err("x = 1\n\u{FEFF}x");
    assert!(e.contains("byte order mark"), "{e}");
}

#[test]
fn a_word_call_argument_is_a_whole_expression() {
    assert_eq!(run("x = 4\nabs x - 10"), Value::Int(6));
    assert_eq!(run("abs -3"), Value::Int(3));
    assert_eq!(run("abs 1 - 5"), Value::Int(4));
    assert_eq!(run("len [1, 2, 3]"), Value::Int(3));
    assert_eq!(run("[1, 2, 3] | len"), Value::Int(3));
    assert!(iron_shell::parser::parse_program("x = 4\nprint x + 1").is_ok());
    assert!(iron_shell::parser::parse_program("print -1").is_ok());
    assert!(iron_shell::parser::parse_program("print {a: 1}").is_ok());
    assert!(iron_shell::parser::parse_program("print (1+2)").is_ok());
}

#[test]
fn a_subtraction_of_a_bound_name_is_still_arithmetic() {
    assert_eq!(run("n = 5\nn -1"), Value::Int(4));
    assert_eq!(run("n = 5\nn - 1"), Value::Int(4));
}

#[test]
fn a_word_call_as_an_assignment_value_names_the_fix() {
    for src in ["r = ls \".\"", "let r = ls \".\"", "x = len [1,2,3]"] {
        let e = parse_err(src);
        assert!(
            e.contains("cannot be an assignment's value"),
            "{src:?}: {e}"
        );
    }
}

#[test]
fn a_spaced_bracket_hint_says_how_to_index() {
    let e = parse_err("x = [1, 2]\nx = x [0]");
    assert!(e.contains("with no space"), "{e}");
}

#[test]
fn indexing_after_a_projection_applies_to_the_projected_result() {
    let rows = "rows = [{n: [1, 2]}, {n: [3, 4]}]\n";
    assert_eq!(
        run(&format!("{rows}rows |.n[1]")),
        Value::Array(vec![Value::Int(3), Value::Int(4)])
    );
    assert!(iron_shell::parser::parse_program(
        "x = [[{n: [1]}]]
x | first |.n[0]"
    )
    .is_ok());
}

#[test]
fn trailing_commas_are_accepted_in_calls_and_lambda_parameters() {
    assert_eq!(run("len([1, 2],)"), Value::Int(2));
    assert_eq!(run("f = fn(a,) => a + 1\nf(1,)"), Value::Int(2));
}

#[test]
fn named_function_and_braced_lambda_bodies_have_hints() {
    let e = parse_err("fn add(a, b) { a + b }");
    assert!(e.contains("add = fn(a, b) => ..."), "{e}");
    let e = parse_err("fn add(a, b) => a + b");
    assert!(e.contains("add = fn(a, b) => ..."), "{e}");
    let e = parse_err("f = fn(x) => { x }");
    assert!(e.contains("one expression"), "{e}");
}

#[test]
fn help_with_a_module_name_lists_its_members() {
    let v = run("help(\"file\")");
    let Value::Record(r) = v else {
        panic!("help(\"file\") should be a Record")
    };
    assert_eq!(r.get("module"), Some(&Value::Str("file".into())));
    let Some(Value::Array(members)) = r.get("members") else {
        panic!("members should be an Array")
    };
    assert!(members.iter().any(|m| match m {
        Value::Record(m) => m.get("name") == Some(&Value::Str("patch".into())),
        _ => false,
    }));
}

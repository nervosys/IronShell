//! Round 28: `match` patterns, the messages around them, and the parse hints
//! for loops, `def` and a pipe inside an argument list.

use iron_shell::safety::SafetyError;
use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn parse_error(src: &str) -> (String, iron_shell::parser::ParseError) {
    let err = iron_shell::parser::parse_program(src).expect_err("refused");
    let pe = err
        .downcast_ref::<iron_shell::parser::ParseError>()
        .unwrap_or_else(|| panic!("not an E_PARSE error: {err}"))
        .clone();
    (err.to_string(), pe)
}

/// The structured refusal `src` ends in, when it runs.
fn refusal(src: &str) -> SafetyError {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    let e = iron_shell::eval::eval_program(&stmts, &mut env).expect_err("refused");
    e.downcast_ref::<SafetyError>()
        .cloned()
        .unwrap_or_else(|| panic!("structured error expected: {e:#}"))
}

fn s(text: &str) -> Value {
    Value::Str(text.into())
}

// ---- numeric literal patterns use the equality `==` uses ----

#[test]
fn an_int_pattern_matches_an_equal_float() {
    assert_eq!(run(r#"match 1.0 { 1 => "int", _ => "other" }"#), s("int"));
}

#[test]
fn a_float_pattern_matches_an_equal_int() {
    assert_eq!(
        run(r#"match 1 { 1.0 => "float", _ => "other" }"#),
        s("float")
    );
}

#[test]
fn a_float_literal_pattern_parses_and_matches() {
    assert_eq!(run(r#"match 2.5 { 2.5 => "x", _ => "y" }"#), s("x"));
    assert_eq!(run(r#"match 3 { 2.5 => "x", _ => "y" }"#), s("y"));
}

#[test]
fn a_negative_number_pattern_parses_and_matches() {
    assert_eq!(run(r#"match -1 { -1 => "neg", _ => "other" }"#), s("neg"));
    assert_eq!(
        run(r#"match -2.5 { -2.5 => "neg", _ => "other" }"#),
        s("neg")
    );
    assert_eq!(run(r#"match 1 { -1 => "neg", _ => "other" }"#), s("other"));
}

// ---- the no-arm-matched refusal names the value ----

#[test]
fn no_arm_matching_names_the_value_and_the_catch_all() {
    let se = refusal("match 3 { 1 => \"a\", 2 => \"b\" }");
    assert_eq!(se.message, "match: no arm matches 3");
    assert_eq!(se.hint, "add `_ => ...` as the last arm to catch the rest");
}

#[test]
fn no_arm_matching_quotes_a_string_value() {
    let se = refusal(r#"match "x" { 1 => "a" }"#);
    assert_eq!(se.message, "match: no arm matches \"x\"");
}

// ---- a record literal can be the scrutinee; record patterns bind ----

#[test]
fn a_record_literal_scrutinee_matches_a_record_pattern() {
    assert_eq!(
        run(r#"match {name: "x", n: 3} { {name: n} => n, _ => "none" }"#),
        // `{name: n}` binds `n` to the `name` field.
        Value::Str("x".into())
    );
}

#[test]
fn a_record_pattern_matches_a_record_held_in_a_variable() {
    assert_eq!(
        run("r = {name: \"x\"}\nmatch r { {name: n} => n, _ => \"none\" }"),
        s("x")
    );
    assert_eq!(
        run("r = {name: \"x\"}\nmatch r { {name: \"y\"} => 1, _ => \"none\" }"),
        s("none")
    );
}

// ---- loops and `def` get a hint that says what to write ----

#[test]
fn a_while_without_parentheses_gets_the_loop_hint() {
    let (_, pe) = parse_error("while true { print(1) }\n");
    assert!(
        pe.hint
            .contains("IronShell has no loop statements; iterate with map / where / reduce / each"),
        "{}",
        pe.hint
    );
}

#[test]
fn def_gets_the_functions_are_values_hint() {
    let (text, pe) = parse_error("def f(): return 1\n");
    assert!(
        pe.hint.contains("functions are values: f = fn(x) => x + 1"),
        "{}",
        pe.hint
    );
    assert!(text.contains("`def` starts a function"), "{text}");
}

// ---- a pipe inside a call's argument says how to parenthesize it ----

#[test]
fn a_pipe_left_unparenthesized_in_an_argument_gets_the_binds_loosest_hint() {
    let (text, pe) = parse_error("print(\"Hello\" | upper == \"HELLO\")\n");
    assert!(
        pe.hint
            .contains("`|` binds loosest: parenthesize the pipeline"),
        "{}",
        pe.hint
    );
    assert!(text.contains("expected ')' after arguments"), "{text}");
}

// ---- a bare reassignment is an assignment, not a `let` ----

#[test]
fn a_bare_reassignment_of_an_immutable_name_says_assign() {
    let se = refusal("x = 0\nx = x + 1\n");
    assert!(se.message.starts_with("assign:"), "{}", se.message);
    assert!(se
        .message
        .contains("Cannot reassign immutable variable 'x'"));
    assert!(se.hint.contains("mut x = 0"), "{}", se.hint);
}

#[test]
fn a_let_that_rebinds_an_immutable_name_still_says_let() {
    let se = refusal("let x = 0\nlet x = 1\n");
    assert!(se.message.starts_with("let:"), "{}", se.message);
}

#[test]
fn a_mutable_name_can_be_reassigned_bare() {
    assert_eq!(run("mut x = 0\nx = x + 1\nx"), Value::Int(1));
}

// ---- a nested field assignment names the missing record and creates it ----

#[test]
fn a_nested_assignment_into_a_missing_field_says_how_to_create_it() {
    let se = refusal("mut r = {}\nr.a.b = 1\n");
    assert!(
        se.message.contains("r has no field a to assign into"),
        "{}",
        se.message
    );
    assert_eq!(se.hint, "create it first: r.a = {b: 1}");
}

#[test]
fn a_deeper_missing_field_writes_the_whole_record_it_needs() {
    let se = refusal("mut r = {}\nr.a.b.c = \"x\"\n");
    assert_eq!(se.hint, "create it first: r.a = {b: {c: \"x\"}}");
}

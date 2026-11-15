//! Parser round 43b: values that were dropped silently, and parse hints.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

/// The error text (message and hint, as the JSON form carries them).
fn parse_err(src: &str) -> String {
    let e = iron_shell::parser::parse_program(src).expect_err("must not parse");
    let hint = e
        .downcast_ref::<iron_shell::parser::ParseError>()
        .map(|p| p.hint.clone())
        .unwrap_or_default();
    format!("{e} || {hint}")
}

fn ints(v: &[i64]) -> Value {
    Value::Array(v.iter().map(|i| Value::Int(*i)).collect())
}

// ---- item 1: two expressions on one line ----

#[test]
fn two_expressions_on_one_line_are_a_parse_error() {
    for src in ["x = 5 6\nx", "1 2", "\"a\" \"b\"", "x = 1 y = 2"] {
        let e = parse_err(src);
        assert!(e.contains("line 1, column"), "{src:?}: {e}");
        assert!(e.contains("newline or `;`"), "{src:?}: {e}");
    }
}

#[test]
fn newline_and_semicolon_still_separate_statements() {
    assert_eq!(run("x = 5\ny = 6\nx + y"), Value::Int(11));
    assert_eq!(run("x = 5; y = 6; x + y"), Value::Int(11));
    assert_eq!(run("x = 5 ; y = 6\nx * y"), Value::Int(30));
}

// ---- item 2: indexing the result of a pipeline stage ----

#[test]
fn a_stage_call_can_be_indexed() {
    assert_eq!(run("\"a,b\" | split(\",\")[1]"), Value::Str("b".into()));
    assert_eq!(
        run("s = \"a,b\" | split(\",\")[1]\ns"),
        Value::Str("b".into())
    );
    assert_eq!(run("[3, 4, 5] | sort()[0]"), Value::Int(3));
}

#[test]
fn a_bare_stage_can_be_indexed() {
    assert_eq!(run("[3, 4, 5] | reverse[0]"), Value::Int(5));
}

#[test]
fn a_stage_result_can_be_indexed_and_piped_on() {
    assert_eq!(
        run("[3, 4, 5] | reverse[0] | fn(x) => x * 2"),
        Value::Int(10)
    );
    assert_eq!(run("[[1, 2], [3, 4]] | reverse[0]"), ints(&[3, 4]));
}

// ---- item 3: quotes ----

#[test]
fn a_doubled_single_quote_is_a_parse_error() {
    let e = parse_err("a = 'it''s'");
    assert!(e.contains("line 1, column"), "{e}");
    assert!(e.contains("double-quoted string"), "{e}");
}

#[test]
fn a_triple_quote_is_a_parse_error() {
    let e = parse_err("\"\"\"triple\"\"\"");
    assert!(e.contains("triple-quoted strings are not supported"), "{e}");
    let e = parse_err("'''triple'''");
    assert!(e.contains("triple-quoted strings are not supported"), "{e}");
}

#[test]
fn empty_and_adjacent_strings_are_unchanged() {
    assert_eq!(run("\"\""), Value::Str(String::new()));
    assert_eq!(run("''"), Value::Str(String::new()));
    assert_eq!(run("'a' + 'b'"), Value::Str("ab".into()));
}

// ---- item 4: hints ----

#[test]
fn a_comparison_after_a_stage_hints_at_parentheses() {
    for src in ["\"Hello\" | upper == \"HELLO\"", "[1, 2] | len > 1"] {
        let e = parse_err(src);
        assert!(e.contains("parenthesize the pipe"), "{src:?}: {e}");
    }
}

#[test]
fn block_comments_say_to_use_hash() {
    let e = parse_err("/* c */ 1");
    assert!(e.contains("block comments are not supported"), "{e}");
}

#[test]
fn finally_is_named() {
    let e = parse_err("try { 1 } catch e { 2 } finally { 3 }");
    assert!(e.contains("no `finally`"), "{e}");
}

#[test]
fn unsupported_patterns_are_named() {
    let e = parse_err("match x { \"a\" | \"b\" => 1, _ => 2 }");
    assert!(e.contains("or-patterns are not supported"), "{e}");
    let e = parse_err("match x { 1..5 => 1, _ => 2 }");
    assert!(e.contains("range patterns are not supported"), "{e}");
    let e = parse_err("match x { [h, ..t] => h, _ => 2 }");
    assert!(e.contains("rest patterns are not supported"), "{e}");
}

// ---- item 5: trailing commas and duplicate keys ----

#[test]
fn trailing_commas_are_accepted() {
    assert_eq!(run("[1, 2, 3,]"), ints(&[1, 2, 3]));
    assert_eq!(run("{a: 1, b: 2,}.b"), Value::Int(2));
    assert_eq!(run("[]"), Value::Array(vec![]));
}

#[test]
fn a_double_comma_is_still_an_error() {
    assert!(iron_shell::parser::parse_program("[1,,2]").is_err());
}

#[test]
fn a_duplicate_record_key_is_a_parse_error() {
    let e = parse_err("{a: 1, a: 2}");
    assert!(e.contains("duplicate key `a`"), "{e}");
    assert!(e.contains("line 1, column 8"), "{e}");
}

// ---- item 7: .5 ----

#[test]
fn a_leading_point_hints_at_zero() {
    let e = parse_err("x = .5");
    assert!(e.contains("write 0.5"), "{e}");
}

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

// ---- 3. else if ------------------------------------------------------------

#[test]
fn else_if_takes_the_branch_whose_condition_holds() {
    assert_eq!(
        run("if false { 1 } else if true { 2 } else { 3 }"),
        Value::Int(2)
    );
    assert_eq!(
        run("if true { 1 } else if true { 2 } else { 3 }"),
        Value::Int(1)
    );
}

#[test]
fn else_if_falls_through_to_the_final_else() {
    assert_eq!(
        run("if false { 1 } else if false { 2 } else { 3 }"),
        Value::Int(3)
    );
}

#[test]
fn else_if_without_a_final_else_yields_null() {
    assert_eq!(run("if false { 1 } else if false { 2 }"), Value::Null);
}

// ---- 4. Parser hints for `?:` and `?.` -------------------------------------

/// The hint of the parse error for `src`; panics if it parses.
fn parse_hint(src: &str) -> String {
    let e = iron_shell::parser::parse_program(src).expect_err(src);
    e.downcast_ref::<iron_shell::parser::ParseError>()
        .unwrap_or_else(|| panic!("not a ParseError for {src}: {e:#}"))
        .hint
        .clone()
}

#[test]
fn a_ternary_names_the_if_form_as_its_hint() {
    let want = "IronShell has no ?: operator; write if cond { a } else { b }";
    assert_eq!(parse_hint("true ? 1 : 2"), want);
    assert_eq!(parse_hint("x = true ? 1 : 2"), want);
}

#[test]
fn optional_chaining_names_the_null_test_as_its_hint() {
    assert_eq!(
        parse_hint("a?.b"),
        "IronShell has no ?. operator; test for null first: if a == null { null } else { a.b }"
    );
}

// ---- 5. ontology_describe of an unknown name -------------------------------

// ---- 6. merge and format_number builtins ------------------------------------

#[test]
fn merge_takes_the_second_records_fields() {
    assert_eq!(run("merge({a: 1, b: 2}, {b: 3}).b"), Value::Int(3));
    assert_eq!(run("merge({a: 1, b: 2}, {b: 3}).a"), Value::Int(1));
    assert_eq!(run("merge({a: 1}, {c: 4}).c"), Value::Int(4));
}

#[test]
fn merge_pipes_its_first_record() {
    assert_eq!(run("x = {a: 1}\nx | merge({c: 4}).c"), Value::Int(4));
    assert_eq!(run("x = {a: 1}\nx | merge({a: 9}).a"), Value::Int(9));
}

#[test]
fn merge_is_shallow() {
    let v = run("merge({n: {x: 1, y: 2}}, {n: {x: 5}}).n.x");
    assert_eq!(v, Value::Int(5));
}

#[test]
fn merge_refuses_a_non_record_argument() {
    assert_eq!(
        run(r#"try { merge({a: 1}, 2) } catch e { e.error.code }"#),
        Value::Str("E_BAD_ARG".into())
    );
    assert_eq!(
        run(r#"try { merge([1], {a: 1}) } catch e { e.error.code }"#),
        Value::Str("E_BAD_ARG".into())
    );
}

#[test]
fn format_number_writes_exactly_the_decimals_asked() {
    assert_eq!(
        run(r#"format_number(1000, 2)"#),
        Value::Str("1000.00".into())
    );
    assert_eq!(run(r#"format_number(7, 0)"#), Value::Str("7".into()));
    assert_eq!(run(r#"format_number(3.5, 3)"#), Value::Str("3.500".into()));
}

#[test]
fn format_number_groups_thousands_with_a_separator() {
    assert_eq!(
        run(r#"format_number(1234567.891, 2, ",")"#),
        Value::Str("1,234,567.89".into())
    );
    assert_eq!(
        run(r#"format_number(1234567, 0, ",")"#),
        Value::Str("1,234,567".into())
    );
    assert_eq!(
        run(r#"format_number(999, 0, ",")"#),
        Value::Str("999".into())
    );
    assert_eq!(
        run(r#"format_number(-1234.5, 1, ",")"#),
        Value::Str("-1,234.5".into())
    );
}

#[test]
fn format_number_rounds_half_away_from_zero() {
    assert_eq!(run(r#"format_number(2.5, 0)"#), Value::Str("3".into()));
    assert_eq!(run(r#"format_number(-0.5, 0)"#), Value::Str("-1".into()));
    assert_eq!(run(r#"format_number(0.125, 2)"#), Value::Str("0.13".into()));
}

#[test]
fn format_number_keeps_the_sign_unless_the_value_rounds_to_zero() {
    assert_eq!(
        run(r#"format_number(-0.001, 2)"#),
        Value::Str("0.00".into())
    );
    assert_eq!(run(r#"format_number(-2, 1)"#), Value::Str("-2.0".into()));
}

#[test]
fn format_number_takes_the_piped_number() {
    assert_eq!(
        run(r#"3.14159 | format_number(3)"#),
        Value::Str("3.142".into())
    );
}

#[test]
fn format_number_refuses_bad_decimals_and_non_numbers() {
    assert_eq!(
        run(r#"try { format_number(1.5, 16) } catch e { e.error.code }"#),
        Value::Str("E_BAD_ARG".into())
    );
    assert_eq!(
        run(r#"try { format_number(1.5, -1) } catch e { e.error.code }"#),
        Value::Str("E_BAD_ARG".into())
    );
    assert_eq!(
        run(r#"try { format_number("1.5", 2) } catch e { e.error.code }"#),
        Value::Str("E_BAD_ARG".into())
    );
}

#[test]
fn ontology_describe_of_an_unknown_name_is_an_error_not_a_record() {
    let mut env = iron_shell::env::Env::new();
    let e = iron_shell::builtins::call(
        "ontology_describe",
        vec![Value::Str("format".into())],
        &mut env,
    )
    .expect_err("an unknown name is an error, not a record with an error field");
    let j = e
        .downcast_ref::<iron_shell::safety::SafetyError>()
        .expect("a structured error")
        .to_json();
    assert_eq!(j["error"]["code"], "E_UNKNOWN_BUILTIN");
}

#[test]
fn else_if_chains_further_than_one_link() {
    let v = run("grade = fn(s) => if s >= 90 { \"A\" } else if s >= 80 { \"B\" } else if s >= 70 { \"C\" } else { \"F\" }\n\
                 [95, 85, 75, 10] | map(grade)");
    assert_eq!(v, strs(&["A", "B", "C", "F"]));
}

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

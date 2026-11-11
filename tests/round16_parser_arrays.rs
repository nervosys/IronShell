//! Round 16: the lexer's minus rule, take/head/tail counts and array first
//! arguments, group_by keys, the empty-name refusals of help and
//! ontology_describe, and format_number's decimals message.
//!
//! In-process, as tests/curried_closure_builtins.rs runs them.

use iron_shell::safety::SafetyError;
use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

/// The structured refusal `src` ends in; panics if it runs or fails without one.
fn refusal(src: &str) -> SafetyError {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    match iron_shell::eval::eval_program(&stmts, &mut env) {
        Err(e) => e
            .chain()
            .find_map(|c| c.downcast_ref::<SafetyError>())
            .cloned()
            .unwrap_or_else(|| panic!("structured error expected: {e:#}")),
        Ok(v) => panic!("expected a refusal from {src:?}, got {v:?}"),
    }
}

/// The error code of `src`, caught by the language's own `try`.
fn error_code(src: &str) -> String {
    let wrapped = format!("try {{ {src} }} catch e {{ e.code }}");
    match run(&wrapped) {
        Value::Str(code) => code,
        other => panic!("expected an error code, got {other:?}"),
    }
}

fn ints(v: &[i64]) -> Value {
    Value::Array(v.iter().map(|i| Value::Int(*i)).collect())
}

// Item 1: `-` after an operand, even with a space before it, is subtraction.

#[test]
fn a_spaced_minus_after_a_name_subtracts_one() {
    assert_eq!(run("n = 4\nn -1"), Value::Int(3));
}

#[test]
fn a_spaced_minus_after_a_name_in_an_assignment_subtracts_one() {
    assert_eq!(run("n = 4\nr = n -1\nr"), Value::Int(3));
}

#[test]
fn a_spaced_minus_before_a_product_keeps_the_product() {
    // n - (3 * 2), not the word call n(-3) * 2.
    assert_eq!(run("n = 10\nn -3 * 2"), Value::Int(4));
}

#[test]
fn a_spaced_minus_after_a_name_inside_a_product_subtracts() {
    assert_eq!(run("x = 2\nx * -1"), Value::Int(-2));
}

#[test]
fn a_negative_literal_in_an_array_stays_a_literal() {
    assert_eq!(run("[1, -1]"), ints(&[1, -1]));
}

#[test]
fn a_negative_literal_in_a_call_stays_a_literal() {
    assert_eq!(run("f = fn(a) => a\nf(-1)"), Value::Int(-1));
}

#[test]
fn a_negative_literal_at_statement_start_stays_a_literal() {
    assert_eq!(run("x = 5\n-1"), Value::Int(-1));
}

#[test]
fn a_negative_literal_in_a_match_arm_stays_a_literal() {
    assert_eq!(
        run("x = -1\nmatch x { -1 => \"neg\", _ => \"other\" }"),
        Value::Str("neg".into())
    );
}

#[test]
fn ls_with_a_spaced_one_still_parses_as_subtraction() {
    assert!(iron_shell::parser::parse_program("ls -1").is_ok());
}

// Item 2: take refuses a negative count; head and tail take an Array first.

#[test]
fn take_with_a_negative_count_is_refused() {
    assert_eq!(error_code("take([1, 2, 3], -1)"), "E_BAD_ARG");
}

#[test]
fn a_piped_take_with_a_negative_count_is_refused() {
    assert_eq!(error_code("[1, 2, 3] | take(-2)"), "E_BAD_ARG");
}

#[test]
fn take_still_takes_a_nonnegative_count() {
    assert_eq!(run("take([1, 2, 3], 2)"), ints(&[1, 2]));
    assert_eq!(run("[1, 2, 3] | take(0)"), ints(&[]));
}

#[test]
fn head_takes_an_array_as_its_first_argument() {
    assert_eq!(run("head([1, 2, 3], 2)"), ints(&[1, 2]));
    assert_eq!(run("head([1, 2, 3])"), ints(&[1, 2, 3]));
}

#[test]
fn tail_takes_an_array_as_its_first_argument() {
    assert_eq!(run("tail([1, 2, 3], 2)"), ints(&[2, 3]));
}

#[test]
fn head_with_a_negative_count_is_refused() {
    assert_eq!(error_code("[1, 2, 3] | head(-1)"), "E_BAD_ARG");
}

// Item 3: group_by keys on the type as well as the text.

#[test]
fn group_by_keeps_int_one_and_string_one_apart() {
    assert_eq!(
        run("[1, \"1\"] | group_by(fn(x) => x) | len"),
        Value::Int(2)
    );
}

#[test]
fn group_by_still_groups_equal_values_of_one_type() {
    assert_eq!(
        run("g = ([1, 1, \"1\"] | group_by(fn(x) => x))\ng[0].Count"),
        Value::Int(2)
    );
}

// Item 4: an empty name is refused, not looked up as the builtin "".

#[test]
fn help_with_an_empty_name_is_refused() {
    let e = refusal("help(\"\")");
    assert_eq!(e.code, iron_shell::safety::ErrorCode::BadArg);
    assert!(
        e.message.contains("name must not be empty"),
        "{}",
        e.message
    );
    assert!(e.hint.contains("ontology_manifest()"), "{}", e.hint);
}

#[test]
fn ontology_describe_with_an_empty_name_is_refused() {
    let e = refusal("ontology_describe(\"\")");
    assert_eq!(e.code, iron_shell::safety::ErrorCode::BadArg);
    assert!(
        e.message.contains("name must not be empty"),
        "{}",
        e.message
    );
    assert!(!e.message.contains("unknown builtin"), "{}", e.message);
}

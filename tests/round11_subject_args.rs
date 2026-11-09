//! Round 11: the conversions that take a value take it as the first argument
//! when nothing is piped, as well as through the pipe. With no value at all,
//! the refusal's hint shows both forms.

use iron_shell::env::Env;
use iron_shell::safety::SafetyError;
use iron_shell::value::Value;

fn run(src: &str) -> anyhow::Result<Value> {
    let mut env = Env::new();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn refusal(src: &str) -> SafetyError {
    let err = run(src).expect_err("the call must be refused");
    err.downcast_ref::<SafetyError>()
        .expect("a structured refusal")
        .clone()
}

#[test]
fn to_json_takes_its_value_as_the_argument() {
    assert_eq!(
        run("to_json([1, 2])").unwrap(),
        Value::Str("[1,2]".to_string())
    );
    assert_eq!(
        run("[1, 2] | to_json").unwrap(),
        Value::Str("[1,2]".to_string())
    );
}

#[test]
fn from_csv_and_to_csv_take_their_subject_as_the_argument() {
    assert_eq!(run("from_csv(\"n\\n7\\n\") | len").unwrap(), Value::Int(1));
    assert_eq!(
        run("to_csv([{n: 7}]) | from_csv | len").unwrap(),
        Value::Int(1)
    );
}

#[test]
fn a_conversion_with_no_value_says_both_forms() {
    let se = refusal("to_json()");
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert!(
        se.hint.contains("v | to_json") && se.hint.contains("to_json(v)"),
        "{}",
        se.hint
    );
    let se = refusal("from_csv()");
    assert!(se.hint.contains("from_csv(v)"), "{}", se.hint);
    let se = refusal("to_csv()");
    assert!(se.hint.contains("to_csv(v)"), "{}", se.hint);
}

#[test]
fn a_wrong_type_for_the_argument_is_still_refused() {
    assert_eq!(refusal("from_csv(5)").code.as_str(), "E_BAD_ARG");
}

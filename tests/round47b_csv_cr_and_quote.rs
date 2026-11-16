//! `to_csv` quotes a field holding a bare CR, and `from_csv` names an
//! unterminated quoted field.

use iron_shell::value::Value;

fn try_run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn run(src: &str) -> Value {
    try_run(src).expect("runs")
}

#[test]
fn to_csv_quotes_a_bare_cr_and_round_trips() {
    let text = run("to_csv([{a: \"x\\ry\"}])");
    assert_eq!(text, Value::Str("a\n\"x\ry\"\n".to_string()));
    assert_eq!(
        run("from_csv(to_csv([{a: \"x\\ry\"}, {a: \"z\"}])) | map(fn(r) => r.a)"),
        Value::Array(vec![
            Value::Str("x\ry".to_string()),
            Value::Str("z".to_string())
        ])
    );
}

#[test]
fn from_csv_names_an_unterminated_quote() {
    let err = try_run("from_csv(\"a,b\\n\\\"1,2\\n\")").expect_err("refused");
    let text = err.to_string();
    assert!(
        text.contains("unterminated quoted field starting on line 2"),
        "{text}"
    );
    assert!(!text.contains("found 1 fields"), "{text}");
}

#[test]
fn from_csv_still_reports_a_short_row() {
    let err = try_run("from_csv(\"a,b\\n1\\n\")").expect_err("refused");
    assert!(err.to_string().contains("found 1 fields"), "{err}");
}

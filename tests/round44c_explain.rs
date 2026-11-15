//! `explain_effects` judges only the arguments that are paths for the builtin,
//! not any String that reads like one, so its prediction matches the run.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn decision(code: &str) -> String {
    let escaped = code.replace('\', "\\\\").replace('"', "\\\"");
    let v = run(&format!("explain_effects(\"{escaped}\").decision"));
    match v {
        Value::Str(s) => s,
        other => panic!("{other:?}"),
    }
}

#[test]
fn json_text_is_not_a_path() {
    assert_ne!(decision(r#"from_json("{\"a\":1}")"#), "deny");
}

#[test]
fn sql_text_is_not_a_path() {
    assert_ne!(
        decision(r#"sql_value([{n:1}], "select count(*) from t")"#),
        "deny"
    );
    assert_ne!(
        decision(r#"[{n:1}] | sql("select * from t where s = '../x'")"#),
        "deny"
    );
}

#[test]
fn a_real_path_outside_the_workspace_is_still_denied() {
    assert_eq!(decision(r#"file.read("../x")"#), "deny");
}

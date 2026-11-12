//! Round 25: a missing argument is named by the parameter the caller would
//! write, not by the internal name of the piped subject.
//!
//! An agent call with an args object reached `missing required argument
//! 'subject'` for `sql_value()`. The subject has no name of its own (its
//! signature field is a type), so the message names the first declared
//! parameter, `source_or_query`. Each test runs in process.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};
use serde_json::json;

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

/// Send one agent request and return the response as JSON.
fn agent_call(request: &str) -> serde_json::Value {
    let out = iron_shell::agent_api::process_json_request(request).expect("a response");
    serde_json::from_str(&out).expect("response is JSON")
}

#[test]
fn sql_value_with_no_arguments_names_source_or_query() {
    let text = eval("sql_value()").expect_err("no source").to_string();
    assert!(text.contains("source_or_query"), "{text}");
    assert!(!text.contains("'subject'"), "{text}");
}

#[test]
fn an_agent_call_to_sql_value_without_its_source_names_source_or_query() {
    let v = agent_call(r#"{"action":"call","builtin":"sql_value","args":{"query":"select 1"}}"#);
    assert_eq!(v["success"], json!(false), "{v}");
    let message = v["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("missing required argument 'source_or_query' for 'sql_value'"),
        "{message}"
    );
    assert!(!message.contains("'subject'"), "{message}");
}

#[test]
fn an_agent_call_to_date_add_without_its_subject_names_the_first_parameter() {
    // date_add's subject is the date; its first declared parameter is `amount`.
    let v = agent_call(r#"{"action":"call","builtin":"date_add","args":{"unit":"days"}}"#);
    assert_eq!(v["success"], json!(false), "{v}");
    let message = v["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("missing required argument 'amount' for 'date_add'"),
        "{message}"
    );
    assert!(!message.contains("'subject'"), "{message}");
}

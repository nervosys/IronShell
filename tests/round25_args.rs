//! Round 25: a missing subject in an agent call says what `subject` is.
//!
//! An agent call with an args object that left out the value a builtin works
//! on answered only `missing required argument 'subject'`, a key no declared
//! parameter is called. `subject` is the right key (the tool schema lists it
//! first), so the message keeps it, says what it holds, and lists every key.

use serde_json::json;

/// Send one agent request and return the response as JSON.
fn agent_call(request: &str) -> serde_json::Value {
    let out = iron_shell::agent_api::process_json_request(request).expect("a response");
    serde_json::from_str(&out).expect("response is JSON")
}

fn missing_message(request: &str) -> String {
    let v = agent_call(request);
    assert_eq!(v["success"], json!(false), "{v}");
    v["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn a_missing_subject_is_explained_and_the_keys_are_listed() {
    let message =
        missing_message(r#"{"action":"call","builtin":"date_add","args":{"unit":"days"}}"#);
    assert!(
        message.contains("missing required argument 'subject' for 'date_add'"),
        "{message}"
    );
    assert!(message.contains("the value it works on"), "{message}");
    assert!(message.contains("'input'"), "{message}");
    assert!(
        message.contains("valid parameters: subject, amount, unit"),
        "{message}"
    );
    // It does not name `amount`, which is not what is missing.
    assert!(!message.contains("argument 'amount'"), "{message}");
}

#[test]
fn the_subject_key_it_names_is_accepted() {
    let v = agent_call(
        r#"{"action":"call","builtin":"date_add","args":{"subject":"2024-01-31","amount":1,"unit":"months"}}"#,
    );
    assert_eq!(v["success"], json!(true), "{v}");
    assert!(v.to_string().contains("2024-02-29"), "{v}");
}

//! Round 33: the POSIX engine's syntax errors, `agent execute` exit statuses,
//! MCP discovery as JSON, one list of schema formats, `pick`'s return type,
//! base64 as a pure builtin, bare `hex_*` names, and the `in` hint.
//!
//! The binary runs with IRONSHELL_SESSION_DIR in a temp dir of its own, and
//! that dir as its working folder.

use iron_shell::value::Value;
use serde_json::Value as Json;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let root = std::env::temp_dir().join(format!("ae-round33-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("session")).unwrap();
        Sandbox(root)
    }

    fn ish(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(&self.0)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", self.0.join("session"))
            .env_remove("IRONSHELL_ALLOW_SH")
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_AGENT")
            .env_remove("IRONSHELL_WORKSPACE")
            .output()
            .expect("run ish")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).into_owned()
}

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

// 1
#[test]
fn item1_an_unterminated_quote_is_e_parse_exit_2_before_anything_runs() {
    let sb = Sandbox::new("quote");
    for script in [
        "echo \"abc",
        "echo 'abc",
        "echo a\necho \"abc",
        "echo $(echo a",
    ] {
        let out = sb.ish(&["-b", "-c", script]);
        let err = text(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{script:?}: {err}");
        assert!(out.stdout.is_empty(), "{script:?}: {}", text(&out.stdout));
        assert!(err.contains("E_PARSE"), "{script:?}: {err}");
        assert!(err.contains("line"), "{script:?}: {err}");
    }
}

// 2
#[test]
fn item2_a_paren_in_argument_position_is_a_syntax_error_not_a_function() {
    let sb = Sandbox::new("paren");
    for script in ["echo (", "cat (", "echo a(b"] {
        let out = sb.ish(&["-b", "-c", script]);
        let err = text(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{script:?}: {err}");
        assert!(
            err.contains("syntax error near unexpected token `('"),
            "{script:?}: {err}"
        );
        assert!(!err.contains("function definition"), "{script:?}: {err}");
    }
    // A real definition is still refused as one, and a subshell as a subshell.
    for (script, what) in [
        ("f() { echo hi; }", "function definition"),
        ("f () { echo hi; }", "function definition"),
        ("(echo hi)", "subshell"),
    ] {
        let out = sb.ish(&["-b", "-c", script]);
        let err = text(&out.stderr);
        assert_eq!(out.status.code(), Some(77), "{script:?}: {err}");
        assert!(err.contains(what), "{script:?}: {err}");
    }
}

// 3
#[test]
fn item3_agent_execute_exits_with_the_status_of_the_error_code() {
    let sb = Sandbox::new("execute");
    let exec =
        |request: &str| sb.ish(&["--agent", "--workspace", ".", "agent", "execute", request]);

    // The file exists, so rm reaches the approval gate (a missing file is
    // E_NOT_FOUND first).
    std::fs::write(sb.0.join("f"), "x").unwrap();
    let out = exec(r#"{"action":"call","builtin":"rm","args":["f"]}"#);
    assert_eq!(out.status.code(), Some(75), "{}", text(&out.stdout));

    let out = exec(r#"{"action":"call","builtin":"no_such_builtin_zz","args":[]}"#);
    assert_eq!(out.status.code(), Some(127), "{}", text(&out.stdout));

    let out = exec(r#"{"action":"eval","code":"1 + 1"}"#);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));

    // A request that is not a request stays 1.
    let out = exec("not json");
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
}

#[test]
fn item3_a_file_name_given_as_the_request_says_it_is_json_text() {
    let sb = Sandbox::new("execfile");
    std::fs::write(sb.0.join("req.json"), r#"{"action":"eval","code":"1"}"#).unwrap();
    let out = sb.ish(&["agent", "execute", "req.json"]);
    assert_eq!(out.status.code(), Some(1));
    let v: Json = serde_json::from_str(&text(&out.stdout)).expect("JSON response");
    assert_eq!(v["error"]["code"], "E_PARSE", "{v}");
    let hint = v["error"]["hint"].as_str().unwrap_or_default();
    assert!(hint.contains("file"), "{hint}");
    assert!(hint.contains("JSON"), "{hint}");
}

// 4
#[test]
fn item4_mcp_ontology_describe_returns_json_text() {
    let res = iron_shell::mcp::McpServer::new().handle_rpc(&serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "ontology_describe", "arguments": { "name": "pick" } }
    }));
    let res = res.expect("a response");
    assert_eq!(res["result"]["isError"], false, "{res}");
    let body = res["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    let v: Json = serde_json::from_str(body).unwrap_or_else(|e| panic!("not JSON ({e}): {body}"));
    assert_eq!(v["builtin"], "pick", "{v}");
}

// 5
#[test]
fn item5_schema_help_and_the_unknown_format_error_list_the_same_formats() {
    let sb = Sandbox::new("schema");
    let help = text(&sb.ish(&["agent", "schema", "--help"]).stdout);
    let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
    let bad = sb.ish(&["agent", "schema", "-f", "bogus"]);
    let err = text(&bad.stderr);
    assert_ne!(bad.status.code(), Some(0));
    assert!(err.contains("Unknown schema format"), "{err}");
    let err = err.split_whitespace().collect::<Vec<_>>().join(" ");
    for name in iron_shell::agent_api::SCHEMA_FORMATS_HINT.split(", ") {
        assert!(help.contains(name), "--help lacks {name}: {help}");
        assert!(err.contains(name), "the error lacks {name}: {err}");
    }
    for name in ["ontology", "azure_openai", "jsonld", "owl", "shacl"] {
        assert!(help.contains(name), "--help lacks {name}: {help}");
    }
}

// 6
#[test]
fn item6_pick_declares_an_array_or_a_record() {
    let sig = iron_shell::signature::signature_of("pick").expect("pick is declared");
    assert!(sig.returns.contains("Array"), "{}", sig.returns);
    assert!(sig.returns.contains("Record"), "{}", sig.returns);
    assert_eq!(
        run(r#"typeof(pick({a: 1, b: 2}, "a"))"#),
        Value::Str("Record".into())
    );
    assert_eq!(
        run(r#"typeof(pick([{a: 1, b: 2}], "a"))"#),
        Value::Str("Array".into())
    );
}

// 7
#[test]
fn item7_in_memory_encoders_are_pure() {
    for name in [
        "base64_decode",
        "base64_encode",
        "crypto_base64_decode",
        "crypto_base64_encode",
        "crypto_hex_decode",
        "crypto_hex_encode",
        "url_decode",
        "url_encode",
    ] {
        assert_eq!(
            iron_shell::safety::effect_of(name).as_str(),
            "pure",
            "{name}"
        );
    }
}

// 8
#[test]
fn item8_bare_hex_names_resolve() {
    assert_eq!(run(r#"hex_encode("hi")"#), Value::Str("6869".into()));
    assert_eq!(run(r#"hex_decode("6869")"#), Value::Str("hi".into()));
    assert_eq!(run(r#""hi" | hex_encode()"#), Value::Str("6869".into()));
    assert_eq!(
        run(r#"base64_decode(base64_encode("hi"))"#),
        Value::Str("hi".into())
    );
}

// 9
#[test]
fn item9_in_is_not_an_operator_and_is_reported_once() {
    let src = "rows = [{labels: [\"bug\"]}]\n\
               a = rows | where(fn(r) => \"bug\" in r.labels)\n\
               b = rows | where(fn(r) => \"bug\" in r.labels)\n";
    let err = iron_shell::parser::parse_program(src).expect_err("`in` is not an operator");
    let msg = format!("{err}");
    assert!(!msg.contains("found"), "{msg}");
    assert_eq!(msg.lines().count(), 1, "{msg}");
    let pe = err
        .downcast_ref::<iron_shell::parser::ParseError>()
        .expect("a ParseError");
    assert!(
        pe.hint
            .contains("`in` is not an operator; use contains(list, x) or x | in(list)"),
        "{}",
        pe.hint
    );
    // The builtin the hint names works both ways.
    assert_eq!(run(r#""bug" | in(["bug", "x"])"#), Value::Bool(true));
}

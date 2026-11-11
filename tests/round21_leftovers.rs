//! Round 21: the last open items from rounds 19 and 20.
//!
//! - `sql`'s `query` is an ordinary optional String again. A trailing params
//!   Array reaches `params` because the validator lets the last argument skip
//!   one optional parameter whose type it does not match.
//! - A repeated key is refused at any depth of an agent request, and on the MCP
//!   stdio transport, naming its path.
//! - `IRONSHELL_REFUSE_LINKED_READS=1` refuses reads through a hard link under
//!   the jail. It is off by default, because package stores hard-link files.

use iron_shell::agent_api::parse_request;
use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

// ---------- sql: params after an omitted query ----------

#[test]
fn sql_value_binds_a_trailing_array_past_the_query() {
    let v = eval(r#"sql_value(":memory:", "select ? * 6", [7])"#).expect("binds");
    assert_eq!(v, Value::Int(42));
}

#[test]
fn piped_rows_take_the_query_then_the_params() {
    let v = eval(r#"[{n: 1}, {n: 5}, {n: 9}] | sql("select n from t where n > ?", [4]) | len"#)
        .expect("binds");
    assert_eq!(v, Value::Int(2));
}

#[test]
fn sql_with_a_third_string_is_refused_as_one_argument_too_many() {
    let e = eval(r#"sql(":memory:", "select 1", "select 2", "select 3")"#)
        .expect_err("query is no longer variadic");
    assert!(e.to_string().contains("E_BAD_ARG"), "{e}");
}

#[test]
fn the_sql_signature_shows_query_as_one_optional_string() {
    let sig = iron_shell::signature::signature_of("sql").expect("sql is declared");
    let rendered = sig.render();
    assert!(!rendered.contains("query..."), "{rendered}");
    assert!(rendered.contains("query?: String"), "{rendered}");
    assert!(rendered.contains("params?: Array"), "{rendered}");
}

// ---------- repeated keys at any depth ----------

#[test]
fn a_repeated_key_inside_args_is_refused_by_path() {
    let text = r#"{"action":"call","builtin":"file_write","args":{"path":"a.txt","path":"b.txt"}}"#;
    let err = parse_request(text).expect_err("refused");
    assert_eq!(err.code.as_deref(), Some("E_PARSE"), "{err:?}");
    assert!(
        err.error.unwrap_or_default().contains("args.path"),
        "the message names the path"
    );
}

#[test]
fn a_request_without_repeats_still_parses() {
    let text = r#"{"action":"call","builtin":"upper","args":{"subject":"a","x":{"subject":1}}}"#;
    // `x` is not a parameter of upper, but that is the call's refusal, not the
    // parser's: the same key in two different objects is not a repeat.
    assert!(parse_request(text).is_ok());
}

fn session(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ish_r21_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn mcp_stdio_refuses_a_line_with_a_repeated_key() {
    let dir = session("mcp");
    let mut child = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["mcp", "stdio"])
        .current_dir(&dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ish mcp stdio");
    let mut stdin = child.stdin.take().expect("stdin");
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{{"name":"ironshell","name":"ontology_manifest","arguments":{{}}}}}}"#
    )
    .expect("write request");
    drop(stdin);
    let out = child.wait_with_output().expect("wait");
    let text = String::from_utf8_lossy(&out.stdout);
    let reply: serde_json::Value = text
        .lines()
        .find_map(|l| serde_json::from_str(l).ok())
        .unwrap_or_else(|| panic!("no JSON reply: {text}"));
    assert_eq!(reply["id"], 4, "{reply}");
    assert_eq!(reply["error"]["code"], -32600, "{reply}");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("params.name"),
        "{reply}"
    );
}

// ---------- reads through a hard link ----------

struct Jail {
    inside: PathBuf,
    secret: PathBuf,
}

fn jail(tag: &str) -> Jail {
    let root = session(tag);
    let inside = root.join("ws");
    let outside = root.join("outside");
    std::fs::create_dir_all(&inside).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let secret = outside.join("secret.txt");
    std::fs::write(&secret, "outside-content").unwrap();
    std::fs::hard_link(&secret, inside.join("hl.txt")).expect("hard link in the same volume");
    Jail { inside, secret }
}

fn agent_read(ws: &Path, refuse: bool, args: &[&str]) -> String {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["--agent", "--workspace", "."])
        .args(args)
        .current_dir(ws)
        .env("IRONSHELL_SESSION_DIR", ws.join(".session"))
        .env("NO_COLOR", "1");
    if refuse {
        cmd.env("IRONSHELL_REFUSE_LINKED_READS", "1");
    } else {
        cmd.env_remove("IRONSHELL_REFUSE_LINKED_READS");
    }
    let out = cmd.output().expect("run ish");
    String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr)
}

#[test]
fn a_linked_read_is_allowed_by_default() {
    let j = jail("read_default");
    let text = agent_read(&j.inside, false, &["-c", r#"read_text("hl.txt")"#]);
    assert!(text.contains("outside-content"), "{text}");
}

#[test]
fn the_operator_setting_refuses_a_linked_read() {
    let j = jail("read_refused");
    let text = agent_read(&j.inside, true, &["-c", r#"read_text("hl.txt")"#]);
    assert!(text.contains("E_POLICY_DENY"), "{text}");
    assert!(!text.contains("outside-content"), "{text}");
    assert!(j.secret.exists());
}

#[test]
fn the_operator_setting_refuses_a_linked_read_in_the_posix_subset() {
    let j = jail("read_posix");
    let text = agent_read(&j.inside, true, &["-b", "-c", "cat hl.txt"]);
    assert!(!text.contains("outside-content"), "{text}");
    assert!(text.contains("hard link"), "{text}");
}

#[test]
fn an_unlinked_file_reads_with_the_setting_on() {
    let j = jail("read_plain");
    std::fs::write(j.inside.join("plain.txt"), "plain-content").unwrap();
    let text = agent_read(&j.inside, true, &["-c", r#"read_text("plain.txt")"#]);
    assert!(text.contains("plain-content"), "{text}");
}

//! Round 48a: schema, naming, CSV, help and `ish -b` fixes.

use iron_shell::value::Value;
use serde_json::{json, Value as Json};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn text(src: &str) -> String {
    match run(src) {
        Value::Str(s) => s,
        other => format!("{other:?}"),
    }
}

fn scratch_dir() -> std::path::PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("ae_r48a_{}_{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run `ish <args>` with `stdin`; the exit code, stdout and stderr.
fn run_ish(args: &[&str], stdin: &[u8]) -> (Option<i32>, String, String) {
    let dir = scratch_dir();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .env("IRONSHELL_SESSION_DIR", &dir)
        .env_remove("IRONSHELL_DETERMINISTIC")
        .env_remove("IRONSHELL_TOKEN_BUDGET")
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ish");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin)
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait for ish");
    let _ = std::fs::remove_dir_all(&dir);
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn execute(request: &str) -> Json {
    let (code, out, err) = run_ish(&["agent", "execute"], request.as_bytes());
    assert_eq!(code, Some(0), "stderr: {err}");
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {out}"))
}

// ---- item 1: required parameters are declared required ----

#[test]
fn item1_required_params_are_not_optional_in_the_schema() {
    for (name, required) in [
        ("ai", vec!["prompt"]),
        ("error", vec!["message"]),
        ("kg_add", vec!["type", "name"]),
        ("kg_relate", vec!["source", "target", "relation"]),
        ("rag_index", vec!["content"]),
        ("semantic_cache", vec!["query", "response"]),
        ("trace", vec!["label_or_value"]),
    ] {
        let sig = iron_shell::signature::signature_of(name).expect(name);
        for want in required {
            let p = sig.params.iter().find(|p| p.name == want).expect(want);
            assert!(p.required, "{name}.{want} should be required");
        }
    }
}

#[test]
fn item1_piped_forms_still_work() {
    assert_eq!(
        text(r#"(try { "boom" | error() } catch e { e.message })"#),
        "boom"
    );
    assert_eq!(
        text(r#"(try { kg_relate("a", "b") } catch e { e.error.code })"#),
        "E_BAD_ARG"
    );
}

// ---- item 2: variadic named binding ----

#[test]
fn item2_values_names_the_array_of_min_and_max() {
    let v = execute(r#"{"action":"call","builtin":"min","args":{"values":[3,1,2]}}"#);
    assert_eq!(v["success"], json!(true), "{v}");
    assert_eq!(v["result"], json!(1), "{v}");
    let v = execute(r#"{"action":"call","builtin":"max","args":{"values":[3,1,2]}}"#);
    assert_eq!(v["result"], json!(3), "{v}");
}

// ---- item 3: x.y with an undefined x ----

#[test]
fn item3_an_undefined_receiver_is_an_unknown_name() {
    assert_eq!(
        text(r#"try { zzzz_r48.parse("x") } catch e { e.code }"#),
        "E_UNKNOWN_NAME"
    );
    assert_eq!(
        text(r#"try { zzzz_r48.field } catch e { e.code }"#),
        "E_UNKNOWN_NAME"
    );
    assert!(text(r#"try { zzzz_r48.parse(1) } catch e { e.hint }"#)
        .contains("`zzzz_r48` is not a variable, builtin or module"));
}

// ---- item 4: from_csv quoting ----

#[test]
fn item4_from_csv_refuses_a_quote_left_open_at_the_end() {
    for src in [
        r#"try { from_csv("a,b\n1,\"x\n") } catch e { e.error.code }"#,
        r#"try { from_csv("a\n\"x") } catch e { e.error.code }"#,
    ] {
        assert_eq!(text(src), "E_BAD_ARG", "{src}");
    }
    assert!(text(r#"try { from_csv("a\n\"x") } catch e { e.message }"#)
        .contains("unterminated quoted field starting on line 2"));
}

#[test]
fn item4_from_csv_refuses_text_after_a_closing_quote() {
    let msg = text(r#"try { from_csv("a\n\"x\"y\n") } catch e { e.message }"#);
    assert!(
        msg.contains("unexpected text after a closing quote on line 2"),
        "{msg}"
    );
    // Well-formed quoting still reads.
    assert_eq!(
        text(r#"from_csv("a,b\n\"x,1\",\"he said \"\"hi\"\"\"\n") | map(fn(r) => r.b) | first"#),
        "he said \"hi\""
    );
}

// ---- item 5: explain resolves names as help does ----

#[test]
fn item5_explain_resolves_module_and_underscore_spellings() {
    let dotted = text(r#"explain("file.read").name"#);
    let under = text(r#"explain("file_read").name"#);
    assert!(!dotted.is_empty());
    assert_eq!(dotted, under);
    assert_eq!(
        text(r#"try { explain("zz_no_such_thing_r48") } catch e { e.error.code }"#),
        "E_NOT_FOUND"
    );
    assert!(
        text(r#"try { explain("zz_no_such_thing_r48") } catch e { e.error.hint }"#)
            .contains("complete(")
    );
}

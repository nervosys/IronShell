//! A JSON null is a null, not the text "<null>".
//!
//! The value space has `Value::Null`, and `from_json` already maps a JSON null
//! to it. The serializers used to map it back through `value_to_string`, which
//! prints "<null>", so `json.stringify`, `to-json`, `save_json` wrote the text
//! "<null>" and exited 0. `to-csv` wrote "<null>" into the cell instead of an
//! empty field.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ae-json-null-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cwd")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Scratch { root }
    }

    fn cwd(&self) -> PathBuf {
        self.root.join("cwd")
    }

    fn ish(&self, expr: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-c", expr])
            .current_dir(self.cwd())
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .output()
            .expect("run ish")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn stdout_of(s: &Scratch, expr: &str) -> String {
    let out = s.ish(expr);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{expr} failed: {err}");
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

fn json_of(s: &Scratch, expr: &str) -> serde_json::Value {
    let text = stdout_of(s, expr);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{expr} printed {text:?}, not JSON: {e}"))
}

#[test]
fn json_stringify_of_a_parsed_null_is_a_json_null() {
    let s = Scratch::new("stringify");
    let v = json_of(&s, r#"json.stringify(json.parse("{\"n\": null}"))"#);
    assert_eq!(v, serde_json::json!({"n": null}));
}

#[test]
fn json_stringify_of_a_bare_null_is_null() {
    let s = Scratch::new("bare");
    assert_eq!(stdout_of(&s, "json.stringify(null)"), "null");
}

#[test]
fn json_stringify_of_a_null_inside_an_array_is_null() {
    let s = Scratch::new("array");
    let v = json_of(&s, r#"json.stringify(from_json("[null]"))"#);
    assert_eq!(v, serde_json::json!([null]));
}

#[test]
fn to_json_writes_null_fields_as_null() {
    let s = Scratch::new("tojson");
    let v = json_of(&s, r#"{n: null, m: 1} | to_json"#);
    assert_eq!(v, serde_json::json!({"n": null, "m": 1}));
}

#[test]
fn to_csv_writes_a_null_cell_as_empty() {
    let s = Scratch::new("csv");
    let text = stdout_of(&s, r#"[{a: null, b: 1}] | to_csv"#);
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows, vec!["a,b", ",1"], "csv was {text:?}");
}

#[test]
fn save_json_writes_a_null_element_as_null() {
    let s = Scratch::new("save");
    let out = s.ish(r#"save_json("x.json", {n: [1, null]})"#);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = std::fs::read_to_string(s.cwd().join("x.json")).expect("x.json written");
    let v: serde_json::Value = serde_json::from_str(&text).expect("x.json is JSON");
    assert_eq!(v, serde_json::json!({"n": [1, null]}), "file was {text}");
}

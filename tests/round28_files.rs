//! Round 28: the file and format writers keep the bytes they were given.
//!
//! - `file_insert` at line 1 of a UTF-8 BOM file keeps the BOM as the first bytes.
//! - `file_edit` keeps a leading BOM and writes CRLF when every line break is
//!   CRLF; its refusals name the edit and the key that is wrong.
//! - `to_csv` refuses a repeated name in an explicit column list, and a NaN or
//!   infinite Float cell, as `to_json` refuses them.
//! - `open` on a directory says it is a directory, not a file.
//! - `to_yaml` double-quotes and escapes control characters; `from_yaml` reads
//!   `0o` and `0x` integers, and `to_yaml` quotes strings that look like them.
//! - `file_write` names the element of a mixed Array that is not a String.
//!
//! In process, against real files in a temp directory. Serialised, since the
//! journal is process-global.

use iron_shell::journal;
use iron_shell::safety::SafetyError;
use iron_shell::value::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

struct Session {
    dir: PathBuf,
}

impl Session {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae_r28_files_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::env::set_var("IRONSHELL_JOURNAL", "on");
        std::env::set_var("IRONSHELL_SESSION_DIR", dir.join("state"));
        journal::clear();
        Self { dir }
    }
    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        journal::clear();
        std::env::remove_var("IRONSHELL_JOURNAL");
        std::env::remove_var("IRONSHELL_SESSION_DIR");
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn call(name: &str, args: Vec<Value>) -> Result<Value, anyhow::Error> {
    let mut env = iron_shell::env::Env::new();
    iron_shell::builtins::call(name, args, &mut env)
}

/// The structured refusal a call answers with; panics if it succeeded or was
/// not structured.
fn refusal(name: &str, args: Vec<Value>) -> SafetyError {
    let err = call(name, args).expect_err("the call is refused");
    err.downcast_ref::<SafetyError>()
        .expect("a structured refusal")
        .clone()
}

fn record(pairs: &[(&str, Value)]) -> Value {
    let mut m = BTreeMap::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    Value::Record(m)
}

fn s(text: &str) -> Value {
    Value::Str(text.to_string())
}

fn text(v: Value) -> String {
    match v {
        Value::Str(t) => t,
        other => panic!("expected a String, got {other:?}"),
    }
}

#[test]
fn to_csv_refuses_a_repeated_column_name() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let rows = Value::Array(vec![record(&[("a", Value::Int(1))])]);
    let se = refusal("to_csv", vec![rows, Value::Array(vec![s("a"), s("a")])]);
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert!(
        se.message.contains("duplicate CSV column 'a'"),
        "{}",
        se.message
    );
}

#[test]
fn to_csv_refuses_a_nan_or_infinite_float_cell_with_its_path() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let nan = Value::Array(vec![record(&[
        ("a", Value::Float(1.5)),
        ("b", Value::Float(f64::NAN)),
    ])]);
    let se = refusal("to_csv", vec![nan]);
    assert_eq!(se.code.as_str(), "E_BAD_VALUE", "{}", se.message);
    assert!(se.message.contains("rows[0].b is NaN"), "{}", se.message);

    let inf = Value::Array(vec![
        record(&[("a", Value::Float(1.0))]),
        record(&[("a", Value::Float(f64::NEG_INFINITY))]),
    ]);
    let se = refusal("to_csv", vec![inf]);
    assert_eq!(se.code.as_str(), "E_BAD_VALUE", "{}", se.message);
    assert!(
        se.message.contains("rows[1].a is negative infinity"),
        "{}",
        se.message
    );
}


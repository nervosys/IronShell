//! Round 25: the file builtins say what they refused.
//!
//! - `file_write` and `file_append` refuse what they cannot write as text,
//!   instead of writing Rust Debug text.
//! - A directory given to an edit builtin is E_BAD_ARG "is a directory", not
//!   E_OS_DENIED, which Windows reports for a directory opened as a file.
//! - `file_patch` given a unified diff, and `file_edit` given an out-of-range
//!   line, say what they take, not the generic "takes String".
//!
//! `file_write("w.txt", {a: 1})` wrote `Record({"a": Int(1)})` and reported
//! success. A String is written as is and an Array of Strings is joined; any
//! other value is refused with E_BAD_ARG before a byte is written, and the hint
//! says how to convert it.
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
        let dir = std::env::temp_dir().join(format!("ae_r25_files_{tag}_{}", std::process::id()));
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

#[test]
fn file_write_refuses_a_record_and_writes_nothing() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let s = Session::new("record");
    let target = s.path("w.txt");
    let se = refusal(
        "file_write",
        vec![Value::Str(target.clone()), record(&[("a", Value::Int(1))])],
    );
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert!(se.message.contains("Record"), "{}", se.message);
    assert!(se.hint.contains("to_json(value)"), "{}", se.hint);
    assert!(se.hint.contains("to_toml(value)"), "{}", se.hint);
    assert!(
        !std::path::Path::new(&target).exists(),
        "nothing is written"
    );
}

#[test]
fn file_write_refuses_an_int_with_the_str_hint() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let s = Session::new("int");
    let target = s.path("n.txt");
    let se = refusal(
        "file_write",
        vec![Value::Str(target.clone()), Value::Int(42)],
    );
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert!(se.message.contains("Int"), "{}", se.message);
    assert!(se.hint.contains("str(value)"), "{}", se.hint);
    assert!(
        !std::path::Path::new(&target).exists(),
        "nothing is written"
    );
}

#[test]
fn file_write_refuses_a_piped_record_too() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let s = Session::new("piped");
    let target = s.path("p.txt");
    let mut env = iron_shell::env::Env::new();
    let err = iron_shell::builtins::call_with_input(
        "file_write",
        vec![Value::Str(target.clone())],
        Some(record(&[("a", Value::Int(1))])),
        &mut env,
    )
    .expect_err("a piped Record is refused");
    let se = err
        .downcast_ref::<SafetyError>()
        .expect("a structured refusal");
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert!(
        !std::path::Path::new(&target).exists(),
        "nothing is written"
    );
}

#[test]
fn file_write_refuses_an_array_holding_a_non_string() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let s = Session::new("array_int");
    let target = s.path("a.txt");
    let se = refusal(
        "file_write",
        vec![
            Value::Str(target.clone()),
            Value::Array(vec![Value::Str("ok".into()), Value::Int(7)]),
        ],
    );
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert!(se.message.contains("Int"), "{}", se.message);
    assert!(
        !std::path::Path::new(&target).exists(),
        "nothing is written"
    );
}

#[test]
fn file_write_still_writes_a_string_and_joins_an_array_of_strings() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let s = Session::new("ok");
    let plain = s.path("plain.txt");
    call(
        "file_write",
        vec![Value::Str(plain.clone()), Value::Str("hello".into())],
    )
    .expect("a String is written");
    assert_eq!(std::fs::read_to_string(&plain).unwrap(), "hello");

    let joined = s.path("joined.txt");
    call(
        "file_write",
        vec![
            Value::Str(joined.clone()),
            Value::Array(vec![Value::Str("a".into()), Value::Str("b".into())]),
        ],
    )
    .expect("an Array of Strings is written");
    assert_eq!(std::fs::read_to_string(&joined).unwrap(), "a\nb");
}

#[test]
fn file_append_refuses_a_record_and_leaves_the_file_alone() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let s = Session::new("append");
    let target = s.path("log.txt");
    call(
        "file_write",
        vec![Value::Str(target.clone()), Value::Str("keep".into())],
    )
    .expect("setup write");
    let se = refusal(
        "file_append",
        vec![Value::Str(target.clone()), record(&[("a", Value::Int(1))])],
    );
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert!(se.hint.contains("to_json(value)"), "{}", se.hint);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
}

#[test]
fn file_replace_on_a_directory_is_a_bad_value_not_a_permission_problem() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let s = Session::new("dir");
    let dir = s.path("somedir");
    std::fs::create_dir_all(&dir).expect("a directory");
    let se = refusal(
        "file_replace",
        vec![
            Value::Str(dir.clone()),
            Value::Str("a".into()),
            Value::Str("b".into()),
        ],
    );
    assert_ne!(se.code.as_str(), "E_OS_DENIED", "{}", se.message);
    // Linux answers EISDIR as its own I/O error, whose text is capitalised.
    assert!(
        se.message.to_lowercase().contains("is a directory"),
        "{}",
        se.message
    );
    if cfg!(windows) {
        assert_eq!(se.code.as_str(), "E_BAD_ARG", "{}", se.message);
        assert!(
            se.message.contains("is a directory, not a file"),
            "{}",
            se.message
        );
    }
}

#[test]
fn file_patch_given_a_unified_diff_names_the_record_form() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let s = Session::new("diff");
    let target = s.path("p.txt");
    call(
        "file_write",
        vec![Value::Str(target.clone()), Value::Str("a\n".into())],
    )
    .expect("setup write");
    let diff = "--- a/p.txt\n+++ b/p.txt\n@@ -1 +1 @@\n-a\n+b\n";
    let se = refusal(
        "file_patch",
        vec![Value::Str(target.clone()), Value::Str(diff.into())],
    );
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert!(
        se.message.contains("patches must be an array of records"),
        "{}",
        se.message
    );
    assert!(se.hint.contains("[{find, replace}]"), "{}", se.hint);
    assert!(se.hint.contains("unified diff"), "{}", se.hint);
    assert!(!se.hint.contains("takes String"), "{}", se.hint);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "a\n");
}

#[test]
fn file_edit_out_of_range_names_the_line_count() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let s = Session::new("edit");
    let target = s.path("e.txt");
    call(
        "file_write",
        vec![Value::Str(target.clone()), Value::Str("a\nb".into())],
    )
    .expect("setup write");
    let se = refusal(
        "file_edit",
        vec![
            Value::Str(target.clone()),
            record(&[
                ("op", Value::Str("replace".into())),
                ("line", Value::Int(9)),
                ("content", Value::Str("x".into())),
            ]),
        ],
    );
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert!(
        se.message.contains("start line out of range"),
        "{}",
        se.message
    );
    assert!(se.hint.contains("2 lines"), "{}", se.hint);
    assert!(se.hint.contains("from 1"), "{}", se.hint);
    assert!(!se.hint.contains("takes String"), "{}", se.hint);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "a\nb");
}

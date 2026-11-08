//! An edit that finds nothing is an error, not a success with a flag in it.
//!
//! `file_replace`, `file_patch` and `file_insert` refuse a pattern or anchor
//! that the file does not contain, with `E_NOT_FOUND`, and write nothing. The
//! refusal names the file and says what to do. A successful edit keeps its
//! record, with its counts.

use iron_shell::env::Env;
use iron_shell::safety::{ErrorCode, SafetyError};
use iron_shell::value::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

/// A private directory with one file in it. Its session state goes there too,
/// so no test writes the user's journal.
struct Dir {
    root: PathBuf,
}

impl Dir {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ae_edit_nf_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        std::env::set_var("IRONSHELL_SESSION_DIR", root.join("state"));
        Self { root }
    }

    fn file(&self, name: &str, body: &str) -> String {
        let path = self.root.join(name);
        std::fs::write(&path, body).expect("seed the file");
        path.to_string_lossy().into_owned()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::env::remove_var("IRONSHELL_SESSION_DIR");
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn call(name: &str, args: Vec<Value>) -> anyhow::Result<Value> {
    let mut env = Env::new();
    iron_shell::builtins::call(name, args, &mut env)
}

fn refusal(name: &str, args: Vec<Value>) -> SafetyError {
    call(name, args)
        .expect_err("an edit whose pattern is not in the file must be refused")
        .downcast_ref::<SafetyError>()
        .expect("a structured refusal")
        .clone()
}

fn field(v: &Value, key: &str) -> Value {
    match v {
        Value::Record(m) => m.get(key).cloned().unwrap_or(Value::Null),
        other => panic!("expected a record, got {other:?}"),
    }
}

fn patch(find: &str, replace: &str) -> Value {
    let mut m = BTreeMap::new();
    m.insert("find".to_string(), Value::Str(find.to_string()));
    m.insert("replace".to_string(), Value::Str(replace.to_string()));
    Value::Record(m)
}

#[test]
fn file_replace_with_no_match_is_e_not_found_and_writes_nothing() {
    let _g = lock();
    let dir = Dir::new("replace_miss");
    let path = dir.file("note.txt", "alpha\n");

    let se = refusal(
        "file_replace",
        vec![
            Value::Str(path.clone()),
            Value::Str("missing".into()),
            Value::Str("x".into()),
        ],
    );

    assert_eq!(se.code, ErrorCode::NotFound);
    assert_eq!(se.code.as_str(), "E_NOT_FOUND");
    assert_eq!(
        se.message,
        format!("file_replace: pattern not found in {path}")
    );
    assert!(se.hint.contains("Nothing was written"), "{}", se.hint);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "alpha\n");
}

#[test]
fn file_replace_success_keeps_its_record() {
    let _g = lock();
    let dir = Dir::new("replace_hit");
    let path = dir.file("note.txt", "alpha\n");

    let r = call(
        "file_replace",
        vec![
            Value::Str(path.clone()),
            Value::Str("alpha".into()),
            Value::Str("beta".into()),
        ],
    )
    .expect("a pattern the file contains is replaced");

    assert_eq!(field(&r, "success"), Value::Bool(true));
    assert_eq!(field(&r, "written"), Value::Bool(true));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "beta\n");
}

#[test]
fn file_patch_with_a_missing_pattern_is_refused_and_writes_nothing() {
    let _g = lock();
    let dir = Dir::new("patch_miss");
    let path = dir.file("p.txt", "a1 b2\n");

    let se = refusal(
        "file_patch",
        vec![
            Value::Str(path.clone()),
            Value::Array(vec![patch("a1", "A1"), patch("zz9", "Z")]),
        ],
    );

    assert_eq!(se.code.as_str(), "E_NOT_FOUND");
    assert!(
        se.message.starts_with("file_patch: pattern not found in"),
        "{}",
        se.message
    );
    assert!(se.hint.contains("patch 1"), "{}", se.hint);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a1 b2\n");
}

#[test]
fn file_insert_with_a_missing_anchor_is_refused_and_writes_nothing() {
    let _g = lock();
    let dir = Dir::new("insert_miss");
    let path = dir.file("i.txt", "alpha\nbeta\n");

    let se = refusal(
        "file_insert",
        vec![
            Value::Str(path.clone()),
            Value::Str("no-such-anchor".into()),
            Value::Str("new line".into()),
        ],
    );

    assert_eq!(se.code.as_str(), "E_NOT_FOUND");
    assert!(se.message.contains("no-such-anchor"), "{}", se.message);
    assert!(se.hint.contains("Nothing was written"), "{}", se.hint);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "alpha\nbeta\n");
}

#[test]
fn file_insert_after_a_present_anchor_succeeds() {
    let _g = lock();
    let dir = Dir::new("insert_hit");
    let path = dir.file("i.txt", "alpha\nbeta\n");

    let r = call(
        "file_insert",
        vec![
            Value::Str(path.clone()),
            Value::Str("alpha".into()),
            Value::Str("new line".into()),
        ],
    )
    .expect("an anchor the file contains is a valid insert");

    assert_eq!(field(&r, "success"), Value::Bool(true));
    assert_eq!(field(&r, "at_line"), Value::Int(2));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "alpha\nnew line\nbeta\n"
    );
}

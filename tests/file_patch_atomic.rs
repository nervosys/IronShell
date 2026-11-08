//! `file_patch` is all-or-nothing: a batch in which any patch does not apply
//! writes nothing, and says which patch failed.
//!
//! Reproduced on 14.0.0: `file_patch("p.txt", [{"search":"a1","replace":"A1"},
//! {"search":"zz9","replace":"Z"}])` wrote `A1` into the file and returned
//! success=false. The same call with every patch applying is unchanged, and so
//! is a single patch that does not apply (nothing was written before either).
//! Runs in process, against real files in a temp directory, because the claim
//! is about the filesystem and the journal. Serialised, since the journal is
//! process-global.

use iron_shell::journal;
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
        let dir = std::env::temp_dir().join(format!("ae_patch_{tag}_{}", std::process::id()));
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

fn call(name: &str, args: Vec<Value>) -> Result<Value, String> {
    let mut env = iron_shell::env::Env::new();
    iron_shell::builtins::call(name, args, &mut env).map_err(|e| e.to_string())
}

fn patch(search: &str, replace: &str) -> Value {
    let mut m = BTreeMap::new();
    m.insert("search".to_string(), Value::Str(search.to_string()));
    m.insert("replace".to_string(), Value::Str(replace.to_string()));
    Value::Record(m)
}

fn field(v: &Value, key: &str) -> Value {
    match v {
        Value::Record(m) => m.get(key).cloned().unwrap_or(Value::Null),
        other => panic!("expected a record, got {other:?}"),
    }
}

/// The refusal a batch gets when one of its patterns is not in the file.
fn refusal(s: &Session, file: &str, patches: Vec<Value>) -> iron_shell::safety::SafetyError {
    let mut env = iron_shell::env::Env::new();
    let err = iron_shell::builtins::call(
        "file_patch",
        vec![Value::Str(s.path(file)), Value::Array(patches)],
        &mut env,
    )
    .expect_err("a batch with a pattern the file does not contain is an error");
    err.downcast_ref::<iron_shell::safety::SafetyError>()
        .expect("a structured refusal")
        .clone()
}

fn run_patch(s: &Session, file: &str, patches: Vec<Value>) -> Value {
    call(
        "file_patch",
        vec![Value::Str(s.path(file)), Value::Array(patches)],
    )
    .expect("file_patch returns a record, not an error")
}

#[test]
fn a_batch_with_one_missing_patch_writes_nothing() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("partial");
    let p = s.path("p.txt");
    std::fs::write(&p, "a1 b2\n").expect("seed");

    let se = refusal(&s, "p.txt", vec![patch("a1", "A1"), patch("zz9", "Z")]);

    assert_eq!(
        std::fs::read_to_string(&p).expect("read"),
        "a1 b2\n",
        "the first patch must not be written when the second does not apply"
    );
    assert_eq!(se.code, iron_shell::safety::ErrorCode::NotFound);
    assert_eq!(se.code.as_str(), "E_NOT_FOUND");
    assert!(se.hint.contains("patch 1"), "{}", se.hint);
}

#[test]
fn a_failed_batch_records_no_journal_entry() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("journal");
    std::fs::write(s.path("p.txt"), "a1 b2\n").expect("seed");

    let _ = refusal(&s, "p.txt", vec![patch("a1", "A1"), patch("zz9", "Z")]);

    assert_eq!(
        journal::entries().len(),
        0,
        "a batch that wrote nothing must not enter the journal"
    );
}

#[test]
fn the_first_failing_patch_is_the_one_reported() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("first");
    let p = s.path("p.txt");
    std::fs::write(&p, "a1 b2\n").expect("seed");

    let se = refusal(
        &s,
        "p.txt",
        vec![patch("a1", "A1"), patch("zz9", "Z"), patch("b2", "B2")],
    );

    assert_eq!(std::fs::read_to_string(&p).expect("read"), "a1 b2\n");
    assert!(se.hint.contains("patch 1"), "{}", se.hint);
    assert!(se.hint.contains("zz9"), "{}", se.hint);
}

#[test]
fn a_single_patch_that_does_not_apply_writes_nothing() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("single");
    let p = s.path("p.txt");
    std::fs::write(&p, "a1 b2\n").expect("seed");

    let se = refusal(&s, "p.txt", vec![patch("zz9", "Z")]);

    assert_eq!(std::fs::read_to_string(&p).expect("read"), "a1 b2\n");
    assert_eq!(se.code.as_str(), "E_NOT_FOUND");
    assert!(se.hint.contains("patch 0"), "{}", se.hint);
}

#[test]
fn a_batch_that_all_applies_is_written_and_journalled() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("all");
    let p = s.path("p.txt");
    std::fs::write(&p, "a1 b2\n").expect("seed");

    let r = run_patch(&s, "p.txt", vec![patch("a1", "A1"), patch("b2", "B2")]);

    assert_eq!(std::fs::read_to_string(&p).expect("read"), "A1 B2\n");
    assert_eq!(field(&r, "success"), Value::Bool(true));
    assert_eq!(field(&r, "written"), Value::Bool(true));
    assert_eq!(field(&r, "failed_patch"), Value::Null);
    assert_eq!(field(&r, "patches_applied"), Value::Int(2));
    assert_eq!(
        journal::entries().len(),
        1,
        "a batch that wrote the file must be journalled"
    );
}

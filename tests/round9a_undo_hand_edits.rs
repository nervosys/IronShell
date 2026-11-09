//! `undo` does not overwrite a file that changed after its step.
//!
//! Each step records the state it left its file in. When a hand edit (or
//! anything outside the journal) changed the file since, undo refuses with
//! E_BAD_STATE and names the file, unless `force` is set.

use iron_shell::journal;
use iron_shell::value::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

/// The journal and `IRONSHELL_JOURNAL` are process-global.
static LOCK: Mutex<()> = Mutex::new(());

struct Session {
    dir: PathBuf,
}

impl Session {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae_r9a_undo_{tag}_{}", std::process::id()));
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

/// `undo(1, {force: true})`.
fn forced_undo() -> Result<Value, String> {
    let mut options = BTreeMap::new();
    options.insert("force".to_string(), Value::Bool(true));
    call("undo", vec![Value::Int(1), Value::Record(options)])
}

fn field(v: &Value, key: &str) -> Value {
    match v {
        Value::Record(m) => m.get(key).cloned().unwrap_or(Value::Null),
        other => panic!("expected a record, got {other:?}"),
    }
}

fn s(text: &str) -> Value {
    Value::Str(text.to_string())
}

#[test]
fn a_hand_edit_after_a_replace_is_refused_and_the_file_keeps_it() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let session = Session::new("replace_then_hand_edit");
    let p = session.path("stale.txt");
    std::fs::write(&p, "v1").expect("seed");
    call("file_replace", vec![s(&p), s("v1"), s("v2")]).expect("replace");
    std::fs::write(&p, "v3-by-hand").expect("hand edit");

    let refused = call("undo", vec![Value::Int(1)]).expect_err("undo must refuse");
    assert!(
        refused.contains("changed after this step"),
        "the refusal names the change: {refused}"
    );
    assert!(refused.contains("E_BAD_STATE"), "{refused}");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "v3-by-hand");
    assert_eq!(journal::entries().len(), 1, "the step stays journalled");

    let forced = forced_undo().expect("forced undo");
    assert_eq!(field(&forced, "restored"), Value::Int(1));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "v1");
    assert_eq!(journal::entries().len(), 0);
}

#[test]
fn a_file_a_step_created_and_changed_since_is_refused() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let session = Session::new("created_then_changed");
    let p = session.path("made.txt");
    call("file_write", vec![s(&p), s("step")]).expect("write");
    std::fs::write(&p, "someone else's text").expect("hand edit");

    assert!(call("undo", vec![Value::Int(1)]).is_err());
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "someone else's text",
        "the created file keeps the hand edit"
    );
    let forced = forced_undo().expect("forced undo");
    assert_eq!(field(&forced, "removed"), Value::Int(1));
    assert!(!std::path::Path::new(&p).exists());
}

#[test]
fn an_undo_with_no_hand_edit_still_restores() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let session = Session::new("no_hand_edit");
    let p = session.path("plain.txt");
    std::fs::write(&p, "v1").expect("seed");
    call("file_replace", vec![s(&p), s("v1"), s("v2")]).expect("replace");

    let undone = call("undo", vec![Value::Int(1)]).expect("undo");
    assert_eq!(field(&undone, "restored"), Value::Int(1));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "v1");
}

#[test]
fn a_file_removed_by_hand_after_its_step_is_restored_not_refused() {
    // Nothing is there to discard, so the restore loses nothing.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let session = Session::new("removed_by_hand");
    let p = session.path("gone.txt");
    std::fs::write(&p, "v1").expect("seed");
    call("file_replace", vec![s(&p), s("v1"), s("v2")]).expect("replace");
    std::fs::remove_file(&p).expect("remove by hand");

    let undone = call("undo", vec![Value::Int(1)]).expect("undo");
    assert_eq!(field(&undone, "restored"), Value::Int(1));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "v1");
}

#[test]
fn a_refusal_after_an_undo_that_worked_is_an_outcome_not_an_error() {
    // The newer step is undone; the older one's file was changed since, so the
    // undo stops there and reports it, with the first step done.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let session = Session::new("refusal_second");
    let a = session.path("a.txt");
    let b = session.path("b.txt");
    std::fs::write(&a, "a0").expect("seed a");
    std::fs::write(&b, "b0").expect("seed b");
    call("file_replace", vec![s(&a), s("a0"), s("a1")]).expect("replace a");
    call("file_replace", vec![s(&b), s("b0"), s("b1")]).expect("replace b");
    std::fs::write(&a, "a-by-hand").expect("hand edit a");

    let report = call("undo", vec![Value::Int(2)]).expect("the first step undoes");
    assert_eq!(field(&report, "restored"), Value::Int(1));
    assert_eq!(field(&report, "complete"), Value::Bool(false));
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "b0");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "a-by-hand");
}

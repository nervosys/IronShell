//! In agent mode, deleting a path that is not there says so before it asks for
//! an approval token.
//!
//! `rm("missing.txt")` used to answer `E_NEEDS_APPROVAL` and issue a token to
//! delete nothing. Now a plain removal of an absent path inside the workspace is
//! `E_NOT_FOUND`. A path that exists still goes through the gate, and a path
//! outside the workspace still answers to the jail.

use iron_shell::builtins;
use iron_shell::env::Env;
use iron_shell::safety::{ErrorCode, SafetyError};
use iron_shell::value::Value;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

struct Ws {
    root: std::path::PathBuf,
    outside: std::path::PathBuf,
}

impl Ws {
    fn new(tag: &str) -> Self {
        let base = std::env::temp_dir().join(format!("ae_rmmg_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("ws");
        std::fs::create_dir_all(&root).expect("workspace");
        std::env::set_var("IRONSHELL_MODE", "agent");
        std::env::set_var("IRONSHELL_WORKSPACE", &root);
        std::env::remove_var("IRONSHELL_APPROVE");
        std::env::remove_var("IRONSHELL_APPROVE_ALL");
        Self {
            outside: base.join("outside_missing.txt"),
            root,
        }
    }
    fn path(&self, name: &str) -> String {
        self.root.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Ws {
    fn drop(&mut self) {
        std::env::remove_var("IRONSHELL_WORKSPACE");
        std::env::remove_var("IRONSHELL_MODE");
        if let Some(base) = self.root.parent() {
            let _ = std::fs::remove_dir_all(base);
        }
    }
}

fn call(name: &str, args: Vec<Value>) -> anyhow::Result<Value> {
    let mut env = Env::new();
    builtins::call(name, args, &mut env)
}

fn refusal(r: &anyhow::Result<Value>) -> Option<SafetyError> {
    match r {
        Err(e) => e.downcast_ref::<SafetyError>().cloned(),
        Ok(_) => None,
    }
}

#[test]
fn removing_a_missing_file_is_not_found_and_issues_no_token() {
    let _g = lock();
    let ws = Ws::new("missing");
    let r = call("rm", vec![Value::Str(ws.path("missing.txt"))]);
    let e = refusal(&r).expect("an error");
    assert_eq!(e.code, ErrorCode::NotFound, "{r:?}");
    assert!(
        e.approval.is_none(),
        "no token for a delete that cannot happen"
    );
}

#[test]
fn removing_an_existing_file_still_needs_approval() {
    let _g = lock();
    let ws = Ws::new("exists");
    std::fs::write(ws.path("c.txt"), "x").unwrap();
    let r = call("rm", vec![Value::Str(ws.path("c.txt"))]);
    let e = refusal(&r).expect("an error");
    assert_eq!(e.code, ErrorCode::NeedsApproval, "{r:?}");
    assert!(e.approval.is_some(), "the gate still issues its token");
    assert!(
        std::path::Path::new(&ws.path("c.txt")).exists(),
        "a refused delete must not delete"
    );
}

#[test]
fn force_on_a_missing_file_still_goes_through_the_gate() {
    // `force` is a success for an absent file, so it is not a "does not exist"
    // refusal. It keeps the gate it had.
    let _g = lock();
    let ws = Ws::new("force");
    let r = call(
        "rm",
        vec![Value::Str(ws.path("missing.txt")), Value::Bool(true)],
    );
    let e = refusal(&r).expect("an error");
    assert_eq!(e.code, ErrorCode::NeedsApproval, "{r:?}");
}

#[test]
fn a_missing_path_outside_the_workspace_still_answers_to_the_jail() {
    let _g = lock();
    let ws = Ws::new("outside");
    let r = call(
        "rm",
        vec![Value::Str(ws.outside.to_string_lossy().into_owned())],
    );
    let e = refusal(&r).expect("an error");
    assert_eq!(e.code, ErrorCode::OutsideWorkspace, "{r:?}");
}

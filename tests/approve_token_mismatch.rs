//! A set `IRONSHELL_APPROVE` that approves nothing says so.
//!
//! The gate ignored a token that did not match the call, and answered with a
//! fresh `E_NEEDS_APPROVAL` that did not mention the variable at all. The
//! refusal's hint now says the variable was set and that none of its tokens
//! approve this call. An unset variable, and one holding the right token, do not
//! get that note.

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
}

impl Ws {
    fn new(tag: &str) -> Self {
        let base = std::env::temp_dir().join(format!("ae_apmm_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("ws");
        std::fs::create_dir_all(&root).expect("workspace");
        std::env::set_var("IRONSHELL_MODE", "agent");
        std::env::set_var("IRONSHELL_WORKSPACE", &root);
        std::env::remove_var("IRONSHELL_APPROVE");
        std::env::remove_var("IRONSHELL_APPROVE_ALL");
        Self { root }
    }
    fn path(&self, name: &str) -> String {
        self.root.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Ws {
    fn drop(&mut self) {
        std::env::remove_var("IRONSHELL_APPROVE");
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

fn refusal(r: &anyhow::Result<Value>) -> SafetyError {
    match r {
        Err(e) => e
            .downcast_ref::<SafetyError>()
            .cloned()
            .unwrap_or_else(|| panic!("structured error expected: {e:#}")),
        Ok(v) => panic!("expected a refusal, got {v:?}"),
    }
}

const NOTE: &str = "none of its tokens approve this call";

#[test]
fn a_set_variable_that_matches_nothing_is_named_in_the_hint() {
    let _g = lock();
    let ws = Ws::new("miss");
    std::fs::write(ws.path("c.txt"), "x").unwrap();
    std::env::set_var("IRONSHELL_APPROVE", "apv_0000000000000000");
    let e = refusal(&call("rm", vec![Value::Str(ws.path("c.txt"))]));
    assert_eq!(e.code, ErrorCode::NeedsApproval);
    assert!(
        e.hint.contains("IRONSHELL_APPROVE was set"),
        "the hint must say the variable was set: {}",
        e.hint
    );
    assert!(e.hint.contains(NOTE), "{}", e.hint);
    assert!(
        e.hint.contains("bound to the exact action and target"),
        "{}",
        e.hint
    );
    // The hint still names the token the call needs, so the next try can use it.
    let token = e.approval.expect("token").token;
    assert!(e.hint.contains(&token), "{}", e.hint);
}

#[test]
fn an_unset_variable_gets_no_such_note() {
    let _g = lock();
    let ws = Ws::new("unset");
    std::fs::write(ws.path("c.txt"), "x").unwrap();
    let e = refusal(&call("rm", vec![Value::Str(ws.path("c.txt"))]));
    assert_eq!(e.code, ErrorCode::NeedsApproval);
    assert!(!e.hint.contains(NOTE), "{}", e.hint);
}

#[test]
fn a_variable_with_only_blanks_gets_no_such_note() {
    let _g = lock();
    let ws = Ws::new("blank");
    std::fs::write(ws.path("c.txt"), "x").unwrap();
    std::env::set_var("IRONSHELL_APPROVE", " , ");
    let e = refusal(&call("rm", vec![Value::Str(ws.path("c.txt"))]));
    assert!(!e.hint.contains(NOTE), "{}", e.hint);
}

#[test]
fn the_matching_token_approves_and_the_note_is_not_needed() {
    let _g = lock();
    let ws = Ws::new("match");
    std::fs::write(ws.path("c.txt"), "x").unwrap();
    let first = refusal(&call("rm", vec![Value::Str(ws.path("c.txt"))]));
    let token = first.approval.expect("token").token;

    // A wrong token in front of the right one still approves the call.
    std::env::set_var("IRONSHELL_APPROVE", format!("apv_0000000000000000,{token}"));
    let r = call("rm", vec![Value::Str(ws.path("c.txt"))]);
    assert!(r.is_ok(), "the matching token must approve: {r:?}");
    assert!(!std::path::Path::new(&ws.path("c.txt")).exists());
}

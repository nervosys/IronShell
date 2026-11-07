//! A move needs one approval token, and that token approves exactly one move.
//!
//! `file_move` guarded its source and its destination as two separate actions,
//! so the first refusal named the source's token and a re-run with it was
//! refused again for the destination. Only `IRONSHELL_APPROVE="tok1,tok2"` made
//! the move go through, and the hint named one token.
//!
//! Now the move is one guarded action over both paths. Its token is bound to
//! the source and the destination together, so approving it approves that
//! source-to-destination pair and no other.

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
        let root = std::env::temp_dir().join(format!("ae_fmot_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
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
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn call(name: &str, args: Vec<Value>) -> anyhow::Result<Value> {
    let mut env = Env::new();
    builtins::call(name, args, &mut env)
}

fn needs_approval(r: &anyhow::Result<Value>) -> Option<SafetyError> {
    match r {
        Err(e) => e
            .downcast_ref::<SafetyError>()
            .filter(|s| s.code == ErrorCode::NeedsApproval)
            .cloned(),
        Ok(_) => None,
    }
}

#[test]
fn one_token_covers_the_whole_move_and_the_hinted_rerun_succeeds() {
    let _g = lock();
    let ws = Ws::new("one");
    std::fs::write(ws.path("c.txt"), "payload").unwrap();

    let first = call(
        "file_move",
        vec![
            Value::Str(ws.path("c.txt")),
            Value::Str(ws.path("moved.txt")),
        ],
    );
    let refusal = needs_approval(&first).expect("a move in agent mode needs approval");
    let token = refusal.approval.expect("the refusal carries a token").token;
    assert!(
        refusal.hint.contains(&format!("IRONSHELL_APPROVE={token}")),
        "the hint must name the one token: {}",
        refusal.hint
    );

    // The hint's re-run, with exactly the token it names.
    std::env::set_var("IRONSHELL_APPROVE", &token);
    let second = call(
        "file_move",
        vec![
            Value::Str(ws.path("c.txt")),
            Value::Str(ws.path("moved.txt")),
        ],
    );
    assert!(
        second.is_ok(),
        "one re-run with the named token must complete the move: {second:?}"
    );
    assert!(!std::path::Path::new(&ws.path("c.txt")).exists());
    assert_eq!(
        std::fs::read_to_string(ws.path("moved.txt")).unwrap(),
        "payload"
    );
}

#[test]
fn a_token_for_one_move_does_not_approve_another() {
    let _g = lock();
    let ws = Ws::new("bind");
    std::fs::write(ws.path("a.txt"), "a").unwrap();

    let first = call(
        "file_move",
        vec![Value::Str(ws.path("a.txt")), Value::Str(ws.path("b.txt"))],
    );
    let token_for_b = needs_approval(&first)
        .and_then(|s| s.approval)
        .expect("token for a to b")
        .token;

    // The same source with another destination is a different action.
    std::env::set_var("IRONSHELL_APPROVE", &token_for_b);
    let other = call(
        "file_move",
        vec![Value::Str(ws.path("a.txt")), Value::Str(ws.path("z.txt"))],
    );
    let refusal = needs_approval(&other).expect("a different destination needs its own token");
    assert_ne!(refusal.approval.expect("token").token, token_for_b);
    assert!(
        std::path::Path::new(&ws.path("a.txt")).exists(),
        "the refused move must not have run"
    );
    assert!(!std::path::Path::new(&ws.path("z.txt")).exists());
}

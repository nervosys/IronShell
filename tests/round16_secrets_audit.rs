//! Secret handling and audit defects (round 16). Each test drives the real
//! binary in agent mode, in a scratch workspace with its own session directory,
//! as an agent harness would.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh scratch directory: `proj` is the workspace, `session` holds the
/// journal, so no test shares state with another or with the repo.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round16_secrets_audit")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj")).expect("workspace dir");
    std::fs::create_dir_all(dir.join("session")).expect("session dir");
    dir
}

/// Run `code` in agent mode with `dir/proj` as the workspace.
fn ish_agent(dir: &Path, code: &str) -> Output {
    let ws = dir.join("proj");
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--agent", "--workspace"])
        .arg(&ws)
        .args(["-c", code])
        .current_dir(&ws)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_APPROVE_ALL")
        .env_remove("IRONSHELL_APPROVE")
        .env_remove("IRONSHELL_WORKSPACE")
        .output()
        .expect("run ish")
}

// ── Item 1: the audit log names a written path, never the content written ──

#[test]
fn an_audit_entry_for_a_content_write_names_the_path_but_not_the_content() {
    let dir = scratch("audit_content");
    let ws = dir.join("proj");
    std::fs::write(
        ws.join(".env"),
        "API_KEY=sk-test-123456789\nDB_PASSWORD=hunter2hunter2\n",
    )
    .expect("seed .env");

    let out = ish_agent(
        &dir,
        r#"file_replace(".env", "hunter2hunter2", "rotated-pw-9988")"#,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let log = std::fs::read_to_string(ws.join(".ironshell").join("audit.log"))
        .expect("the agent-mode audit log");
    assert!(log.contains("file_replace"), "{log}");
    assert!(log.contains("allow_unguarded"), "{log}");
    assert!(log.contains(".env"), "{log}");
    assert!(log.contains("<2 more arguments>"), "{log}");
    assert!(!log.contains("hunter2hunter2"), "{log}");
    assert!(!log.contains("rotated-pw-9988"), "{log}");
}

// ── Item 3: cat over several files keeps a file that lacks a newline apart ──

fn run_in_process(src: &str) -> iron_shell::value::Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

#[test]
fn cat_of_files_without_trailing_newlines_keeps_their_lines_apart() {
    let dir = scratch("cat_join");
    let proj = dir.join("proj");
    std::fs::write(proj.join("a.txt"), "one").expect("write a");
    std::fs::write(proj.join("b.txt"), "two").expect("write b");
    let glob = format!("{}/*.txt", proj.display().to_string().replace('\\', "/"));

    // The glob matches in either order; both orders must keep the lines apart.
    let text = run_in_process(&format!(r#"cat("{glob}")"#));
    assert!(
        text == iron_shell::value::Value::Str("one\ntwo".into())
            || text == iron_shell::value::Value::Str("two\none".into()),
        "{text:?}"
    );
    let n = run_in_process(&format!(r#"cat("{glob}") | lines | len"#));
    assert_eq!(n, iron_shell::value::Value::Int(2));
}

// ── Item 4: `+` concatenates two Arrays ──

#[test]
fn plus_concatenates_two_arrays_into_a_new_array() {
    use iron_shell::value::Value;
    let v = run_in_process("[1, 2] + [3]");
    assert_eq!(
        v,
        Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn the_typechecker_accepts_array_plus_array() {
    use iron_shell::types::Type;
    let t = iron_shell::typecheck::infer_last_type("[1] + [2]").expect("typechecks");
    assert_eq!(t, Type::Array(Box::new(Type::Int)));
}

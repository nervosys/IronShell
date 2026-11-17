//! Round 49c: gaps a regression test of the workspace jail found.
//!
//! One section per item; the CLI tests drive the real binary in a scratch
//! directory with its own session directory.

#![allow(dead_code)]

use iron_shell::value::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn decision(code: &str) -> String {
    let escaped = code.replace('\\', "\\\\").replace('"', "\\\"");
    match run(&format!("explain_effects(\"{escaped}\").decision")) {
        Value::Str(s) => s,
        other => panic!("{other:?}"),
    }
}

struct Scratch {
    root: PathBuf,
    ws: PathBuf,
    outside: PathBuf,
    session: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ae-r49c-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let s = Scratch {
            ws: root.join("ws"),
            outside: root.join("outside"),
            session: root.join("session"),
            root,
        };
        for p in [&s.ws, &s.outside, &s.session] {
            std::fs::create_dir_all(p).unwrap();
        }
        s
    }

    /// Agent mode, `ws` the workspace.
    fn agent(&self, expr: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["--agent", "--workspace"])
            .arg(&self.ws)
            .args(["-c", expr])
            .current_dir(&self.ws)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_APPROVE_ALL")
            .env_remove("IRONSHELL_APPROVE")
            .env_remove("IRONSHELL_WORKSPACE")
            .output()
            .expect("run ish")
    }

    /// A plain `ish` with extra raw arguments.
    fn plain(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(&self.ws)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_WORKSPACE")
            .output()
            .expect("run ish")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let jx = self.ws.join("jx");
        if std::fs::symlink_metadata(&jx).is_ok() {
            let _ = std::fs::remove_dir(&jx).or_else(|_| std::fs::remove_file(&jx));
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

// ── Item 5: a read does not name an alternate data stream ──

#[cfg(windows)]
#[test]
fn a_read_of_a_stream_name_is_refused_and_explain_agrees() {
    let s = Scratch::new("ads");
    let out = s.agent(r#"file.read("x.txt:ads")"#);
    let shown = text(&out);
    assert!(!out.status.success(), "{shown}");
    assert!(shown.contains("E_BAD_ARG"), "{shown}");
    assert!(shown.contains("alternate data stream"), "{shown}");
    assert_eq!(decision(r#"file.read("x.txt:ads")"#), "deny");
}

#[test]
fn an_ordinary_read_name_is_not_a_stream() {
    assert_ne!(decision(r#"file.read("x.txt")"#), "deny");
}

// ── Item 4: explain judges an Exec builtin's path parameter ──

#[test]
fn explain_denies_sqlite_exec_on_an_outside_database() {
    let outside = std::env::temp_dir().join("r49c-outside.db");
    let lit = outside.to_string_lossy().replace('\\', "/");
    assert_eq!(
        decision(&format!(r#"db_sqlite_exec("{lit}", "select 1")"#)),
        "deny"
    );
    assert_eq!(
        decision(&format!(r#"db.sqlite_exec("{lit}", "select 1")"#)),
        "deny"
    );
}

// ── Item 7: extra arguments after -c say why they are probably there ──

#[test]
fn arguments_after_c_hint_at_a_split_quote() {
    let s = Scratch::new("cargs");
    let out = s.plain(&["-c", "1 + 1", "x"]);
    let shown = text(&out);
    assert!(shown.contains("arguments after -c are ignored"), "{shown}");
    assert!(shown.contains("split it at a quote"), "{shown}");
    assert!(shown.contains(".ish file"), "{shown}");
}

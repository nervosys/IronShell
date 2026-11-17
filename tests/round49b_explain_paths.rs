//! `--explain` lists the path arguments each call's jail check judges, and
//! prints one compact line when stdout is not a terminal or in agent mode.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round49b_explain_paths")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("ws")).expect("workspace");
    std::fs::create_dir_all(dir.join("session")).expect("session");
    dir
}

fn explain(dir: &Path, extra: &[&str], src: &str) -> String {
    let mut args: Vec<&str> = extra.to_vec();
    args.extend_from_slice(&["--explain", "-c", src]);
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(&args)
        .current_dir(dir.join("ws"))
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("run ish");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn each_call_lists_the_paths_it_judges() {
    let dir = scratch("paths");
    let text = explain(&dir, &[], r#"cat("a.txt"); ls("src")"#);
    let json: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    let calls = json["calls"].as_array().expect("calls");
    assert_eq!(calls[0]["paths"], serde_json::json!(["a.txt"]), "{text}");
    assert_eq!(calls[1]["paths"], serde_json::json!(["src"]), "{text}");
}

#[test]
fn a_call_with_no_path_has_no_paths_key() {
    let dir = scratch("nopaths");
    let text = explain(&dir, &[], r#"upper("x")"#);
    let json: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert!(json["calls"][0].get("paths").is_none(), "{text}");
}

#[test]
fn piped_stdout_gets_one_compact_line() {
    let dir = scratch("compact");
    let text = explain(&dir, &[], r#"cat("a.txt")"#);
    assert_eq!(text.trim_end().lines().count(), 1, "{text}");
    assert!(text.contains("\"decision\":"), "{text}");
}

#[test]
fn agent_mode_is_compact_too() {
    let dir = scratch("agent");
    let text = explain(&dir, &["--agent", "--workspace", "."], r#"cat("a.txt")"#);
    assert_eq!(text.trim_end().lines().count(), 1, "{text}");
}

//! Round 22: documentation and a handle read whole are shown whole.
//!
//! `help("undo")` in agent mode is a 6.5 KB record, over the 2 KB handle
//! threshold, so it came back as `@handle h1` with a type-only preview, and
//! `handle("h1")` then came back as `@handle h2`: the full text could not be
//! read without a narrowing pipeline. Reading documentation, or reading a
//! handle whole, now prints the value; a large ordinary result is still handled.

use std::path::PathBuf;
use std::process::Command;

fn session(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ish_r22h_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("session dir");
    dir
}

fn agent(dir: &PathBuf, code: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--agent", "-c", code])
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env_remove("IRONSHELL_HANDLE_BYTES")
        .output()
        .expect("run ish");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn help_is_shown_whole_in_agent_mode() {
    let dir = session("help");
    let out = agent(&dir, r#"help("undo")"#);
    assert!(!out.contains("@handle"), "{out}");
    assert!(out.contains("journal"), "the description is printed: {out}");
}

#[test]
fn ontology_describe_is_shown_whole_in_agent_mode() {
    let dir = session("describe");
    let out = agent(&dir, r#"ontology_describe("undo")"#);
    assert!(!out.contains("@handle"), "{out}");
}

#[test]
fn a_large_result_is_still_a_handle_and_reads_back_whole() {
    let dir = session("whole");
    let produced = agent(&dir, "range(1, 600) | map(fn(i) => { n: i, m: i * 7919 })");
    let id = produced
        .lines()
        .find_map(|l| l.strip_prefix("@handle "))
        .unwrap_or_else(|| panic!("a large result is still handled: {produced}"))
        .trim()
        .to_string();
    let whole = agent(&dir, &format!("handle(\"{id}\")"));
    assert!(
        !whole.contains("@handle"),
        "reading a handle whole mints no new one: {whole}"
    );
    assert!(
        whole.contains("4743481"),
        "the last row (599 * 7919) is there: {whole}"
    );
}

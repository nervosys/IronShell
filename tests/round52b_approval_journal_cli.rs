//! Round 52b: approval blast radius, approval token text, undo of a step done
//! twice, and CLI argument errors in agent mode.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round52b")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj")).expect("workspace dir");
    std::fs::create_dir_all(dir.join("session")).expect("session dir");
    dir
}

fn ish_agent(dir: &Path, code: &str, approve: Option<&str>) -> Output {
    let ws = dir.join("proj");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["--agent", "--workspace"])
        .arg(&ws)
        .args(["-c", code])
        .current_dir(&ws)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_APPROVE_ALL")
        .env_remove("IRONSHELL_APPROVE")
        .env_remove("IRONSHELL_WORKSPACE");
    if let Some(token) = approve {
        cmd.env("IRONSHELL_APPROVE", token);
    }
    cmd.output().expect("run ish")
}

fn first_error(out: &Output) -> serde_json::Value {
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().next().unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {err:?}"))
}

// ── B1: a recursive delete reports the size of the tree ──

#[test]
fn rmdir_recursive_blast_radius_counts_the_tree() {
    let dir = scratch("b1_rmdir");
    let d = dir.join("proj").join("d");
    std::fs::create_dir_all(d.join("a")).unwrap();
    std::fs::create_dir_all(d.join("b")).unwrap();
    for f in ["x1", "x2", "a/y1", "a/y2", "b/z1", "b/z2"] {
        std::fs::write(d.join(f), "x").unwrap();
    }
    let out = ish_agent(&dir, r#"rmdir("d", true)"#, None);
    let json = first_error(&out);
    assert_eq!(json["error"]["code"], "E_NEEDS_APPROVAL", "{json}");
    let radius = &json["error"]["approval"]["blast_radius"];
    assert_eq!(radius["files"], 6, "{json}");
    assert_eq!(radius["dirs"], 2, "{json}");
    assert!(d.exists(), "a refused delete removes nothing");
}

#[test]
fn the_token_does_not_change_with_the_tree() {
    let dir = scratch("b1_token");
    let d = dir.join("proj").join("d");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("one"), "x").unwrap();
    let first = first_error(&ish_agent(&dir, r#"rmdir("d", true)"#, None));
    std::fs::write(d.join("two"), "x").unwrap();
    let second = first_error(&ish_agent(&dir, r#"rmdir("d", true)"#, None));
    assert_eq!(
        first["error"]["approval"]["token"], second["error"]["approval"]["token"],
        "counts describe the action; the token binds action and target"
    );
}

// ── B2: the hint says what a token approves ──

#[test]
fn the_approval_hint_says_a_token_is_good_for_every_later_run() {
    let dir = scratch("b2_hint");
    std::fs::write(dir.join("proj").join("f.txt"), "x").unwrap();
    let json = first_error(&ish_agent(&dir, r#"rm("f.txt")"#, None));
    let hint = json["error"]["hint"].as_str().unwrap_or("");
    assert!(
        hint.contains("exact action on that exact target every time it is presented"),
        "{hint}"
    );
    assert!(
        hint.contains("until it is removed from IRONSHELL_APPROVE"),
        "{hint}"
    );
}

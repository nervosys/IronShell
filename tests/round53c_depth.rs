//! Round 53c: reading a file more than 50 directories below the workspace root
//! was `E_UNKNOWN` ("Path exceeds maximum depth of 50 ...", hint "grep failed
//! without a specific error code"). It is `E_BUDGET_EXCEEDED`, names the limit
//! and says what to do.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round53c_depth")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("session");
    std::fs::create_dir_all(dir.join("ws")).expect("ws");
    dir
}

fn deep(dir: &Path, levels: usize) {
    let mut p = dir.join("ws").join("tree");
    for _ in 0..levels {
        p.push("d");
    }
    std::fs::create_dir_all(&p).expect("deep dirs");
    std::fs::write(p.join("x.txt"), "x marks\n").expect("file");
}

fn agent(dir: &Path, code: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--agent", "-c", code])
        .current_dir(dir.join("ws"))
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_WORKSPACE")
        .output()
        .expect("run ish")
}

#[test]
fn a_file_62_deep_is_a_coded_budget_error_with_a_hint() {
    let dir = scratch("deep");
    deep(&dir, 62);
    let out = agent(&dir, r#"grep("x", "tree/**/*.txt")"#);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    let json: serde_json::Value = serde_json::from_str(err.lines().next().unwrap_or(""))
        .unwrap_or_else(|e| {
            panic!("not JSON ({e}): {} bytes of stderr", err.len());
        });
    assert_eq!(json["error"]["code"], "E_BUDGET_EXCEEDED", "{json}");
    let message = json["error"]["message"].as_str().unwrap_or("");
    assert!(message.contains("50"), "the limit is not named: {message}");
    let hint = json["error"]["hint"].as_str().unwrap_or("");
    assert!(!hint.contains("without a specific error code"), "{hint}");
    assert!(hint.contains("nearer") || hint.contains("narrow"), "{hint}");
}

#[test]
fn a_shallow_tree_is_read_as_before() {
    let dir = scratch("shallow");
    deep(&dir, 10);
    let out = agent(&dir, r#"grep("x", "tree/**/*.txt") | len"#);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "1");
}

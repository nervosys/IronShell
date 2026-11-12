//! Round 23: the file-editing, journal and agent-facing messages that a
//! workflow relies on. Each test runs the real `ish` binary against a temp
//! directory, so the journal, the session directory and the exit codes are the
//! ones an agent sees.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ish_r23w_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script in `dir` with the journal on, and returns
/// (stdout, stderr, exit code). A script echoes its last value.
fn run_script(dir: &Path, source: &str) -> (String, String, i32) {
    let script = dir.join("round23.ish");
    std::fs::write(&script, source).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg(&script)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_JOURNAL", "on")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("run ish");
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn undo_reverses_a_json_set() {
    let dir = scratch("json_undo");
    let original = "{\n  \"version\": \"1\"\n}\n";
    std::fs::write(dir.join("config.json"), original).expect("seed file");
    let (_, err, code) = run_script(
        &dir,
        "json_set(\"config.json\", \"version\", \"2\")\nundo()\n",
    );
    assert_eq!(code, 0, "stderr: {err}");
    let after = std::fs::read_to_string(dir.join("config.json")).expect("read back");
    assert_eq!(after, original, "undo restores the original bytes");
    let _ = std::fs::remove_dir_all(&dir);
}

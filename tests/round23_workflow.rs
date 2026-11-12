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

#[test]
fn json_set_refuses_an_index_into_a_missing_array() {
    let dir = scratch("json_index_missing");
    let original = "{}\n";
    std::fs::write(dir.join("config.json"), original).expect("seed file");
    let (out, err, code) = run_script(&dir, "json_set(\"config.json\", \"tags.0\", \"x\")\n");
    let text = format!("{out}{err}");
    assert_ne!(code, 0, "the index is refused: {text}");
    assert!(
        text.contains("tags is missing; an array index needs an existing array"),
        "{text}"
    );
    let after = std::fs::read_to_string(dir.join("config.json")).expect("read back");
    assert_eq!(after, original, "a refused edit leaves the file alone");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_numeric_key_on_an_existing_object_is_an_object_key() {
    let dir = scratch("json_numeric_key");
    std::fs::write(dir.join("config.json"), "{\"a\": {}}\n").expect("seed file");
    let (_, err, code) = run_script(&dir, "json_set(\"config.json\", \"a.0\", \"x\")\n");
    assert_eq!(code, 0, "stderr: {err}");
    let after = std::fs::read_to_string(dir.join("config.json")).expect("read back");
    assert!(after.contains("\"0\""), "the key is created: {after}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audit_verify_of_a_missing_log_is_an_error_naming_the_path() {
    let dir = scratch("audit_missing");
    let (out, err, code) = run_script(&dir, "audit_verify(\"no-such-audit.log\")\n");
    let text = format!("{out}{err}");
    assert_ne!(code, 0, "a missing log is not a verdict: {text}");
    assert!(text.contains("E_NOT_FOUND"), "{text}");
    assert!(text.contains("no-such-audit.log"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_write_into_a_missing_folder_says_to_create_the_folder() {
    let dir = scratch("write_missing_folder");
    let (out, err, code) = run_script(&dir, "file_write(\"nodir/new.txt\", \"x\")\n");
    let text = format!("{out}{err}");
    assert_ne!(code, 0, "{text}");
    assert!(
        text.contains("the folder") && text.contains("does not exist"),
        "the folder is what is missing: {text}"
    );
    assert!(text.contains("mkdir"), "the hint says to create it: {text}");
    assert!(
        !text.contains("file.write first"),
        "writing the file cannot create its folder, so the hint must not say to: {text}"
    );
    assert!(!dir.join("nodir").exists(), "nothing is created");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_append_creates_a_missing_file_and_undo_removes_it() {
    let dir = scratch("append_creates");
    let (_, err, code) = run_script(&dir, "file_append(\"log.txt\", \"one\")\n");
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(
        std::fs::read_to_string(dir.join("log.txt")).expect("created"),
        "one"
    );
    let (_, err, code) = run_script(&dir, "file_append(\"new.txt\", \"x\")\nundo()\n");
    assert_eq!(code, 0, "stderr: {err}");
    assert!(
        !dir.join("new.txt").exists(),
        "undo removes the file the append created"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_append_into_a_missing_folder_names_the_folder() {
    let dir = scratch("append_missing_folder");
    let (out, err, code) = run_script(&dir, "file_append(\"nodir/log.txt\", \"x\")\n");
    let text = format!("{out}{err}");
    assert_ne!(code, 0, "{text}");
    assert!(text.contains("does not exist"), "{text}");
    assert!(!dir.join("nodir").exists(), "nothing is created");
    let _ = std::fs::remove_dir_all(&dir);
}

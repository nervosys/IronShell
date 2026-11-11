//! Round 17: inputs and limits, as a tester ran them over large and awkward
//! files.
//!
//! - A UTF-8 byte-order mark, which Windows PowerShell writes by default, is
//!   not part of the text: `cat`, `file.read`, `read_text` and `open` drop it,
//!   and `json.parse` ignores it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh scratch directory: `proj` is the working directory, `session` holds
/// the journal, so no test shares state with another.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round17_inputs_limits")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj")).expect("project dir");
    std::fs::create_dir_all(dir.join("session")).expect("session dir");
    dir
}

/// Run `code` by `ish -c` with `dir` as the working directory.
fn ish_in(dir: &Path, code: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", code])
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_AGENT")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_APPROVE")
        .output()
        .expect("run ish")
}

/// The stdout of a run that must succeed, trimmed.
fn ok_stdout(out: &Output) -> String {
    assert!(
        out.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const BOM: &[u8] = b"\xEF\xBB\xBF";

#[test]
fn open_json_strips_a_leading_bom() {
    let dir = scratch("bom_open_json");
    let proj = dir.join("proj");
    let mut bytes = BOM.to_vec();
    bytes.extend_from_slice(b"{\"name\": \"ae\"}");
    std::fs::write(proj.join("bom.json"), bytes).unwrap();
    let out = ish_in(&proj, r#"open("bom.json").name == "ae""#);
    assert_eq!(ok_stdout(&out), "true");
}

#[test]
fn json_parse_of_file_read_strips_a_leading_bom() {
    let dir = scratch("bom_file_read_json");
    let proj = dir.join("proj");
    let mut bytes = BOM.to_vec();
    bytes.extend_from_slice(b"{\"name\": \"ae\"}");
    std::fs::write(proj.join("bom.json"), bytes).unwrap();
    let out = ish_in(&proj, r#"json.parse(file.read("bom.json")).name == "ae""#);
    assert_eq!(ok_stdout(&out), "true");
}

#[test]
fn json_parse_ignores_a_leading_bom_in_its_text() {
    let dir = scratch("bom_json_parse_text");
    let proj = dir.join("proj");
    let code = format!(
        "json.parse(\"{}{{\\\"name\\\": \\\"ae\\\"}}\").name == \"ae\"",
        '\u{FEFF}'
    );
    let out = ish_in(&proj, &code);
    assert_eq!(ok_stdout(&out), "true");
}

#[test]
fn from_json_ignores_a_leading_bom_in_its_text() {
    let dir = scratch("bom_from_json_text");
    let proj = dir.join("proj");
    let code = format!(
        "from_json(\"{}{{\\\"name\\\": \\\"ae\\\"}}\").name == \"ae\"",
        '\u{FEFF}'
    );
    let out = ish_in(&proj, &code);
    assert_eq!(ok_stdout(&out), "true");
}

#[test]
fn open_toml_strips_a_leading_bom() {
    let dir = scratch("bom_open_toml");
    let proj = dir.join("proj");
    let mut bytes = BOM.to_vec();
    bytes.extend_from_slice(b"name = \"ae\"\n");
    std::fs::write(proj.join("bom.toml"), bytes).unwrap();
    let out = ish_in(&proj, r#"open("bom.toml").name == "ae""#);
    assert_eq!(ok_stdout(&out), "true");
}

#[test]
fn open_text_and_cat_and_file_read_strip_a_leading_bom() {
    let dir = scratch("bom_text");
    let proj = dir.join("proj");
    let mut bytes = BOM.to_vec();
    bytes.extend_from_slice(b"hello\n");
    std::fs::write(proj.join("bom.txt"), bytes).unwrap();
    let out = ish_in(
        &proj,
        r#"open("bom.txt") == "hello\n" && cat("bom.txt") == "hello\n" && file.read("bom.txt") == "hello\n""#,
    );
    assert_eq!(ok_stdout(&out), "true");
}

#[test]
fn a_file_without_a_bom_is_read_unchanged() {
    let dir = scratch("no_bom_text");
    let proj = dir.join("proj");
    std::fs::write(proj.join("plain.txt"), "hello\n").unwrap();
    let out = ish_in(&proj, r#"cat("plain.txt") == "hello\n""#);
    assert_eq!(ok_stdout(&out), "true");
}

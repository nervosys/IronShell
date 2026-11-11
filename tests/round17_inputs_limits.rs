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

/// A text reader's refusal of bytes that are not UTF-8: E_BAD_VALUE, named for
/// the builtin, with the byte offset of the first bad byte and a hint that
/// says what to do. It is not an I/O failure.
fn assert_not_utf8_refusal(out: &Output, builtin: &str, offset: &str) {
    assert!(!out.status.success(), "non-UTF-8 text must be refused");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(stderr.contains("E_BAD_VALUE"), "code: {stderr}");
    assert!(
        stderr.contains(&format!("{builtin}: `")),
        "builtin: {stderr}"
    );
    assert!(stderr.contains(offset), "offset: {stderr}");
    assert!(
        stderr.contains("the file is not UTF-8 text; convert it to UTF-8 first"),
        "hint: {stderr}"
    );
    assert!(
        !stderr.contains("stream did not contain") && !stderr.contains("E_IO"),
        "not an I/O failure: {stderr}"
    );
}

#[test]
fn cat_of_a_file_that_is_not_utf8_names_the_first_bad_byte() {
    let dir = scratch("not_utf8_cat");
    let proj = dir.join("proj");
    std::fs::write(proj.join("bad.txt"), b"ok\n\xFF\n").unwrap();
    let out = ish_in(&proj, r#"cat("bad.txt")"#);
    assert_not_utf8_refusal(&out, "cat", "byte offset 3");
}

#[test]
fn file_read_of_a_latin1_file_is_refused_at_the_bad_byte() {
    let dir = scratch("not_utf8_file_read");
    let proj = dir.join("proj");
    std::fs::write(proj.join("latin1.txt"), b"caf\xE9\n").unwrap();
    let out = ish_in(&proj, r#"file.read("latin1.txt")"#);
    assert_not_utf8_refusal(&out, "cat", "byte offset 3");
}

#[test]
fn open_of_a_file_that_is_not_utf8_is_refused_at_the_bad_byte() {
    let dir = scratch("not_utf8_open");
    let proj = dir.join("proj");
    std::fs::write(proj.join("bad.txt"), b"C\xC3\x28").unwrap();
    let out = ish_in(&proj, r#"open("bad.txt")"#);
    assert_not_utf8_refusal(&out, "open", "byte offset 1");
}

#[test]
fn read_text_of_a_file_that_is_not_utf8_is_refused_at_the_bad_byte() {
    let dir = scratch("not_utf8_read_text");
    let proj = dir.join("proj");
    std::fs::write(proj.join("bad.txt"), b"ok\n\xFF\n").unwrap();
    let out = ish_in(&proj, r#"read_text("bad.txt")"#);
    assert_not_utf8_refusal(&out, "read_text", "byte offset 3");
}

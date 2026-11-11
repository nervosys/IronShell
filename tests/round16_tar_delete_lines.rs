//! Round 16 fixes for `tar_create` and `delete_lines`.
//!
//! `tar_create` checks that every member exists before tar runs, so a missing
//! one is E_NOT_FOUND naming the path the caller wrote. It refuses an archive
//! name that is not .tar, .tar.gz or .tgz with E_BAD_ARG.
//!
//! `delete_lines` takes a `{regex: ...}` selector. A literal selector that
//! deletes nothing, and holds regex metacharacters, says so in a `note`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh scratch directory: `proj` is the working directory, `session` holds
/// the journal, so no test shares state with another.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round16_tar_delete_lines")
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

/// The structured refusal on the first stderr line.
fn first_error(out: &Output) -> serde_json::Value {
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().next().unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {err:?}"))
}

#[test]
fn tar_create_refuses_a_missing_member_before_running_tar() {
    let dir = scratch("tar_missing_member");
    let proj = dir.join("proj");
    std::fs::create_dir_all(proj.join("work")).unwrap();
    let out = ish_in(&proj, r#"tar_create("work/m.tar.gz", "work/nope.txt")"#);
    assert!(!out.status.success(), "a missing member must fail");
    let json = first_error(&out);
    assert_eq!(json["error"]["code"], "E_NOT_FOUND", "{json}");
    let message = json["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("no file named 'work/nope.txt'"),
        "the message names the member as written: {message}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("Couldn't visit") && !stderr.contains("E_TOOL_FAILED"),
        "tar must not run: {stderr}"
    );
    assert!(
        !proj.join("work").join("m.tar.gz").exists(),
        "no archive is written"
    );
}

#[test]
fn tar_create_refuses_a_zip_name_and_points_at_zip_create() {
    let dir = scratch("tar_zip_name");
    let proj = dir.join("proj");
    std::fs::write(proj.join("a.txt"), "alpha\n").unwrap();
    let out = ish_in(&proj, r#"tar_create("x.zip", "a.txt")"#);
    assert!(!out.status.success(), "a .zip name must be refused");
    let json = first_error(&out);
    assert_eq!(json["error"]["code"], "E_BAD_ARG", "{json}");
    let hint = json["error"]["hint"].as_str().unwrap_or_default();
    assert!(
        hint.contains("zip_create"),
        "the hint names zip_create: {hint}"
    );
    assert!(!proj.join("x.zip").exists(), "nothing is written");
}

#[test]
fn tar_create_refuses_any_other_extension() {
    let dir = scratch("tar_other_ext");
    let proj = dir.join("proj");
    std::fs::write(proj.join("a.txt"), "alpha\n").unwrap();
    for name in ["x.bz2", "x.tar.bz2", "x.7z"] {
        let out = ish_in(&proj, &format!(r#"tar_create("{name}", "a.txt")"#));
        let json = first_error(&out);
        assert_eq!(json["error"]["code"], "E_BAD_ARG", "{name}: {json}");
        assert!(!proj.join(name).exists(), "{name}: nothing is written");
    }
}

/// The text of a file under the project directory.
fn read_in(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join("proj").join(name)).unwrap_or_default()
}

#[test]
fn delete_lines_regex_selector_deletes_the_matching_lines() {
    let dir = scratch("delete_lines_regex");
    let out = ish_in(
        &dir.join("proj"),
        r#"file_write("f.txt", "keep\ntmp1\ntmp22\n"); delete_lines("f.txt", {regex: "^tmp[0-9]+$"})"#,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(read_in(&dir, "f.txt"), "keep\n");
}

#[test]
fn delete_lines_refuses_an_invalid_regex_and_leaves_the_file() {
    let dir = scratch("delete_lines_bad_regex");
    let out = ish_in(
        &dir.join("proj"),
        r#"file_write("f.txt", "a\nb\n"); delete_lines("f.txt", {regex: "a("})"#,
    );
    assert!(!out.status.success(), "an invalid regex must fail");
    let json = first_error(&out);
    assert_eq!(json["error"]["code"], "E_BAD_ARG", "{json}");
    assert_eq!(read_in(&dir, "f.txt"), "a\nb\n", "the file is untouched");
}

#[test]
fn delete_lines_notes_a_literal_pattern_with_regex_syntax_that_matched_nothing() {
    let dir = scratch("delete_lines_note");
    let out = ish_in(
        &dir.join("proj"),
        r#"file_write("f.txt", "t.x\ntt\n"); let r = delete_lines("f.txt", {pattern: "t.*"}); file_write("out.json", json.stringify(r))"#,
    );
    assert!(
        out.status.success(),
        "a literal selector that matches nothing still succeeds: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&read_in(&dir, "out.json")).unwrap();
    assert_eq!(v["lines_deleted"], 0, "{v}");
    assert_eq!(
        v["note"], "matched literally; use {regex: ...} for a regular expression",
        "{v}"
    );
    assert_eq!(read_in(&dir, "f.txt"), "t.x\ntt\n", "nothing was deleted");
}

#[test]
fn delete_lines_adds_no_note_when_a_literal_selector_deletes_something() {
    let dir = scratch("delete_lines_no_note");
    let out = ish_in(
        &dir.join("proj"),
        r#"file_write("f.txt", "t.x\ntt\n"); let r = delete_lines("f.txt", {contains: "t.x"}); file_write("out.json", json.stringify(r))"#,
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_str(&read_in(&dir, "out.json")).unwrap();
    assert_eq!(v["lines_deleted"], 1, "{v}");
    assert!(v.get("note").is_none(), "no note: {v}");
}

#[test]
fn delete_lines_names_itself_and_lists_its_selector_forms_when_refused() {
    let dir = scratch("delete_lines_refusals");
    let proj = dir.join("proj");
    let range = ish_in(
        &proj,
        r#"file_write("f.txt", "a\nb\nc\n"); delete_lines("f.txt", 3, 2)"#,
    );
    let json = first_error(&range);
    assert_eq!(json["error"]["code"], "E_BAD_ARG", "{json}");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("delete_lines range end must not precede start"),
        "{json}"
    );

    let selector = ish_in(&proj, r#"delete_lines("f.txt", {unknown: "x"})"#);
    let json = first_error(&selector);
    assert_eq!(json["error"]["code"], "E_BAD_ARG", "{json}");
    let hint = json["error"]["hint"].as_str().unwrap_or_default();
    assert!(
        hint.contains("{regex: pattern}"),
        "the hint lists the forms: {hint}"
    );
    assert!(hint.contains("line Int"), "{hint}");
}

//! How values render as text: an `ish -c` result, the REPL, `print`, and agent
//! mode (`--agent`).
//!
//! A string inside a container is written as an IronShell string literal, with
//! its quotes, backslashes and control characters escaped, so the text reads
//! back as the same data. A nested collection is shown in full: an agent reads
//! `[len=2]` as the data, and it is not.
//!
//! These drive the real binary, because the renderers are what is under test.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round29_display")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    dir
}

/// Run `ish` with `args` in `dir`. Returns the trimmed stdout, and fails the
/// test when the run does not succeed.
fn ish(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run ish");
    assert!(
        out.status.success(),
        "ish {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Run `code` under agent mode, confined to `dir`.
fn agent(dir: &Path, code: &str) -> String {
    let ws = dir.to_str().expect("utf-8 scratch path");
    ish(dir, &["--agent", "--workspace", ws, "-c", code])
}

#[test]
fn strings_inside_an_array_are_escaped_literals() {
    let dir = scratch("array_escapes");
    assert_eq!(
        ish(
            &dir,
            &["-c", r#"["it\"s", "back\\slash", "tab\there", ""]"#]
        ),
        r#"["it\"s", "back\\slash", "tab\there", ""]"#
    );
}

#[test]
fn strings_inside_a_record_are_escaped_literals() {
    let dir = scratch("record_escapes");
    assert_eq!(ish(&dir, &["-c", r#"{a: "x\"y"}"#]), r#"{a: "x\"y"}"#);
}

#[test]
fn control_characters_inside_a_container_are_escaped() {
    let dir = scratch("control_escapes");
    assert_eq!(
        ish(
            &dir,
            &["-c", r#"["line\nbreak", "cr\rhere", "bell\u{7}x"]"#]
        ),
        r#"["line\nbreak", "cr\rhere", "bell\u{7}x"]"#
    );
}

#[test]
fn a_dollar_brace_inside_a_container_is_not_left_to_interpolate() {
    let dir = scratch("dollar_escape");
    assert_eq!(ish(&dir, &["-c", r#"["\${x}"]"#]), r#"["\${x}"]"#);
}

#[test]
fn a_top_level_string_stays_raw() {
    let dir = scratch("top_level_raw");
    assert_eq!(ish(&dir, &["-c", r#""it\"s""#]), r#"it"s"#);
}

#[test]
fn the_printed_container_reads_back_as_the_same_text() {
    // Idempotence: printing a value, then evaluating that text, prints the same
    // text again. This is the property an escaping bug breaks.
    let dir = scratch("round_trip");
    let first = ish(
        &dir,
        &[
            "-c",
            r#"["it\"s", "back\\slash", "tab\there", "line\nbreak", "bell\u{7}x", "\${d}"]"#,
        ],
    );
    let second = ish(&dir, &["-c", first.as_str()]);
    assert_eq!(second, first);
}

#[test]
fn print_writes_escaped_literals_for_strings_in_a_container() {
    let dir = scratch("print_escapes");
    assert_eq!(
        ish(&dir, &["-c", r#"print(["it\"s", "a\nb"])"#]),
        r#"["it\"s", "a\nb"]"#
    );
}

#[test]
fn nested_arrays_and_records_render_in_full() {
    let dir = scratch("nested_full");
    assert_eq!(ish(&dir, &["-c", "[[1, 2]]"]), "[[1, 2]]");
    assert_eq!(
        ish(&dir, &["-c", "{a: [1, 2], b: {c: [3]}}"]),
        "{a: [1, 2], b: {c: [3]}}"
    );
    assert_eq!(
        ish(&dir, &["-c", "[[], {}, [[]], \"\"]"]),
        "[[], {}, [[]], \"\"]"
    );
}

#[test]
fn an_empty_array_renders_as_brackets() {
    let dir = scratch("empty_array");
    assert_eq!(ish(&dir, &["-c", "[]"]), "[]");
    assert_eq!(ish(&dir, &["-c", "{a: []}"]), "{a: []}");
}

#[test]
fn a_nested_array_past_one_hundred_elements_says_how_many_are_left() {
    let dir = scratch("nested_cap");
    let out = ish(&dir, &["-c", "[arr.range(150)]"]);
    assert!(out.starts_with("[[0, 1, 2, "), "{out}");
    assert!(out.ends_with(", 99, ... (50 more)]]"), "{out}");
    assert!(!out.contains("len="), "{out}");
}

#[test]
fn agent_mode_quotes_strings_inside_a_list_so_names_with_commas_stay_apart() {
    let dir = scratch("agent_names");
    std::fs::write(dir.join("a,b.txt"), "x").expect("write a,b.txt");
    std::fs::write(dir.join("inside.txt"), "x").expect("write inside.txt");
    let out = agent(&dir, "ls() | map(fn(f) => f.name)");
    assert!(out.starts_with('[') && out.ends_with(']'), "{out}");
    assert!(out.contains(r#""a,b.txt""#), "{out}");
    assert!(out.contains(r#""inside.txt""#), "{out}");
}

#[test]
fn agent_mode_keeps_a_numeric_looking_string_a_string() {
    let dir = scratch("agent_numeric_string");
    assert_eq!(agent(&dir, r#"["1", 1, "a,b"]"#), r#"["1",1,"a,b"]"#);
}

#[test]
fn agent_mode_quotes_strings_inside_a_record() {
    let dir = scratch("agent_record");
    assert_eq!(agent(&dir, r#"{a: "x,y", b: 2}"#), r#"{a="x,y" b=2}"#);
}

#[test]
fn agent_mode_writes_an_empty_array_as_brackets() {
    let dir = scratch("agent_empty");
    assert_eq!(agent(&dir, "[]"), "[]");
}

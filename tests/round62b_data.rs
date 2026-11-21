//! Round 62b: grep back-references, TOML datetimes, `reverse` by grapheme
//! cluster, header-only CSV, `git_log` renames and merges, and the position of
//! an out-of-range JSON number.

use std::path::{Path, PathBuf};
use std::process::Command;

fn dir(tag: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round62b_data")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("session")).unwrap();
    std::fs::create_dir_all(root.join("ws")).unwrap();
    root
}

/// Runs `code` with `--deterministic` in `<root>/ws`; returns stdout and stderr.
fn ish(root: &Path, code: &str) -> (String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--deterministic", "-c", code])
        .current_dir(root.join("ws"))
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", root.join("session"))
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_WORKSPACE")
        .output()
        .expect("run ish");
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    )
}

fn out(root: &Path, code: &str) -> String {
    ish(root, code).0
}

#[test]
fn grep_back_references_in_bre() {
    let root = dir("bre");
    std::fs::write(
        root.join("ws").join("f"),
        "abab\nab\naabb\nabba\nABAB\nxyyx\n",
    )
    .unwrap();
    assert_eq!(out(&root, r#"grep("\\(ab\\)\\1", "f")"#), r#"["abab"]"#);
    assert_eq!(
        out(&root, r#"grep("\\(a\\)\\(b\\)\\2\\1", "f")"#),
        r#"["abba"]"#
    );
    // A piped String takes the same engine.
    assert_eq!(
        out(&root, r#""zz\nabab" | grep("\\(ab\\)\\1")"#),
        r#"["abab"]"#
    );
    // No back-reference means no match, not an error.
    assert_eq!(out(&root, r#"grep("\\(q\\)\\1", "f")"#), "[]");
}

#[test]
fn grep_back_references_in_ere() {
    let root = dir("ere");
    std::fs::write(root.join("ws").join("f"), "abab\nab\nxyyx\n").unwrap();
    assert_eq!(
        out(&root, r#"grep("(ab)\\1", "f", {extended: true})"#),
        r#"["abab"]"#
    );
    assert_eq!(
        out(&root, r#"grep("(x)(y)\\2\\1", "f", {extended: true})"#),
        r#"["xyyx"]"#
    );
}

#[test]
fn grep_back_references_with_options() {
    let root = dir("opts");
    std::fs::write(root.join("ws").join("f"), "abab\nab\nxx\nABAB\n").unwrap();
    assert_eq!(
        out(
            &root,
            r#"grep("\\(ab\\)\\1", "f", {ignore_case: true, line_numbers: true})"#
        ),
        r#"[{"file":"f","line":1,"text":"abab"},{"file":"f","line":4,"text":"ABAB"}]"#
    );
    assert_eq!(
        out(
            &root,
            r#"grep("\\(ab\\)\\1", "f", {line_numbers: true}) | len"#
        ),
        "1"
    );
    // Case matters without ignore_case.
    assert_eq!(out(&root, r#"grep("\\(ab\\)\\1", "f")"#), r#"["abab"]"#);
}

#[test]
fn grep_without_a_back_reference_is_unchanged() {
    let root = dir("plain");
    std::fs::write(root.join("ws").join("f"), "a|b\nab\n").unwrap();
    assert_eq!(out(&root, r#"grep("a|b", "f")"#), r#"["a|b"]"#);
    // An escaped backslash before a digit is not a back-reference.
    assert_eq!(out(&root, r#"grep("a\\\\1", "f")"#), "[]");
}

#[test]
fn toml_datetimes_round_trip_unquoted() {
    let root = dir("toml");
    let text = "a = 1979-05-27T07:32:00Z\nb = 1979-05-27\nc = 07:32:00\nd = 1979-05-27T07:32:00\ne = \"hi\"\n";
    std::fs::write(root.join("ws").join("t.toml"), text).unwrap();
    let written = out(&root, r#"open("t.toml") | to_toml"#);
    let expected = serde_json::to_string(text).unwrap();
    assert_eq!(written, expected);
    // The value stays a String when read.
    assert_eq!(
        out(&root, r#"open("t.toml").a"#),
        r#""1979-05-27T07:32:00Z""#
    );
}

#[test]
fn text_that_only_looks_like_a_date_stays_quoted() {
    let root = dir("tomlstr");
    // Not exactly the TOML grammar: a month 13, a trailing word, a lowercase z.
    for s in ["1979-13-27", "1979-05-27 is a date", "1979-05-27t07:32:00z"] {
        let code = format!(r#"to_toml({{a: "{s}"}})"#);
        let got = out(&root, &code);
        assert_eq!(got, format!("\"a = \\\"{s}\\\"\\n\""), "for {s}");
    }
}

#[test]
fn reverse_keeps_grapheme_clusters() {
    let root = dir("reverse");
    assert_eq!(out(&root, r#""abc" | reverse"#), r#""cba""#);
    // e + combining acute, then x: the accent stays on the e.
    assert_eq!(
        out(&root, r#"reverse("e\u0301x")"#),
        "\"x\u{e9}\"".replace('\u{e9}', "e\u{301}")
    );
    // A ZWJ family emoji is one cluster.
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let got = out(&root, &format!(r#"reverse("a{family}b")"#));
    assert_eq!(got, format!("\"b{family}a\""));
}

#[test]
fn header_only_csv_keeps_its_columns() {
    let root = dir("csv");
    std::fs::write(root.join("ws").join("h.csv"), "a,b\n").unwrap();
    assert_eq!(out(&root, r#"from_csv("a,b\n")"#), "[]");
    assert_eq!(out(&root, r#"csv_columns("h.csv")"#), r#"["a","b"]"#);
    assert_eq!(out(&root, r#"to_csv([], ["a", "b"])"#), r#""a,b\n""#);
    assert_eq!(
        out(&root, r#"sql_value("h.csv", "select count(*) from h")"#),
        "0"
    );
    assert_eq!(
        out(&root, r#"sql_value("h.csv", "select count(b) from h")"#),
        "0"
    );
}

#[test]
fn json_number_out_of_range_names_the_number() {
    let root = dir("json");
    let (stdout, stderr) = ish(&root, r#"from_json("[1, 1e400]")"#);
    let all = format!("{stdout}{stderr}");
    assert!(all.contains("1e400"), "{all}");
    assert!(all.contains("column 5"), "{all}");
}

#[test]
fn git_log_lists_renames_and_merge_files() {
    let root = dir("git");
    let ws = root.join("ws");
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .args(["-c", "user.email=a@b", "-c", "user.name=n"])
            .args(args)
            .current_dir(&ws)
            .env("GIT_CONFIG_GLOBAL", "")
            .output()
            .expect("git");
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    };
    git(&["init", "-q"]);
    std::fs::write(ws.join("a.txt"), "1\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "one"]);
    git(&["mv", "a.txt", "b.txt"]);
    git(&["commit", "-qm", "two"]);
    git(&["checkout", "-qb", "side"]);
    std::fs::write(ws.join("c.txt"), "2\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "side"]);
    git(&["checkout", "-q", "-"]);
    std::fs::write(ws.join("d.txt"), "3\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "main"]);
    git(&["merge", "-q", "--no-ff", "side", "-m", "merge"]);

    let rows = out(&root, r#"git_log({n: 10, files: true})"#);
    let v: serde_json::Value = serde_json::from_str(&rows).expect(&rows);
    let by = |subject: &str| {
        v.as_array()
            .unwrap()
            .iter()
            .find(|r| r["subject"] == subject)
            .unwrap_or_else(|| panic!("no {subject} in {rows}"))
            .clone()
    };
    let two = by("two");
    assert_eq!(two["files"], serde_json::json!(["b.txt"]));
    assert_eq!(
        two["renames"],
        serde_json::json!([{"from": "a.txt", "to": "b.txt"}])
    );
    let one = by("one");
    assert_eq!(one["files"], serde_json::json!(["a.txt"]));
    assert_eq!(one["renames"], serde_json::json!([]));
    // A merge lists what it brought in over its first parent.
    assert_eq!(by("merge")["files"], serde_json::json!(["c.txt"]));
    // The second-parent commit is still listed.
    assert_eq!(by("side")["files"], serde_json::json!(["c.txt"]));
}

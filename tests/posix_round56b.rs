//! Round 56b: `grep -m` / `-b`, the group separator under `-o` and zero
//! context, and arithmetic errors in the in-process POSIX engine (`ish -b`).
//!
//! Every expected output below was produced by GNU grep 3.11 and GNU bash 5.2
//! on Debian, with the same input, except the nesting limit, which is ish's own.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_r56b_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `ish -b -c <script>` in `dir`. Returns (stdout, stderr, exit code).
fn ish_bash(script: &str, dir: &Path) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", script])
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("state"))
        .env("IRONSHELL_WORKSPACE", dir)
        .env_remove("IRONSHELL_MODE")
        .output()
        .expect("run ish");
    (
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn check(dir: &Path, cases: &[(&str, &str, i32)]) {
    let seven = r"printf 'a\nb\nc\nd\ne\nf\ng\n' | ";
    for (cmd, want, want_code) in cases {
        let (out, err, code) = ish_bash(&format!("{seven}{cmd}"), dir);
        assert_eq!(
            (out.as_str(), code),
            (*want, *want_code),
            "{cmd}: stderr {err}"
        );
    }
}

#[test]
fn zero_context_still_separates_groups() {
    let dir = scratch("ctx0");
    check(
        &dir,
        &[
            ("grep -A0 -e c -e f", "c\n--\nf\n", 0),
            ("grep -C0 -e c -e d -e f", "c\nd\n--\nf\n", 0),
            ("grep -B0 -n -e b -e g", "2:b\n--\n7:g\n", 0),
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_matching_with_context_keeps_the_separator() {
    let dir = scratch("octx");
    check(
        &dir,
        &[
            ("grep -o -A1 -e c -e f", "c\n--\nf\n", 0),
            ("grep -on -A1 -e c -e f", "3:c\n--\n6:f\n", 0),
            ("grep -o -C1 -e b -e c -e g", "b\nc\n--\ng\n", 0),
            // d is c's context line, so the groups touch: no separator.
            ("grep -o -A1 -e c -e d", "c\nd\n", 0),
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn max_count_stops_after_n_selected_lines() {
    let dir = scratch("maxc");
    check(
        &dir,
        &[
            ("grep -m1 -e c -e f", "c\n", 0),
            ("grep -m 2 -n -e b -e d -e f", "2:b\n4:d\n", 0),
            // The trailing context of the last selected line is printed, even
            // where those lines match.
            ("grep --max-count=1 -A2 -e c -e d", "c\nd\ne\n", 0),
            ("grep -n -m1 -A2 -e c -e d", "3:c\n4-d\n5-e\n", 0),
            ("grep -m1 -A3 -e c -e f", "c\nd\ne\nf\n", 0),
            ("grep -m1 -c -e c -e f", "1\n", 0),
            ("grep -m0 -e c", "", 1),
            // A negative count is no limit.
            ("grep -m -1 -e c -e f", "c\nf\n", 0),
        ],
    );
    let (out, err, code) = ish_bash("echo c | grep -m x c", &dir);
    assert_eq!((out.as_str(), code), ("", 2), "{err}");
    assert!(err.contains("grep: invalid max count"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn byte_offset_prefixes_each_line_or_match() {
    let dir = scratch("boff");
    check(
        &dir,
        &[
            ("grep -b -e c -e e", "4:c\n8:e\n", 0),
            ("grep -bn -e d", "4:6:d\n", 0),
            ("grep -ob -e d", "6:d\n", 0),
            ("grep --byte-offset -o -e e", "8:e\n", 0),
            ("grep -b -A1 -e b", "2:b\n4-c\n", 0),
        ],
    );
    // Under -o each match carries its own offset, not its line's.
    let (out, err, code) = ish_bash("printf 'xx ab ab\\n' | grep -ob ab", &dir);
    assert_eq!((out.as_str(), code), ("3:ab\n6:ab\n", 0), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_arithmetic_error_discards_its_line_and_the_script_goes_on() {
    let dir = scratch("arith");
    let (out, err, code) = ish_bash("echo $((1/0)); echo same-line\necho after", &dir);
    assert_eq!(out, "after\n", "{err}");
    assert!(err.contains("division by 0"), "{err}");
    assert_eq!(code, 0, "{err}");
    let (out, err, _) = ish_bash("echo $((16#ff)) $((2#101)) $((36#zz)) $((0x))", &dir);
    assert_eq!(out, "255 5 1295 0\n", "{err}");
    // Deep nesting is an error, not a stack overflow.
    let deep = format!(
        "echo $(({}1{}))\necho survived",
        "(".repeat(5000),
        ")".repeat(5000)
    );
    let (out, err, code) = ish_bash(&deep, &dir);
    assert_eq!((out.as_str(), code), ("survived\n", 0), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_matching_prints_the_matches_of_every_line_it_would_print() {
    let dir = scratch("ov");
    // Under -o -v a selected line holds no match and prints nothing; a context
    // line is a matching line, and prints its match after `-`.
    check(
        &dir,
        &[
            ("grep -o -v c", "", 0),
            ("grep -o -v -n -b c", "", 0),
            ("grep -o -v -m2 c", "", 0),
            ("grep -ovc c", "6\n", 0),
            ("grep -o -v -n -A1 c", "3-c\n", 0),
            ("grep -o -v -C1 -e c -e f", "c\nf\n", 0),
            ("grep -o -v -n -b -B1 -e c -e g", "3-4-c\n", 0),
            ("grep -o -v .", "", 1),
        ],
    );
    let (out, err, code) = ish_bash(
        r"printf 'c1\nzz\nc2c3\nzz\nzz\nzz\nc4\n' | grep -o -n -C1 c.",
        &dir,
    );
    assert_eq!(
        (out.as_str(), code),
        ("1:c1\n3:c2\n3:c3\n--\n7:c4\n", 0),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

//! Round 27: the in-process POSIX subset (`ish -b`) matches GNU coreutils and
//! bash where it used to differ.
//!
//! `sort -k` with the default blank separator and no `-s` broke ties by input
//! order instead of GNU's last resort (the whole line). The expected outputs
//! below are GNU's, written as literals so the test needs no GNU tools.

use std::process::Command;

/// Run `script` under `ish -b -c` in a folder holding `files`, and return
/// stdout, stderr and the exit status. PATH is an empty folder, so a declined
/// command cannot be answered by a GNU program: it fails with stderr, which
/// makes these cases prove the in-process code ran.
fn run(tag: &str, files: &[(&str, &str)], script: &str) -> (String, String, i32) {
    let root = std::env::temp_dir().join(format!("ae-round27-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let work = root.join("work");
    let session = root.join("session");
    let empty_path = root.join("empty-path");
    for dir in [&work, &session, &empty_path] {
        std::fs::create_dir_all(dir).unwrap();
    }
    for (name, text) in files {
        std::fs::write(work.join(name), text).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", script])
        .current_dir(&work)
        .env("NO_COLOR", "1")
        .env("LC_ALL", "C")
        .env("PATH", &empty_path)
        .env("IRONSHELL_SESSION_DIR", &session)
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&root);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

const DUP: &str = "b 2\na 1\nb 1\na 2\n";
const DUP_CSV: &str = "b,2\na,1\nb,1\na,2\n";
const NUMS: &str = "1 b\n1 a\n";

#[test]
fn sort_k_ties_fall_back_to_the_whole_line() {
    // GNU: sort -k1,1 dup.txt (the blank-separated default, no -t)
    let (out, err, code) = run("k1", &[("dup.txt", DUP)], "sort -k1,1 dup.txt");
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("a 1\na 2\nb 1\nb 2\n", "", 0)
    );
}

#[test]
fn sort_k_with_a_separator_falls_back_to_the_whole_line() {
    // GNU: sort -t, -k1,1 dup.csv
    let (out, err, code) = run("kt", &[("dup.csv", DUP_CSV)], "sort -t, -k1,1 dup.csv");
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("a,1\na,2\nb,1\nb,2\n", "", 0)
    );
}

#[test]
fn sort_s_keeps_ties_in_input_order() {
    // GNU: sort -s -k1,1 dup.txt
    let (out, err, code) = run("s", &[("dup.txt", DUP)], "sort -s -k1,1 dup.txt");
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("a 1\na 2\nb 2\nb 1\n", "", 0)
    );
    // GNU: sort -s -t, -k1,1 dup.csv
    let (out, _, _) = run("st", &[("dup.csv", DUP_CSV)], "sort -s -t, -k1,1 dup.csv");
    assert_eq!(out, "a,1\na,2\nb,2\nb,1\n");
}

#[test]
fn sort_r_reverses_the_last_resort_too() {
    // GNU: sort -r -k1,1 dup.txt (the whole comparison is reversed)
    let (out, err, code) = run("r", &[("dup.txt", DUP)], "sort -r -k1,1 dup.txt");
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("b 2\nb 1\na 2\na 1\n", "", 0)
    );
}

#[test]
fn sort_n_equal_numbers_fall_back_to_the_whole_line() {
    // GNU: sort -n nums.txt
    let (out, err, code) = run("n", &[("nums.txt", NUMS)], "sort -n nums.txt");
    assert_eq!((out.as_str(), err.as_str(), code), ("1 a\n1 b\n", "", 0));
    // GNU: sort -rn nums.txt
    let (out, _, _) = run("rn", &[("nums.txt", NUMS)], "sort -rn nums.txt");
    assert_eq!(out, "1 b\n1 a\n");
}

#[test]
fn sort_u_with_keys_keeps_the_first_of_each_run_in_input_order() {
    // GNU: sort -u -k1,1 dup.txt (no last resort under -u)
    let (out, err, code) = run("u", &[("dup.txt", DUP)], "sort -u -k1,1 dup.txt");
    assert_eq!((out.as_str(), err.as_str(), code), ("a 1\nb 2\n", "", 0));
}

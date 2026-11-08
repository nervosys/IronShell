//! `grep` prints a file name before each match only when GNU grep does: with
//! several file operands, or when `-r` walks a directory. A single file
//! operand under `-r` prints no name, and `-H`/`-h` force or suppress it.
//!
//! The expected outputs are literals recorded from GNU grep 3 on these files.

use std::process::Command;

fn grep_run(tag: &str, script: &str) -> (String, i32) {
    let root = std::env::temp_dir().join(format!("ae-posix-grep-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("session")).unwrap();
    std::fs::create_dir_all(root.join("tree")).unwrap();
    std::fs::create_dir_all(root.join("empty-path")).unwrap();
    std::fs::write(
        root.join("tree").join("top.rs"),
        "TODO fix\nnothing\ntodo again\nplain\n",
    )
    .unwrap();
    std::fs::write(root.join("other.txt"), "todo one\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", script])
        .current_dir(&root)
        .env("NO_COLOR", "1")
        // An empty PATH: grep must run in-process, or the run fails with stderr.
        .env("PATH", root.join("empty-path"))
        .env("IRONSHELL_SESSION_DIR", root.join("session"))
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&root);
    assert!(out.stderr.is_empty(), "{script}: {:?}", out.stderr);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn one_file_operand_under_r_prints_no_name() {
    // GNU: grep -rv todo tree/top.rs
    assert_eq!(
        grep_run("a", "grep -rv todo tree/top.rs"),
        ("TODO fix\nnothing\nplain\n".into(), 0)
    );
    // GNU: grep -r todo tree/top.rs
    assert_eq!(
        grep_run("b", "grep -r todo tree/top.rs"),
        ("todo again\n".into(), 0)
    );
    // GNU: grep -r todo ./tree/top.rs
    assert_eq!(
        grep_run("c", "grep -r todo ./tree/top.rs"),
        ("todo again\n".into(), 0)
    );
}

#[test]
fn a_directory_under_r_prints_names() {
    // GNU: grep -r todo tree
    assert_eq!(
        grep_run("d", "grep -r todo tree"),
        ("tree/top.rs:todo again\n".into(), 0)
    );
    // GNU: grep -r todo (searches the working directory). The walk's order is
    // the filesystem's, which GNU grep's is too, so the lines are compared sorted.
    let (out, status) = grep_run("e", "grep -r todo");
    assert_eq!(status, 0);
    assert_eq!(
        sorted_lines(&out),
        sorted_lines("other.txt:todo one\ntree/top.rs:todo again\n")
    );
}

fn sorted_lines(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.lines().map(str::to_string).collect();
    v.sort();
    v
}

#[test]
fn several_operands_print_names_and_h_and_upper_h_override() {
    // GNU: grep todo tree/top.rs other.txt
    assert_eq!(
        grep_run("f", "grep todo tree/top.rs other.txt"),
        ("tree/top.rs:todo again\nother.txt:todo one\n".into(), 0)
    );
    // GNU: grep -h todo tree/top.rs other.txt
    assert_eq!(
        grep_run("g", "grep -h todo tree/top.rs other.txt"),
        ("todo again\ntodo one\n".into(), 0)
    );
    // GNU: grep -H todo tree/top.rs (-H forces the name on one operand)
    assert_eq!(
        grep_run("h", "grep -H todo tree/top.rs"),
        ("tree/top.rs:todo again\n".into(), 0)
    );
    // GNU: grep -rH todo tree/top.rs
    assert_eq!(
        grep_run("i", "grep -rH todo tree/top.rs"),
        ("tree/top.rs:todo again\n".into(), 0)
    );
    // GNU: grep -rh todo tree (-h suppresses names under a directory)
    assert_eq!(
        grep_run("j", "grep -rh todo tree"),
        ("todo again\n".into(), 0)
    );
    // GNU: grep todo other.txt
    assert_eq!(
        grep_run("k", "grep todo other.txt"),
        ("todo one\n".into(), 0)
    );
}

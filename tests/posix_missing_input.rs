//! A missing input file gives the exit status GNU coreutils gives: `sort`
//! exits 2 ("sort: cannot read: ..."), where it used to exit 1. The other
//! utilities of the subset were measured against GNU on the same file and
//! already matched (cat, head, tail and wc exit 1; grep exits 2); they are
//! pinned here so a change to one of them shows up.
//!
//! The expected values are literals recorded from GNU coreutils 9 and grep 3.

use std::process::Command;

fn run(tag: &str, script: &str) -> (String, String) {
    let root = std::env::temp_dir().join(format!("ae-posix-missing-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("session")).unwrap();
    std::fs::create_dir_all(root.join("empty-path")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", script])
        .current_dir(&root)
        .env("NO_COLOR", "1")
        // An empty PATH: these utilities must run in-process, not as programs.
        .env("PATH", root.join("empty-path"))
        .env("IRONSHELL_SESSION_DIR", root.join("session"))
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&root);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn sort_of_a_missing_file_exits_2_and_says_cannot_read() {
    // GNU: sort nonexistent.txt; echo "rc=$?"
    let (out, err) = run("sort", "sort nonexistent.txt; echo \"rc=$?\"");
    assert_eq!(out, "rc=2\n");
    assert_eq!(
        err,
        "sort: cannot read: nonexistent.txt: No such file or directory\n"
    );
}

#[test]
fn other_utilities_keep_the_gnu_status_for_a_missing_file() {
    // GNU: cat, head, tail and wc exit 1; grep exits 2.
    for (tool, want) in [
        ("cat", 1),
        ("head", 1),
        ("tail", 1),
        ("wc", 1),
        ("grep x", 2),
    ] {
        let (out, _) = run(
            tool.split(' ').next().unwrap(),
            &format!("{tool} nonexistent.txt; echo \"rc=$?\""),
        );
        assert_eq!(out, format!("rc={want}\n"), "{tool}");
    }
}

//! `sed` in file mode reports whether the file changed. A pattern that matches
//! nothing leaves the file alone and says "unchanged"; one that changes text
//! says "modified". These drive the real binary in a scratch folder.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("sed_file_status")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    std::fs::write(dir.join("f.txt"), "alpha\nbeta\n").expect("seed file");
    dir
}

fn ish_c(dir: &Path, code: &str) -> (Option<i32>, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", code])
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .output()
        .expect("run ish");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_pattern_that_matches_nothing_is_unchanged_and_not_rewritten() {
    let dir = scratch("no_match");
    let (code, out, err) = ish_c(&dir, "sed(\"zzz\", \"y\", \"f.txt\", {file: true}).status");
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("unchanged"), "{out:?}");
    assert!(!out.contains("modified"), "{out:?}");
    assert_eq!(
        std::fs::read_to_string(dir.join("f.txt")).unwrap(),
        "alpha\nbeta\n"
    );
}

#[test]
fn a_replacement_that_changes_text_is_modified() {
    let dir = scratch("match");
    let (code, out, err) = ish_c(
        &dir,
        "sed(\"beta\", \"gamma\", \"f.txt\", {file: true}).status",
    );
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("modified"), "{out:?}");
    assert_eq!(
        std::fs::read_to_string(dir.join("f.txt")).unwrap(),
        "alpha\ngamma\n"
    );
}

#[test]
fn a_replacement_equal_to_the_match_is_unchanged() {
    let dir = scratch("same_text");
    let (code, out, err) = ish_c(
        &dir,
        "sed(\"beta\", \"beta\", \"f.txt\", {file: true}).status",
    );
    assert_eq!(code, Some(0), "{err}");
    assert!(out.contains("unchanged"), "{out:?}");
    assert!(!out.contains("modified"), "{out:?}");
}

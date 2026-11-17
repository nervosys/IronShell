//! Round 48b: glob of directories, UTF-16 reads, and the hints for names and
//! argument shapes a tester tripped over.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round48b_misc")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj")).expect("project dir");
    std::fs::create_dir_all(dir.join("session")).expect("session dir");
    dir
}

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

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn glob_with_a_trailing_slash_returns_only_directories() {
    let dir = scratch("glob_dirs");
    let proj = dir.join("proj");
    std::fs::create_dir_all(proj.join("g").join("sub")).expect("dirs");
    std::fs::create_dir_all(proj.join("g").join("sub").join("deep")).expect("dirs");
    std::fs::write(proj.join("g").join("f.txt"), b"x").expect("file");
    std::fs::write(proj.join("g").join("sub").join("h.txt"), b"x").expect("file");
    let one = ish_in(&proj, r#"glob("g/*/")"#);
    let one = text(&one);
    assert!(one.contains("g/sub/"), "{one}");
    assert!(!one.contains("f.txt"), "{one}");
    let deep = ish_in(&proj, r#"glob("g/**/")"#);
    let deep = text(&deep);
    assert!(deep.contains("deep"), "{deep}");
    assert!(!deep.contains(".txt"), "{deep}");
}

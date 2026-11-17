//! `tar_create` / `zip_create` of `.`: the contents are stored directly (no
//! extra folder named after the current directory), and IronShell's own
//! `.ironshell` state folder is left out unless it is named as a member.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round49b_archive_dot")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    let proj = dir.join("proj");
    std::fs::create_dir_all(proj.join("src")).expect("project dir");
    std::fs::create_dir_all(proj.join(".ironshell")).expect("state dir");
    std::fs::create_dir_all(dir.join("session")).expect("session dir");
    std::fs::write(proj.join("src").join("main.rs"), b"fn main() {}").expect("file");
    std::fs::write(proj.join(".ironshell").join("audit.log"), b"secret").expect("log");
    dir
}

fn ish_in(dir: &Path, code: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", code])
        .current_dir(dir.join("proj"))
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_AGENT")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_APPROVE")
        .output()
        .expect("run ish")
}

fn out(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn tar_of_dot_has_relative_names_and_skips_the_state_dir() {
    let dir = scratch("tar");
    let made = ish_in(&dir, r#"tar_create("../all.tar.gz", ".")"#);
    assert!(made.status.success(), "{}", out(&made));
    let listed = ish_in(
        &dir,
        r#"tar_list("../all.tar.gz") | map(fn(e) => e.path) | join(",")"#,
    );
    let text = out(&listed);
    assert!(listed.status.success(), "{text}");
    assert!(text.contains("src/main.rs"), "{text}");
    assert!(!text.contains("proj/"), "{text}");
    assert!(!text.contains(".ironshell"), "{text}");
    assert!(!text.contains("audit.log"), "{text}");
}

#[test]
fn tar_names_the_state_dir_explicitly() {
    let dir = scratch("tar_explicit");
    let made = ish_in(&dir, r#"tar_create("../s.tar", ".ironshell")"#);
    assert!(made.status.success(), "{}", out(&made));
    let listed = ish_in(
        &dir,
        r#"tar_list("../s.tar") | map(fn(e) => e.path) | join(",")"#,
    );
    assert!(out(&listed).contains("audit.log"), "{}", out(&listed));
}

#[test]
fn zip_of_dot_has_relative_names_and_skips_the_state_dir() {
    let dir = scratch("zip");
    let made = ish_in(&dir, r#"zip_create("../all.zip", ".")"#);
    assert!(made.status.success(), "{}", out(&made));
    let listed = ish_in(
        &dir,
        r#"zip_list("../all.zip") | map(fn(e) => e.name) | join(",")"#,
    );
    let text = out(&listed);
    assert!(listed.status.success(), "{text}");
    assert!(text.contains("src/main.rs"), "{text}");
    assert!(!text.contains("proj/"), "{text}");
    assert!(!text.contains(".ironshell"), "{text}");
}

#[test]
fn zip_skips_the_state_dir_inside_a_named_folder() {
    let dir = scratch("zip_inner");
    std::fs::create_dir_all(dir.join("proj").join("sub").join(".ironshell")).expect("dir");
    std::fs::write(
        dir.join("proj").join("sub").join(".ironshell").join("j"),
        b"x",
    )
    .expect("file");
    std::fs::write(dir.join("proj").join("sub").join("a.txt"), b"a").expect("file");
    let made = ish_in(&dir, r#"zip_create("../s.zip", "sub")"#);
    assert!(made.status.success(), "{}", out(&made));
    let listed = ish_in(
        &dir,
        r#"zip_list("../s.zip") | map(fn(e) => e.name) | join(",")"#,
    );
    let text = out(&listed);
    assert!(text.contains("sub/a.txt"), "{text}");
    assert!(!text.contains(".ironshell"), "{text}");
}

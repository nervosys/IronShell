//! `tar_create` and `tar_extract` handle non-ASCII member names on every OS.
//!
//! The system tar (bsdtar on Windows) crashed on a name such as `ü ñ/日本.txt`
//! when creating, and stopped partway when extracting. Both are in-process now
//! for a plain or gzip tar.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round48b_archive_names")
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

fn round_trip(archive: &str) {
    let dir = scratch(&archive.replace('.', "_"));
    let proj = dir.join("proj");
    let folder = proj.join("src").join("\u{fc} \u{f1}");
    std::fs::create_dir_all(&folder).expect("non-ASCII folder");
    std::fs::write(folder.join("\u{65e5}\u{672c}.txt"), b"hello").expect("file");
    std::fs::write(proj.join("src").join("plain.txt"), b"plain").expect("file");

    let made = ish_in(&proj, &format!(r#"tar_create("{archive}", "src")"#));
    assert!(made.status.success(), "tar_create failed: {}", text(&made));
    assert!(proj.join(archive).is_file());

    let extracted = ish_in(&proj, &format!(r#"tar_extract("{archive}", "out")"#));
    assert!(
        extracted.status.success(),
        "tar_extract failed: {}",
        text(&extracted)
    );
    let got = std::fs::read(
        proj.join("out")
            .join("src")
            .join("\u{fc} \u{f1}")
            .join("\u{65e5}\u{672c}.txt"),
    )
    .expect("the non-ASCII file was extracted");
    assert_eq!(got, b"hello");
    let plain = std::fs::read(proj.join("out").join("src").join("plain.txt")).expect("plain");
    assert_eq!(plain, b"plain");
}

#[test]
fn a_tar_of_non_ascii_names_round_trips() {
    round_trip("pack.tar");
}

#[test]
fn a_tar_gz_of_non_ascii_names_round_trips() {
    round_trip("pack.tar.gz");
}

#[test]
fn tar_create_still_refuses_an_existing_archive() {
    let dir = scratch("existing");
    let proj = dir.join("proj");
    std::fs::write(proj.join("a.txt"), b"x").expect("file");
    assert!(ish_in(&proj, r#"tar_create("a.tar", "a.txt")"#)
        .status
        .success());
    let again = ish_in(&proj, r#"tar_create("a.tar", "a.txt")"#);
    assert!(!again.status.success());
    assert!(text(&again).contains("already exists"), "{}", text(&again));
}

#[test]
fn tar_create_does_not_store_the_archive_in_itself() {
    let dir = scratch("self");
    let proj = dir.join("proj");
    std::fs::create_dir_all(proj.join("d")).expect("dir");
    std::fs::write(proj.join("d").join("a.txt"), b"x").expect("file");
    let made = ish_in(&proj, r#"tar_create("d/d.tar", "d")"#);
    assert!(made.status.success(), "{}", text(&made));
    let listed = ish_in(&proj, r#"tar_list("d/d.tar") | map(fn(e) => e.path)"#);
    assert!(listed.status.success(), "{}", text(&listed));
    assert!(!text(&listed).contains("d.tar"), "{}", text(&listed));
}

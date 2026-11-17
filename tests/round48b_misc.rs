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

#[test]
fn a_utf16_file_is_named_as_utf16_not_as_a_bad_byte() {
    let dir = scratch("utf16");
    let proj = dir.join("proj");
    let mut le: Vec<u8> = vec![0xFF, 0xFE];
    for u in "hi\n".encode_utf16() {
        le.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(proj.join("le.txt"), &le).expect("file");
    let mut be: Vec<u8> = vec![0xFE, 0xFF];
    for u in "hi\n".encode_utf16() {
        be.extend_from_slice(&u.to_be_bytes());
    }
    std::fs::write(proj.join("be.txt"), &be).expect("file");
    std::fs::write(proj.join("bin.dat"), [0x00u8, 0x9F, 0x92, 0x96]).expect("file");
    for call in ["file.read", "open", "cat"] {
        let out = ish_in(&proj, &format!(r#"{call}("le.txt")"#));
        assert!(!out.status.success());
        let t = text(&out);
        assert!(
            t.contains("is UTF-16 text (little-endian BOM)"),
            "{call}: {t}"
        );
        assert!(t.contains("Set-Content -Encoding utf8"), "{call}: {t}");
    }
    let t = text(&ish_in(&proj, r#"file.read("be.txt")"#));
    assert!(t.contains("is UTF-16 text (big-endian BOM)"), "{t}");
    let t = text(&ish_in(&proj, r#"file.read("bin.dat")"#));
    assert!(t.contains("is not UTF-8 text"), "{t}");
    assert!(t.contains("crypto_hash_file"), "{t}");
}

#[test]
fn file_digest_and_hmac_spellings_name_the_crypto_builtins() {
    for name in [
        "file_hash",
        "hash_file",
        "sha256sum",
        "checksum",
        "file_checksum",
    ] {
        let dym = iron_shell::builtins::did_you_mean(name);
        assert_eq!(dym, vec!["crypto_hash_file".to_string()], "{name}");
    }
    assert_eq!(
        iron_shell::builtins::did_you_mean("hmac"),
        vec!["crypto_hmac".to_string()]
    );
}

#[test]
fn crypto_hmac_with_the_algorithm_first_says_the_argument_order() {
    let dir = scratch("hmac");
    let proj = dir.join("proj");
    let t = text(&ish_in(&proj, r#"crypto_hmac("sha256", "key", "msg")"#));
    assert!(
        t.contains(
            r#"crypto_hmac takes (data, key, algorithm): write crypto_hmac("msg", "key", "sha256")"#
        ),
        "{t}"
    );
    let ok = ish_in(&proj, r#"crypto_hmac("msg", "key", "sha256") | len"#);
    assert!(ok.status.success(), "{}", text(&ok));
}

#[test]
fn an_array_of_archive_members_gets_a_hint_for_separate_arguments() {
    let dir = scratch("array_members");
    let proj = dir.join("proj");
    std::fs::write(proj.join("b.txt"), b"x").expect("file");
    for (call, example) in [
        (
            r#"zip_create("a.zip", ["b.txt"])"#,
            r#"zip_create("a.zip", "src", "b.txt")"#,
        ),
        (
            r#"tar_create("a.tar", ["b.txt"])"#,
            r#"tar_create("a.tar", "src", "b.txt")"#,
        ),
    ] {
        let out = ish_in(&proj, call);
        assert!(!out.status.success(), "{call}");
        let t = text(&out);
        assert!(t.contains("separate arguments"), "{t}");
        assert!(t.contains(example), "{t}");
    }
    std::fs::write(proj.join("a.zip"), b"").expect("file");
    let t = text(&ish_in(&proj, r#"zip_add("a.zip", ["b.txt"])"#));
    assert!(t.contains("separate arguments"), "{t}");
}

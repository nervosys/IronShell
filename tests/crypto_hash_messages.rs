//! The `crypto_hash` refusals name the builtin as the caller wrote it, say the
//! Int in `crypto_hash("sha256", 5)` is the data, and refuse a piped value
//! followed by two arguments instead of ignoring the second.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_cryptohash_msg_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, i32) {
    let dir = scratch(tag);
    let script = dir.join("hash.ish");
    std::fs::write(&script, source).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg(&script)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn a_lone_algorithm_name_is_named_as_crypto_hash_when_called_so() {
    let (err, code) = run_script("lone", "crypto_hash(\"sha256\")\n");
    assert_ne!(code, 0);
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(
        err.contains("crypto_hash: `sha256` is an algorithm name"),
        "{err}"
    );
    assert!(!err.contains("crypto.hash: `sha256`"), "{err}");
}

#[test]
fn a_lone_algorithm_name_is_named_as_crypto_hash_dot_when_called_so() {
    let (err, code) = run_script("lone_dot", "crypto.hash(\"sha256\")\n");
    assert_ne!(code, 0);
    assert!(
        err.contains("crypto.hash: `sha256` is an algorithm name"),
        "{err}"
    );
}

#[test]
fn a_non_string_data_after_the_algorithm_is_named_as_data() {
    let (err, code) = run_script("int_data", "crypto_hash(\"sha256\", 5)\n");
    assert_ne!(code, 0);
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(err.contains("expected data: String, got Int"), "{err}");
    assert!(!err.contains("algorithm: String"), "{err}");
}

#[test]
fn a_piped_value_with_two_arguments_is_an_arity_error() {
    let (err, code) = run_script("piped_two", "\"abc\" | crypto_hash(\"sha256\", \"abc\")\n");
    assert_ne!(code, 0, "the second argument was silently ignored");
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(
        err.contains("a piped value takes one argument, the algorithm; got 2 arguments"),
        "{err}"
    );
}

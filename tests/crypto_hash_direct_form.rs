//! `crypto.hash("sha256", data)`, the form AGENTS.md shows, run through the
//! real binary; and the signature lists the data parameter.
//!
//! Reproduced on 14.0.0: `crypto.hash("sha256", "abc")` failed with E_BAD_ARG
//! "expected algorithm md5, sha1, sha256, sha384 or sha512, got \"abc\"",
//! because the first argument was read as the data. The forms that already
//! worked are pinned here too: `crypto.hash("abc")` (SHA-256 by default), the
//! piped `"abc" | crypto_hash("sha256")`, and `crypto_hash("md5", "sha256")`,
//! which still hashes the text "md5". A script echoes its last value, so each
//! one ends in a bare expression.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_cryptohash_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stdout, stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, String, i32) {
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
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn assert_prints(tag: &str, source: &str, expected: &str) {
    let (out, err, code) = run_script(tag, source);
    assert_eq!(code, 0, "source {source:?} failed: {err}");
    assert_eq!(out, expected, "source {source:?}");
}

const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const MD5_ABC: &str = "900150983cd24fb0d6963f7d28e17f72";

#[test]
fn the_agents_md_form_hashes_the_data_with_the_named_algorithm() {
    // AGENTS.md: `code = ...` then a call with a variable; here the data is a
    // variable too.
    assert_prints(
        "agents",
        "data = \"abc\"\ncrypto.hash(\"sha256\", data)\n",
        SHA256_ABC,
    );
}

#[test]
fn the_two_argument_form_takes_a_literal_algorithm_and_data() {
    assert_prints("literal", "crypto.hash(\"sha256\", \"abc\")\n", SHA256_ABC);
    assert_prints("md5", "crypto.hash(\"md5\", \"abc\")\n", MD5_ABC);
}

#[test]
fn the_one_argument_form_still_uses_sha256() {
    assert_prints("default", "crypto.hash(\"abc\")\n", SHA256_ABC);
}

#[test]
fn the_piped_form_is_unchanged() {
    assert_prints("piped", "\"abc\" | crypto_hash(\"sha256\")\n", SHA256_ABC);
}

#[test]
fn a_call_that_already_worked_keeps_its_meaning() {
    // Data "md5" with algorithm "sha256", which is what the one-argument form
    // of the same text gives.
    assert_prints(
        "compat",
        "crypto.hash(\"md5\", \"sha256\") == crypto.hash(\"md5\")\n",
        "true",
    );
}

#[test]
fn an_unknown_algorithm_in_either_position_is_still_refused() {
    let dir = scratch("unknown");
    let script = dir.join("unknown.ish");
    std::fs::write(&script, "crypto.hash(\"nope\", \"abc\")\n").expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg(&script)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&dir);
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert_ne!(
        out.status.code(),
        Some(0),
        "an unknown algorithm was accepted"
    );
    assert!(err.contains("E_BAD_ARG"), "{err}");
}

#[test]
fn the_signature_lists_the_data_parameter_first() {
    let v = iron_shell::agent_api::ontology_describe_json("crypto_hash");
    let names: Vec<String> = v["parameters"]
        .as_array()
        .expect("parameters")
        .iter()
        .map(|p| p["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(names, vec!["data", "algorithm"], "{v}");
}

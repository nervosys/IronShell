//! Path names in error messages: a missing file is shown without the Windows
//! `\\?\` verbatim prefix, a refusal from the read jail names its builtin, and
//! a carriage return in a path gets the same forward-slash hint a tab or
//! newline does. These drive the real binary in a scratch folder.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("path_message_polish")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    dir
}

fn ish(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_WORKSPACE")
        .output()
        .expect("run ish")
}

#[test]
fn a_missing_file_is_shown_without_the_verbatim_prefix() {
    let dir = scratch("missing_sed");
    let out = ish(
        &dir,
        &[
            "-c",
            "sed(\"a\", \"b\", \"no-such-file.txt\", {file: true})",
        ],
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!err.contains("\\\\?\\"), "verbatim prefix shown: {err}");
    assert!(err.contains("no-such-file.txt"), "{err}");
}

#[test]
fn a_refusal_from_sed_names_sed() {
    let dir = scratch("outside_sed");
    let out = ish(
        &dir,
        &[
            "--agent",
            "-c",
            "sed(\"a\", \"b\", \"../definitely-not-here.txt\", {file: true})",
        ],
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().next().unwrap_or("");
    let json: serde_json::Value =
        serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {err:?}"));
    assert_eq!(json["error"]["code"], "E_OUTSIDE_WORKSPACE", "{json}");
    let message = json["error"]["message"].as_str().unwrap_or("");
    // The write guard (sed_replace) or the read jail (sed) may refuse first;
    // either way the message starts with the builtin's name, not ": ".
    assert!(
        message.starts_with("sed: ") || message.starts_with("sed_replace: "),
        "{message}"
    );
}

#[test]
fn a_carriage_return_in_a_missing_path_gets_the_forward_slash_hint() {
    let dir = scratch("cr_missing");
    let out = ish(&dir, &["-c", "file.read(\"proj\\r11b/x.txt\")"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("carriage return"), "{err}");
    assert!(err.contains("forward slashes"), "{err}");
}

#[test]
fn a_carriage_return_in_an_outside_workspace_path_gets_the_hint() {
    let dir = scratch("cr_outside");
    let out = ish(
        &dir,
        &["--agent", "-c", "file.read(\"../proj\\r11b/x.txt\")"],
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().next().unwrap_or("");
    let json: serde_json::Value =
        serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {err:?}"));
    assert_eq!(json["error"]["code"], "E_OUTSIDE_WORKSPACE", "{json}");
    let hint = json["error"]["hint"].as_str().unwrap_or("");
    assert!(hint.contains("carriage return"), "{hint}");
    assert!(hint.contains("forward slashes"), "{hint}");
}

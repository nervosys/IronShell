//! `ish script.ish a b c` passes the words after the script to it.
//!
//! The script reads them with `args()`, and `ish -b script.sh a b` also names
//! them `$1`..`$9` and `$#`. Under `-c` there is no script, so there are no
//! arguments, and the words after `-c` are not arguments either. Every existing
//! invocation shape still parses: the subcommands and the flags before the file.
//!
//! These drive the real binary, because the words are split by the argument
//! parser before anything in the shell sees them.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh directory for one test's script and session, under the test target
/// dir, never under the user's home or the working tree.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("script_args")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    dir
}

fn write_script(dir: &Path, file: &str, body: &str) -> PathBuf {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("write the script");
    path
}

/// Run `ish` with `args` in `dir`. Returns (exit status, stdout, stderr).
fn ish(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env("IRONSHELL_ALLOW_SH", "")
        .env_remove("IRONSHELL_MODE")
        .output()
        .expect("run ish");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn the_words_after_the_script_are_its_arguments() {
    let dir = scratch("count");
    let script = write_script(&dir, "count.ish", "len(args())\n");
    let script = script.to_str().unwrap();
    let (code, out, err) = ish(&dir, &[script, "a", "b", "c"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "3");
}

#[test]
fn flags_after_the_script_go_to_the_script() {
    let dir = scratch("flags");
    let script = write_script(&dir, "flags.ish", "args()\n");
    let script = script.to_str().unwrap();
    let (code, out, err) = ish(&dir, &[script, "--out", "-x"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("--out") && out.contains("-x"), "{out}");
}

#[test]
fn a_script_with_no_arguments_has_an_empty_args() {
    let dir = scratch("none");
    let script = write_script(&dir, "none.ish", "len(args())\n");
    let script = script.to_str().unwrap();
    let (code, out, _) = ish(&dir, &[script]);
    assert_eq!(code, 0);
    assert_eq!(out, "0");
}

#[test]
fn under_c_there_is_no_script_and_no_arguments() {
    let dir = scratch("dash_c");
    let (code, out, _) = ish(&dir, &["-c", "len(args())", "x", "y"]);
    assert_eq!(code, 0);
    assert_eq!(out, "0");
}

#[test]
fn bash_mode_names_the_arguments_as_positional_parameters() {
    let dir = scratch("posix_args");
    let script = write_script(&dir, "pos.sh", "echo \"$1-$2-$#\"\n");
    let script = script.to_str().unwrap();
    let (code, out, err) = ish(&dir, &["-b", script, "a", "b"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "a-b-2");
}

#[test]
fn bash_mode_with_no_arguments_leaves_the_parameters_unset() {
    let dir = scratch("posix_none");
    let script = write_script(&dir, "none.sh", "echo \"[$1][$#]\"\n");
    let script = script.to_str().unwrap();
    let (code, out, err) = ish(&dir, &["-b", script]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[][0]");
}

#[test]
fn subcommands_and_leading_flags_still_parse() {
    let dir = scratch("subcommands");
    let (code, _, err) = ish(&dir, &["agent", "--help"]);
    assert_eq!(code, 0, "{err}");
    let (code, out, err) = ish(&dir, &["--budget", "100000", "-c", "1 + 1"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "2");
}

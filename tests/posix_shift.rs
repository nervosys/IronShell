//! `shift [N]` in the in-process POSIX subset, and the refusal up front of the
//! shell builtins the subset does not run (they used to run as programs of the
//! same name). These drive the real binary, as posix_positional_params does.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh directory for one test's script and session, under the test target
/// dir, never under the user's home or the working tree.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("posix_shift")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    dir
}

/// Run `ish -b` with `args` in `dir`. Returns (exit status, stdout, stderr).
fn ish(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_ALLOW_SH")
        .env_remove("IRONSHELL_MODE")
        .output()
        .expect("run ish");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn script(dir: &Path, body: &str) -> String {
    let path = dir.join("shift.sh");
    std::fs::write(&path, body).expect("write the script");
    path.to_str().unwrap().to_string()
}

#[test]
fn shift_without_a_count_drops_one_parameter() {
    let dir = scratch("default");
    let s = script(
        &dir,
        "shift\necho \"[$#] [$1]\"\nshift\necho \"[$#] [$1]\"\n",
    );
    let (code, out, err) = ish(&dir, &["-b", &s, "a", "b", "c"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[2] [b]\n[1] [c]");
}

#[test]
fn shift_with_a_count_drops_that_many() {
    let dir = scratch("counted");
    let s = script(&dir, "shift 2\necho \"[$#] [$1]\"\n");
    let (code, out, err) = ish(&dir, &["-b", &s, "a", "b", "c", "d"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[2] [c]");
}

#[test]
fn shift_past_the_last_parameter_fails_and_keeps_them() {
    let dir = scratch("out_of_range");
    let s = script(
        &dir,
        "shift 5\nx=$?\necho \"[$x] [$#] [$1]\"\nshift\nshift\necho \"[$?] [$#]\"\n",
    );
    let (code, out, err) = ish(&dir, &["-b", &s, "a", "b"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[1] [2] [a]\n[0] [0]");
    assert!(err.contains("shift count out of range"), "{err}");
}

#[test]
fn shift_with_no_parameters_is_out_of_range() {
    let dir = scratch("empty");
    let s = script(&dir, "shift\necho \"[$?] [$#]\"\n");
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[1] [0]");
    assert!(err.contains("shift count out of range"), "{err}");
}

#[test]
fn getopts_is_refused_before_anything_runs() {
    let dir = scratch("getopts");
    let s = script(&dir, "echo before\ngetopts ab opt\n");
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 77, "{err}");
    assert!(out.is_empty(), "ran something: {out:?}");
    assert!(
        err.contains(
            "`getopts` builtin at line 2, column 1 is not in the in-process POSIX subset, so nothing in this script was run"
        ),
        "{err}"
    );
}

#[test]
fn eval_and_set_options_are_refused_before_anything_runs() {
    let dir = scratch("eval_set");
    let s = script(&dir, "echo before\neval echo hi\n");
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 77, "{err}");
    assert!(out.is_empty(), "ran something: {out:?}");
    assert!(err.contains("`eval` builtin at line 2, column 1"), "{err}");

    let s = script(&dir, "echo before\nset -e\n");
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 77, "{err}");
    assert!(out.is_empty(), "ran something: {out:?}");
    assert!(
        err.contains("`set` with options at line 2, column 1"),
        "{err}"
    );
}

#[test]
fn set_dash_dash_still_sets_the_parameters() {
    let dir = scratch("set_dash_dash");
    let s = script(&dir, "set -- x y z\nshift\necho \"[$#] [$1]\"\n");
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[2] [y]");
}

//! The POSIX subset runs `$@`, `$*`, `"$@"`, `"$*"` and `$#` over the script's
//! positional parameters, and `ish -b -c 'cmd' a b` names `a` and `b` `$1` and
//! `$2`. These drive the real binary, because the words are split by the
//! argument parser before the shell sees them.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh directory for one test's script and session, under the test target
/// dir, never under the user's home or the working tree.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("posix_positional_params")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    dir
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
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn script(dir: &Path, body: &str) -> String {
    let path = dir.join("params.sh");
    std::fs::write(&path, body).expect("write the script");
    path.to_str().unwrap().to_string()
}

#[test]
fn quoted_at_gives_each_argument_its_own_word() {
    let dir = scratch("quoted_at");
    let s = script(&dir, "for a in \"$@\"; do echo \"[$a]\"; done\n");
    let (code, out, err) = ish(&dir, &["-b", &s, "x y", "z"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[x y]\n[z]");
}

#[test]
fn quoted_at_with_no_arguments_is_no_word() {
    let dir = scratch("quoted_at_none");
    let s = script(&dir, "for a in \"$@\"; do echo \"[$a]\"; done\necho done\n");
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "done");
}

#[test]
fn quoted_star_joins_the_arguments_into_one_word() {
    let dir = scratch("quoted_star");
    let s = script(&dir, "for a in \"$*\"; do echo \"[$a]\"; done\n");
    let (code, out, err) = ish(&dir, &["-b", &s, "a", "b c"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[a b c]");
}

#[test]
fn unquoted_star_and_count_expand() {
    let dir = scratch("star_count");
    let s = script(&dir, "echo $* $#\n");
    let (code, out, err) = ish(&dir, &["-b", &s, "a", "b"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "a b 2");
}

#[test]
fn dash_c_takes_its_words_as_positional_parameters() {
    let dir = scratch("dash_c_args");
    let (code, out, err) = ish(&dir, &["-b", "-c", "echo $1 $#", "a", "b"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "a 2");
}

#[test]
fn plain_dash_c_has_no_positional_parameters() {
    let dir = scratch("plain_dash_c");
    let (code, out, _) = ish(&dir, &["-c", "len(args())", "x", "y"]);
    assert_eq!(code, 0);
    assert_eq!(out, "0");
}

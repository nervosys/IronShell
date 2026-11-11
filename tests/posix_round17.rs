//! Round-17 bash-compatibility fixes in the in-process POSIX subset: `read`
//! field splitting and its status at an unterminated last line, `echo -e` and
//! `printf` `\xHH`, `${10}`, `printf` `%e`/`%g`, the here-string refusal, and
//! running a shell script in a child `ish -b`. These drive the real binary, as
//! posix_shift does.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh directory for one test's script and session, under the test target
/// dir, never under the user's home or the working tree.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("posix_round17")
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

fn script(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write the script");
    path.to_str().unwrap().to_string()
}

#[test]
fn braced_positional_parameters_past_nine_are_one_parameter() {
    let dir = scratch("braced_positional");
    let s = script(&dir, "p.sh", "echo \"[${10}] [${11}] [${12}] [$10]\"\n");
    let (code, out, err) = ish(
        &dir,
        &[
            "-b", &s, "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k",
        ],
    );
    assert_eq!(code, 0, "{err}");
    // `$10` without braces is `$1` then `0`, as in bash.
    assert_eq!(out, "[j] [k] [] [a0]");
}

#[test]
fn braced_positional_past_nine_under_nounset_is_unbound() {
    let dir = scratch("braced_nounset");
    let s = script(&dir, "p.sh", "set -u\necho ${12}\n");
    let (code, out, err) = ish(&dir, &["-b", &s, "a"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.is_empty(), "ran something: {out:?}");
    assert!(err.contains("12: unbound variable"), "{err}");
}

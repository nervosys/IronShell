//! An assignment (`x=word`) in `ish -b` does no field splitting or trimming. A
//! command substitution in it loses only its trailing newlines, as in bash.
//!
//! The assignment used to split its value like an unquoted word, so the leading
//! blanks and newlines of `$(printf ...)` vanished: `x=$(printf '   a  \n')`
//! gave `[a]` where bash gives `[   a  ]`. These drive the real binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn run(tag: &str, script: &str) -> Output {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("posix_assign_substitution")
        .join(tag);
    let _ = std::fs::remove_dir_all(&root);
    let session: PathBuf = root.join("session");
    std::fs::create_dir_all(&session).expect("scratch dir");
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", script])
        .current_dir(&root)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", &session)
        .env_remove("IRONSHELL_ALLOW_SH")
        .env_remove("IRONSHELL_MODE")
        .output()
        .expect("run ish")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn leading_blanks_and_trailing_newline_of_a_substitution_are_kept() {
    let out = run("blanks", "x=$(printf '   a  \\n'); echo \"[$x]\"\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out), "[   a  ]\n");
}

#[test]
fn leading_newlines_are_kept_and_only_trailing_ones_go() {
    let out = run("newlines", "y=$(printf '\\n\\nc\\n\\n'); echo \"[$y]\"\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out), "[\n\nc]\n");
}

#[test]
fn an_unquoted_variable_in_an_assignment_is_not_split() {
    let out = run("var", "y=\"  a  b  \"; x=$y; echo \"[$x]\"\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out), "[  a  b  ]\n");
}

#[test]
fn an_assignment_with_a_substitution_and_text_keeps_both() {
    let out = run("mixed", "x=pre$(printf ' b\\n')post; echo \"[$x]\"\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out), "[pre bpost]\n");
}

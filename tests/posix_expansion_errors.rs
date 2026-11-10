//! The messages of expansion errors in `ish -b` name the script and the line, as
//! bash does: `<script>: line N: NAME: unbound variable` under `set -u`.
//!
//! The message used to be `ish: NAME: unbound variable`, with no line, so a long
//! script gave no way to find the expansion that failed.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh scratch folder for one test, with `script` written as `name`.
fn folder(tag: &str, name: &str, script: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("posix_expansion_errors")
        .join(tag);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("session")).expect("scratch dir");
    std::fs::write(root.join(name), script).expect("write script");
    root
}

fn ish(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b"])
        .args(args)
        .current_dir(root)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", root.join("session"))
        .env_remove("IRONSHELL_ALLOW_SH")
        .env_remove("IRONSHELL_MODE")
        .output()
        .expect("run ish")
}

#[test]
fn a_script_file_names_itself_and_the_line_of_an_unbound_name() {
    let root = folder(
        "file_nounset",
        "deploy.sh",
        "set -u\n\necho ok\necho $MISSING_ISH_NAME\necho never\n",
    );
    let out = ish(&root, &["deploy.sh"]);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "ok\n");
    assert!(
        err.contains("deploy.sh: line 4: MISSING_ISH_NAME: unbound variable"),
        "{err}"
    );
}

#[test]
fn a_dash_c_script_is_named_ish_with_its_line() {
    let root = folder("dash_c_nounset", "unused.sh", "");
    let out = ish(&root, &["-c", "echo one\nset -u\necho \"$NOPE_ISH_X\"\n"]);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(
        err.contains("ish: line 3: NOPE_ISH_X: unbound variable"),
        "{err}"
    );
}

#[test]
fn a_question_mark_form_prints_its_own_message_without_set_u() {
    let root = folder(
        "colon_question",
        "csvcol.sh",
        "echo start\ncol=${1:?usage: csvcol.sh COLUMN}\necho never\n",
    );
    let out = ish(&root, &["csvcol.sh"]);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "start\n");
    assert!(
        err.contains("csvcol.sh: line 2: 1: usage: csvcol.sh COLUMN"),
        "{err}"
    );
}

#[test]
fn a_question_mark_form_without_a_message_uses_the_shell_default() {
    let root = folder("bare_question", "unused.sh", "");
    let out = ish(&root, &["-c", "x=\necho \"${x:?}\"\n"]);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(
        err.contains("ish: line 2: x: parameter null or not set"),
        "{err}"
    );
    // Without the colon only an unset name fails; an empty one is kept.
    let out = ish(&root, &["-c", "x=\necho \"[${x?}]\"\n"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "[]\n");
    let out = ish(&root, &["-c", "echo \"${NOPE_ISH_Q?}\"\n"]);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(err.contains("NOPE_ISH_Q: parameter not set"), "{err}");
}

#[test]
fn a_question_mark_form_with_a_value_expands_to_it() {
    let root = folder("set_question", "unused.sh", "");
    let out = ish(&root, &["-c", "x=v\necho \"${x:?missing}\"\n"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "v\n");
}

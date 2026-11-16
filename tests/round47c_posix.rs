//! Round 47c: `~` without HOME, `$'...'`, brace expansion refused, `cd -`.
//! These drive the real binary in the in-process POSIX engine (`ish -b`).

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round47c_posix")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    dir
}

/// (status, stdout with CRLF folded, stderr); HOME and USERPROFILE removed
/// when `no_home`.
fn ish(dir: &Path, script: &str, no_home: bool) -> (i32, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["-b", "-c", script])
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_ALLOW_SH")
        .env_remove("IRONSHELL_MODE");
    if no_home {
        cmd.env_remove("HOME");
    }
    let out = cmd.output().expect("run ish");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn tilde_expands_to_the_home_directory_without_home() {
    let d = scratch("tilde");
    let home = dirs::home_dir().expect("a home directory");
    let home = home.to_string_lossy().into_owned();
    let (code, out, err) = ish(&d, "echo ~", true);
    assert_eq!(code, 0, "stderr: {err}");
    assert!(!out.trim().is_empty(), "~ must not be empty");
    assert_eq!(out.trim_end(), home);
    let (_, out, _) = ish(&d, "echo ~/x", true);
    assert_eq!(out.trim_end(), format!("{home}/x"));
    let (_, out, _) = ish(&d, "echo $HOME", true);
    assert_eq!(out.trim_end(), home);
}

#[test]
fn ansi_c_quoting_decodes_escapes() {
    let d = scratch("ansic");
    let (code, out, err) = ish(&d, r"echo $'a\tb'", false);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "a\tb\n");
    let (_, out, _) = ish(&d, r"printf '%s' $'x\x41\101\\\'y'", false);
    assert_eq!(out, r"xAA\'y");
}

#[test]
fn brace_expansion_is_refused_and_literal_braces_are_not() {
    let d = scratch("brace");
    for script in ["echo {a,b}", "echo x{1..3}"] {
        let (code, out, err) = ish(&d, script, false);
        assert_eq!(code, 77, "{script}: stderr: {err}");
        assert_eq!(out, "", "nothing may run");
        assert!(err.contains("brace expansion"), "stderr: {err}");
    }
    let (code, out, err) = ish(&d, "x=1; echo {} {a} '{a,b}' \"{a,b}\" ${x}", false);
    assert_eq!(code, 0, "stderr: {err}");
    assert_eq!(out, "{} {a} {a,b} {a,b} 1\n");
}

#[test]
fn cd_dash_prints_the_directory_it_returns_to() {
    let d = scratch("cddash");
    std::fs::create_dir_all(d.join("sub")).unwrap();
    let (code, out, err) = ish(&d, "cd sub; cd ..; cd -", false);
    assert_eq!(code, 0, "stderr: {err}");
    let line = out.trim_end();
    assert!(
        line.ends_with("sub"),
        "cd - must print the new directory, got {out:?}"
    );
}

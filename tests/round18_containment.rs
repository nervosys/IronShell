//! Agent-mode containment defects (round 18). Each test drives the real binary
//! in agent mode, in a scratch workspace with its own session directory, as a
//! red-team harness would, and checks what reached the disk outside the jail.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh scratch directory: `proj` is the workspace, `outside` holds files the
/// jail must not touch, and `session` holds the journal.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round18_containment")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj")).expect("workspace dir");
    std::fs::create_dir_all(dir.join("outside")).expect("outside dir");
    std::fs::create_dir_all(dir.join("session")).expect("session dir");
    dir
}

/// Run `args` in agent mode with `dir/proj` as the workspace.
fn ish_agent(dir: &Path, args: &[&str]) -> Output {
    let ws = dir.join("proj");
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--agent", "--workspace"])
        .arg(&ws)
        .args(args)
        .current_dir(&ws)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_APPROVE_ALL")
        .env_remove("IRONSHELL_APPROVE")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_SCRIPT_DEPTH")
        .output()
        .expect("run ish")
}

/// A file outside the workspace, `outside/t.txt`, with `proj/hl.txt` a hard link
/// to it. Returns the outside file's path.
fn hard_link_outside(dir: &Path) -> PathBuf {
    let outside = dir.join("outside").join("t.txt");
    std::fs::write(&outside, "orig\n").expect("outside file");
    std::fs::hard_link(&outside, dir.join("proj").join("hl.txt")).expect("hard link");
    outside
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// ── Item 1: an in-place write through a hard link is refused ──

#[test]
fn redirect_into_a_hard_link_is_refused_and_the_outside_file_is_unchanged() {
    let dir = scratch("hl_redirect");
    let outside = hard_link_outside(&dir);
    let out = ish_agent(&dir, &["-b", "-c", "echo ZZ > hl.txt"]);
    assert!(
        !out.status.success(),
        "a write through a hard link must fail"
    );
    let err = stderr_of(&out);
    assert!(err.contains("has 2 hard links"), "{err}");
    assert!(err.contains("remove the link or write a new file"), "{err}");
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "orig\n");
}

#[test]
fn append_redirect_into_a_hard_link_is_refused() {
    let dir = scratch("hl_append");
    let outside = hard_link_outside(&dir);
    let out = ish_agent(&dir, &["-b", "-c", "echo ZZ >> hl.txt"]);
    assert!(!out.status.success());
    assert!(stderr_of(&out).contains("has 2 hard links"));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "orig\n");
}

#[test]
fn tee_into_a_hard_link_is_refused_and_the_outside_file_is_unchanged() {
    let dir = scratch("hl_tee");
    let outside = hard_link_outside(&dir);
    let out = ish_agent(&dir, &["-b", "-c", "echo ZZ | tee hl.txt"]);
    assert!(!out.status.success());
    assert!(stderr_of(&out).contains("has 2 hard links"));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "orig\n");
}

#[test]
fn touch_of_a_hard_link_is_refused() {
    let dir = scratch("hl_touch");
    let outside = hard_link_outside(&dir);
    let before = std::fs::metadata(&outside).unwrap().modified().unwrap();
    let out = ish_agent(&dir, &["-b", "-c", "touch hl.txt"]);
    assert!(!out.status.success());
    assert!(stderr_of(&out).contains("has 2 hard links"));
    assert_eq!(
        std::fs::metadata(&outside).unwrap().modified().unwrap(),
        before
    );
}

#[test]
fn file_append_into_a_hard_link_is_refused() {
    let dir = scratch("hl_file_append");
    let outside = hard_link_outside(&dir);
    let out = ish_agent(&dir, &["-c", r#"file_append("hl.txt", "ZZ")"#]);
    assert!(!out.status.success());
    assert!(
        stderr_of(&out).contains("hard links"),
        "{}",
        stderr_of(&out)
    );
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "orig\n");
}

#[test]
fn a_plain_file_in_the_workspace_still_writes() {
    let dir = scratch("hl_plain");
    let _outside = hard_link_outside(&dir);
    let out = ish_agent(&dir, &["-b", "-c", "echo ok > plain.txt"]);
    assert!(
        out.status.success(),
        "a plain workspace file must still be writable: {}",
        stderr_of(&out)
    );
    let text = std::fs::read_to_string(dir.join("proj").join("plain.txt")).unwrap();
    assert_eq!(text.trim(), "ok");
}

// ── Item 2: a script outside the workspace is refused before it runs ──

/// A script in the workspace runs; the same script outside it is refused by
/// every spelling, and nothing it would write appears in the workspace.
#[cfg(windows)]
#[test]
fn a_script_outside_the_workspace_is_refused_before_it_runs() {
    let dir = scratch("script_outside");
    let script = "#!/bin/sh\necho ran > ran.txt\n";
    std::fs::write(dir.join("outside").join("x.sh"), script).unwrap();
    std::fs::write(
        dir.join("proj").join("inside.sh"),
        "#!/bin/sh\necho inside\n",
    )
    .unwrap();
    let absolute = dir
        .join("outside")
        .join("x.sh")
        .to_string_lossy()
        .replace('\\', "/");

    let inside = ish_agent(&dir, &["-b", "-c", "sh inside.sh"]);
    assert!(
        inside.status.success(),
        "a script inside the workspace must still run: {}",
        stderr_of(&inside)
    );
    assert_eq!(String::from_utf8_lossy(&inside.stdout).trim(), "inside");

    for code in [
        "sh ../outside/x.sh".to_string(),
        "bash ../outside/x.sh".to_string(),
        absolute.clone(),
    ] {
        let out = ish_agent(&dir, &["-b", "-c", code.as_str()]);
        let err = stderr_of(&out);
        assert!(!out.status.success(), "`{code}` must be refused");
        assert!(err.contains("outside the workspace"), "`{code}`: {err}");
        assert!(err.contains("x.sh"), "the refusal names the script: {err}");
        assert!(
            !dir.join("proj").join("ran.txt").exists(),
            "`{code}` ran the script"
        );
    }
}

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

// ── Item 3: a script that runs itself stops, and the depth cannot be reset ──

/// A script that calls itself nests children until the limit, then stops with
/// the budget refusal instead of exhausting memory.
#[cfg(windows)]
#[test]
fn a_script_that_runs_itself_stops_at_the_nesting_limit() {
    let dir = scratch("script_bomb");
    std::fs::write(
        dir.join("proj").join("bomb.sh"),
        "#!/bin/sh\n./bomb.sh \"$1x\"\n",
    )
    .unwrap();
    let out = ish_agent(&dir, &["-b", "-c", "./bomb.sh x"]);
    let err = stderr_of(&out);
    assert!(
        !out.status.success(),
        "a self-calling script must stop: {err}"
    );
    assert!(
        err.contains("scripts nest at most 32 deep"),
        "the refusal names the limit: {err}"
    );
    assert!(err.contains("E_BUDGET_EXCEEDED"), "{err}");
}

/// The depth variable is an IRONSHELL_ name, so an agent cannot set it for a
/// script to reset the count.
#[cfg(windows)]
#[test]
fn an_agent_cannot_reset_the_script_depth_for_a_script() {
    let dir = scratch("depth_reset");
    std::fs::write(dir.join("proj").join("x.sh"), "#!/bin/sh\necho hi\n").unwrap();
    let out = ish_agent(&dir, &["-b", "-c", "IRONSHELL_SCRIPT_DEPTH=0 ./x.sh"]);
    let err = stderr_of(&out);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("IRONSHELL_SCRIPT_DEPTH"), "{err}");
    assert!(err.contains("E_POLICY_DENY"), "{err}");
    assert!(out.stdout.is_empty(), "the script must not run");
}

// ── Item 4: agent mode is not told to set IRONSHELL_ALLOW_SH ──

/// Agent mode refuses to set IRONSHELL_ALLOW_SH, so a refusal in agent mode
/// keeps what to do instead and drops the suggestion to set that variable.
#[test]
fn agent_mode_refusals_do_not_offer_allow_sh() {
    let dir = scratch("hint_agent");
    let builtin = ish_agent(&dir, &["-b", "-c", "eval 'echo hi'"]);
    let err = stderr_of(&builtin);
    assert!(!builtin.status.success(), "{err}");
    assert!(err.contains("`eval` builtin"), "{err}");
    assert!(
        !err.contains("IRONSHELL_ALLOW_SH"),
        "agent mode must not be told to set it: {err}"
    );

    let arithmetic = ish_agent(&dir, &["-b", "-c", "echo $((1+2))"]);
    let err = stderr_of(&arithmetic);
    assert!(err.contains("compute in ish (ish -c)"), "{err}");
    assert!(!err.contains("IRONSHELL_ALLOW_SH"), "{err}");
}

// ── Item 5: a failed write says it failed to write ──

/// An NTFS alternate-stream name is refused before the write (round 38: as
/// E_BAD_ARG naming the stream), never reported as a failed read.
#[cfg(windows)]
#[test]
fn a_failed_write_to_a_stream_name_says_cannot_write() {
    let dir = scratch("stream_write");
    let out = ish_agent(&dir, &["-c", r#"file_write("inside.txt:stream1", "x")"#]);
    let err = stderr_of(&out);
    assert!(!out.status.success(), "{err}");
    assert!(err.contains("alternate data stream"), "{err}");
    assert!(!err.contains("cannot read"), "{err}");
}

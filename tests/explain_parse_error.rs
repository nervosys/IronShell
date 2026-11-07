//! `ish --explain` on a file that does not parse reports it as a run does.
//!
//! The parse error used to come out of `--explain` wrapped as a builtin
//! argument error, so stderr was `Error: {"error":{...}}` with the JSON nested
//! inside a string, and the exit status was 1. A run of the same file prints the
//! parser's own `E_PARSE` JSON and exits 2. `--explain` now matches the run.

use std::process::Command;

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_expl_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ish(args: &[&str], session: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .env("IRONSHELL_SESSION_DIR", session)
        .env("IRONSHELL_MODE", "agent")
        .output()
        .expect("run ish")
}

#[test]
fn explain_of_a_parse_error_matches_a_run_of_the_same_file() {
    let dir = scratch("parse");
    let file = dir.join("bad.ish");
    std::fs::write(&file, "x = [1, 2\nprint(x)\n").unwrap();
    let file_s = file.to_string_lossy().into_owned();

    let explained = ish(&["--explain", &file_s], &dir);
    let ran = ish(&[&file_s], &dir);

    assert_eq!(
        explained.status.code(),
        Some(2),
        "--explain must exit 2 on a parse error, as a run does: {:?}",
        String::from_utf8_lossy(&explained.stderr)
    );
    assert_eq!(ran.status.code(), Some(2), "the run exits 2");

    let err: serde_json::Value = serde_json::from_slice(&explained.stderr)
        .unwrap_or_else(|e| panic!("stderr is not clean JSON ({e}): {:?}", explained.stderr));
    assert_eq!(err["error"]["code"], "E_PARSE", "{err}");
    assert!(
        err["error"]["message"].is_string(),
        "the message is a plain string, not nested JSON: {err}"
    );

    let run_err: serde_json::Value = serde_json::from_slice(&ran.stderr)
        .unwrap_or_else(|e| panic!("run stderr is not clean JSON ({e})"));
    assert_eq!(
        err, run_err,
        "--explain and a run must report the same error"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn explain_of_a_good_file_still_reports_effects() {
    let dir = scratch("good");
    let file = dir.join("good.ish");
    std::fs::write(&file, "x = 1\nprint(x)\n").unwrap();
    let out = ish(&["--explain", &file.to_string_lossy()], &dir);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("the effect report is JSON on stdout");
    assert!(report.is_object(), "{report}");
    let _ = std::fs::remove_dir_all(&dir);
}

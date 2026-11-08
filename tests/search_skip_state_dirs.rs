//! `search_regex` with no path does not search IronShell's own state directory,
//! run through the real binary in a temporary project.
//!
//! Reproduced on 14.0.0: a search for a word the shell had logged matched
//! `./.ironshell/audit.log`, so the answer was the log and not the project. The
//! recursive walk already skips `.git`, `target`, `node_modules` and similar; the
//! state directory is now skipped too. A match in the project itself is still
//! found. A script echoes its last value, so the script ends in a bare expression.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn search_regex_skips_the_ironshell_state_directory() {
    let dir: PathBuf = std::env::temp_dir().join(format!("ae_skipstate_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".ironshell")).expect("state dir");
    std::fs::create_dir_all(dir.join("src")).expect("src dir");
    std::fs::write(dir.join(".ironshell").join("audit.log"), "needle logged\n").expect("log");
    std::fs::write(dir.join("src").join("a.txt"), "needle in project\n").expect("src file");
    let script = dir.join("search.ish");
    std::fs::write(&script, "search_regex(\"needle\")\n").expect("script");

    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg(&script)
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("run ish");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(
        stdout.contains("needle in project"),
        "project match missing: {stdout}"
    );
    assert!(
        !stdout.contains("audit.log"),
        "state log was searched: {stdout}"
    );
    assert!(
        !stdout.contains("needle logged"),
        "state log was searched: {stdout}"
    );
}

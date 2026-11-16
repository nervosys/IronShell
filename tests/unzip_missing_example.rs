//! The `unzip` help example says a missing archive is E_NOT_FOUND. This runs
//! the same call through the real binary so the example and the behaviour
//! cannot drift apart again.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_unzip_missing_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn unzip_of_a_missing_archive_is_e_not_found() {
    let dir = scratch("code");
    let script = dir.join("unzip.ish");
    std::fs::write(
        &script,
        "(try { unzip(\"no-such.zip\") } catch e { e.error.code })\n",
    )
    .expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg(&script)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .current_dir(&dir)
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(stdout, "E_NOT_FOUND", "{stdout}");
}

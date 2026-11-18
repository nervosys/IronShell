//! Round 53c: a missing script file named a builtin that does not exist:
//! `{"builtin":"run","message":"run: no file at ..."}`. `run` is not a builtin;
//! the program is `ish`.

use std::process::Command;

#[test]
fn a_missing_script_is_named_ish_not_run() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("round53c_missing_script");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("session");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--agent", "nope.ish"])
        .current_dir(&dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .output()
        .expect("run ish");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    let json: serde_json::Value = serde_json::from_str(err.lines().next().unwrap_or(""))
        .unwrap_or_else(|e| panic!("not JSON ({e}): {} bytes of stderr", err.len()));
    assert_eq!(json["error"]["code"], "E_NOT_FOUND", "{json}");
    assert_eq!(json["error"]["builtin"], "ish", "{json}");
    let message = json["error"]["message"].as_str().unwrap_or("");
    assert!(message.starts_with("ish: no file"), "{message}");
    assert!(!message.contains("run:"), "{message}");
}

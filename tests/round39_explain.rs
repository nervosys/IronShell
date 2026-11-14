//! `--explain` states agent mode's decision: `shell` is refused as `sh` is, and
//! a literal path outside the workspace is `deny` even from a human session.

use std::process::Command;

fn explain(code: &str) -> serde_json::Value {
    let dir = std::env::temp_dir().join("ish_round39_explain");
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--explain", "-c", code])
        .current_dir(&dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env_remove("IRONSHELL_ALLOW_SH")
        .env_remove("IRONSHELL_WORKSPACE")
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).expect("explain prints JSON")
}

#[test]
fn shell_is_denied_as_sh_is() {
    assert_eq!(explain(r#"sh("echo hi")"#)["decision"], "deny");
    assert_eq!(explain(r#"shell("echo hi")"#)["decision"], "deny");
}

#[test]
fn a_literal_path_outside_the_workspace_is_denied() {
    for code in [r#"file.write("../out.txt", "hi")"#, r#"ls("../")"#] {
        assert_eq!(explain(code)["decision"], "deny", "{code}");
    }
    assert_eq!(
        explain(r#"file.write("out.txt", "hi")"#)["decision"],
        "allow"
    );
}

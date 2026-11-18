//! Round 53c: an approval token for `sh(["ish", "payload.ish"])` kept approving
//! after the agent rewrote `payload.ish`, because the descriptor held only the
//! command line. It now also holds a short content hash of each operand that
//! names an existing regular file inside the workspace.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round53c_approval_hash")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("session");
    std::fs::create_dir_all(dir.join("proj")).expect("proj");
    std::fs::write(dir.join("proj").join("payload.ish"), "1 + 1\n").expect("payload");
    dir
}

fn agent(dir: &Path, code: &str, approve: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["--agent", "-c", code])
        .current_dir(dir.join("proj"))
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("IRONSHELL_ALLOW_SH", "1")
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_APPROVE_ALL")
        .env_remove("IRONSHELL_APPROVE");
    if let Some(token) = approve {
        cmd.env("IRONSHELL_APPROVE", token);
    }
    cmd.output().expect("run ish")
}

fn first_error(out: &Output) -> serde_json::Value {
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().next().unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {err:?}"))
}

fn call() -> String {
    let exe = env!("CARGO_BIN_EXE_ish").replace('\\', "/");
    format!(r#"sh(["{exe}", "payload.ish"])"#)
}

#[test]
fn rewriting_the_script_needs_a_new_approval() {
    let dir = scratch("rewrite");
    let first = first_error(&agent(&dir, &call(), None));
    assert_eq!(first["error"]["code"], "E_NEEDS_APPROVAL", "{first}");
    let hashes = &first["error"]["approval"]["blast_radius"]["operand_hashes"];
    let shown = hashes["payload.ish"].as_str().unwrap_or("");
    assert_eq!(shown.len(), 16, "no 16-hex content hash: {first}");
    let token = first["error"]["approval"]["token"]
        .as_str()
        .expect("token")
        .to_string();
    let hint = first["error"]["hint"].as_str().unwrap_or("");
    assert!(hint.contains("contents"), "the hint is silent: {hint}");

    // The token approves the unchanged script.
    let ok = agent(&dir, &call(), Some(&token));
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );

    // After the script is rewritten it no longer does.
    std::fs::write(dir.join("proj").join("payload.ish"), "2 + 2\n").expect("rewrite");
    let again = first_error(&agent(&dir, &call(), Some(&token)));
    assert_eq!(again["error"]["code"], "E_NEEDS_APPROVAL", "{again}");
    let token2 = again["error"]["approval"]["token"]
        .as_str()
        .expect("token2");
    assert_ne!(token2, token);
    assert_ne!(
        again["error"]["approval"]["blast_radius"]["operand_hashes"]["payload.ish"],
        first["error"]["approval"]["blast_radius"]["operand_hashes"]["payload.ish"]
    );
}

#[test]
fn a_command_naming_no_workspace_file_keeps_its_token() {
    let dir = scratch("nofile");
    let exe = env!("CARGO_BIN_EXE_ish").replace('\\', "/");
    let code = format!(r#"sh(["{exe}", "--version"])"#);
    let first = first_error(&agent(&dir, &code, None));
    assert!(
        first["error"]["approval"]["blast_radius"]
            .get("operand_hashes")
            .is_none(),
        "{first}"
    );
}

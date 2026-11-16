//! Round 47a: error hints that pointed the wrong way, run through the real binary.

use std::process::Command;

/// Runs `ish -c src` with `extra` flags first; returns (stdout, stderr, exit code).
fn run(extra: &[&str], src: &str) -> (String, String, i32) {
    let dir = std::env::temp_dir().join(format!(
        "ae_r47a_{}_{}",
        std::process::id(),
        src.len() + extra.len()
    ));
    let _ = std::fs::create_dir_all(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(extra)
        .arg("-c")
        .arg(src)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .current_dir(&dir)
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn unknown_module_member_does_not_suggest_none() {
    let (out, err, code) = run(&["--agent"], "file.nope()");
    let all = format!("{out}{err}");
    assert_ne!(code, 0, "{all}");
    assert!(all.contains("E_UNKNOWN_BUILTIN"), "{all}");
    assert!(!all.contains("did you mean: None"), "{all}");
    assert!(!all.contains("\"None\""), "{all}");
    assert!(all.contains("\"retryable\":false"), "{all}");
}

#[test]
fn unterminated_single_quote_says_use_double_quotes() {
    let (out, err, code) = run(&[], "'a\'b'");
    let all = format!("{out}{err}");
    assert_ne!(code, 0, "{all}");
    assert!(all.contains("takes no escapes"), "{all}");
    assert!(all.contains("double-quoted"), "{all}");
    assert!(!all.contains("or escape the quote inside it"), "{all}");
    // A double-quoted one still may escape.
    let (out, err, _) = run(&[], "\"abc");
    assert!(format!("{out}{err}").contains("escape the quote"), "{out}{err}");
}

#[test]
fn unclosed_hole_in_single_quotes_points_to_double_quotes() {
    let (out, err, code) = run(&[], "'a ${x'");
    let all = format!("{out}{err}");
    assert_ne!(code, 0, "{all}");
    assert!(all.contains("double-quoted string with"), "{all}");
}

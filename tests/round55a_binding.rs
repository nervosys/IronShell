//! Round 55a: what an exec approval binds (quoted file names, the file cap,
//! environment names that differ in case, links) and the env denylist.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round55a_binding")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("session");
    std::fs::create_dir_all(dir.join("proj")).expect("proj");
    dir
}

fn base(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.current_dir(dir.join("proj"))
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("IRONSHELL_ALLOW_SH", "1")
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_APPROVE_ALL")
        .env_remove("IRONSHELL_APPROVE");
    cmd
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The first 600 characters of a run's output, for a failure message.
fn head(out: &Output) -> String {
    text(out).chars().take(600).collect()
}

/// The first `apv_` token in the output of a run, panicking without it.
fn token_of(out: &Output) -> String {
    let t = text(out);
    let i = t
        .find("apv_")
        .unwrap_or_else(|| panic!("no approval token in: {}", head(out)));
    t[i..i + 20].to_string()
}

fn posix_token(dir: &Path, script: &str) -> String {
    let out = base(dir)
        .args(["-b", "--agent", "-c", script])
        .output()
        .expect("run ish");
    token_of(&out)
}

fn sh_token(dir: &Path, call: &str) -> String {
    let out = base(dir)
        .args(["--agent", "-c", call])
        .output()
        .expect("run ish");
    token_of(&out)
}

// ---- A1: a name inside quotes is bound whole ------------------------------

#[test]
fn a_quoted_file_name_with_a_space_is_bound() {
    let dir = scratch("quoted");
    let proj = dir.join("proj");
    std::fs::write(proj.join("a b.ps1"), "one").expect("write");
    let posix = [
        "pwsh -NoProfile -File 'a b.ps1'",
        r#"pwsh -NoProfile -Command "& './a b.ps1'""#,
    ];
    let sh_call = r#"sh(["pwsh", "-NoProfile", "-Command", "& 'a b.ps1'"])"#;
    let before: Vec<String> = posix.iter().map(|s| posix_token(&dir, s)).collect();
    let before_sh = sh_token(&dir, sh_call);
    std::fs::write(proj.join("a b.ps1"), "two").expect("rewrite");
    for (i, script) in posix.iter().enumerate() {
        assert_ne!(
            before[i],
            posix_token(&dir, script),
            "editing 'a b.ps1' left `{script}` approved"
        );
    }
    assert_ne!(before_sh, sh_token(&dir, sh_call), "sh() kept its token");
    // Unchanged, the token is stable.
    assert_eq!(sh_token(&dir, sh_call), sh_token(&dir, sh_call));
}

//! Round 56a: an approval token binds the content of a workspace file a command
//! names, however the name is spelled (extended-length and device prefixes,
//! `file:` URLs, response files, glued options, `+` and `,` joiners).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round56a_approval_binding")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("session");
    std::fs::create_dir_all(dir.join("proj")).expect("proj");
    std::fs::write(dir.join("proj").join("x.txt"), "one\n").expect("x.txt");
    dir
}

fn agent(dir: &Path, code: &str) -> Output {
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
    cmd.output().expect("run ish")
}

fn first_error(out: &Output) -> serde_json::Value {
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().next().unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {err}"))
}

/// The approval token of `sh([ish, arg])` run from the scratch workspace.
fn token_of(dir: &Path, arg: &str) -> String {
    let exe = env!("CARGO_BIN_EXE_ish").replace('\\', "/");
    let arg = arg.replace('\\', "\\\\");
    let code = format!(r#"sh(["{exe}", "{arg}"])"#);
    let err = first_error(&agent(dir, &code));
    assert_eq!(err["error"]["code"], "E_NEEDS_APPROVAL", "{err}");
    err["error"]["approval"]["token"]
        .as_str()
        .expect("token")
        .to_string()
}

/// The token of the call changes when the file the spelling names changes.
fn assert_bound(dir: &Path, arg: &str) {
    let file = dir.join("proj").join("x.txt");
    std::fs::write(&file, "one\n").expect("write one");
    let before = token_of(dir, arg);
    std::fs::write(&file, "two, longer\n").expect("write two");
    let after = token_of(dir, arg);
    assert_ne!(before, after, "the token ignores the content of {arg}");
}

#[cfg(windows)]
#[test]
fn extended_length_and_device_prefixes_are_bound() {
    let dir = scratch("prefixes");
    let plain = dir
        .join("proj")
        .join("x.txt")
        .canonicalize()
        .expect("canon");
    // `canonicalize` gives `\\?\C:\...`; strip it for the plain spelling.
    let plain = plain
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string();
    let forward = plain.replace('\\', "/");
    assert_bound(&dir, &format!(r"\\?\{plain}"));
    assert_bound(&dir, &format!("//?/{forward}"));
    assert_bound(&dir, &format!(r"\\.\{plain}"));
    assert_bound(&dir, &format!("file:///{forward}"));
    let proj_name = "proj";
    let dotted = plain.replace(
        &format!(r"\{proj_name}\"),
        &format!(r"\{proj_name}\..\{proj_name}\"),
    );
    assert_bound(&dir, &format!(r"\\?\{dotted}"));
}

#[cfg(windows)]
#[test]
fn a_unc_extended_path_is_not_probed_or_bound() {
    // It must still be refused for approval, quickly, without a network probe.
    let dir = scratch("unc");
    let exe = env!("CARGO_BIN_EXE_ish").replace('\\', "/");
    let code = format!(r#"sh(["{exe}", "\\\\?\\UNC\\ae-no-such-host\\share\\x.txt"])"#);
    let started = std::time::Instant::now();
    let err = first_error(&agent(&dir, &code));
    assert_eq!(err["error"]["code"], "E_NEEDS_APPROVAL", "{err}");
    assert!(started.elapsed().as_secs() < 20, "the UNC name was probed");
}

#[test]
fn a_file_url_is_bound() {
    let dir = scratch("fileurl");
    let plain = dir.join("proj").join("x.txt");
    let plain = plain.to_string_lossy().replace('\\', "/");
    let url = if plain.starts_with('/') {
        format!("file://{plain}")
    } else {
        format!("file:///{plain}")
    };
    assert_bound(&dir, &url);
}

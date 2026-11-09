//! A pipeline whose later stage names no builtin is refused before its first
//! stage runs. `file.write("p", "a") | nosuchfn()` used to write `p` and only
//! then fail with E_UNKNOWN_BUILTIN. These drive the real binary, in a scratch
//! folder under the test target dir, so a write that happened would show.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("pipeline_unknown_stage")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    dir
}

fn ish_c(dir: &Path, code: &str) -> (Option<i32>, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", code])
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .output()
        .expect("run ish");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn an_unknown_later_stage_runs_nothing_before_it() {
    let dir = scratch("write_then_unknown");
    let (code, _, err) = ish_c(&dir, "file.write(\"p.txt\", \"a\") | nosuchfn()");
    assert_ne!(code, Some(0), "{err}");
    assert!(err.contains("E_UNKNOWN_BUILTIN"), "{err}");
    assert!(err.contains("nosuchfn"), "{err}");
    assert!(
        !dir.join("p.txt").exists(),
        "the first stage ran before the unknown stage was refused"
    );
}

#[test]
fn an_unknown_stage_in_the_middle_runs_nothing_either() {
    let dir = scratch("middle");
    let (code, _, err) = ish_c(&dir, "file.write(\"p.txt\", \"a\") | nosuchfn() | len");
    assert_ne!(code, Some(0), "{err}");
    assert!(err.contains("nosuchfn"), "{err}");
    assert!(!dir.join("p.txt").exists(), "{err}");
}

#[test]
fn a_user_function_stage_is_not_refused() {
    let dir = scratch("user_fn");
    let (code, _, err) = ish_c(&dir, "double = fn(x) => x * 2\n[1, 2, 3] | map(double)");
    assert_eq!(code, Some(0), "{err}");
}

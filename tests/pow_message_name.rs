//! `pow`'s refusals name `pow` when it is called as `pow`, and `math.pow` when
//! it is called as `math.pow`; they never say `math_pow`, which is no name a
//! caller wrote.

use std::path::PathBuf;
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_pow_msg_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `source` as a script file and returns (stderr, exit code).
fn run_script(tag: &str, source: &str) -> (String, i32) {
    let dir = scratch(tag);
    let script = dir.join("pow.ish");
    std::fs::write(&script, source).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg(&script)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn a_direct_pow_nan_is_named_pow() {
    let (err, code) = run_script("nan", "pow(-2, 0.5)\n");
    assert_ne!(code, 0);
    assert!(err.contains("pow: -2 ** 0.5 is not a real number"), "{err}");
    assert!(!err.contains("math_pow"), "{err}");
}

#[test]
fn a_direct_pow_overflow_is_named_pow() {
    let (err, code) = run_script("overflow", "pow(10.0, 400)\n");
    assert_ne!(code, 0);
    assert!(err.contains("pow: 10 ** 400 overflows a Float"), "{err}");
    assert!(!err.contains("math_pow"), "{err}");
}

#[test]
fn a_module_pow_nan_is_named_as_called() {
    let (err, code) = run_script("module_nan", "math.pow(-2, 0.5)\n");
    assert_ne!(code, 0);
    assert!(
        err.contains("math.pow: -2 ** 0.5 is not a real number"),
        "{err}"
    );
    assert!(!err.contains("math_pow"), "{err}");
}

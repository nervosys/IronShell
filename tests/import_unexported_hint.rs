//! A name a module defines without `pub` is not exported, and the failure now
//! says so, run through the real binary. Before this, `import "mathlib.ish"`
//! then `triple(2)` said only "unknown builtin: triple" (exit 127), and
//! `import "mathlib.ish" as m` gave an empty record with no reason. The pub rule
//! itself is unchanged, and successful scripts keep their exit codes.
//! A script echoes its last value, so each one ends in a bare expression.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_unexp_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Writes the modules into a fresh temp directory, runs `script` there, and
/// returns (stdout, stderr, exit code).
fn run_with_modules(tag: &str, modules: &[(&str, &str)], script: &str) -> (String, String, i32) {
    let dir = scratch(tag);
    for (name, body) in modules {
        std::fs::write(dir.join(name), body).expect("write module");
    }
    let path: &Path = &dir.join("main.ish");
    std::fs::write(path, script).expect("write script");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg(path)
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

const MATHLIB: &str = "triple = fn(x) => x * 3\npub double = fn(x) => x * 2\n";

#[test]
fn an_unexported_function_called_by_bare_name_says_mark_it_pub() {
    let (_, err, code) = run_with_modules(
        "bare",
        &[("mathlib.ish", MATHLIB)],
        "import \"mathlib.ish\"\ntriple(2)\n",
    );
    assert_eq!(code, 127, "{err}");
    assert!(
        err.contains("`triple` is defined in mathlib.ish but not exported; mark it `pub`"),
        "no pub hint: {err}"
    );
}

#[test]
fn an_unexported_name_read_through_an_alias_says_mark_it_pub() {
    let (_, err, code) = run_with_modules(
        "alias",
        &[("mathlib.ish", MATHLIB)],
        "import \"mathlib.ish\" as m\nm.triple\n",
    );
    assert_ne!(code, 0);
    assert!(
        err.contains("`triple` is defined in mathlib.ish but not exported; mark it `pub`"),
        "no pub hint: {err}"
    );
}

#[test]
fn a_module_that_exports_nothing_says_so_on_a_missing_field() {
    let (_, err, code) = run_with_modules(
        "empty",
        &[("plain.ish", "1 + 1\n")],
        "import \"plain.ish\" as p\np.anything\n",
    );
    assert_ne!(code, 0);
    assert!(
        err.contains("`p` is the module plain.ish, which exports nothing"),
        "no empty-module hint: {err}"
    );
}

#[test]
fn a_pub_name_still_imports_and_runs() {
    let (out, err, code) = run_with_modules(
        "pub",
        &[("mathlib.ish", MATHLIB)],
        "import \"mathlib.ish\"\ndouble(2)\n",
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "4");
}

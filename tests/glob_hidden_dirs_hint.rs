//! `**` does not enter dot-directories; a grep over a glob that matched nothing
//! says so, names the file a dot-directory would have matched, and words the
//! error as a pattern, not as an identifier.

use std::process::Command;

fn run(dir: &std::path::Path, code: &str) -> (String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .args(["--deterministic", "-c", code])
        .output()
        .expect("run ish");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn a_glob_with_no_match_names_the_pattern_and_the_dot_directory() {
    let dir = std::env::temp_dir().join(format!("ish-glob-hidden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
    std::fs::create_dir_all(dir.join("session")).unwrap();
    std::fs::write(dir.join(".github/workflows/ci.yml"), "name: ci\n").unwrap();

    // The default stays shell-like: `**` skips the dot-directory.
    let (out, _) = run(&dir, r#"glob("**/*.yml")"#);
    assert_eq!(out.trim(), "[]", "glob over ** must not enter .github");
    let (out, _) = run(&dir, r#"glob(".github/**/*.yml")"#);
    assert!(out.contains("ci.yml"), "named dot-directory matches: {out}");

    let (out, err) = run(&dir, r#"grep("name", "**/*.yml")"#);
    let all = format!("{out}{err}");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(all.contains("no file matches '**/*.yml'"), "{all}");
    assert!(all.contains("does not enter dot-directories"), "{all}");
    assert!(all.contains("ci.yml"), "the hint names the file: {all}");
    assert!(!all.contains("file matchings"), "{all}");
}

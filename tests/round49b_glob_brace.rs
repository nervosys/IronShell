//! `glob` (and `ls`, `cat`) expand `{a,b}` alternation: `**/*.{rs,py}` used to
//! answer `[]` because the braces were taken literally.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round49b_glob_brace")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj").join("src")).expect("dirs");
    std::fs::create_dir_all(dir.join("session")).expect("session");
    std::fs::write(dir.join("proj").join("a.rs"), "x").expect("a.rs");
    std::fs::write(dir.join("proj").join("b.py"), "y").expect("b.py");
    std::fs::write(dir.join("proj").join("c.txt"), "z").expect("c.txt");
    std::fs::write(dir.join("proj").join("src").join("d.rs"), "w").expect("d.rs");
    dir
}

fn run(dir: &Path, code: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--deterministic", "-c", code])
        .current_dir(dir.join("proj"))
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_AGENT")
        .env_remove("IRONSHELL_WORKSPACE")
        .output()
        .expect("run ish");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn brace_alternation_matches_either_extension() {
    let dir = scratch("ext");
    assert_eq!(
        run(&dir, r#"glob("**/*.{rs,py}")"#),
        // Each alternative's matches in turn, as bash's brace expansion orders them.
        r#"["a.rs","src/d.rs","b.py"]"#
    );
}

#[test]
fn brace_alternatives_come_in_order_and_are_not_repeated() {
    let dir = scratch("dedup");
    assert_eq!(run(&dir, r#"glob("{b,a}.{py,rs}")"#), r#"["b.py","a.rs"]"#);
    assert_eq!(run(&dir, r#"glob("{*.rs,a.*}")"#), r#"["a.rs"]"#);
}

#[test]
fn nested_braces_and_comma_free_groups() {
    let dir = scratch("nested");
    assert_eq!(
        run(&dir, r#"glob("*.{rs,{py,txt}}")"#),
        r#"["a.rs","b.py","c.txt"]"#
    );
}

#[test]
fn ls_takes_braces_too() {
    let dir = scratch("ls");
    let listed = run(&dir, r#"ls("*.{rs,py}") | map(fn(f) => f.name)"#);
    assert!(
        listed.contains("a.rs") && listed.contains("b.py"),
        "{listed}"
    );
    assert!(!listed.contains("c.txt"), "{listed}");
}

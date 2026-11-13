//! `stat` declares that it can return Null, because `stat` of a missing path
//! is Null. The ontology said `Record` only, so an agent that trusted the
//! declared type would treat the Null as a record.

use std::process::Command;

#[test]
fn fs_stat_declares_a_null_return() {
    let sig = iron_shell::signature::signature_of("fs_stat").expect("fs_stat has a signature");
    assert_eq!(sig.returns, "Record | Null");
}

#[test]
fn stat_of_a_missing_path_is_null_and_the_ontology_says_so() {
    let root = std::env::temp_dir().join(format!("ae-stat-type-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("session")).unwrap();
    let run = |expr: &str| -> String {
        let out = Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-c", expr])
            .current_dir(&root)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", root.join("session"))
            .output()
            .expect("run ish");
        assert!(
            out.status.success(),
            "{expr}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim_end().to_string()
    };
    assert_eq!(run(r#"typeof(stat("nope.txt"))"#), "Null");
    let described = run(r#"ontology_describe("stat")"#);
    assert!(
        // Strings inside a record are quoted in `-c` output.
        described.contains("return_type: \"Record | Null\""),
        "ontology said: {described}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

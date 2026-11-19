//! Round 56c: discovery, help and builtins.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ish-r56c-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cwd").join("sub").join("deep")).unwrap();
        std::fs::write(root.join("cwd").join("sub").join("f.txt"), "hi\n").unwrap();
        std::fs::write(
            root.join("cwd").join("sub").join("deep").join("g.rs"),
            "x\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Scratch { root }
    }

    fn ish(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(self.root.join("cwd"))
            .env("NO_COLOR", "1")
            .env_remove("IRONSHELL_ALLOW_SH")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .output()
            .expect("run ish")
    }

    /// Canonical JSON of the value of `expr` (stdout), or the stderr text on failure.
    fn json(&self, expr: &str) -> String {
        let out = self.ish(&["--deterministic", "-c", expr]);
        let text = if out.status.success() {
            String::from_utf8_lossy(&out.stdout).to_string()
        } else {
            String::from_utf8_lossy(&out.stderr).to_string()
        };
        text.trim().to_string()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn c1_find_returns_files_only_in_glob_form() {
    let s = Scratch::new("c1");
    assert_eq!(
        s.json(r#"find("sub") | sort"#),
        r#"["sub/deep/g.rs","sub/f.txt"]"#
    );
    assert_eq!(s.json(r#"find("sub", "*.rs")"#), r#"["sub/deep/g.rs"]"#);
    assert_eq!(s.json(r#"find("sub/") | len"#), "2");
}

#[test]
fn c2_string_hash_help_points_at_the_file_hash() {
    let s = Scratch::new("c2");
    for name in ["sha256", "md5", "crypto_hash"] {
        let text = s.json(&format!(r#"help("{name}")"#));
        assert!(
            text.contains("crypto_hash_file"),
            "help({name}) does not mention crypto_hash_file: {}",
            &text[..text.len().min(400)]
        );
    }
    let wc = s.json(r#"help("wc")"#);
    assert!(
        wc.contains("reads the file") && wc.contains("counts the text"),
        "help(wc) lacks the file/text distinction: {}",
        &wc[..wc.len().min(400)]
    );
}

#[test]
fn c3_schema_for_one_module_has_no_language_wide_header() {
    let s = Scratch::new("c3");
    let out = s.ish(&["agent", "schema", "--module", "file"]);
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "schema --module file failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("\"builtins\""),
        "no builtins in the module schema (len {})",
        text.len()
    );
    for key in ["\"os_ontology\"", "\"cli_tools\"", "\"syntax\"", "\"ops\""] {
        assert!(
            !text.contains(key),
            "module schema still carries {key} (len {})",
            text.len()
        );
    }
    assert!(text.len() < 60_000, "module schema is {} bytes", text.len());
}

#[test]
fn c4_sort_by_field_on_path_strings_points_at_ls() {
    let s = Scratch::new("c4");
    let hint = s.json(r#"try { fs_walk("sub") | sort_by("size") } catch e { e.error.hint }"#);
    assert!(
        hint.contains("fs_walk returns path strings") && hint.contains("ls("),
        "hint was: {}",
        &hint[..hint.len().min(400)]
    );
}

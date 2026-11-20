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

#[test]
fn c5_help_names_ironshell_and_says_what_wc_returns() {
    let s = Scratch::new("c5");
    let out = s.ish(&["-c", "help()"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.starts_with("IronShell core built-ins"),
        "help() opens with: {}",
        &text[..text.len().min(120)]
    );
    assert!(!text.contains("Æther"), "help() still names Æther");
    assert!(
        text.contains("{lines, words, chars}"),
        "help() wc line is stale"
    );
}

#[test]
fn c6_a_lambda_taking_builtin_says_its_effect_is_its_own() {
    let s = Scratch::new("c6");
    let note = s.json(r#"help("each").effect_note"#);
    assert!(note.contains("judged by that call's own effect"), "{note}");
    // Only where a lambda is taken.
    assert_eq!(s.json(r#"has(help("len"), "effect_note")"#), "false");
}

#[test]
fn c7_tool_search_rows_say_how_to_run_the_program() {
    let s = Scratch::new("c7");
    let row = s.json(r#"tool_search("sha256") | where(.name == "sha256sum") | first"#);
    assert!(
        row.contains(r#""builtin":"crypto_hash_file""#),
        "no in-process builtin named: {row}"
    );
    assert!(
        row.contains(r#""run":"tool_exec(\"sha256sum\", [args])""#),
        "no run form: {row}"
    );
}

#[test]
fn c8_archive_arguments_the_wrong_way_round_name_the_order() {
    let s = Scratch::new("c8");
    for (call, form) in [
        (
            r#"zip_create("sub", "out.zip")"#,
            r#"zip_create(\"sub.zip\", \"sub\")"#,
        ),
        (
            r#"tar_create("sub", "out.tar.gz")"#,
            r#"tar_create(\"sub.tar.gz\", \"sub\")"#,
        ),
    ] {
        let err = s.json(&format!("try {{ {call} }} catch e {{ e.error }}"));
        assert!(err.contains("E_BAD_ARG"), "{call}: {err}");
        assert!(err.contains("is a directory"), "{call}: {err}");
        assert!(err.contains(form), "{call}: {err}");
    }
    assert!(!s.root.join("cwd").join("out.zip").exists());
}

#[test]
fn c9_unknown_names_point_at_the_builtin_that_does_the_work() {
    let s = Scratch::new("c9");
    for (name, want) in [
        ("sha256sum", "crypto_hash_file"),
        ("port_scan", "net_scan"),
        ("check_port", "net_scan"),
        ("python", "sh"),
        ("git_init", "sh"),
    ] {
        let got = s.json(&format!(
            r#"try {{ {name}("x") }} catch e {{ e.error.did_you_mean }}"#
        ));
        assert_eq!(got, format!(r#"["{want}"]"#), "{name}");
    }
}

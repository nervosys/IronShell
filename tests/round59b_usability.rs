//! Round 59b: findings from the first-time-agent run.

use std::path::PathBuf;
use std::process::Command;

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ish-r59b-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cwd")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Scratch { root }
    }

    fn json(&self, expr: &str) -> String {
        let out = Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["--deterministic", "-c", expr])
            .current_dir(self.root.join("cwd"))
            .env("NO_COLOR", "1")
            .env_remove("IRONSHELL_ALLOW_SH")
            .env_remove("IRONSHELL_APPROVE")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .output()
            .expect("run ish");
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
fn a1_format_bytes_writes_binary_and_decimal_units() {
    let s = Scratch::new("a1");
    assert_eq!(s.json("format_bytes(0)"), r#""0 B""#);
    assert_eq!(s.json("format_bytes(512)"), r#""512 B""#);
    assert_eq!(s.json("format_bytes(1536)"), r#""1.5 KiB""#);
    assert_eq!(s.json("format_bytes(1500, false)"), r#""1.5 kB""#);
    assert_eq!(s.json("format_bytes(1048576)"), r#""1.0 MiB""#);
    assert_eq!(s.json("1536 | format_bytes"), r#""1.5 KiB""#);
    // Rounding up into the next unit.
    assert_eq!(s.json("format_bytes(1048575)"), r#""1.0 MiB""#);
    assert_eq!(s.json("format_bytes(9223372036854775807)"), r#""8.0 EiB""#);
}

#[test]
fn a1_format_bytes_refuses_bad_input() {
    let s = Scratch::new("a1b");
    for expr in [
        "format_bytes(-1)",
        r#"format_bytes("12")"#,
        "format_bytes(1, 2)",
    ] {
        let text = s.json(&format!("try {{ {expr} }} catch e {{ e.error.code }}"));
        assert_eq!(text, r#""E_BAD_ARG""#, "{expr}: {text}");
    }
}

#[test]
fn a1_other_names_point_to_format_bytes() {
    let s = Scratch::new("a1c");
    for name in [
        "human_size",
        "format_size",
        "filesize",
        "humanize",
        "human_bytes",
    ] {
        let text = s.json(&format!("{name}(1536)"));
        assert!(text.contains("format_bytes"), "{name}: {text}");
    }
    let found = s.json(
        r#"tool_search("human readable file size") | where(.kind == "builtin") | take(5) | map(.name)"#,
    );
    assert!(found.contains("format_bytes"), "{found}");
}

#[test]
fn a2_the_docs_show_the_bearer_token() {
    let root = env!("CARGO_MANIFEST_DIR");
    for file in ["AGENTS.md", "llms.txt", "llms-full.txt"] {
        let text = std::fs::read_to_string(format!("{root}/{file}")).unwrap();
        assert!(text.contains("Bearer"), "{file} omits the bearer token");
        assert!(
            text.contains("IRONSHELL_API_TOKEN"),
            "{file} omits where the token comes from"
        );
    }
}

#[test]
fn a3_compress_a_folder_finds_the_archive_builtins() {
    let s = Scratch::new("a3");
    let names = s.json(
        r#"tool_search("compress a folder") | where(.kind == "builtin") | take(5) | map(.name)"#,
    );
    assert!(names.contains("zip_create"), "{names}");
    assert!(names.contains("tar_create"), "{names}");
    // The single-file compressors refuse a directory, so they rank below.
    let gzip =
        s.json(r#"tool_search("compress a folder") | map(.name) | index_of("gzip_compress")"#);
    let zip = s.json(r#"tool_search("compress a folder") | map(.name) | index_of("zip_create")"#);
    assert!(
        zip.parse::<i64>().unwrap() < gzip.parse::<i64>().unwrap() || gzip == "-1",
        "zip {zip} gzip {gzip}"
    );
}

#[test]
fn a3_delete_a_file_has_no_noisy_external_tail() {
    let s = Scratch::new("a3b");
    let names = s.json(r#"tool_search("delete a file") | map(.name)"#);
    assert!(names.contains(r#""rm""#), "{names}");
    for noise in ["rsync", "ncdu", "exa"] {
        assert!(!names.contains(&format!("\"{noise}\"")), "{names}");
    }
}

#[test]
fn a3_run_text_follows_the_declared_effect() {
    let s = Scratch::new("a3c");
    let rm = s.json(r#"tool_search("delete a file") | where(.name == "rm") | first | .run"#);
    assert!(rm.contains("needs approval in agent mode"), "{rm}");
    let b64 = s.json(r#"tool_search("base64 encode") | where(.kind == "builtin") | first | .run"#);
    assert!(!b64.contains("approval"), "{b64}");
    assert!(b64.contains("call it directly"), "{b64}");
}

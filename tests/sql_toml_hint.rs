//! `sql` on a file it cannot query as a database names the formats it can read.
//! The hint listed `.json/.csv` and left out `.jsonl`, which `sql` reads too.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ae-sql-toml-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cwd")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Scratch { root }
    }

    fn cwd(&self) -> PathBuf {
        self.root.join("cwd")
    }

    fn ish(&self, expr: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-c", expr])
            .current_dir(self.cwd())
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .output()
            .expect("run ish")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn sql_on_a_toml_file_names_jsonl_among_the_readable_formats() {
    let s = Scratch::new("hint");
    std::fs::write(s.cwd().join("cfg.toml"), "name = \"x\"\n").unwrap();
    let out = s.ish(r#"sql_value("cfg.toml", "select count(*) from cfg")"#);
    let t = text(&out);
    assert!(!out.status.success(), "should be refused: {t}");
    assert!(
        t.contains("name a .json, .jsonl, .csv or .tsv file"),
        "the hint does not name .jsonl: {t}"
    );
}

#[test]
fn sql_reads_a_jsonl_file_as_a_table_named_after_it() {
    let s = Scratch::new("jsonl");
    std::fs::write(s.cwd().join("rows.jsonl"), "{\"a\": 1}\n{\"a\": 2}\n").unwrap();
    let out = s.ish(r#"sql_value("rows.jsonl", "select count(*) from rows")"#);
    let t = text(&out);
    assert!(out.status.success(), "jsonl query failed: {t}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "2", "{t}");
}

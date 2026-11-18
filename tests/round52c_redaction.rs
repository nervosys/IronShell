//! Round 52c: gaps in agent-mode secret redaction.
//!
//! A bare `sk-` key, a PEM block with no END line, a value echoed by an error
//! message, and the stderr note, which counts secrets (not values) and is
//! printed on every path that redacts. All keys here are fake.

use std::path::PathBuf;
use std::process::{Command, Output};

const SK: &str = "sk-test-123456789";
const AWS: &str = "AKIAFAKEFAKEFAKE1234";

struct Dirs {
    root: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-r52c-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ws")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Dirs { root }
    }

    fn ws(&self) -> PathBuf {
        self.root.join("ws")
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.ws().join(name), text).unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(self.ws())
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", self.ws())
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .env("IRONSHELL_ALLOW_SH", "")
            .output()
            .expect("run ish")
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn a_bare_sk_key_is_redacted_without_a_name() {
    let d = Dirs::new("sk");
    d.write("s.env", &format!("{SK}\n"));
    let o = d.run(&["-c", r#"(file.read("s.env") | lines)[0]"#]);
    assert!(!out(&o).contains("123456789"), "{}", out(&o));
    assert!(out(&o).contains("[REDACTED]"), "{}", out(&o));
    let o = d.run(&["-c", r#""x sk-proj-abcdefABCDEF123456_-z y""#]);
    assert!(!out(&o).contains("abcdef"), "{}", out(&o));
}

#[test]
fn prose_with_sk_inside_a_word_is_left_alone() {
    let d = Dirs::new("prose");
    let o = d.run(&["-c", r#""the task-force-assessment-plan is ready""#]);
    assert_eq!(out(&o).trim(), "the task-force-assessment-plan is ready");
}

#[test]
fn a_pem_block_with_no_end_line_is_redacted_to_the_end() {
    let d = Dirs::new("pem");
    let o = d.run(&[
        "-c",
        r#""head -----BEGIN RSA PRIVATE KEY-----|MIIEFAKEFAKEFAKE|more""#,
    ]);
    let s = out(&o);
    assert!(!s.contains("MIIEFAKE"), "{s}");
    assert!(s.contains("head"), "{s}");
}

#[test]
fn an_error_does_not_echo_a_secret() {
    let d = Dirs::new("err");
    let o = d.run(&["-c", &format!(r#"int("{SK}")"#)]);
    let e = err(&o);
    assert!(!o.status.success());
    assert!(!e.contains("123456789"), "{e}");
    assert!(e.contains("[REDACTED]"), "{e}");
    assert!(e.contains("1 secret redacted"), "{e}");
}

#[test]
fn the_note_counts_secrets_not_values() {
    let d = Dirs::new("count");
    d.write("four.env", &format!("a={AWS}\nb={SK}\nc={SK}\nd={AWS}\n"));
    let o = d.run(&["-c", r#"file.read("four.env")"#]);
    assert!(err(&o).contains("4 secrets redacted"), "{}", err(&o));
}

#[test]
fn the_note_is_printed_under_deterministic_and_for_posix_output() {
    let d = Dirs::new("paths");
    d.write("s.env", &format!("{SK}\n"));
    let o = d.run(&["--deterministic", "-c", r#"file.read("s.env")"#]);
    assert!(!out(&o).contains("123456789"), "{}", out(&o));
    assert!(err(&o).contains("1 secret redacted"), "{}", err(&o));
    let o = d.run(&["--agent", "-b", "-c", "cat s.env | tr q Q"]);
    assert!(!out(&o).contains("123456789"), "{}", out(&o));
    assert!(err(&o).contains("secret redacted"), "{}", err(&o));
}

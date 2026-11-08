//! A secret-shaped name in a failed call is redacted in the error it returns.
//!
//! `file.read("AWS_SECRET_ACCESS_KEY=AKIA….txt")` answered with the name in
//! its message, its `got` field and nowhere else scrubbed, so the key reached
//! the agent's context. File content was already redacted; error text was not.
//! These drive the real binary in agent mode, the way an agent does.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    workspace: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root =
            std::env::temp_dir().join(format!("ae-error-redaction-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            workspace: root.join("ws"),
            session: root.join("session"),
            root,
        };
        for p in [&d.workspace, &d.session] {
            std::fs::create_dir_all(p).unwrap();
        }
        d
    }

    fn ish(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(&self.workspace)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", &self.workspace)
            .env("IRONSHELL_SESSION_DIR", &self.session)
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

fn all_output(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn a_secret_shaped_file_name_is_not_echoed_in_the_error() {
    let d = Dirs::new("aws");
    let out = d.ish(&[
        "-c",
        r#"file.read("AWS_SECRET_ACCESS_KEY=AKIAFAKEFAKEFAKE1234.txt")"#,
    ]);
    let text = all_output(&out);
    assert!(!out.status.success(), "the read should fail: {text}");
    assert!(
        !text.contains("AKIAFAKEFAKEFAKE1234"),
        "the key came back in the error: {text}"
    );
    assert!(text.contains("[REDACTED]"), "nothing was redacted: {text}");
}

#[test]
fn an_anthropic_key_shaped_file_name_is_not_echoed_in_the_error() {
    let d = Dirs::new("ant");
    let key = "sk-ant-api03-FAKEFAKEFAKEFAKEFAKEFAKE1234";
    let out = d.ish(&["-c", &format!(r#"file.read("{key}.txt")"#)]);
    let text = all_output(&out);
    assert!(!out.status.success(), "the read should fail: {text}");
    assert!(
        !text.contains(key),
        "the key came back in the error: {text}"
    );
}

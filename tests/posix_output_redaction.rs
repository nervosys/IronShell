//! The POSIX subset's output is redacted the way builtin output is.
//!
//! `file.read` redacts a secret in a file's content, but `ish -b -c "cat
//! creds.env"` printed the AWS key and the Anthropic key in the clear: the
//! subset wrote its bytes straight to the terminal. Its stdout and stderr now
//! take the same redaction, and output with no secret in it is unchanged.
//!
//! The key shapes are fake, and the file lives in a temp directory.

use std::path::PathBuf;
use std::process::{Command, Output};

const AWS_KEY: &str = "AKIAFAKEFAKEFAKE1234";
const ANT_KEY: &str = "sk-ant-api03-FAKEFAKEFAKEFAKEFAKEFAKE1234";

struct Dirs {
    root: PathBuf,
    workspace: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root =
            std::env::temp_dir().join(format!("ae-posix-redact-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            workspace: root.join("ws"),
            session: root.join("session"),
            root,
        };
        for p in [&d.workspace, &d.session] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(
            d.workspace.join("creds.env"),
            format!("AWS_ACCESS_KEY_ID={AWS_KEY}\nANTHROPIC={ANT_KEY}\nplain line\n"),
        )
        .unwrap();
        d
    }

    fn sh(&self, script: &str, redact: Option<&str>) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
        cmd.args(["-b", "-c", script])
            .current_dir(&self.workspace)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", &self.workspace)
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env("IRONSHELL_ALLOW_SH", "");
        if let Some(v) = redact {
            cmd.env("IRONSHELL_REDACT", v);
        }
        cmd.output().expect("run ish -b")
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn cat_does_not_print_a_secret_in_the_clear() {
    let d = Dirs::new("cat");
    let out = d.sh("cat creds.env", None);
    let s = stdout(&out);
    assert!(out.status.success(), "{s}");
    assert!(!s.contains(AWS_KEY), "the AWS key was printed: {s}");
    assert!(!s.contains(ANT_KEY), "the Anthropic key was printed: {s}");
    assert!(s.contains("[REDACTED]"), "nothing was redacted: {s}");
    assert!(s.contains("plain line"), "ordinary text was lost: {s}");
}

#[test]
fn grep_does_not_print_a_secret_in_the_clear() {
    let d = Dirs::new("grep");
    let out = d.sh("grep FAKE creds.env", None);
    let s = stdout(&out);
    assert!(!s.contains(AWS_KEY) && !s.contains(ANT_KEY), "{s}");
    assert!(s.contains("[REDACTED]"), "{s}");
}

#[test]
fn an_error_naming_a_secret_shaped_file_is_redacted_too() {
    let d = Dirs::new("err");
    let name = format!("AWS_SECRET_ACCESS_KEY={AWS_KEY}.txt");
    let out = d.sh(&format!("cat {name}"), None);
    let both = format!("{}{}", stdout(&out), String::from_utf8_lossy(&out.stderr));
    assert!(!out.status.success(), "{both}");
    assert!(
        !both.contains(AWS_KEY),
        "the key came back in the error: {both}"
    );
}

#[test]
fn ordinary_output_is_byte_exact() {
    let d = Dirs::new("exact");
    let out = d.sh("echo hello; echo 'two words'; printf 'a\\tb\\n'", None);
    assert_eq!(stdout(&out), "hello\ntwo words\na\tb\n");
}

#[test]
fn redaction_can_be_switched_off_for_trusted_automation() {
    let d = Dirs::new("off");
    let out = d.sh("cat creds.env", Some("off"));
    let s = stdout(&out);
    assert!(
        s.contains(AWS_KEY),
        "the opt-out did not print the file: {s}"
    );
}

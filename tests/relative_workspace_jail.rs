//! A relative `--workspace` (or `IRONSHELL_WORKSPACE`) still jails `..`.
//!
//! With `--workspace proj`, `file_write("../x", ...)` was resolved to the
//! relative `proj/../x`, and the jail joined the root to it a second time,
//! judging `proj/proj/../x` (inside) while the OS wrote `x` (outside). Reads were
//! refused, writes were not. These drive the real binary from the parent of the
//! workspace, as an agent harness would.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-rel-ws-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("proj")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Dirs { root }
    }

    fn ish(&self, args: &[&str], env_ws: Option<&str>) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ish"));
        c.args(args)
            .current_dir(&self.root)
            .env_remove("IRONSHELL_WORKSPACE")
            .env_remove("IRONSHELL_PINNED_ROOT")
            .env_remove("IRONSHELL_AUDIT_LOG")
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .env("IRONSHELL_ALLOW_SH", "");
        if let Some(ws) = env_ws {
            c.env("IRONSHELL_WORKSPACE", ws);
        }
        c.output().expect("run ish")
    }
}

impl Drop for Dirs {
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
fn a_parent_relative_write_is_refused_under_a_relative_workspace_flag() {
    let d = Dirs::new("flag");
    for (i, target) in ["../o1.txt", "sub/../../o2.txt"].iter().enumerate() {
        let src = format!("file_write(\"{target}\", \"x\")");
        let out = d.ish(&["--agent", "--workspace", "proj", "-c", &src], None);
        assert!(!out.status.success(), "case {i}: {}", text(&out));
        assert!(
            text(&out).contains("E_OUTSIDE_WORKSPACE"),
            "case {i}: {}",
            text(&out)
        );
    }
    assert!(!d.root.join("o1.txt").exists(), "o1 was written outside");
    assert!(!d.root.join("o2.txt").exists(), "o2 was written outside");
}

#[test]
fn a_parent_relative_write_is_refused_under_a_relative_workspace_variable() {
    let d = Dirs::new("env");
    let out = d.ish(
        &["--agent", "-c", "file.write(\"../o3.txt\", \"x\")"],
        Some("proj"),
    );
    assert!(!out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("E_OUTSIDE_WORKSPACE"), "{}", text(&out));
    assert!(!d.root.join("o3.txt").exists(), "o3 was written outside");
}

#[test]
fn a_relative_write_inside_a_relative_workspace_lands_in_it() {
    let d = Dirs::new("inside");
    let out = d.ish(
        &[
            "--agent",
            "--workspace",
            "proj",
            "-c",
            "file_write(\"in.txt\", \"x\")",
        ],
        None,
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        std::fs::read_to_string(d.root.join("proj").join("in.txt")).unwrap(),
        "x"
    );
    assert!(!d.root.join("in.txt").exists());
}

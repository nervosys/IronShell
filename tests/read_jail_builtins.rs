//! Read builtins in agent mode refuse paths outside the workspace before they
//! look at the path at all.
//!
//! `file.read("../nope.txt")` answered E_NOT_FOUND while `file.read("../x.txt")`
//! answered E_OUTSIDE_WORKSPACE for an existing file, so the answer told an agent
//! what existed outside the jail. `stat`, `wc`, `file.exists`, the archive readers,
//! `glob` and `ls` read outside too. Each of these must refuse the same way.
//!
//! Layout: `root/ws` is the workspace and the working directory; `root/outside`
//! is a sibling temp directory the agent must not read.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    workspace: PathBuf,
    outside: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-read-jail-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            workspace: root.join("ws"),
            outside: root.join("outside"),
            session: root.join("session"),
            root,
        };
        for p in [&d.workspace, &d.outside, &d.session] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(d.workspace.join("inside.txt"), "inside\n").unwrap();
        std::fs::write(d.outside.join("outside.txt"), "SECRET-OUTSIDE\n").unwrap();
        std::fs::write(d.outside.join("outside.zip"), b"not really a zip").unwrap();
        d
    }

    fn ish(&self, expr: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-c", expr])
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

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The call is refused as outside the workspace, and no outside content shows.
fn refused_as_outside(d: &Dirs, expr: &str) {
    let out = d.ish(expr);
    let t = text(&out);
    assert!(!out.status.success(), "{expr} should fail: {t}");
    assert!(
        t.contains("E_OUTSIDE_WORKSPACE"),
        "{expr} was not refused as outside the workspace: {t}"
    );
    assert!(!t.contains("SECRET-OUTSIDE"), "{expr} leaked content: {t}");
}

#[test]
fn a_missing_outside_file_is_outside_not_not_found() {
    let d = Dirs::new("order");
    refused_as_outside(&d, r#"file.read("../outside/nope.txt")"#);
    refused_as_outside(&d, r#"file.read("../nodir/nope.txt")"#);
}

#[test]
fn an_existing_outside_file_is_outside_too() {
    let d = Dirs::new("read");
    refused_as_outside(&d, r#"file.read("../outside/outside.txt")"#);
    refused_as_outside(&d, r#"cat("../outside/outside.txt")"#);
    refused_as_outside(&d, r#"read_text("../outside/outside.txt")"#);
}

#[test]
fn stat_wc_and_exists_refuse_outside_paths() {
    let d = Dirs::new("stat");
    refused_as_outside(&d, r#"stat("../outside/outside.txt")"#);
    refused_as_outside(&d, r#"stat("../outside/nope.txt")"#);
    refused_as_outside(&d, r#"wc("../outside/outside.txt")"#);
    refused_as_outside(&d, r#"file.exists("../outside/outside.txt")"#);
    refused_as_outside(&d, r#"file.exists("../outside/nope.txt")"#);
}

#[test]
fn archive_readers_refuse_outside_paths() {
    let d = Dirs::new("archive");
    refused_as_outside(&d, r#"zip_list("../outside/outside.zip")"#);
    refused_as_outside(&d, r#"archive_info("../outside/outside.zip")"#);
}

#[test]
fn glob_and_ls_refuse_a_base_outside_the_workspace() {
    let d = Dirs::new("glob");
    refused_as_outside(&d, r#"glob("../*")"#);
    // The sibling's own file name must not come back in the answer either.
    let out = d.ish(r#"glob("../*")"#);
    assert!(!text(&out).contains("outside.txt"), "{}", text(&out));
    refused_as_outside(&d, r#"glob("../outside/*.txt")"#);
    refused_as_outside(&d, r#"ls("../outside")"#);
    let abs = d.outside.join("*.txt");
    let expr = format!("glob(\"{}\")", abs.to_string_lossy().replace('\\', "/"));
    refused_as_outside(&d, &expr);
}

#[test]
fn inside_reads_are_unchanged() {
    let d = Dirs::new("inside");
    let out = d.ish(r#"file.read("inside.txt")"#);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("inside"), "{}", text(&out));
    let out = d.ish(r#"glob("*.txt")"#);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("inside.txt"), "{}", text(&out));
}

//! The POSIX subset's readers refuse paths outside the workspace in agent mode.
//!
//! `ish -b -c "cat ../outside.txt"` printed the file with exit 0 while the
//! ish builtin `file.read` refused the same path with E_OUTSIDE_WORKSPACE. The
//! utilities that read or list a path (cat, head, tail, wc, grep, sort, uniq,
//! cut, sed, ls, find, `<`, test -e and friends, and glob expansion) now apply
//! the same jail, and they refuse before they look at the path.
//!
//! Layout: `root/ws` is the workspace and the working directory; `root/outside`
//! is a sibling temp directory the agent must not read.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    workspace: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-posix-read-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            workspace: root.join("ws"),
            session: root.join("session"),
            root,
        };
        std::fs::create_dir_all(d.workspace.join("sub")).unwrap();
        std::fs::create_dir_all(d.root.join("outside")).unwrap();
        std::fs::create_dir_all(&d.session).unwrap();
        std::fs::write(d.workspace.join("inside.txt"), "inside\n").unwrap();
        std::fs::write(d.workspace.join("sub").join("deep.txt"), "deep\n").unwrap();
        std::fs::write(
            d.root.join("outside").join("outside.txt"),
            "SECRET-OUTSIDE\n",
        )
        .unwrap();
        d
    }

    /// `ish -b -c <script>` in agent mode, from the workspace.
    fn sh(&self, script: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-b", "-c", script])
            .current_dir(&self.workspace)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", &self.workspace)
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env("IRONSHELL_ALLOW_SH", "")
            .output()
            .expect("run ish -b")
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

/// The utility refuses: nonzero status, E_OUTSIDE_WORKSPACE, and no outside content.
fn refused(d: &Dirs, script: &str) {
    let out = d.sh(script);
    let t = text(&out);
    assert!(!out.status.success(), "`{script}` should fail: {t}");
    assert!(
        t.contains("E_OUTSIDE_WORKSPACE"),
        "`{script}` was not refused as outside the workspace: {t}"
    );
    assert!(
        !t.contains("SECRET-OUTSIDE"),
        "`{script}` leaked content: {t}"
    );
}

#[test]
fn content_readers_refuse_an_outside_file() {
    let d = Dirs::new("content");
    for script in [
        "cat ../outside/outside.txt",
        "cat -n ../outside/outside.txt",
        "head -n 1 ../outside/outside.txt",
        "tail -n 1 ../outside/outside.txt",
        "wc -l ../outside/outside.txt",
        "sort ../outside/outside.txt",
        "uniq ../outside/outside.txt",
        "cut -c1-3 ../outside/outside.txt",
        "sed p ../outside/outside.txt",
        "grep SECRET ../outside/outside.txt",
        "cat < ../outside/outside.txt",
    ] {
        refused(&d, script);
    }
}

#[test]
fn a_missing_outside_file_is_outside_not_not_found() {
    let d = Dirs::new("missing");
    for script in ["cat ../outside/nope.txt", "head ../nodir/nope.txt"] {
        let out = d.sh(script);
        let t = text(&out);
        assert!(t.contains("E_OUTSIDE_WORKSPACE"), "`{script}`: {t}");
        assert!(
            !t.contains("E_NOT_FOUND"),
            "`{script}` is an existence oracle: {t}"
        );
    }
}

#[test]
fn recursive_grep_does_not_walk_outside_the_workspace() {
    let d = Dirs::new("grep-r");
    refused(&d, "grep -r SECRET ..");
    refused(&d, "grep -r SECRET ../outside");
    // With no operand, the search starts where the shell is, which is inside.
    let out = d.sh("grep -r deep .");
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("deep"), "{}", text(&out));
}

#[test]
fn directory_listers_refuse_an_outside_directory() {
    let d = Dirs::new("ls");
    for script in ["ls ..", "ls -a ../outside", "find .. -name outside.txt"] {
        let out = d.sh(script);
        let t = text(&out);
        assert!(!out.status.success(), "`{script}` should fail: {t}");
        assert!(t.contains("E_OUTSIDE_WORKSPACE"), "`{script}`: {t}");
        assert!(
            !t.contains("outside.txt"),
            "`{script}` listed the outside file: {t}"
        );
    }
}

#[test]
fn test_file_operators_refuse_an_outside_path() {
    let d = Dirs::new("test");
    let out = d.sh("test -e ../outside/outside.txt; echo $?");
    let t = text(&out);
    assert!(t.contains("E_OUTSIDE_WORKSPACE"), "{t}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).lines().last(),
        Some("2"),
        "the test answered rather than refused: {t}"
    );
    let out = d.sh("test -f ../outside/nope.txt; echo $?");
    let t = text(&out);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).lines().last(),
        Some("2"),
        "{t}"
    );
    assert!(!t.contains("E_NOT_FOUND"), "{t}");
}

#[test]
fn glob_expansion_does_not_list_outside_the_workspace() {
    let d = Dirs::new("glob");
    let out = d.sh("echo ../outside/* ../*");
    let t = text(&out);
    assert!(
        !t.contains("outside.txt"),
        "the glob listed the outside file: {t}"
    );
    assert!(!t.contains("outside/outside"), "{t}");
    let out = d.sh("cat ../outside/*.txt");
    let t = text(&out);
    assert!(!out.status.success(), "{t}");
    assert!(!t.contains("SECRET-OUTSIDE"), "{t}");
    let out = d.sh("echo sub/*");
    assert!(text(&out).contains("sub/deep.txt"), "{}", text(&out));
}

#[test]
fn inside_reads_still_work() {
    let d = Dirs::new("inside");
    let out = d.sh("cat inside.txt; head -n 1 sub/deep.txt; grep deep sub/deep.txt; test -f inside.txt && echo yes");
    let t = text(&out);
    assert!(out.status.success(), "{t}");
    assert!(
        t.contains("inside") && t.contains("deep") && t.contains("yes"),
        "{t}"
    );
}

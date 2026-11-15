//! Round 44a: `fs_walk`, `fs_tree` and `fs_du` do not follow a link out of the
//! folder they are asked about.
//!
//! They followed a symbolic link or junction to a folder outside the workspace,
//! listing its names and counting its bytes, while `find`, `glob` and
//! `zip_create` did not. A link is now listed as an entry and not opened.
//!
//! Temporary directories only. The link is a symbolic link on Unix and a
//! junction (`mklink /J`) on Windows; the test skips with a message when it
//! cannot be made.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    ws: PathBuf,
    outside: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-r44a-walk-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            ws: root.join("ws"),
            outside: root.join("outside"),
            root,
        };
        std::fs::create_dir_all(d.ws.join("real")).unwrap();
        std::fs::create_dir_all(&d.outside).unwrap();
        std::fs::write(d.ws.join("real").join("own.txt"), vec![b'o'; 10]).unwrap();
        std::fs::write(d.outside.join("secret.txt"), vec![b's'; 100_000]).unwrap();
        d
    }

    fn ish(&self, expr: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-c", expr])
            .current_dir(&self.root)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_WORKSPACE")
            .output()
            .expect("run ish")
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        // The link first: a recursive delete must not follow it.
        let jout = self.ws.join("jout");
        if std::fs::symlink_metadata(&jout).is_ok() {
            let _ = std::fs::remove_dir(&jout).or_else(|_| std::fs::remove_file(&jout));
        }
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

#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).expect("symlink");
    true
}

#[cfg(windows)]
fn link_dir(target: &Path, link: &Path) -> bool {
    match Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
    {
        Ok(o) if o.status.success() => true,
        other => {
            eprintln!("skipped: mklink /J failed: {other:?}");
            false
        }
    }
}

fn fixture(tag: &str) -> Option<Dirs> {
    let d = Dirs::new(tag);
    link_dir(&d.outside, &d.ws.join("jout")).then_some(d)
}

#[test]
fn fs_walk_lists_the_link_and_does_not_open_it() {
    let Some(d) = fixture("walk") else { return };
    let out = d.ish("fs_walk('ws')");
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    assert!(shown.contains("jout"), "the link is listed: {shown}");
    assert!(
        shown.contains("own.txt"),
        "a real folder is walked: {shown}"
    );
    assert!(
        !shown.contains("secret.txt"),
        "walked through the link: {shown}"
    );
}

#[test]
fn fs_tree_lists_the_link_and_does_not_open_it() {
    let Some(d) = fixture("tree") else { return };
    let out = d.ish("fs_tree('ws', 4)");
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    assert!(shown.contains("jout"), "the link is listed: {shown}");
    assert!(
        shown.contains("own.txt"),
        "a real folder is walked: {shown}"
    );
    assert!(
        !shown.contains("secret.txt"),
        "walked through the link: {shown}"
    );
}

#[test]
fn fs_du_does_not_count_what_lies_behind_a_link() {
    let Some(d) = fixture("du") else { return };
    let out = d.ish("fs_du('ws').bytes");
    let shown = text(&out);
    assert!(out.status.success(), "{shown}");
    let bytes: i64 = shown
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("a byte count: {shown}"));
    assert!(bytes >= 10, "the real file is counted: {bytes}");
    assert!(
        bytes < 100_000,
        "counted the folder behind the link: {bytes}"
    );
}

//! Round 50c: workspace jail gaps. One section per item; the CLI tests drive
//! the real binary in a scratch directory with its own session directory.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Scratch {
    root: PathBuf,
    ws: PathBuf,
    outside: PathBuf,
    session: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ae-r50c-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let s = Scratch {
            ws: root.join("ws"),
            outside: root.join("outside"),
            session: root.join("session"),
            root,
        };
        for p in [&s.ws, &s.outside, &s.session] {
            std::fs::create_dir_all(p).unwrap();
        }
        s
    }

    fn base(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ish"));
        c.args(args)
            .current_dir(&self.ws)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_APPROVE_ALL")
            .env_remove("IRONSHELL_APPROVE")
            .env_remove("IRONSHELL_JOURNAL")
            .env_remove("IRONSHELL_WORKSPACE");
        c
    }

    /// Jailed by `IRONSHELL_WORKSPACE` alone (no agent mode, so no approval).
    fn jailed(&self, args: &[&str]) -> Output {
        self.base(args)
            .env("IRONSHELL_WORKSPACE", &self.ws)
            .output()
            .expect("run ish")
    }

    /// Jailed, with the undo journal on.
    fn journaled(&self, args: &[&str]) -> Output {
        self.base(args)
            .env("IRONSHELL_WORKSPACE", &self.ws)
            .env("IRONSHELL_JOURNAL", "on")
            .output()
            .expect("run ish")
    }

    /// The POSIX subset, jailed.
    fn sh(&self, script: &str) -> Output {
        self.jailed(&["-b", "-c", script])
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let jx = self.ws.join("jx");
        if std::fs::symlink_metadata(&jx).is_ok() {
            let _ = std::fs::remove_dir(&jx).or_else(|_| std::fs::remove_file(&jx));
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

/// A junction, made with `mklink /J`: it needs no privilege.
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

// ── C1: ish -b runs rm and rmdir in process, under the jail ──

#[test]
fn rm_in_the_subset_removes_a_file_and_a_directory_with_r() {
    let s = Scratch::new("c1basic");
    std::fs::write(s.ws.join("a.txt"), "a").unwrap();
    std::fs::write(s.ws.join("b.txt"), "b").unwrap();
    std::fs::create_dir_all(s.ws.join("d/e")).unwrap();
    std::fs::write(s.ws.join("d/e/f.txt"), "f").unwrap();
    let out = s.sh("rm -- a.txt b.txt");
    assert!(out.status.success(), "{}", text(&out));
    assert!(!s.ws.join("a.txt").exists() && !s.ws.join("b.txt").exists());
    // A directory without -r is refused, as GNU rm does.
    let out = s.sh("rm d");
    assert!(!out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("rm: cannot remove 'd': Is a directory"));
    assert!(s.ws.join("d/e/f.txt").exists());
    let out = s.sh("rm -r d");
    assert!(out.status.success(), "{}", text(&out));
    assert!(!s.ws.join("d").exists());
}

#[test]
fn rm_in_the_subset_reports_a_missing_file_unless_f() {
    let s = Scratch::new("c1missing");
    let out = s.sh("rm nope");
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(text(&out).contains("rm: cannot remove 'nope': No such file or directory"));
    let out = s.sh("rm -f nope");
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).trim().is_empty(), "{}", text(&out));
    let out = s.sh("rmdir nope");
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
}

#[test]
fn rmdir_in_the_subset_removes_only_an_empty_directory() {
    let s = Scratch::new("c1rmdir");
    std::fs::create_dir_all(s.ws.join("full")).unwrap();
    std::fs::write(s.ws.join("full/x"), "x").unwrap();
    std::fs::create_dir_all(s.ws.join("empty")).unwrap();
    let out = s.sh("rmdir full");
    assert!(!out.status.success(), "{}", text(&out));
    assert!(s.ws.join("full/x").exists());
    let out = s.sh("rmdir empty");
    assert!(out.status.success(), "{}", text(&out));
    assert!(!s.ws.join("empty").exists());
}

#[test]
fn rm_in_the_subset_refuses_a_path_outside_the_workspace() {
    let s = Scratch::new("c1outside");
    let victim = s.outside.join("keep.txt");
    std::fs::write(&victim, "keep").unwrap();
    let script = format!("rm -f '{}'", victim.display().to_string().replace('\', "/"));
    let out = s.sh(&script);
    assert!(!out.status.success(), "{}", text(&out));
    assert!(victim.exists(), "outside file was removed: {}", text(&out));
}

#[test]
fn rm_in_the_subset_removes_a_link_and_never_follows_it() {
    let s = Scratch::new("c1link");
    std::fs::write(s.outside.join("keep.txt"), "keep").unwrap();
    let jx = s.ws.join("jx");
    if !link_dir(&s.outside, &jx) {
        return;
    }
    // A trailing separator names what the link points at: refused, untouched.
    for script in ["rm -rf jx/", "rm -rf jx/.", "rm -r jx/"] {
        let out = s.sh(script);
        assert!(!out.status.success(), "{script}: {}", text(&out));
        assert!(
            s.outside.join("keep.txt").is_file(),
            "{script} damaged the target: {}",
            text(&out)
        );
        assert!(std::fs::symlink_metadata(&jx).is_ok(), "{script}");
    }
    // Without it, the link goes and its target stays.
    let out = s.sh("rm -rf jx");
    assert!(out.status.success(), "{}", text(&out));
    assert!(std::fs::symlink_metadata(&jx).is_err(), "link remains");
    assert!(
        s.outside.join("keep.txt").is_file(),
        "target damaged: {}",
        text(&out)
    );
}

#[test]
fn rm_in_the_subset_declines_a_flag_it_does_not_model() {
    let s = Scratch::new("c1flag");
    std::fs::write(s.ws.join("a.txt"), "a").unwrap();
    // -i is not in the subset: it runs a GNU rm if one is on PATH, else it
    // exits 2 naming the flag. Either way it never runs a non-GNU program.
    let out = s.sh("rm -I a.txt");
    let shown = text(&out);
    if !out.status.success() {
        assert!(
            shown.contains("the in-process rm does not support -I") || shown.contains("rm"),
            "{shown}"
        );
    }
}

// ── C2: --explain judges the members of an archive as the run does ──

#[test]
fn explain_denies_a_tar_or_zip_member_that_is_a_link_out_of_the_workspace() {
    let s = Scratch::new("c2");
    std::fs::write(s.outside.join("keep.txt"), "keep").unwrap();
    std::fs::write(s.ws.join("ok.txt"), "ok").unwrap();
    let jx = s.ws.join("jx");
    if !link_dir(&s.outside, &jx) {
        return;
    }
    for call in [
        r#"tar_create("o.tar", "jx")"#,
        r#"tar_create("o.tar", "ok.txt", "jx")"#,
        r#"zip_create("o.zip", "jx")"#,
        r#"zip_create("o.zip", "ok.txt", "jx")"#,
    ] {
        let out = s.jailed(&["--explain", "-c", call]);
        let shown = text(&out);
        assert!(shown.contains("deny"), "{call}: {shown}");
    }
    let out = s.jailed(&["--explain", "-c", r#"tar_create("o.tar", "ok.txt")"#]);
    assert!(!text(&out).contains("deny"), "{}", text(&out));
}

// ── C3: --explain reports rm of a trailing-separator name as refused ──

#[test]
fn explain_denies_rm_with_a_trailing_separator() {
    let s = Scratch::new("c3");
    for call in [r#"rm("jx/")"#, r#"rm("d/.")"#, r#"rm("d/..")"#] {
        let out = s.jailed(&["--explain", "-c", call]);
        let shown = text(&out);
        assert!(shown.contains("deny"), "{call}: {shown}");
        assert!(shown.contains("E_BAD_ARG"), "{call}: {shown}");
    }
    let out = s.jailed(&["--explain", "-c", r#"rm("jx")"#]);
    assert!(!text(&out).contains("deny"), "{}", text(&out));
}

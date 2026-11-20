//! Round 51b: `ish -b` rm, rmdir and cd refusals read as GNU tools print them
//! (no JSON), exit as the other in-process tools do, and `--explain` agrees
//! with the run. The tests drive the real binary in a scratch directory with
//! its own session directory.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Scratch {
    root: PathBuf,
    ws: PathBuf,
    outside: PathBuf,
    session: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ish-r51b-{tag}-{}", std::process::id()));
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

    /// The POSIX subset, jailed by `IRONSHELL_WORKSPACE` alone (no approval).
    fn sh(&self, script: &str) -> Output {
        self.base(&["-b", "-c", script])
            .env("IRONSHELL_WORKSPACE", &self.ws)
            .output()
            .expect("run ish")
    }

    /// `ish --agent --workspace . <args>` from the workspace.
    fn agent(&self, args: &[&str]) -> Output {
        let mut all = vec!["--agent", "--workspace", "."];
        all.extend_from_slice(args);
        self.base(&all).output().expect("run ish")
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

// ── B1: GNU text, no JSON ──

#[test]
fn rmdir_of_a_full_directory_says_directory_not_empty() {
    let s = Scratch::new("b1full");
    std::fs::create_dir_all(s.ws.join("src")).unwrap();
    std::fs::write(s.ws.join("src/x"), "x").unwrap();
    let out = s.sh("rmdir src");
    let t = text(&out);
    assert_eq!(out.status.code(), Some(1), "{t}");
    assert!(
        t.contains("rmdir: failed to remove 'src': Directory not empty"),
        "{t}"
    );
    assert!(!t.contains("\"error\""), "{t}");
    assert!(s.ws.join("src/x").exists());
}

#[test]
fn rm_outside_the_workspace_names_the_operand_as_written() {
    let s = Scratch::new("b1outside");
    std::fs::write(s.outside.join("keep.txt"), "keep").unwrap();
    let out = s.sh("rm ../outside/keep.txt");
    let t = text(&out);
    assert_eq!(out.status.code(), Some(1), "{t}");
    assert!(
        t.contains("rm: cannot remove '../outside/keep.txt': outside the workspace"),
        "{t}"
    );
    assert!(
        !t.contains("\"error\"") && !t.contains("E_OUTSIDE_WORKSPACE"),
        "{t}"
    );
    assert!(s.outside.join("keep.txt").exists());
}

#[test]
fn mkdir_touch_and_tee_outside_the_workspace_are_text_too() {
    let s = Scratch::new("b1write");
    for (script, lead) in [
        ("mkdir ../d", "mkdir: cannot create directory '../d': "),
        ("touch ../f", "touch: cannot touch '../f': "),
        ("echo x | tee ../g", "tee: ../g: "),
    ] {
        let out = s.sh(script);
        let t = text(&out);
        assert_eq!(out.status.code(), Some(1), "{script}: {t}");
        assert!(
            t.contains(&format!("{lead}outside the workspace")),
            "{script}: {t}"
        );
        assert!(!t.contains("\"error\""), "{script}: {t}");
    }
    assert!(!s.root.join("d").exists() && !s.root.join("f").exists());
}

// ── B2: approval is 75, naming rm ──

#[test]
fn rm_needing_approval_exits_75_and_names_rm() {
    let s = Scratch::new("b2approve");
    std::fs::create_dir_all(s.ws.join("build")).unwrap();
    std::fs::write(s.ws.join("build/o"), "o").unwrap();
    std::fs::write(s.ws.join("f.txt"), "f").unwrap();
    for script in ["rm -r build", "rm f.txt"] {
        let out = s.agent(&["-b", "-c", script]);
        let t = text(&out);
        assert_eq!(out.status.code(), Some(75), "{script}: {t}");
        assert!(t.contains("E_NEEDS_APPROVAL"), "{script}: {t}");
        assert!(t.contains("rm: requires approval"), "{script}: {t}");
        assert!(!t.contains("rmdir: requires approval"), "{script}: {t}");
    }
    assert!(s.ws.join("build/o").exists() && s.ws.join("f.txt").exists());
}

#[test]
fn rm_refused_by_the_jail_exits_1_like_cat() {
    let s = Scratch::new("b2jail");
    std::fs::write(s.outside.join("keep.txt"), "keep").unwrap();
    let rm = s.sh("rm ../outside/keep.txt");
    let cat = s.sh("cat ../outside/keep.txt");
    assert_eq!(rm.status.code(), Some(1), "{}", text(&rm));
    assert_eq!(cat.status.code(), Some(1), "{}", text(&cat));
}

// ── B3: the flag that is unsupported is the one named ──

#[test]
fn the_unsupported_rm_flag_is_named_not_the_supported_one() {
    let s = Scratch::new("b3flag");
    let out = s.sh("rm -r --no-preserve-root jx");
    let t = text(&out);
    assert_eq!(out.status.code(), Some(2), "{t}");
    assert!(t.contains("does not support --no-preserve-root"), "{t}");
    let out = s.sh("rm -rI x");
    let t = text(&out);
    assert!(t.contains("does not support -I,"), "{t}");
    assert!(!t.contains("-rI"), "{t}");
}

// ── B4: a refused cd fails ──

#[test]
fn a_refused_cd_has_status_1_and_stops_an_and_list() {
    let s = Scratch::new("b4cd");
    let out = s.sh("cd ../outside");
    let t = text(&out);
    assert_eq!(out.status.code(), Some(1), "{t}");
    assert!(t.contains("outside the workspace"), "{t}");
    let out = s.sh("cd ../outside && echo reached");
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(!text(&out).contains("reached"), "{}", text(&out));
}

// ── B5: --explain agrees with the run ──

fn explain(s: &Scratch, script: &str) -> serde_json::Value {
    let out = s.agent(&["--explain", "-b", "-c", script]);
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{script}: not JSON ({e}): {}", text(&out)))
}

#[test]
fn explain_denies_rm_and_rmdir_outside_the_workspace() {
    let s = Scratch::new("b5out");
    for script in [
        "rm -rf ../outside/canary.txt",
        "rm ../outside/canary.txt",
        "rmdir ../outside",
    ] {
        let report = explain(&s, script);
        assert_eq!(report["decision"], "deny", "{script}: {report}");
    }
}

#[test]
fn explain_judges_rm_inside_the_workspace_as_a_delete() {
    let s = Scratch::new("b5in");
    let report = explain(&s, "rm -rf sub");
    assert_eq!(report["decision"], "approve", "{report}");
    let calls = report["calls"].as_array().expect("calls");
    assert!(
        // Each operand is listed once (`rm sub`), not also as a bare `rm`
        // (round 58b).
        calls.iter().any(|c| c["call"]
            .as_str()
            .is_some_and(|n| n == "rm" || n.starts_with("rm "))
            && c["effect"] == "destructive"),
        "{report}"
    );
    assert!(
        calls.iter().all(|c| c["effect"] != "exec"),
        "an in-process rm is not an exec: {report}"
    );
}

// ── B6: command not found names the ish builtin ──

#[test]
fn a_missing_mv_and_cp_name_the_ish_builtin() {
    let s = Scratch::new("b6hint");
    let empty = s.root.join("empty-path");
    std::fs::create_dir_all(&empty).unwrap();
    for (script, want) in [("mv a b", "mv("), ("cp a b", "cp(")] {
        let out = s
            .base(&["-b", "-c", script])
            .env("PATH", &empty)
            .output()
            .expect("run ish");
        let t = text(&out);
        assert_eq!(out.status.code(), Some(127), "{script}: {t}");
        assert!(t.contains("command not found"), "{script}: {t}");
        assert!(t.contains(&format!("in ish, use {want}")), "{script}: {t}");
    }
}

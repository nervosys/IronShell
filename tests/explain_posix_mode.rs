//! `--explain -b` predicts the POSIX subset's run.
//!
//! `ish --explain -b -c "git --version"` answered decision "allow" with no calls,
//! while the run in agent mode is refused with E_NEEDS_APPROVAL: the explanation
//! read the source as IronShell, which saw no call at all. With `-b` the source
//! is explained as the subset runs it, so the decision matches the run.
//!
//! The workspace and the working directory are temp directories.

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
            std::env::temp_dir().join(format!("ae-explain-posix-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            workspace: root.join("ws"),
            session: root.join("session"),
            root,
        };
        for p in [&d.workspace, &d.session] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(d.workspace.join("inside.txt"), "inside\n").unwrap();
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

    /// The `--explain -b` report for `script`, parsed.
    fn explain(&self, script: &str) -> serde_json::Value {
        let out = self.ish(&["--explain", "-b", "-c", script]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).expect("explain prints JSON")
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_program_is_an_exec_that_agent_mode_approves_as_the_run_does() {
    let d = Dirs::new("exec");
    let report = d.explain("git --version");
    assert_eq!(report["decision"], "approve", "{report}");
    let calls = report["calls"].as_array().expect("calls");
    assert_eq!(calls.len(), 1, "{report}");
    assert_eq!(calls[0]["call"], "git");
    assert_eq!(calls[0]["effect"], "exec");

    // The run gets the same answer: refused for approval, before anything runs.
    let run = d.ish(&["-b", "-c", "git --version"]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(!run.status.success(), "{text}");
    assert!(
        text.contains("E_NEEDS_APPROVAL"),
        "the run was not refused for approval: {text}"
    );
}

#[test]
fn an_in_process_read_is_read_local_and_allowed_as_the_run_allows_it() {
    let d = Dirs::new("read");
    let report = d.explain("cat inside.txt | wc -l");
    assert_eq!(report["decision"], "allow", "{report}");
    let effects: Vec<&str> = report["effects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(effects.contains(&"read_local"), "{report}");

    let run = d.ish(&["-b", "-c", "cat inside.txt | wc -l"]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn a_mix_takes_the_strictest_decision() {
    let d = Dirs::new("mix");
    let report = d.explain("cat inside.txt\ngit status");
    assert_eq!(report["decision"], "approve", "{report}");
}

//! Round 23: the POSIX subset (`ish -b`) and its refusals and expansions.
//!
//! Each case runs the binary with `-b -c` in a fresh temp folder, with
//! IRONSHELL_SESSION_DIR set to a folder of its own.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("ae-round23-posix-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("session")).unwrap();
        std::fs::create_dir_all(root.join("empty-path")).unwrap();
        Sandbox(root)
    }

    fn run_args(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(&self.0)
            .env("NO_COLOR", "1")
            .env("PATH", self.0.join("empty-path"))
            .env("IRONSHELL_SESSION_DIR", self.0.join("session"))
            .output()
            .expect("run ish")
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let out = self.run_args(&["-b", "-c", script]);
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ── 1. `$((` inside a command substitution is refused up front ───────────

#[test]
fn arithmetic_inside_a_command_substitution_refuses_the_whole_script() {
    let sb = Sandbox::new("subst-arith");
    // Each `$((` is named at its own column in the outer source, and nothing
    // runs: the `echo` before it is not printed.
    let cases = [
        ("echo $(echo $((1+1)))", "line 1, column 13"),
        ("echo a\nx=$(echo $((1+1)))\n", "line 2, column 10"),
        ("echo `echo $((1+1))`", "line 1, column 12"),
        ("echo \"$(echo $((2)))\"", "line 1, column 14"),
    ];
    for (script, place) in cases {
        let (out, err, code) = sb.run(script);
        assert_eq!(out, "", "{script}: {err}");
        assert_eq!(code, 77, "{script}: {err}");
        assert!(
            err.contains(&format!(
                "arithmetic expansion $((...)) at {place} is not in the in-process POSIX subset"
            )),
            "{script}: {err}"
        );
        assert!(err.contains("E_POLICY_DENY"), "{script}: {err}");
    }
}

#[test]
fn a_refusal_inside_a_substitution_body_is_reported_up_front() {
    let sb = Sandbox::new("subst-function");
    // A function definition inside `$(...)` is refused with the script, at its
    // own place in the outer source; the `echo` before it does not run.
    let (out, err, code) = sb.run("echo first\nx=$(f() { echo hi; })\n");
    assert_eq!((out.as_str(), code), ("", 77), "{err}");
    assert!(
        err.contains("function definition at line 2, column 5"),
        "{err}"
    );
}

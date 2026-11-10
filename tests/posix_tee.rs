//! `tee` in `ish -b` is in-process, and its files are judged like a `>` redirection.
//!
//! `tee` was not in the subset, so `echo x | tee ../f` ran the real tee program
//! through the effect gate. An approved external program is not jailed, so with
//! a token the write landed outside the workspace. The subset now runs `tee`
//! itself, and each file it names goes through the guard a redirection passes.
//! These drive the real binary in agent mode, in a scratch workspace.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    workspace: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("posix_tee")
            .join(tag);
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            workspace: root.join("ws"),
            session: root.join("session"),
            root,
        };
        std::fs::create_dir_all(&d.workspace).unwrap();
        std::fs::create_dir_all(&d.session).unwrap();
        d
    }

    /// Runs `ish -b -c <script>` in agent mode, from inside the workspace.
    fn bash(&self, script: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-b", "-c", script])
            .current_dir(&self.workspace)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", &self.workspace)
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env_remove("IRONSHELL_APPROVE")
            .env_remove("IRONSHELL_APPROVE_ALL")
            .output()
            .expect("run ish")
    }

    fn outside(&self) -> PathBuf {
        self.root.join("f")
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn tee_writes_its_file_inside_the_workspace_and_passes_stdin_through() {
    let d = Dirs::new("write");
    let out = d.bash("printf 'a\\nb\\n' | tee out.txt\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out), "a\nb\n", "stdin must pass through to stdout");
    assert_eq!(
        std::fs::read_to_string(d.workspace.join("out.txt")).unwrap(),
        "a\nb\n"
    );
}

#[test]
fn tee_dash_a_appends() {
    let d = Dirs::new("append");
    let out = d.bash("echo one > log.txt\necho two | tee -a log.txt\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        std::fs::read_to_string(d.workspace.join("log.txt")).unwrap(),
        "one\ntwo\n"
    );
}

#[test]
fn tee_to_a_file_outside_the_workspace_is_refused_and_writes_nothing() {
    let d = Dirs::new("outside");
    let out = d.bash("echo x | tee ../f; echo $?\n");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        err.contains("outside the workspace"),
        "the refusal must name the jail: {err}"
    );
    assert!(!d.outside().exists(), "a refused tee must write nothing");
    assert!(
        !err.contains("IRONSHELL_APPROVE") && !err.contains("apv_"),
        "an outside target must never yield a token: {err}"
    );
    assert_eq!(
        stdout(&out).lines().last(),
        Some("1"),
        "tee exits 1 when it refuses a file: {out:?}"
    );
}

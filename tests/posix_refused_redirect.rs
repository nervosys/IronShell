//! A redirection the jail refuses is a failed redirection, as in bash.
//!
//! The output file was judged only after the command had run, so `echo x > ../f`
//! printed its refusal and then ran `echo x` anyway, and the script exited 0.
//! Bash opens a redirection before the command: when it cannot, the command does
//! not run and its status is 1, and errexit applies. These drive the real binary
//! in agent mode, from a workspace whose parent is outside the jail.

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
            .join("posix_refused_redirect")
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
fn a_refused_output_redirection_does_not_run_its_command() {
    let d = Dirs::new("out");
    let out = d.bash("echo x > ../f; echo $?\n");
    assert_eq!(
        stdout(&out).trim(),
        "1",
        "the command must not run, and its status must be 1: {out:?}"
    );
    assert!(
        !d.outside().exists(),
        "a refused redirection must write nothing"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("outside the workspace"),
        "{out:?}"
    );
}

#[test]
fn a_refused_error_redirection_does_not_run_its_command() {
    let d = Dirs::new("err");
    let out = d.bash("echo x 2>../f 1>&2; echo $?\n");
    assert_eq!(stdout(&out).trim(), "1", "{out:?}");
    assert!(
        !d.outside().exists(),
        "a refused redirection must write nothing"
    );
}

#[test]
fn a_refused_redirection_stops_the_script_under_errexit() {
    let d = Dirs::new("errexit");
    let out = d.bash("set -e\necho x > ../f\necho after\n");
    assert!(
        !out.status.success(),
        "errexit must end the script: {out:?}"
    );
    assert!(
        !stdout(&out).contains("after"),
        "the script must stop at the refused redirection: {out:?}"
    );
    assert!(
        !d.outside().exists(),
        "a refused redirection must write nothing"
    );
}

#[test]
fn a_redirection_inside_the_workspace_still_runs_its_command() {
    let d = Dirs::new("inside");
    let out = d.bash("echo x > f.txt; echo $?\n");
    assert_eq!(stdout(&out).trim(), "0", "{out:?}");
    assert_eq!(
        std::fs::read_to_string(d.workspace.join("f.txt")).unwrap(),
        "x\n"
    );
}

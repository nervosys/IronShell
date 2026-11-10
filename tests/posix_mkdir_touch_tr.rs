//! `mkdir`, `touch` and `tr` run in-process in `ish -b`.
//!
//! On a stock Windows host none of them is on PATH, so a script that used them
//! failed part-way with "command not found" after printing earlier output.
//! `mkdir` and `touch` judge each operand as a `>` redirection is, so a path
//! outside the workspace is refused and nothing is made. These drive the real
//! binary in agent mode, in a scratch workspace.

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
            .join("posix_mkdir_touch_tr")
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
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn mkdir_p_makes_nested_directories_and_accepts_an_existing_one() {
    let d = Dirs::new("mkdir_p");
    let out = d.bash("mkdir -p build/out/bin\nmkdir -p build/out\necho $?\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out), "0\n");
    assert!(d.workspace.join("build").join("out").join("bin").is_dir());
}

#[test]
fn mkdir_without_p_refuses_an_existing_directory() {
    let d = Dirs::new("mkdir_exists");
    std::fs::create_dir(d.workspace.join("there")).unwrap();
    let out = d.bash("mkdir there\necho $?\n");
    assert_eq!(stdout(&out), "1\n", "{}", stderr(&out));
    assert!(
        stderr(&out).contains("mkdir: cannot create directory 'there': File exists"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn mkdir_outside_the_workspace_is_refused_and_makes_nothing() {
    let d = Dirs::new("mkdir_outside");
    let out = d.bash("mkdir ../escaped\necho $?\n");
    assert_eq!(stdout(&out), "1\n", "{}", stderr(&out));
    assert!(
        stderr(&out).contains("outside the workspace"),
        "{}",
        stderr(&out)
    );
    assert!(
        !d.root.join("escaped").exists(),
        "the directory was made outside"
    );
    assert!(
        !stderr(&out).contains("apv_"),
        "an outside path must not offer a token"
    );
}

#[test]
fn touch_creates_an_empty_file_and_keeps_an_existing_one() {
    let d = Dirs::new("touch");
    std::fs::write(d.workspace.join("kept.txt"), "data").unwrap();
    let out = d.bash("touch new.txt kept.txt\necho $?\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out), "0\n");
    assert_eq!(std::fs::read(d.workspace.join("new.txt")).unwrap(), b"");
    assert_eq!(
        std::fs::read_to_string(d.workspace.join("kept.txt")).unwrap(),
        "data"
    );
}

#[test]
fn touch_outside_the_workspace_is_refused_and_writes_nothing() {
    let d = Dirs::new("touch_outside");
    let out = d.bash("touch ../escaped.txt\necho $?\n");
    assert_eq!(stdout(&out), "1\n", "{}", stderr(&out));
    assert!(
        stderr(&out).contains("outside the workspace"),
        "{}",
        stderr(&out)
    );
    assert!(!d.root.join("escaped.txt").exists());
}

#[test]
fn tr_maps_ranges_and_classes() {
    let d = Dirs::new("tr_map");
    let out =
        d.bash("echo 'Hello, World' | tr a-z A-Z\necho 'MiXeD' | tr '[:upper:]' '[:lower:]'\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out), "HELLO, WORLD\nmixed\n");
}

#[test]
fn tr_deletes_and_squeezes() {
    let d = Dirs::new("tr_ds");
    let out = d.bash(
        "echo 'a1b2c3' | tr -d 0-9\necho 'a    b  c' | tr -s ' '\necho 'a,b,,c' | tr -s ',' ' '\n",
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out), "abc\na b c\na b c\n");
}

#[test]
fn tr_pads_a_short_second_set_with_its_last_character() {
    let d = Dirs::new("tr_pad");
    let out = d.bash("echo abcd | tr abcd xy\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out), "xyyy\n");
}

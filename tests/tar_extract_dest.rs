//! `tar_extract` into a destination that does not exist creates it, as
//! `zip_extract` (unzip -d, Expand-Archive) does. It used to run `tar -C` on the
//! missing path and answer E_TOOL_FAILED "could not chdir", with a hint that
//! blamed the service. The destination still passes the workspace write check
//! first, so an outside destination is refused and nothing is created there.

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
        let root = std::env::temp_dir().join(format!("ae-tar-dest-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            workspace: root.join("ws"),
            outside: root.join("outside"),
            session: root.join("session"),
            root,
        };
        for p in [&d.workspace, &d.session] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(d.workspace.join("a.txt"), "hello\n").unwrap();
        d
    }

    fn ish(&self, expr: &str, agent: bool) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
        cmd.args(["-c", expr])
            .current_dir(&self.workspace)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &self.session);
        if agent {
            cmd.env("IRONSHELL_MODE", "agent")
                .env("IRONSHELL_WORKSPACE", &self.workspace)
                .env("IRONSHELL_ALLOW_SH", "");
        }
        cmd.output().expect("run ish")
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

#[test]
fn tar_extract_creates_a_missing_destination() {
    let d = Dirs::new("create");
    let made = d.ish(r#"tar_create("a.tar", "a.txt")"#, false);
    assert!(made.status.success(), "tar_create failed: {}", text(&made));

    let out = d.ish(r#"tar_extract("a.tar", "newdir")"#, false);
    assert!(out.status.success(), "tar_extract failed: {}", text(&out));
    let extracted = std::fs::read_to_string(d.workspace.join("newdir").join("a.txt"))
        .expect("the member was extracted into the new directory");
    assert_eq!(extracted, "hello\n");
}

#[test]
fn tar_extract_into_an_outside_destination_is_refused_before_anything_is_created() {
    let d = Dirs::new("jail");
    let made = d.ish(r#"tar_create("a.tar", "a.txt")"#, false);
    assert!(made.status.success(), "tar_create failed: {}", text(&made));

    let out = d.ish(r#"tar_extract("a.tar", "../outside/dest")"#, true);
    let t = text(&out);
    assert!(!out.status.success(), "should be refused: {t}");
    assert!(
        t.contains("E_OUTSIDE_WORKSPACE"),
        "not refused as outside: {t}"
    );
    assert!(
        !d.outside.exists(),
        "a directory was created outside the workspace"
    );
}

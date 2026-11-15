//! Round 44a: under `--workspace`, `cd` and relative paths agree.
//!
//! After `cd('sub dir')`, `file.write('x.txt')` (and `file.append`, `touch`,
//! `file.mkdir`) wrote to the workspace root while reads resolved against the
//! working directory, and `file.read('../a.txt')`, `cd('..')` and `ls('..')` were
//! refused although the path is inside the workspace. A relative path is now the
//! working directory's in every mode, and the jail judges that resolved path.
//!
//! These drive the real binary from the workspace's parent, as a harness would.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    ws: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-r44a-cwd-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            ws: root.join("ws"),
            session: root.join("session"),
            root,
        };
        std::fs::create_dir_all(d.ws.join("sub dir").join("deep")).unwrap();
        std::fs::create_dir_all(&d.session).unwrap();
        std::fs::write(d.ws.join("a.txt"), "alpha-root").unwrap();
        std::fs::write(d.ws.join("sub dir").join("deep").join("d.txt"), "delta").unwrap();
        d
    }

    fn ish(&self, script: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["--agent", "--workspace"])
            .arg(&self.ws)
            .args(["-c", script])
            .current_dir(&self.root)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env_remove("IRONSHELL_PINNED_ROOT")
            .output()
            .expect("run ish")
    }

    fn sub(&self) -> PathBuf {
        self.ws.join("sub dir")
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
fn writers_after_cd_land_in_the_working_directory() {
    let d = Dirs::new("write");
    let out = d.ish(
        "cd('sub dir')\nfile.write('x.txt', 'one')\nfile.append('x.txt', 'two')\ntouch('t.txt')\nfile.mkdir('m')\n",
    );
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        std::fs::read_to_string(d.sub().join("x.txt")).unwrap(),
        "onetwo"
    );
    assert!(d.sub().join("t.txt").exists(), "touch landed elsewhere");
    assert!(d.sub().join("m").is_dir(), "file.mkdir landed elsewhere");
    for stray in ["x.txt", "t.txt", "m"] {
        assert!(!d.ws.join(stray).exists(), "{stray} landed in the root");
    }
}

#[test]
fn a_parent_path_inside_the_workspace_is_allowed_after_cd() {
    let d = Dirs::new("parent");
    let out = d.ish("cd('sub dir')\nfile.read('../a.txt')");
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("alpha-root"), "{}", text(&out));

    let out = d.ish("cd('sub dir')\nls('..')");
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("a.txt"), "{}", text(&out));

    // `cd('..')` back to the root, then a root-relative write.
    let out = d.ish("cd('sub dir')\ncd('..')\nfile.write('back.txt', 'x')");
    assert!(out.status.success(), "{}", text(&out));
    assert!(d.ws.join("back.txt").exists());
}

#[test]
fn a_second_cd_is_relative_to_the_first() {
    let d = Dirs::new("deep");
    let out = d.ish("cd('sub dir')\ncd('deep')\nfile.read('d.txt')");
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("delta"), "{}", text(&out));
}

#[test]
fn the_jail_still_judges_the_resolved_path() {
    let d = Dirs::new("jail");
    // Two levels up from `sub dir` is the workspace's parent: outside.
    for script in [
        "cd('sub dir')\nfile.write('../../out.txt', 'x')",
        "cd('sub dir')\nfile.read('../../session')",
        "cd('sub dir')\ncd('../..')",
    ] {
        let out = d.ish(script);
        assert!(!out.status.success(), "{script}: {}", text(&out));
        assert!(
            text(&out).contains("E_OUTSIDE_WORKSPACE"),
            "{script}: {}",
            text(&out)
        );
    }
    assert!(!d.root.join("out.txt").exists());
}

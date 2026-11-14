//! Round 42b: undo puts a file's modification time back with its bytes.
//!
//! Drives the real binary in agent mode with the journal in a temporary
//! session directory; the working directory is outside the repository.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Duration, SystemTime};

struct Dirs {
    root: PathBuf,
    workspace: PathBuf,
    outside: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-round42b-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            workspace: root.join("ws"),
            outside: root.join("outside"),
            session: root.join("session"),
            root,
        };
        for p in [&d.workspace, &d.outside, &d.session] {
            std::fs::create_dir_all(p).unwrap();
        }
        d
    }

    fn ish(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(&self.outside)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", &self.workspace)
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env("IRONSHELL_JOURNAL", "on")
            .env("IRONSHELL_ALLOW_SH", "")
            .env("IRONSHELL_APPROVE_ALL", "1")
            .output()
            .expect("run ish")
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn set_mtime(path: &PathBuf, when: SystemTime) {
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(when).unwrap();
}

fn close(a: SystemTime, b: SystemTime) -> bool {
    let d = match a.duration_since(b) {
        Ok(d) => d,
        Err(e) => e.duration(),
    };
    d < Duration::from_secs(5)
}

#[test]
fn undo_of_an_overwrite_restores_the_old_mtime() {
    let d = Dirs::new("overwrite");
    let a = d.workspace.join("a.txt");
    std::fs::write(&a, "old\n").unwrap();
    let forty_days_ago = SystemTime::now() - Duration::from_secs(40 * 24 * 3600);
    set_mtime(&a, forty_days_ago);

    let w = d.ish(&["-c", r#"file_write("a.txt", "new\n")"#]);
    assert!(w.status.success(), "{w:?}");
    let written = std::fs::metadata(&a).unwrap().modified().unwrap();
    assert!(!close(written, forty_days_ago), "the write moved the mtime");

    let u = d.ish(&["-c", "undo({id: 1})"]);
    assert!(u.status.success(), "{u:?}");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "old\n", "{u:?}");
    let restored = std::fs::metadata(&a).unwrap().modified().unwrap();
    assert!(
        close(restored, forty_days_ago),
        "restored mtime {restored:?}, wanted {forty_days_ago:?}"
    );
}

#[test]
fn undo_of_a_delete_restores_the_old_mtime() {
    let d = Dirs::new("delete");
    let a = d.workspace.join("gone.txt");
    std::fs::write(&a, "keep\n").unwrap();
    let forty_days_ago = SystemTime::now() - Duration::from_secs(40 * 24 * 3600);
    set_mtime(&a, forty_days_ago);

    let r = d.ish(&["-c", r#"rm("gone.txt")"#]);
    assert!(r.status.success(), "{r:?}");
    assert!(!a.exists(), "{r:?}");

    let u = d.ish(&["-c", "undo({id: 1})"]);
    assert!(u.status.success(), "{u:?}");
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "keep\n", "{u:?}");
    let restored = std::fs::metadata(&a).unwrap().modified().unwrap();
    assert!(
        close(restored, forty_days_ago),
        "restored mtime {restored:?}, wanted {forty_days_ago:?}"
    );
}

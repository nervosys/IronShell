//! A BadState hint reads as written. `bad_state` wraps its third argument as a
//! builtin name ("call `X` first, then retry this unchanged"), so a sentence
//! given there came out as "call `remove it first; ...` first". Those call sites
//! now use `bad_state_with_hint`, which takes the hint as given.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ae-bad-state-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cwd")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Scratch { root }
    }

    fn cwd(&self) -> PathBuf {
        self.root.join("cwd")
    }

    fn ish(&self, expr: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-c", expr])
            .current_dir(self.cwd())
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .output()
            .expect("run ish")
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

#[test]
fn zip_create_on_an_existing_archive_gives_the_hint_as_written() {
    let s = Scratch::new("zip");
    std::fs::write(s.cwd().join("exists.zip"), b"not really a zip").unwrap();
    std::fs::write(s.cwd().join("a.txt"), "a\n").unwrap();
    let out = s.ish(r#"zip_create("exists.zip", "a.txt")"#);
    let t = text(&out);
    assert!(!out.status.success(), "should be refused: {t}");
    assert!(t.contains("E_BAD_STATE"), "not a BadState: {t}");
    assert!(
        t.contains("remove it first; the existing file is not overwritten"),
        "hint was not given as written: {t}"
    );
    assert!(
        !t.contains("call `remove it first"),
        "hint was wrapped as a builtin call: {t}"
    );
    // The existing archive is left as it was.
    assert_eq!(
        std::fs::read(s.cwd().join("exists.zip")).unwrap(),
        b"not really a zip"
    );
}

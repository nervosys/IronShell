//! Round 23: two reads past the agent-mode jail, and one path base for reads
//! and writes.
//!
//! - `cp("../outside/secret", "copy")` copied an outside file into the
//!   workspace (the source was deliberately unjailed), after which `cat` read it.
//! - `sql` and `sql_value` opened an outside file named as their source.
//! - Writes resolved a relative path against `--workspace`, reads against the
//!   working directory: run from the workspace's parent, `open("x")` and
//!   `file_write("x")` named different files.

use std::path::PathBuf;
use std::process::Command;

struct Dirs {
    root: PathBuf,
    ws: PathBuf,
    outside: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ish-r23-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            ws: root.join("ws"),
            outside: root.join("outside"),
            root,
        };
        std::fs::create_dir_all(&d.ws).unwrap();
        std::fs::create_dir_all(&d.outside).unwrap();
        std::fs::write(d.outside.join("secret.txt"), "OUTSIDE-SECRET\n").unwrap();
        std::fs::write(d.outside.join("people.csv"), "name,age\nann,31\n").unwrap();
        d
    }

    /// `ish --agent --workspace <ws>` run from `cwd`.
    fn agent(&self, cwd: &PathBuf, workspace: &str, expr: &str) -> String {
        let out = Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["--agent", "--workspace", workspace, "-c", expr])
            .current_dir(cwd)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .env_remove("IRONSHELL_APPROVE")
            .env_remove("IRONSHELL_WORKSPACE")
            .output()
            .expect("run ish");
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr)
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn cp_refuses_an_outside_source() {
    let d = Dirs::new("cp");
    let out = d.agent(&d.ws, ".", r#"cp("../outside/secret.txt", "copy.txt")"#);
    assert!(out.contains("E_OUTSIDE_WORKSPACE"), "{out}");
    assert!(!d.ws.join("copy.txt").exists(), "nothing was copied in");
    let out = d.agent(
        &d.ws,
        ".",
        r#"file_copy("../outside/secret.txt", "copy.txt")"#,
    );
    assert!(out.contains("E_OUTSIDE_WORKSPACE"), "{out}");
    assert!(!out.contains("OUTSIDE-SECRET"), "{out}");
}

#[test]
fn cp_inside_the_workspace_still_works() {
    let d = Dirs::new("cp_in");
    std::fs::write(d.ws.join("a.txt"), "inside").unwrap();
    let out = d.agent(&d.ws, ".", r#"cp("a.txt", "b.txt")"#);
    assert!(!out.contains("E_"), "{out}");
    assert_eq!(
        std::fs::read_to_string(d.ws.join("b.txt")).unwrap(),
        "inside"
    );
}

#[test]
fn sql_refuses_an_outside_source() {
    let d = Dirs::new("sql");
    let out = d.agent(
        &d.ws,
        ".",
        r#"sql_value("../outside/people.csv", "select name from t")"#,
    );
    assert!(out.contains("E_OUTSIDE_WORKSPACE"), "{out}");
    assert!(!out.contains("ann"), "{out}");
    let out = d.agent(
        &d.ws,
        ".",
        r#"sql("../outside/people.csv", "select * from t")"#,
    );
    assert!(out.contains("E_OUTSIDE_WORKSPACE"), "{out}");
    let out = d.agent(&d.ws, ".", r#"sql_value(":memory:", "select 6 * 7")"#);
    assert!(
        out.contains("42"),
        "an in-memory source is not a file: {out}"
    );
}

#[test]
fn reads_and_writes_resolve_a_relative_path_alike() {
    let d = Dirs::new("base");
    // Run from the workspace's parent, naming the workspace by its relative
    // path, as `ish --agent --workspace ws` from `root`.
    let out = d.agent(
        &d.root,
        "ws",
        r#"file_write("ws/a.txt", "hello"); read_text("ws/a.txt")"#,
    );
    assert!(out.contains("hello"), "{out}");
    assert!(d.ws.join("a.txt").exists(), "{out}");
    assert!(!d.ws.join("ws").exists(), "not written under ws/ws: {out}");
    // `a.txt` from the parent is the parent's file, outside the workspace.
    let out = d.agent(&d.root, "ws", r#"file_write("a.txt", "x")"#);
    assert!(out.contains("E_OUTSIDE_WORKSPACE"), "{out}");
    assert!(!d.root.join("a.txt").exists());
}

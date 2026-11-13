//! `sed -i` in `ish -b` edits its files in place, in-process.
//!
//! It used to fall to the real sed program, which the exec gate asks approval
//! for (exit 75, file unchanged). The subset now runs it itself, and each FILE
//! goes through the guard a `>` redirection passes and is journaled, so `undo()`
//! puts it back. These drive the real binary in agent mode, in a scratch
//! workspace whose parent holds the session directory and outside files.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    ws: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("round27_sed_i")
            .join(tag);
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            ws: root.join("ws"),
            session: root.join("session"),
            root,
        };
        std::fs::create_dir_all(&d.ws).unwrap();
        std::fs::create_dir_all(&d.session).unwrap();
        d
    }

    /// Runs `ish --agent --workspace <ws> ARGS` from inside the workspace.
    fn ish(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["--agent", "--workspace"])
            .arg(&self.ws)
            .args(args)
            .current_dir(&self.ws)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env("IRONSHELL_JOURNAL", "on")
            .env_remove("IRONSHELL_APPROVE")
            .env_remove("IRONSHELL_APPROVE_ALL")
            .output()
            .expect("run ish")
    }

    /// Runs `script` with the `-b` POSIX subset.
    fn bash(&self, script: &str) -> Output {
        self.ish(&["-b", "-c", script])
    }

    fn put(&self, name: &str, body: &str) {
        std::fs::write(self.ws.join(name), body).unwrap();
    }

    fn get(&self, name: &str) -> String {
        std::fs::read_to_string(self.ws.join(name)).unwrap()
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn s_with_g_edits_the_file_in_place() {
    let d = Dirs::new("g");
    d.put("f.txt", "bob and bob\nBob\n");
    let out = d.bash("sed -i 's/bob/BOB/g' f.txt\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(stdout(&out), "", "in-place sed prints nothing: {out:?}");
    assert_eq!(d.get("f.txt"), "BOB and BOB\nBob\n");
}

#[test]
fn dash_e_with_separate_flags_still_edits_in_place() {
    let d = Dirs::new("ere");
    d.put("f.txt", "aaa x\n");
    let out = d.bash("sed -i -E 's/a+/b/' f.txt\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(d.get("f.txt"), "b x\n");
}

#[test]
fn dash_n_with_p_keeps_only_the_printed_lines() {
    let d = Dirs::new("quiet");
    d.put("f.txt", "keep 1\ndrop\nkeep 2\n");
    let out = d.bash("sed -n -i '/keep/p' f.txt\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(d.get("f.txt"), "keep 1\nkeep 2\n");
}

#[test]
fn dash_n_without_p_writes_an_empty_file() {
    let d = Dirs::new("quiet_empty");
    d.put("f.txt", "a\nb\n");
    let out = d.bash("sed -n -i 's/a/A/' f.txt\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(d.get("f.txt"), "");
}

#[test]
fn dash_i_with_an_attached_suffix_keeps_the_original_as_a_backup() {
    let d = Dirs::new("bak");
    d.put("f.txt", "alpha\n");
    let out = d.bash("sed -i.bak 's/alpha/beta/' f.txt\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(d.get("f.txt"), "beta\n");
    assert_eq!(
        d.get("f.txt.bak"),
        "alpha\n",
        "the backup keeps the original bytes"
    );
}

#[test]
fn two_files_are_each_edited_with_line_numbers_restarting() {
    let d = Dirs::new("two");
    d.put("a.txt", "1\n2\n3\n");
    d.put("b.txt", "x\ny\n");
    let out = d.bash("sed -i '$d' a.txt b.txt\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(d.get("a.txt"), "1\n2\n", "$ is the last line of each file");
    assert_eq!(d.get("b.txt"), "x\n", "$ is the last line of each file");
}

#[test]
fn a_file_without_a_final_newline_keeps_it_missing() {
    let d = Dirs::new("nonl");
    d.put("g.txt", "one\ntwo");
    let out = d.bash("sed -i 's/two/2/' g.txt\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(d.get("g.txt"), "one\n2");
}

#[test]
fn an_in_place_edit_outside_the_workspace_is_refused_like_a_redirect() {
    let d = Dirs::new("outside");
    let outside = d.root.join("outside.txt");
    std::fs::write(&outside, "keep\n").unwrap();
    let out = d.bash("sed -i 's/keep/lost/' ../outside.txt\n");
    let err = stderr(&out);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        err.contains("outside the workspace"),
        "the refusal must name the jail: {err}"
    );
    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "keep\n",
        "a refused sed -i must leave the file alone"
    );

    // The same refusal as a `>` redirection to the same path.
    let r = Dirs::new("outside_redirect");
    std::fs::write(r.root.join("outside.txt"), "keep\n").unwrap();
    let redirect = r.bash("echo x > ../outside.txt\n");
    assert_eq!(redirect.status.code(), out.status.code(), "{redirect:?}");
    assert!(
        stderr(&redirect).contains("outside the workspace"),
        "{redirect:?}"
    );
}

#[test]
fn a_missing_file_is_reported_as_gnu_sed_reports_it() {
    let d = Dirs::new("missing");
    let out = d.bash("sed -i 's/a/b/' nope.txt\n");
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        stderr(&out).contains("sed: can't read nope.txt: No such file or directory"),
        "{out:?}"
    );
}

#[test]
fn undo_restores_a_file_edited_in_place() {
    let d = Dirs::new("undo");
    d.put("u.txt", "before\n");
    let out = d.bash("sed -i 's/before/after/' u.txt\n");
    assert!(out.status.success(), "{out:?}");
    assert_eq!(d.get("u.txt"), "after\n");

    let undone = d.ish(&["-c", "undo()"]);
    assert!(undone.status.success(), "{undone:?}");
    assert_eq!(d.get("u.txt"), "before\n", "{undone:?}");
}

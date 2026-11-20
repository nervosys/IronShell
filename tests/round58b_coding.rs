//! Round 58b: findings from the coding-chores round.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ish-r58b-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let cwd = root.join("cwd");
        std::fs::create_dir_all(cwd.join("tmpd")).unwrap();
        std::fs::create_dir_all(cwd.join("t4").join("a").join("b").join("c").join("d")).unwrap();
        std::fs::write(cwd.join("f.txt"), "alpha\nTodo beta\n").unwrap();
        std::fs::write(
            cwd.join("t4")
                .join("a")
                .join("b")
                .join("c")
                .join("d")
                .join("e"),
            "",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Scratch { root }
    }

    fn ish(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(self.root.join("cwd"))
            .env("NO_COLOR", "1")
            .env_remove("IRONSHELL_ALLOW_SH")
            .env_remove("IRONSHELL_APPROVE")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .output()
            .expect("run ish")
    }

    fn json(&self, expr: &str) -> String {
        let out = self.ish(&["--deterministic", "-c", expr]);
        let text = if out.status.success() {
            String::from_utf8_lossy(&out.stdout).to_string()
        } else {
            String::from_utf8_lossy(&out.stderr).to_string()
        };
        text.trim().to_string()
    }

    /// stdout+stderr and the exit code of an agent-mode run in the workspace.
    fn agent(&self, expr: &str) -> (String, i32) {
        let out = self.ish(&["--agent", "--workspace", ".", "-c", expr]);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (text, out.status.code().unwrap_or(-1))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn b1_tool_search_lists_builtins_first() {
    let s = Scratch::new("b1");
    let names = s.json(
        r#"tool_search("replace text in a file") | where(.kind == "builtin") | take(3) | map(.name)"#,
    );
    assert!(names.contains("file_replace"), "{names}");
    let rm = s.json(r#"tool_search("delete a file") | where(.kind == "builtin") | map(.name)"#);
    assert!(rm.contains(r#""rm""#), "{rm}");
    // Every builtin row comes before the first external row.
    let kinds = s.json(r#"tool_search("copy files") | map(.kind)"#);
    let last_builtin = kinds.rfind("builtin").expect("a builtin row");
    if let Some(first_external) = kinds.find("external") {
        assert!(last_builtin < first_external, "{kinds}");
    }
    // External rows keep their shape.
    let row = s.json(r#"tool_search("sha256") | where(.name == "sha256sum") | first"#);
    assert!(row.contains(r#""kind":"external""#), "{row}");
    assert!(row.contains(r#""run":"tool_exec("#), "{row}");
    let doc = s.json(r#"help("tool_search")"#);
    assert!(doc.contains("Builtins come first"), "{doc}");
}

#[test]
fn b2_rm_of_a_directory_fails_before_the_approval_gate() {
    let s = Scratch::new("b2");
    let (text, code) = s.agent(r#"rm("tmpd", true)"#);
    assert_eq!(code, 64, "{text}");
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(
        !text.contains("apv_"),
        "an approval token was offered: {text}"
    );
    assert!(text.contains(r#"rmdir(\"tmpd\", true)"#), "{text}");
    assert!(s.root.join("cwd").join("tmpd").is_dir());
}

#[test]
fn b2_file_move_refusals_come_before_the_approval_gate() {
    let s = Scratch::new("b2m");
    let (text, code) = s.agent(r#"file_move("f.txt", "tmpd")"#);
    assert_eq!(code, 64, "{text}");
    assert!(
        text.contains("E_BAD_ARG") && !text.contains("apv_"),
        "{text}"
    );
    let (text, code) = s.agent(r#"file_move("f.txt", "nodir/f.txt")"#);
    assert_eq!(code, 66, "{text}");
    assert!(
        text.contains("E_NOT_FOUND") && !text.contains("apv_"),
        "{text}"
    );
    // A move that can succeed still asks.
    let (text, code) = s.agent(r#"file_move("f.txt", "g.txt")"#);
    assert_eq!(code, 75, "{text}");
}

#[test]
fn b3_missing_folder_hint_does_not_call_mkdirp_different() {
    let s = Scratch::new("b3");
    let (text, _) = s.agent(r#"file_write("nodir/x.txt", "hi")"#);
    assert!(text.contains("any missing parents"), "{text}");
    assert!(!text.contains("mkdirp also"), "{text}");
}

#[test]
fn b4_grep_ignore_case() {
    let s = Scratch::new("b4");
    assert_eq!(
        s.json(r#"grep("todo", "f.txt", {ignore_case: true})"#),
        r#"["Todo beta"]"#
    );
    assert_eq!(s.json(r#"grep("todo", "f.txt")"#), "[]");
    assert_eq!(
        s.json(r#""Todo x\nok" | grep("TODO", {ignore_case: true, line_numbers: true}) | len"#),
        "1"
    );
    // The inline flag is not BRE; it works with extended.
    assert_eq!(s.json(r#"grep("(?i)todo", "f.txt")"#), "[]");
    assert_eq!(
        s.json(r#"grep("(?i)todo", "f.txt", {extended: true})"#),
        r#"["Todo beta"]"#
    );
    let bad = s.json(r#"grep("x", "f.txt", {ignore_case: 1})"#);
    assert!(bad.contains("E_BAD_ARG"), "{bad}");
    let doc = s.json(r#"help("grep")"#);
    assert!(doc.contains("ignore_case") && doc.contains("(?i)"), "{doc}");
}

#[test]
fn b5_sh_in_agent_mode_is_denied_unless_opted_in() {
    let s = Scratch::new("b5");
    let (text, code) = s.agent(r#"sh(["hostname"])"#);
    assert_eq!(code, 77, "{text}");
    assert!(text.contains("E_POLICY_DENY"), "{text}");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--agent", "--workspace", ".", "-c", r#"sh(["hostname"])"#])
        .current_dir(s.root.join("cwd"))
        .env("IRONSHELL_ALLOW_SH", "true")
        .env_remove("IRONSHELL_APPROVE")
        .env("IRONSHELL_SESSION_DIR", s.root.join("session"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(75));
    let agents =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/AGENTS.md")).unwrap();
    assert!(agents.contains("`sh()` is off in agent mode"));
}

#[test]
fn b6_tree_depth_counts_levels_below_the_path() {
    let s = Scratch::new("b6");
    let lines = |expr: &str| s.json(expr).trim_matches('"').split("\\n").count();
    assert_eq!(lines(r#"tree("t4", 0)"#), 1);
    assert_eq!(lines(r#"tree("t4", 1)"#), 2);
    // The default is 3 levels: a, b, c.
    assert_eq!(lines(r#"tree("t4")"#), 4);
    assert!(!s.json(r#"tree("t4")"#).contains('d'));
}

#[test]
fn b6_explain_lists_an_rm_operand_once() {
    let s = Scratch::new("b6e");
    let run = s.ish(&["--explain", "-b", "-c", "rm -r x"]);
    let text = String::from_utf8_lossy(&run.stdout).to_string();
    assert_eq!(text.matches(r#""call":"rm"#).count(), 1, "{text}");
}

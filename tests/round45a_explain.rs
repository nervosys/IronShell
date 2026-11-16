//! Round 45a: `--explain` judges what the run judges.
//!
//! Path arguments of `file.exists`, `glob` and `file.diff`, literal paths that
//! reach a call through a name, an interpolation or a lambda, and the read
//! operands of `grep`, `head`, `tail`, `sort`, `wc`, `cut`, `uniq` and `find`
//! under `-b`, were predicted `allow` or `unknown` while the run refused them
//! with E_OUTSIDE_WORKSPACE. `call("file.read", ...)` took only the unqualified
//! name, and `rm("*.txt")` said the name was outside the workspace.
//!
//! The workspace and the working directory are temp directories.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    workspace: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-round45a-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            workspace: root.join("ws"),
            session: root.join("session"),
            root,
        };
        for p in [&d.workspace, &d.session] {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(d.workspace.join("inside.txt"), "inside\n").unwrap();
        std::fs::write(d.workspace.join("a.txt"), "a\n").unwrap();
        std::fs::write(d.root.join("outside.txt"), "secret-text\n").unwrap();
        d
    }

    fn ish(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(&self.workspace)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", &self.workspace)
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env_remove("IRONSHELL_ALLOW_SH")
            .output()
            .expect("run ish")
    }

    fn decision(&self, extra: &[&str], code: &str) -> String {
        let mut args = vec!["--explain"];
        args.extend_from_slice(extra);
        args.extend_from_slice(&["-c", code]);
        let out = self.ish(&args);
        let report: serde_json::Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{code}: {e}: {}", String::from_utf8_lossy(&out.stderr)));
        report["decision"].as_str().unwrap_or("").to_string()
    }

    fn run(&self, extra: &[&str], code: &str) -> String {
        let mut args: Vec<&str> = extra.to_vec();
        args.extend_from_slice(&["-c", code]);
        let out = self.ish(&args);
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn exists_glob_and_diff_judge_their_path_arguments() {
    let d = Dirs::new("params");
    for code in [
        r#"file.exists("../outside.txt")"#,
        r#"glob("../*.txt") | len"#,
        r#"file.diff("a.txt", "../outside.txt")"#,
    ] {
        assert_eq!(d.decision(&[], code), "deny", "{code}");
        assert!(d.run(&[], code).contains("E_OUTSIDE_WORKSPACE"), "{code}");
    }
    for code in [
        r#"file.exists("a.txt")"#,
        r#"glob("*.txt") | len"#,
        r#"file.diff("a.txt", "inside.txt")"#,
    ] {
        assert_eq!(d.decision(&[], code), "allow", "{code}");
    }
}

#[test]
fn a_literal_reaching_a_call_through_a_name_is_judged() {
    let d = Dirs::new("consts");
    for code in [
        r#"x = "../outside.txt"; file.read(x)"#,
        r#"p = "../n5.txt"; file.write(p, "x")"#,
        r#"d = ".."; file.read("${d}/outside.txt")"#,
        r#"["a.txt","../outside.txt"] | map(fn(p) => file.read(p))"#,
        r#"map(["a.txt","../outside.txt"], fn(p) => file.read(p))"#,
        r#"f = fn(p) => file.write(p, "x"); f("../lam.txt")"#,
    ] {
        assert_eq!(d.decision(&[], code), "deny", "{code}");
    }
    // A name that stays inside, and a value that cannot be known, keep their
    // old verdicts.
    assert_eq!(d.decision(&[], r#"x = "a.txt"; file.read(x)"#), "allow");
    assert_eq!(
        d.decision(
            &[],
            r#"["a.txt","inside.txt"] | map(fn(p) => file.read(p))"#
        ),
        "allow"
    );
    assert_eq!(d.decision(&[], "let f = fn(x) => x\nf(1)"), "unknown");
}

#[test]
fn posix_read_operands_are_judged_like_cats() {
    let d = Dirs::new("posix");
    for script in [
        "grep -r secret ..",
        "grep secret ../outside.txt",
        "head -n 1 ../outside.txt",
        "tail -n 1 ../outside.txt",
        "sort ../outside.txt",
        "wc -l ../outside.txt",
        "cut -f1 ../outside.txt",
        "uniq ../outside.txt",
        "find ..",
        "cat ../outside.txt",
    ] {
        assert_eq!(d.decision(&["-b"], script), "deny", "{script}");
    }
    for script in [
        "grep -r hello .",
        "grep hello inside.txt",
        "head -n 1 inside.txt",
        "sort inside.txt",
        "find . -name a.txt",
    ] {
        assert_eq!(d.decision(&["-b"], script), "allow", "{script}");
    }
    let run = d.run(&["-b"], "head -n 1 ../outside.txt");
    assert!(!run.contains("secret-text"), "{run}");
}

#[test]
fn call_accepts_a_module_qualified_name() {
    let d = Dirs::new("call");
    let out = d.run(&[], r#"call("file.read", "inside.txt")"#);
    assert!(out.contains("inside"), "{out}");
    assert!(!out.contains("E_UNKNOWN_BUILTIN"), "{out}");
    assert_eq!(
        d.decision(&[], r#"call("file.read", "../outside.txt")"#),
        "deny"
    );
    assert_eq!(
        d.decision(&[], r#"call("file.read", "inside.txt")"#),
        "allow"
    );
}

#[test]
fn rm_does_not_expand_globs_and_says_so() {
    let d = Dirs::new("rmglob");
    let out = d.run(&[], r#"rm("*.txt")"#);
    assert!(out.contains("E_BAD_ARG"), "{out}");
    assert!(out.contains("does not expand globs"), "{out}");
    assert!(!out.contains("outside the workspace"), "{out}");
    assert!(d.workspace.join("a.txt").exists());
    assert_eq!(d.decision(&[], r#"rm("*.txt")"#), "deny");
}

//! Round 46a: `--explain` must not predict `allow` for a path it cannot know.
//!
//! Round 45 folded names, interpolation and lambdas into literals, and the
//! folding was wrong in two ways: a `mut` name rebound later was judged by its
//! first value, and a path computed at run time was silently allowed. Each
//! case here runs the explanation and the real run under agent mode, in a temp
//! workspace, and fails if the explanation says `allow` for a run the jail
//! refused.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    workspace: PathBuf,
    session: PathBuf,
}

struct Outcome {
    decision: String,
    unresolved: Vec<String>,
    run: Output,
}

impl Outcome {
    fn refused(&self) -> bool {
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&self.run.stdout),
            String::from_utf8_lossy(&self.run.stderr)
        );
        self.run.status.code() == Some(77) || text.contains("E_OUTSIDE_WORKSPACE")
    }
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-round46a-{tag}-{}", std::process::id()));
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
        std::fs::write(d.workspace.join("ptr.txt"), "../x7").unwrap();
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
            .env("P46", "../x8")
            .env_remove("IRONSHELL_ALLOW_SH")
            .output()
            .expect("run ish")
    }

    /// The explanation of `code` and the real run of it.
    fn both(&self, code: &str) -> Outcome {
        let out = self.ish(&["--explain", "-c", code]);
        let report: serde_json::Value = serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{code}: {e}: {}", String::from_utf8_lossy(&out.stderr)));
        Outcome {
            decision: report["decision"].as_str().unwrap_or("").to_string(),
            unresolved: report["unresolved"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            run: self.ish(&["-c", code]),
        }
    }

    /// No false allow, and no file escaped the workspace.
    fn no_false_allow(&self, code: &str) -> Outcome {
        let o = self.both(code);
        if o.refused() {
            assert_ne!(o.decision, "allow", "false allow for a refused run: {code}");
        }
        assert_ne!(o.decision, "allow", "{code}: a path that is not known");
        for escaped in ["x1", "x2", "x3", "x4", "x5", "x6", "x7", "x8", "x9"] {
            assert!(
                !self.root.join(escaped).exists()
                    && !self.root.join(escaped.to_uppercase()).exists(),
                "{code}: wrote {escaped} outside the workspace"
            );
        }
        o
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_mut_name_rebound_later_is_not_a_constant() {
    let d = Dirs::new("mut");
    for code in [
        r#"mut p = "a.txt"; p = "../x1"; file.write(p, "hi")"#,
        r#"mut p = "ok5.txt"; f = fn() => file.write(p, "1"); p = "../x5"; f()"#,
        // Bound twice, without `mut`.
        r#"p = "a.txt"; p = "../x1"; file.write(p, "hi")"#,
        // A name rebound after the call that reads it still counts.
        r#"mut p = "../x1"; file.write(p, "hi"); p = "b.txt""#,
    ] {
        let o = d.no_false_allow(code);
        assert!(
            o.unresolved.iter().any(|u| u.contains("path is computed")),
            "{code}: {:?}",
            o.unresolved
        );
    }
    // A function bound twice is not judged by its first body.
    let code = r#"f = fn() => file.write("a.txt", "1"); f = fn() => file.write("../x5", "1"); f()"#;
    d.no_false_allow(code);
}

#[test]
fn a_name_bound_once_is_still_a_constant() {
    let d = Dirs::new("const");
    for code in [
        r#"p = "b.txt"; file.write(p, "x")"#,
        r#"p = "a.txt"; f = fn() => file.read(p); f()"#,
        r#"["a.txt", "inside.txt"] | each(fn(f) => file.read(f))"#,
    ] {
        let o = d.both(code);
        assert_eq!(o.decision, "allow", "{code}: {:?}", o.unresolved);
        assert!(!o.refused(), "{code}");
    }
    let o = d.both(r#"p = "../x1"; file.write(p, "hi")"#);
    assert_eq!(o.decision, "deny");
    assert!(o.refused());
}

#[test]
fn a_computed_path_is_unknown_not_allow() {
    let d = Dirs::new("computed");
    for code in [
        r#"n = str.upper("x2"); file.write("../" + n, "hi")"#,
        r#"xs = ["../x3"]; xs | each(fn(f) => file.write(f, "hi"))"#,
        r#"xs = ["../x3"] + ["../x4"]; xs | each(fn(f) => file.write(f, "hi"))"#,
        r#"r = {path: "../x6"}; file.write(r.path, "hi")"#,
        r#"file.write(file.read("ptr.txt"), "hi")"#,
        r#"file.write(env("P46"), "hi")"#,
        r#"f = fn(p) => file.write(p, "hi"); f(str.upper("../x9"))"#,
        r#"file.read(args()[0])"#,
    ] {
        let o = d.no_false_allow(code);
        assert_eq!(o.decision, "unknown", "{code}");
        assert!(
            o.unresolved.iter().any(|u| u.contains("path is computed")),
            "{code}: {:?}",
            o.unresolved
        );
    }
}

#[test]
fn a_computed_path_does_not_hide_a_deny_or_an_approval() {
    let d = Dirs::new("strict");
    let o = d.both(r#"file.write("../x1", "hi"); file.write(env("P46"), "hi")"#);
    assert_eq!(o.decision, "deny");
    let o = d.both(r#"rm(env("P46"))"#);
    assert_eq!(o.decision, "approve");
}

#[test]
fn concatenation_of_known_strings_folds_like_interpolation() {
    let d = Dirs::new("concat");
    let a = d.both(r#"p = "x1"; file.write("${"../" + p}", "hi")"#);
    let b = d.both(r#"p = "x1"; file.write("../" + p, "hi")"#);
    let c = d.both(r#"file.write("../" + "x1", "hi")"#);
    for o in [&a, &b, &c] {
        assert_eq!(o.decision, "deny");
        assert!(o.refused());
    }
    let inside = d.both(r#"p = "c.txt"; file.write("" + p, "hi")"#);
    assert_eq!(inside.decision, "allow");
    assert!(!inside.refused());
    assert!(d.workspace.join("c.txt").exists());
}

#[test]
fn a_climbing_prefix_with_an_unknown_rest_is_not_judged_deny() {
    // `../ws/...` climbs out and comes back in: the jail resolves `..`
    // lexically, so the run accepts it. A prefix that climbs is no proof of a
    // refusal, so the verdict is unknown, never deny and never allow.
    let d = Dirs::new("prefix");
    let o = d.both(r#"file.write("../ws/" + str.lower("IN.TXT"), "hi")"#);
    assert_eq!(o.decision, "unknown");
    let o = d.both(r#"file.write("../" + str.lower("X1"), "hi")"#);
    assert_eq!(o.decision, "unknown");
    assert!(o.refused());
}

#[test]
fn the_nesting_limit_message_states_the_real_limit() {
    // Deep recursion needs more than a test thread's default stack.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let src = format!("{}1{}", "(".repeat(300), ")".repeat(300));
            let err = iron_shell::parser::parse_program(&src)
                .expect_err("300 parentheses are past the limit")
                .to_string();
            assert!(err.contains("nested more than"), "{err}");
            assert!(err.contains("about 256"), "{err}");
            // 200 parentheses are inside the limit.
            let ok = format!("{}1{}", "(".repeat(200), ")".repeat(200));
            assert!(iron_shell::parser::parse_program(&ok).is_ok());
        })
        .unwrap()
        .join()
        .unwrap();
}

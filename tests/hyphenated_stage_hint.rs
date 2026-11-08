//! A pipeline stage written with a hyphen, `x | from-csv`, is not IronShell
//! syntax: the builtin is `from_csv`. The error used to name the token the
//! user never wrote ("unexpected token From") with no hint, and `to-csv`
//! answered "unknown builtin: to". Both now name the underscore spelling.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root =
            std::env::temp_dir().join(format!("ae-hyphen-stage-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cwd")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Scratch { root }
    }

    fn ish(&self, expr: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-c", expr])
            .current_dir(self.root.join("cwd"))
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
fn hyphenated_format_stages_name_the_underscore_builtin() {
    let s = Scratch::new("hint");
    for (expr, underscore) in [
        (r#""a,b\n1,2" | from-csv"#, "from_csv"),
        (r#""a: 1" | from-yaml"#, "from_yaml"),
        (r#""[1]" | from-json"#, "from_json"),
        (r#"[{a: 1}] | to-csv"#, "to_csv"),
    ] {
        let out = s.ish(expr);
        let t = text(&out);
        assert!(!out.status.success(), "{expr} should be refused: {t}");
        assert!(t.contains("E_PARSE"), "{expr} was not an E_PARSE: {t}");
        assert!(
            t.contains(&format!(
                "hint: IronShell builtin names use underscores: write `{underscore}`"
            )),
            "{expr} did not name `{underscore}`: {t}"
        );
        assert!(
            !t.contains("unknown builtin"),
            "{expr} was reported as an unknown builtin: {t}"
        );
    }
}

#[test]
fn underscore_format_stages_still_run() {
    let s = Scratch::new("ok");
    let out = s.ish(r#""a,b\n1,2" | from_csv | len"#);
    let t = text(&out);
    assert!(out.status.success(), "from_csv stage failed: {t}");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "1", "{t}");
}

#[test]
fn subtraction_is_not_read_as_a_stage_name() {
    let s = Scratch::new("sub");
    let out = s.ish("10-4");
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "6");
}

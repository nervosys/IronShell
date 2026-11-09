//! Round 11: a text that does not parse has one code everywhere. `open` of a
//! bad TOML, JSON or YAML file, `json_parse` of bad text and `from_toml` all
//! give E_BAD_VALUE, not retryable, with the line and column in the hint.

use iron_shell::env::Env;
use iron_shell::safety::SafetyError;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn refusal(src: &str) -> SafetyError {
    let mut env = Env::new();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    let err =
        iron_shell::eval::eval_program(&stmts, &mut env).expect_err("the program must be refused");
    err.downcast_ref::<SafetyError>()
        .expect("a structured refusal")
        .clone()
}

fn scratch(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ae-round11-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("cwd")).unwrap();
    std::fs::create_dir_all(root.join("session")).unwrap();
    root
}

fn ish_in(root: &Path, src: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", src])
        .current_dir(root.join("cwd"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_ALLOW_SH")
        .env("IRONSHELL_SESSION_DIR", root.join("session"))
        .output()
        .expect("run ish")
}

#[test]
fn json_parse_of_bad_text_is_e_bad_value_and_not_retryable() {
    let se = refusal(r#"json_parse("{")"#);
    assert_eq!(se.code.as_str(), "E_BAD_VALUE");
    assert!(!se.is_retryable());
    assert!(se.hint.contains("line 1, column"), "{}", se.hint);
}

#[test]
fn from_toml_of_bad_text_is_the_same_code() {
    let se = refusal(r#"from_toml("a = = 1")"#);
    assert_eq!(se.code.as_str(), "E_BAD_VALUE");
    assert!(!se.is_retryable());
}

#[test]
fn open_of_bad_toml_json_and_yaml_is_e_bad_value() {
    let root = scratch("open");
    let cwd = root.join("cwd");
    std::fs::write(cwd.join("bad.toml"), "a = 1\nb = = 2\n").unwrap();
    std::fs::write(cwd.join("bad.json"), "{\"a\": }\n").unwrap();
    std::fs::write(cwd.join("bad.yaml"), "a: [1, 2\n").unwrap();
    for file in ["bad.toml", "bad.json", "bad.yaml"] {
        let code = ish_in(
            &root,
            &format!(r#"try {{ open("{file}") }} catch e {{ e.error.code }}"#),
        );
        assert_eq!(
            String::from_utf8_lossy(&code.stdout).trim(),
            "E_BAD_VALUE",
            "{file}: {}",
            String::from_utf8_lossy(&code.stderr)
        );
        let retry = ish_in(
            &root,
            &format!(r#"try {{ open("{file}") }} catch e {{ e.retryable }}"#),
        );
        assert_eq!(
            String::from_utf8_lossy(&retry.stdout).trim(),
            "false",
            "{file}"
        );
    }
    let toml = ish_in(&root, r#"try { open("bad.toml") } catch e { e.hint }"#);
    assert!(
        String::from_utf8_lossy(&toml.stdout).contains("line 2, column"),
        "{}",
        String::from_utf8_lossy(&toml.stdout)
    );
    let _ = std::fs::remove_dir_all(&root);
}

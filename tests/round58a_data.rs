//! Round 58a: nulls in row data (M1), a CSV delimiter option (M2), and an
//! empty secret cell under redaction (M4).

use std::path::PathBuf;
use std::process::{Command, Output};

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

/// The code and hint of the structured error a program ends in.
fn refusal(src: &str) -> (String, String) {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    let err = iron_shell::eval::eval_program(&stmts, &mut env).expect_err("refused");
    let se = err
        .downcast_ref::<iron_shell::safety::SafetyError>()
        .expect("a structured refusal");
    (format!("{:?}", se.code), se.hint.clone())
}

const PEOPLE: &str = r#"from_csv("name,amount\nann,10\nbob,\ncy,3\n")"#;

#[test]
fn comparing_a_null_names_the_cause_and_the_fix() {
    let (code, hint) = refusal(&format!("{PEOPLE} | where(.amount > 5)"));
    assert_eq!(code, "BadArg");
    assert!(hint.contains("null"), "{hint}");
    assert!(hint.contains("coalesce"), "{hint}");
    assert!(hint.contains("!= null"), "{hint}");
}

#[test]
fn aggregates_and_sorts_name_the_null() {
    for src in [
        format!("{PEOPLE} | map(fn(r) => r.amount) | sum"),
        format!("{PEOPLE} | map(fn(r) => r.amount) | avg"),
        format!("{PEOPLE} | map(fn(r) => r.amount) | max"),
        format!("{PEOPLE} | map(fn(r) => r.amount) | sort"),
        format!(r#"{PEOPLE} | sort_by("amount")"#),
        format!("{PEOPLE} | map(fn(r) => r.amount * 2)"),
    ] {
        let (_, hint) = refusal(&src);
        assert!(hint.contains("coalesce"), "{src}: {hint}");
    }
}

#[test]
fn a_non_null_mismatch_keeps_its_own_hint() {
    let (_, hint) = refusal(r#"[1, "a"] | sum"#);
    assert!(!hint.contains("coalesce"), "{hint}");
}

#[test]
fn coalesce_replaces_only_null() {
    assert_eq!(run("coalesce(null, 0)"), Value::Int(0));
    assert_eq!(run("coalesce(5, 0)"), Value::Int(5));
    assert_eq!(run(r#"coalesce("", "x")"#), Value::Str(String::new()));
    assert_eq!(run("coalesce(false, true)"), Value::Bool(false));
    assert_eq!(run(r#"null | coalesce("n/a")"#), Value::Str("n/a".into()));
}

#[test]
fn coalesce_fills_a_csv_column_before_a_filter_or_a_sum() {
    assert_eq!(
        run(&format!(
            "{PEOPLE} | map(fn(r) => coalesce(r.amount, 0)) | sum"
        )),
        Value::Int(13)
    );
    assert_eq!(
        run(&format!(
            "{PEOPLE} | where(.amount != null && .amount > 5) | len"
        )),
        Value::Int(1)
    );
    assert_eq!(
        run(&format!("{PEOPLE} | map(fn(r) => r.amount | coalesce(0))")),
        Value::Array(vec![Value::Int(10), Value::Int(0), Value::Int(3)])
    );
}

#[test]
fn coalesce_refuses_a_wrong_argument_count() {
    assert_eq!(refusal("coalesce(1)").0, "BadArg");
}

#[test]
fn from_csv_takes_a_delimiter() {
    assert_eq!(
        run(r#"(from_csv("a;b\n1;2\n", {delimiter: ";"}) | first).b"#),
        Value::Int(2)
    );
    assert_eq!(
        run(r#"("a|b\n1|2\n" | from_csv({delimiter: "|"}) | first).a"#),
        Value::Int(1)
    );
    // A quoted field may hold the default comma.
    assert_eq!(
        run(r#"(from_csv("a;b\n\"x,y\";2\n", {delimiter: ";"}) | first).a"#),
        Value::Str("x,y".into())
    );
    // Without it the file is one column, as before.
    assert_eq!(
        run(r#"(from_csv("a;b\n1;2\n") | first) | keys | len"#),
        Value::Int(1)
    );
}

#[test]
fn a_bad_delimiter_is_refused_with_a_hint() {
    for opts in [
        r#"{delimiter: ";;"}"#,
        r#"{delimiter: ""}"#,
        r#"{delimiter: "é"}"#,
        r#"{delimiter: "\""}"#,
        "{delimiter: 1}",
        r#"{sep: ";"}"#,
    ] {
        let (code, hint) = refusal(&format!(r#"from_csv("a;b\n1;2\n", {opts})"#));
        assert_eq!(code, "BadArg", "{opts}");
        assert!(!hint.is_empty(), "{opts}");
    }
}

struct Dirs {
    root: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-r58a-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ws")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Dirs { root }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(self.root.join("ws"))
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", self.root.join("ws"))
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .env_remove("IRONSHELL_REDACT")
            .output()
            .expect("run ish")
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn an_empty_secret_cell_prints_as_empty_and_is_not_counted() {
    let d = Dirs::new("empty");
    let out = d.run(&[
        "-c",
        r#"[{id: 1, password: "hunter2"}, {id: 2, password: ""}, {id: 3, password: "x"}]"#,
    ]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!stdout.contains("hunter2"), "{stdout}");
    // Not one @const cell for all three rows: the rows differ.
    assert!(!stdout.contains("@const password"), "{stdout}");
    let redacted = stdout.matches("[REDACTED]").count();
    assert_eq!(redacted, 2, "{stdout}");
    assert!(stderr.contains("2 secrets redacted"), "{stderr}");
}

#[test]
fn an_empty_secret_field_in_a_record_is_not_redacted() {
    let d = Dirs::new("record");
    let out = d.run(&["-c", r#"{user: "a", token: "", note: "b"}"#]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!stdout.contains("[REDACTED]"), "{stdout}");
    assert!(!stderr.contains("redacted"), "{stderr}");
}

#[test]
fn a_set_secret_field_is_still_redacted() {
    let d = Dirs::new("set");
    let out = d.run(&["-c", r#"{user: "a", password: "hunter2"}"#]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("[REDACTED]"), "{stdout}");
    assert!(!stdout.contains("hunter2"), "{stdout}");
}

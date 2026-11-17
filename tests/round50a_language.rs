//! Round 50a: language and data.
//!
//! A1/A2 annotated `mut` bindings, A3 `count_by`, A4 `group_by` `Key`, A5 hints
//! for builtins used as callbacks, A6 `to_json` indent, A7 AECON quoting of
//! radix-looking strings, A8 `Lambda` in the annotation list.

use iron_shell::safety::SafetyError;
use iron_shell::value::Value;
use std::path::PathBuf;
use std::process::Command;

fn run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn ok(src: &str) -> Value {
    run(src).unwrap_or_else(|e| panic!("{src}: {e:#}"))
}

fn refused(src: &str) -> SafetyError {
    let e = run(src).expect_err(src);
    e.downcast_ref::<SafetyError>()
        .unwrap_or_else(|| panic!("{src}: not a SafetyError: {e:#}"))
        .clone()
}

fn strs(v: &[&str]) -> Value {
    Value::Array(v.iter().map(|s| Value::Str((*s).to_string())).collect())
}

// ---- A1: an annotated binding keeps its type ----

#[test]
fn reassigning_an_annotated_mut_to_another_type_is_refused() {
    let e = refused(r#"let mut m: Int = 1; m = "x"; m"#);
    assert_eq!(e.code.as_str(), "E_BAD_ARG");
    assert!(e.message.contains("m"), "{}", e.message);
    assert_eq!(e.expected, "Int");
    assert_eq!(e.got, "String");
    assert!(e.hint.contains("declared Int"), "{}", e.hint);
}

#[test]
fn reassigning_an_annotated_mut_to_the_same_type_works() {
    assert_eq!(ok("let mut m: Int = 1; m = 2; m"), Value::Int(2));
    assert_eq!(ok("let mut m: Int = 1; m = m + 1; m"), Value::Int(2));
}

#[test]
fn an_unannotated_mut_stays_free() {
    assert_eq!(
        ok(r#"let mut m = 1; m = "x"; m"#),
        Value::Str("x".to_string())
    );
}

#[test]
fn a_field_of_an_annotated_record_may_change_type() {
    assert_eq!(
        ok(r#"let mut r: Record = {a: 1}; r.a = "x"; r.a"#),
        Value::Str("x".to_string())
    );
}

// ---- A2: `mut m: Int = 1` ----

#[test]
fn mut_with_an_annotation_is_let_mut() {
    assert_eq!(ok("mut m: Int = 1; m = 5; m"), Value::Int(5));
    let e = refused(r#"mut m: Int = 1; m = "x""#);
    assert_eq!(e.code.as_str(), "E_BAD_ARG");
    let e = refused(r#"mut m: Int = "x""#);
    assert_eq!(e.code.as_str(), "E_BAD_ARG");
}

// ---- A3: count_by ----

#[test]
fn count_by_takes_a_builtin_name() {
    assert_eq!(
        ok(r#"["a", "bb", "cc"] | count_by(len) | map(fn(r) => r.key)"#),
        Value::Array(vec![Value::Int(2), Value::Int(1)])
    );
    assert_eq!(
        ok(r#"["a.rs", "b.rs", "c.py"] | count_by(path_extension) | first | fn(r) => r.count"#),
        Value::Int(2)
    );
}

#[test]
fn count_by_counts_an_int_and_an_equal_float_as_one_key() {
    assert_eq!(
        ok(r#"[1, 1.0, 2] | count_by(fn(x) => x) | map(fn(r) => r.count)"#),
        Value::Array(vec![Value::Int(2), Value::Int(1)])
    );
    // The first key seen is the one shown.
    assert_eq!(
        ok(r#"[1.0, 1] | count_by(fn(x) => x) | first | fn(r) => r.key"#),
        Value::Float(1.0)
    );
}

#[test]
fn count_by_a_missing_field_counts_under_null() {
    assert_eq!(
        ok(r#"[{a: 1}, {b: 2}] | count_by(.a) | map(fn(r) => r.key)"#),
        Value::Array(vec![Value::Int(1), Value::Null])
    );
    assert_eq!(
        ok(r#"[{a: 1}, {b: 2}, {b: 3}] | count_by(fn(r) => r.a) | first | fn(r) => r.count"#),
        Value::Int(2)
    );
}

// ---- A4: group_by Key ----

#[test]
fn group_by_has_a_typed_key_beside_the_text_name() {
    assert_eq!(
        ok(r#"[{n: 1}, {n: 2}, {n: 1}] | group_by("n") | map(fn(g) => g.Key)"#),
        Value::Array(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        ok(r#"[{n: 1}, {n: 2}] | group_by("n") | map(fn(g) => g.Name)"#),
        strs(&["1", "2"])
    );
    assert_eq!(
        ok(r#"[1, 2] | group_by(fn(x) => null) | map(fn(g) => g.Key == null)"#),
        Value::Array(vec![Value::Bool(true)])
    );
    assert_eq!(
        ok(r#"[1, 2] | group_by(fn(x) => null) | map(fn(g) => g.Name)"#),
        strs(&["<null>"])
    );
}

// ---- A5: hints ----

#[test]
fn reduce_given_a_builtin_says_it_takes_a_two_argument_lambda() {
    let e = refused("[1, 2, 3] | reduce(max, 0)");
    assert!(
        e.hint
            .contains("reduce takes a two-argument lambda, e.g. reduce(fn(a, b) => "),
        "{}",
        e.hint
    );
}

#[test]
fn map_given_a_module_says_it_is_a_module() {
    let e = refused("[1, 2, 3] | map(str)");
    assert!(e.hint.contains("`str` is a module"), "{}", e.hint);
    assert!(e.hint.contains("str."), "{}", e.hint);
    assert!(e.hint.contains("lambda"), "{}", e.hint);
}

#[test]
fn a_builtin_callback_missing_an_argument_says_to_use_a_lambda() {
    let e = refused(r#"["a,b"] | map(split)"#);
    assert!(
        e.hint
            .contains("a builtin callback receives only the element"),
        "{}",
        e.hint
    );
}

// ---- A6: to_json indent ----

#[test]
fn to_json_takes_a_bool_or_a_non_negative_int_only() {
    assert_eq!(
        ok(r#"to_json({a: 1}, true)"#),
        Value::Str("{\n  \"a\": 1\n}".to_string())
    );
    assert_eq!(
        ok(r#"to_json({a: 1}, false)"#),
        Value::Str("{\"a\":1}".to_string())
    );
    assert_eq!(
        ok(r#"to_json({a: 1}, 4)"#),
        Value::Str("{\n    \"a\": 1\n}".to_string())
    );
    assert_eq!(ok(r#"to_json({a: 1}, 16) | lines | len"#), Value::Int(3));
    for src in [
        r#"to_json({a: 1}, "pretty")"#,
        r#"to_json({a: 1}, "true")"#,
        r#"to_json({a: 1}, -1)"#,
        r#"to_json({a: 1}, 17)"#,
        r#"to_json({a: 1}, 1.5)"#,
        r#"to_json({a: 1}, null)"#,
    ] {
        assert_eq!(refused(src).code.as_str(), "E_BAD_ARG", "{src}");
    }
}

// ---- A7: AECON quoting ----

#[test]
fn aecon_quotes_a_string_that_reads_as_a_radix_number() {
    let dir: PathBuf = std::env::temp_dir().join(format!("ish_r50a_aecon_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    for cell in ["0x1", "0o7", "0b1"] {
        let code = format!(r#"[{{s: "{cell}"}}, {{s: "zz"}}]"#);
        let out = Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["--agent", "-c", &code])
            .current_dir(&dir)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", dir.join("session"))
            .env_remove("IRONSHELL_DETERMINISTIC")
            .output()
            .expect("run ish");
        let text = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n");
        assert!(text.contains(&format!("\"{cell}\"")), "{cell}: {text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- A8: Lambda is in the annotation list ----

#[test]
fn the_annotation_hints_list_lambda() {
    let e = refused("let x: Foo = 1");
    assert!(e.hint.contains("Record, Lambda, Null"), "{}", e.hint);
    assert_eq!(ok("let f: Lambda = fn(x) => x; f(3)"), Value::Int(3));
}

//! Round 10, agent A: regex edits that match empty text, an empty file_insert,
//! malformed `\u` escapes, unclosed brackets, the hints for a block that holds
//! statements and for `??`, a builtin read as a module, and where a failing
//! lambda was written. The undo cases are in round9a_undo_hand_edits.rs.

use iron_shell::parser::ParseError;
use iron_shell::value::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn run_err(src: &str) -> String {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    match iron_shell::eval::eval_program(&stmts, &mut env) {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.to_string(),
    }
}

/// The parse error for `src`, which must not parse.
fn parse_err(src: &str) -> anyhow::Error {
    match iron_shell::parser::parse_program(src) {
        Ok(_) => panic!("expected a parse error for {src:?}"),
        Err(e) => e,
    }
}

/// The `ParseError` behind the parse failure of `src`.
fn parse_error(src: &str) -> ParseError {
    let e = parse_err(src);
    e.downcast_ref::<ParseError>()
        .cloned()
        .unwrap_or_else(|| panic!("not a ParseError: {e:#}"))
}

/// A scratch directory of this test's own, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae_r10a_fixes_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        Scratch(dir)
    }

    fn file(&self, name: &str, text: &str) -> String {
        let p = self.0.join(name);
        std::fs::write(&p, text).expect("seed");
        p.to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn call(name: &str, args: Vec<Value>) -> Result<Value, String> {
    let mut env = iron_shell::env::Env::new();
    iron_shell::builtins::call(name, args, &mut env).map_err(|e| e.to_string())
}

fn s(text: &str) -> Value {
    Value::Str(text.to_string())
}

/// `{file: true}`, the file mode of sed.
fn file_mode() -> Value {
    Value::Record(BTreeMap::from([("file".to_string(), Value::Bool(true))]))
}

/// Runs the ish binary on `script`, with `dir` as its working directory.
/// Returns the exit code and stderr.
fn run_bin(dir: &Path, script: &str) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .arg(script)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("run ish");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

// ---- 1. A regex that matches empty text is refused in file mode ----

#[test]
fn sed_file_mode_refuses_a_pattern_that_matches_empty_text_and_writes_nothing() {
    let dir = Scratch::new("sed_empty");
    let p = dir.file("t1.txt", "alpha\nbeta\ngamma");
    for pattern in ["x*", "(?:)"] {
        let err = call("sed", vec![s(pattern), s("y"), s(&p), file_mode()])
            .expect_err("an empty-matching pattern is refused");
        assert!(err.contains("E_BAD_ARG"), "{err}");
        assert!(
            err.contains(&format!(
                "sed: the pattern `{pattern}` matches empty text, which would insert the replacement between every character"
            )),
            "{err}"
        );
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "alpha\nbeta\ngamma",
            "{pattern} must not write the file"
        );
    }
}

#[test]
fn the_empty_matching_refusal_names_sed_when_sed_replace_was_called() {
    let dir = Scratch::new("sed_alias");
    let p = dir.file("t2.txt", "one\ntwo");
    let err = call("sed_replace", vec![s("x*"), s("y"), s(&p), file_mode()]).expect_err("refused");
    assert!(err.contains("sed: the pattern `x*`"), "{err}");
    assert!(!err.contains("sed_replace"), "{err}");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "one\ntwo");
}

#[test]
fn sed_file_mode_still_edits_with_a_pattern_that_needs_text() {
    let dir = Scratch::new("sed_ok");
    let p = dir.file("t3.txt", "aaa");
    call("sed", vec![s("a+"), s("x"), s(&p), file_mode()]).expect("edits");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "x");
}

// ---- 3. An empty file_insert is refused ----

#[test]
fn file_insert_of_empty_content_is_refused_and_leaves_the_file_alone() {
    let dir = Scratch::new("insert_empty");
    let p = dir.file("f.txt", "one\ntwo\n");
    let err = call("file_insert", vec![s(&p), s("end"), s("")]).expect_err("refused");
    assert!(err.contains("file_insert: nothing to insert"), "{err}");
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "one\ntwo\n");
}

// ---- 4. Malformed \u escapes are E_PARSE ----

#[test]
fn a_malformed_unicode_escape_is_a_parse_error_with_its_position() {
    for (src, escape) in [
        (r#""\u00zz""#, r"\u00zz"),
        (r#""\uD800""#, r"\uD800"),
        (r#""\u{110000}""#, r"\u{110000}"),
        (r#""\u{}""#, r"\u{}"),
        (r#""\u12""#, r"\u12"),
        (r#""\u{D800}""#, r"\u{D800}"),
    ] {
        let pe = parse_error(src);
        assert!(
            pe.message.starts_with(&format!(
                "invalid unicode escape `{escape}` at line 1, column 2"
            )),
            "{src}: {}",
            pe.message
        );
        assert_eq!((pe.line, pe.column), (Some(1), Some(2)), "{src}");
        assert!(pe.hint.contains("four hex digits"), "{src}: {}", pe.hint);
    }
}

#[test]
fn well_formed_unicode_escapes_and_unknown_escapes_still_work() {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program("\"\\u0041\\u{42}\\q\"").expect("parses");
    let v = iron_shell::eval::eval_program(&stmts, &mut env).expect("runs");
    assert_eq!(v, Value::Str("AB\\q".to_string()));
}

#[test]
fn a_parse_error_carries_its_line_and_column_in_the_json_form() {
    let text = parse_err("x = 1\ny = \"\\u12\"").to_string();
    assert!(text.contains(r#""line":2,"column":6"#), "{text}");
}

// ---- 5. Unclosed brackets name their opening line ----

#[test]
fn an_unclosed_square_bracket_names_the_line_it_was_opened_on() {
    let pe = parse_error("a = [1,\n2,\n3\nb = 4");
    assert!(
        pe.message
            .contains("'[' opened at line 1 is not closed; expected ']'"),
        "{}",
        pe.message
    );
}

#[test]
fn an_unclosed_brace_names_the_line_it_was_opened_on() {
    let pe = parse_error("r = {a: 1,\nb: 2\n");
    assert!(
        pe.message
            .contains("'{' opened at line 1 is not closed; expected '}'"),
        "{}",
        pe.message
    );
}

// ---- 7. A block holds one expression, and the hint says so ----

#[test]
fn a_second_statement_in_a_block_gets_the_block_hint() {
    for src in [
        "if true { x = 1; x + 1 }",
        "if true { 1; 2 }",
        "f = fn() => { print(\"s\"); 42 }",
    ] {
        let pe = parse_error(src);
        assert!(
            pe.hint
                .contains("a block holds one expression; bind values before it"),
            "{src}: {}",
            pe.hint
        );
    }
}

// ---- 8. `??` and a builtin used as a module ----

#[test]
fn double_question_mark_is_named_as_no_operator() {
    let pe = parse_error("null ?? 1");
    assert!(
        pe.message
            .starts_with("`??` at line 1, column 6 is not an operator"),
        "{}",
        pe.message
    );
    assert!(pe.hint.contains("there is no ?? operator"), "{}", pe.hint);
}

#[test]
fn a_builtin_read_as_a_module_says_it_is_a_builtin() {
    let err = run_err("time.now()");
    assert!(
        err.contains("`time` is a builtin, not a module: call time()"),
        "{err}"
    );
}

// ---- 6. A failing lambda names where it was written ----

#[test]
fn a_failing_lambda_names_the_line_it_was_defined_on() {
    let dir = Scratch::new("lambda_line");
    let script = dir.file("main.ish", "bad = fn(x) => x + \"a\"\nk = 1\nr = bad(1)\n");
    let (code, err) = run_bin(&dir.0, &script);
    assert_ne!(code, 0, "{err}");
    assert!(
        err.contains("at line 3 (in `bad`, defined at line 1)"),
        "{err}"
    );
}

#[test]
fn a_failing_module_lambda_names_its_file() {
    let dir = Scratch::new("lambda_module");
    std::fs::create_dir_all(dir.0.join("lib")).expect("lib dir");
    std::fs::write(
        dir.0.join("lib").join("bad.ish"),
        "pub let h = 1\npub let g = fn(x) => x + \"a\"\n",
    )
    .expect("module");
    let script = dir.file(
        "main.ish",
        "import { g } from \"lib/bad.ish\"\nx = 1\nr = g(1)\n",
    );
    let (code, err) = run_bin(&dir.0, &script);
    assert_ne!(code, 0, "{err}");
    assert!(err.contains("at line 3 (in `g`, defined at "), "{err}");
    assert!(err.contains("bad.ish line 2)"), "{err}");
}

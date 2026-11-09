//! Round 10b: the agent-tester findings on the round-9 builtins and hints.
//!
//! One test group per item. In-process tests evaluate source through the same
//! entry points the other contract tests use; the CLI tests spawn the binary
//! with a scratch directory, for the behaviour that reads a file.

use iron_shell::safety::SafetyError;
use iron_shell::value::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Output};

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

/// The structured error a source line fails with.
fn run_err(src: &str) -> SafetyError {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    let err = iron_shell::eval::eval_program(&stmts, &mut env).expect_err("fails");
    err.downcast_ref::<SafetyError>()
        .unwrap_or_else(|| panic!("not a structured error: {err:#}"))
        .clone()
}

fn text(v: Value) -> String {
    match v {
        Value::Str(s) => s,
        other => panic!("expected a String, got {other:?}"),
    }
}

// 1. regex_find and regex_find_all carry the named groups.

#[test]
fn regex_find_names_its_groups() {
    assert_eq!(
        text(run(
            r#"regex_find("2024-02", "(?P<y>[0-9]+)-(?P<m>[0-9]+)").named.m"#
        )),
        "02"
    );
    assert_eq!(
        text(run(
            r#"regex_find("2024-02", "(?P<y>[0-9]+)-(?P<m>[0-9]+)").named.y"#
        )),
        "2024"
    );
    // The positional groups are still there.
    assert_eq!(
        run(r#"regex_find("2024-02", "(?P<y>[0-9]+)-(?P<m>[0-9]+)").groups | len"#),
        Value::Int(2)
    );
}

#[test]
fn a_pattern_without_named_groups_has_an_empty_named_record() {
    match run(r#"regex_find("ab", "b").named"#) {
        Value::Record(m) => assert!(m.is_empty(), "expected an empty record, got {m:?}"),
        other => panic!("expected a Record, got {other:?}"),
    }
}

#[test]
fn a_named_group_that_did_not_take_part_is_null() {
    assert_eq!(run(r#"regex_find("a", "(?P<x>b)?a").named.x"#), Value::Null);
}

#[test]
fn regex_find_all_records_carry_named_groups() {
    assert_eq!(
        text(run(
            "x = regex_find_all(\"1a2\", \"(?P<d>[0-9])\")\nx[1].named.d"
        )),
        "2"
    );
}

// 2. The size cap: a refusal quotes a short prefix and names the cap.

#[test]
fn a_huge_pattern_is_quoted_briefly_and_the_cap_is_named() {
    let pattern = format!("({}", "a".repeat(100_000));
    let mut env = iron_shell::modules::env_with_modules();
    let err = iron_shell::builtins::call(
        "regex_find",
        vec![Value::Str("x".into()), Value::Str(pattern)],
        &mut env,
    )
    .expect_err("an unclosed group is refused");
    let se = err
        .downcast_ref::<SafetyError>()
        .expect("structured refusal");
    assert!(
        se.message.contains("(100001 characters)"),
        "message: {}",
        se.message
    );
    assert!(
        se.message.len() < 400,
        "the message quotes the whole pattern: {} bytes",
        se.message.len()
    );
    assert!(se.hint.contains("1 MiB"), "hint: {}", se.hint);
}

// 3. "\${" is a literal; "${name}" in a regex replacement is interpolation.

#[test]
fn an_escaped_dollar_brace_is_literal_text() {
    assert_eq!(text(run(r#""\${d}""#)), "${d}");
}

#[test]
fn a_named_group_replacement_is_written_with_an_escape() {
    assert_eq!(
        text(run(
            r#"regex_replace("2024-02", "(?P<y>[0-9]+)-(?P<m>[0-9]+)", "\${m}/\${y}")"#
        )),
        "02/2024"
    );
}

#[test]
fn interpolation_still_works_beside_an_escape() {
    assert_eq!(text(run("x = 1\n\"v${x} \\${x}\"")), "v1 ${x}");
}

#[test]
fn an_unknown_name_in_a_hole_says_how_to_write_a_literal() {
    let e = run_err(r#""${d}""#);
    assert_eq!(
        e.hint,
        r"${...} interpolates in strings; write \${ for a literal ${"
    );
}

// 4. date_parse: a format that does not match is named as the cause, and the
// no-format forms accept a space or no zone.

#[test]
fn a_date_that_misses_its_format_names_the_format() {
    let e = run_err(r#"date_parse("2024-02-29T10:00:00Z", "%Y")"#);
    assert!(
        e.message
            .contains("date_parse: `2024-02-29T10:00:00Z` does not match the format `%Y`"),
        "message: {}",
        e.message
    );
    assert!(e.hint.contains("chrono strftime"), "hint: {}", e.hint);
}

#[test]
fn date_parse_reads_a_space_or_no_zone_as_utc() {
    assert_eq!(
        text(run(r#"date_parse("2024-02-29 10:00:00")"#)),
        "2024-02-29T10:00:00Z"
    );
    assert_eq!(
        text(run(r#"date_parse("2024-02-29T10:00:00")"#)),
        "2024-02-29T10:00:00Z"
    );
}

// 5. Invalid TOML: not retryable, and the hint names the position or the range.

#[test]
fn an_integer_past_64_bits_is_named_as_out_of_range() {
    let e = run_err(r#"from_toml("n = 9223372036854775808")"#);
    assert!(
        e.hint.contains("out of the 64-bit range") && e.hint.contains("line 1"),
        "hint: {}",
        e.hint
    );
    assert!(!e.is_retryable());
}

#[test]
fn a_toml_syntax_error_gives_its_line_and_column() {
    let e = run_err(r#"from_toml("a = 1\nb = = 2")"#);
    assert!(
        e.hint.starts_with("fix the TOML at line 2, column "),
        "hint: {}",
        e.hint
    );
    assert!(!e.is_retryable());
}

fn scratch(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("ae-round10b-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("cwd")).unwrap();
    std::fs::create_dir_all(root.join("session")).unwrap();
    root
}

fn ish_in(root: &PathBuf, src: &str) -> Output {
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
fn open_of_invalid_toml_is_not_retryable_and_gives_the_position() {
    let root = scratch("toml");
    std::fs::write(root.join("cwd").join("bad.toml"), "a = 1\nb = = 2\n").unwrap();
    let hint = ish_in(&root, r#"try { open("bad.toml") } catch e { e.hint }"#);
    let out = String::from_utf8_lossy(&hint.stdout).to_string();
    assert!(
        out.contains("fix the TOML at line 2, column"),
        "stdout: {out}"
    );
    let retry = ish_in(&root, r#"try { open("bad.toml") } catch e { e.retryable }"#);
    assert_eq!(String::from_utf8_lossy(&retry.stdout).trim(), "false");
    let _ = std::fs::remove_dir_all(&root);
}

// 6. A top-level builtin reached through a module names how to call it.

#[test]
fn a_top_level_builtin_as_a_module_member_says_how_to_call_it() {
    let e = run_err(r#"str.slice("abc", 1)"#);
    assert_eq!(
        e.hint,
        "`slice` is a top-level builtin, not a member of str: call slice(s, ...) or s | slice(...)"
    );
}

// 7. Did-you-mean names the builtin a misspelling was meant as.

fn suggestions(name: &str) -> Vec<String> {
    iron_shell::builtins::did_you_mean(name)
}

#[test]
fn the_date_spellings_suggest_the_date_builtins() {
    assert!(suggestions("parse_date").contains(&"date_parse".to_string()));
    assert!(suggestions("strptime").contains(&"date_parse".to_string()));
    assert!(suggestions("strftime").contains(&"date_format".to_string()));
}

#[test]
fn the_regex_spellings_suggest_the_regex_builtins() {
    for name in ["re_find", "re_search", "regex_search"] {
        assert!(
            suggestions(name).contains(&"regex_find".to_string()),
            "{name}: {:?}",
            suggestions(name)
        );
    }
    assert!(suggestions("re_match").contains(&"regex_match".to_string()));
}

#[test]
fn the_padding_spellings_suggest_the_pad_builtins() {
    assert!(suggestions("lpad").contains(&"pad_left".to_string()));
    assert!(suggestions("rjust").contains(&"pad_left".to_string()));
    assert!(suggestions("rpad").contains(&"pad_right".to_string()));
    assert!(suggestions("ljust").contains(&"pad_right".to_string()));
}

#[test]
fn the_toml_spellings_suggest_the_toml_builtins() {
    assert!(suggestions("toml").contains(&"from_toml".to_string()));
    assert!(suggestions("parse_toml").contains(&"from_toml".to_string()));
}

#[test]
fn raise_suggests_error_then_throw() {
    let s = suggestions("raise");
    assert_eq!(s.first().map(String::as_str), Some("error"));
    assert!(s.contains(&"throw".to_string()), "{s:?}");
}

#[test]
fn a_padding_spelling_through_a_module_suggests_the_pad_builtin() {
    let e = run_err(r#"str.rjust("7", 3, "0")"#);
    assert!(
        e.did_you_mean.contains(&"pad_left".to_string()),
        "did_you_mean: {:?}",
        e.did_you_mean
    );
}

// 8. Wrong hints: a builtin's own hint is kept, and a type list is de-duplicated.

#[test]
fn slice_of_null_names_its_subject_and_its_types() {
    let e = run_err("slice(null, 1)");
    assert_eq!(
        e.message,
        "slice takes a String or Array and Int indices; got Null"
    );
    assert!(!e.hint.contains("Int or Int"), "hint: {}", e.hint);
}

#[test]
fn file_insert_at_line_zero_keeps_its_own_hint() {
    let root = scratch("insert");
    std::fs::write(root.join("cwd").join("f.txt"), "one\ntwo\n").unwrap();
    let out = ish_in(
        &root,
        r#"try { file_insert("f.txt", 0, "x") } catch e { e.hint }"#,
    );
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        text.contains("use 1 to insert before the first line"),
        "stdout: {text}; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_record_of_named_groups_is_a_record() {
    // The named record is a Record in its own right, so it can be piped.
    let v = run(r#"regex_find("2024-02", "(?P<y>[0-9]+)-(?P<m>[0-9]+)").named"#);
    let mut expected = BTreeMap::new();
    expected.insert("m".to_string(), Value::Str("02".into()));
    expected.insert("y".to_string(), Value::Str("2024".into()));
    assert_eq!(v, Value::Record(expected));
}

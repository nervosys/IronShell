//! Round 26: a CSV header that names a column twice is refused rather than
//! dropping a column, and the same holds for open and sql. Each test runs in
//! process through the parser and evaluator.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

/// The message of a call that must be refused.
fn refusal(src: &str) -> String {
    eval(src).expect_err(src).to_string()
}

/// A temp directory holding one file, and that file's path with forward slashes.
fn temp_file(tag: &str, name: &str, text: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("ae_round26_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    let p = path.to_string_lossy().replace('\\', "/");
    (dir, p)
}

// ── item 1: a duplicate CSV header is refused ───────────────────────────

#[test]
fn from_csv_refuses_a_header_that_names_a_column_twice() {
    let text = refusal(r#"from_csv("a,a\n1,2\n")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(
        text.contains("duplicate CSV column 'a' at columns 1 and 2"),
        "{text}"
    );
    assert!(text.contains("rename one column"), "{text}");
}

#[test]
fn from_csv_names_the_duplicate_and_its_later_column_number() {
    let text = refusal(r#"from_csv("id,name,name\n1,x,y\n")"#);
    assert!(
        text.contains("duplicate CSV column 'name' at columns 2 and 3"),
        "{text}"
    );
}

#[test]
fn from_csv_refuses_a_duplicate_header_even_with_no_rows() {
    let text = refusal(r#"from_csv("a,a\n")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
}

#[test]
fn from_csv_matches_header_names_exactly_so_case_differences_are_two_columns() {
    assert_eq!(
        eval(r#"from_csv("a,A\n1,2\n") | first | fn(r) => r.A"#).unwrap(),
        Value::Int(2)
    );
}

#[test]
fn open_refuses_a_csv_file_with_a_duplicate_header() {
    let (dir, p) = temp_file("dupopen", "dup.csv", "id,name,name\n1,x,y\n");
    let got = refusal(&format!(r#"open("{p}")"#));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(got.contains("E_BAD_ARG"), "{got}");
    assert!(
        got.contains("duplicate CSV column 'name' at columns 2 and 3"),
        "{got}"
    );
}

#[test]
fn sql_refuses_a_csv_file_with_a_duplicate_header() {
    let (dir, p) = temp_file("dupsql", "dup.csv", "a,a\n1,2\n");
    let got = refusal(&format!(r#"sql_value("{p}", "select count(*) from dup")"#));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(got.contains("E_BAD_ARG"), "{got}");
    assert!(
        got.contains("duplicate CSV column 'a' at columns 1 and 2"),
        "{got}"
    );
}

// ── item 3: a JSON lines error names the file's line ────────────────────

fn jsonl_refusal(tag: &str, text: &str) -> String {
    let (dir, p) = temp_file(tag, "bad.jsonl", text);
    let got = refusal(&format!(r#"open("{p}")"#));
    let _ = std::fs::remove_dir_all(&dir);
    got
}

#[test]
fn a_bad_jsonl_record_reports_its_file_line_not_its_line_in_the_record() {
    let got = jsonl_refusal("jl_line2", "{\"a\": 1}\n{\"a\" 2}\n");
    assert!(got.contains("E_BAD_VALUE"), "{got}");
    assert!(
        got.contains("is not valid JSON lines: line 2, column "),
        "{got}"
    );
    assert!(!got.contains("at line 1"), "{got}");
}

#[test]
fn blank_lines_before_a_bad_jsonl_record_count_toward_its_line() {
    let got = jsonl_refusal("jl_blank", "\n  \n{\"a\": 1}\n{\"a\" 2}\n");
    assert!(
        got.contains("is not valid JSON lines: line 4, column "),
        "{got}"
    );
    assert!(!got.contains("line 1,"), "{got}");
}

#[test]
fn a_leading_blank_line_moves_the_reported_line_by_one() {
    let got = jsonl_refusal("jl_one_blank", "\n{\"a\": 1}\n{\"a\" 2}\n");
    assert!(
        got.contains("is not valid JSON lines: line 3, column "),
        "{got}"
    );
}

#[test]
fn a_clean_jsonl_file_still_reads_every_record() {
    let (dir, p) = temp_file("jl_clean", "ok.jsonl", "{\"a\": 1}\n\n{\"a\": 2}\n");
    let got = eval(&format!(r#"open("{p}") | len"#));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got.unwrap(), Value::Int(2));
}

// ── item 2: a replacement may only name a group the pattern has ─────────

#[test]
fn regex_replace_refuses_a_group_number_the_pattern_does_not_have() {
    let text = refusal(r#"regex_replace("a1", "([0-9])", "<$5>")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(
        text.contains("refers to group '5', which the pattern does not have"),
        "{text}"
    );
    assert!(text.contains("the pattern has 1 group(s): 1"), "{text}");
}

#[test]
fn regex_replace_refuses_a_name_that_is_a_group_number_and_letters() {
    let text = refusal(r#"regex_replace("a1", "([0-9])", "<$1x>")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("group '1x'"), "{text}");
    assert!(
        text.contains("write ${1}x to put x after group 1"),
        "{text}"
    );
}

#[test]
fn regex_replace_lists_the_named_groups_it_has() {
    let text = refusal(r#"regex_replace("2024-02", "(?P<y>[0-9]+)-(?P<m>[0-9]+)", "$yr")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(
        text.contains("the pattern has 2 group(s): 1 (y), 2 (m)"),
        "{text}"
    );
}

#[test]
fn regex_replace_refuses_any_reference_when_the_pattern_has_no_groups() {
    let text = refusal(r#"regex_replace("a", "a", "$1")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("no groups besides the whole match"), "{text}");
}

#[test]
fn regex_replace_valid_references_work_as_before() {
    assert_eq!(
        eval(r#"regex_replace("a1", "([0-9])", "<$1>")"#).unwrap(),
        Value::Str("a<1>".to_string())
    );
    assert_eq!(
        eval(r#"regex_replace("a1", "([0-9])", "[$0]")"#).unwrap(),
        Value::Str("a[1]".to_string())
    );
    assert_eq!(
        eval(r#"regex_replace("a1", "([0-9])", "<\${1}x>")"#).unwrap(),
        Value::Str("a<1x>".to_string())
    );
    assert_eq!(
        eval(r#"regex_replace("2024-02", "(?P<y>[0-9]+)-(?P<m>[0-9]+)", "\${m}/\${y}")"#).unwrap(),
        Value::Str("02/2024".to_string())
    );
}

#[test]
fn regex_replace_double_dollar_and_a_bare_dollar_are_literal() {
    assert_eq!(
        eval(r#"regex_replace("a1", "([0-9])", "$$1")"#).unwrap(),
        Value::Str("a$1".to_string())
    );
    assert_eq!(
        eval(r#"regex_replace("a1", "([0-9])", "$ 5")"#).unwrap(),
        Value::Str("a$ 5".to_string())
    );
}

#[test]
fn regex_replace_refusal_is_caught_as_e_bad_arg() {
    assert_eq!(
        eval(r#"(try { regex_replace("a1", "([0-9])", "<$5>") } catch e { e.error.code })"#)
            .unwrap(),
        Value::Str("E_BAD_ARG".to_string())
    );
}

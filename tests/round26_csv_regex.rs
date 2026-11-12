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

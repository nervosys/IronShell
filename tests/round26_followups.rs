//! Round 26 follow-ups: the paths the round's fixes left behind.
//!
//! - `to_csv` wrote a NaN inside a nested cell as 0, as `to_json` did.
//! - A YAML quoted scalar followed by a comment kept its quotes.
//! - `db_csv_query` let a repeated header replace the earlier column.
//! - `sql` over a JSON lines file placed a bad record at "line 1" after
//!   naming the file's own line.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_round26f_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn slash(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[test]
fn to_csv_refuses_a_nan_inside_a_nested_cell() {
    // No ish reader makes a NaN any more, so bind one built in Rust.
    let mut env = Env::new();
    env.set_var(
        "v",
        Value::Array(vec![Value::Float(1.0), Value::Float(f64::NAN)]),
    )
    .expect("binds");
    let stmts = parse_program(r#"[{a: v}] | to_csv"#).expect("parse");
    let text = eval_program(&stmts, &mut env)
        .expect_err("a NaN in a nested cell")
        .to_string();
    assert!(text.contains("E_BAD_VALUE"), "{text}");
    assert!(text.contains("rows[0].a"), "{text}");
}

#[test]
fn to_csv_still_writes_a_nested_cell_as_json() {
    let got = eval(r#"to_csv([{a: [1, 2]}]) | lines | last"#).unwrap();
    assert_eq!(got, Value::Str("\"[1,2]\"".into()));
}

#[test]
fn a_quoted_yaml_scalar_followed_by_a_comment_drops_the_comment() {
    let got = eval(r#"from_yaml("a: \"x\" # note").a"#).unwrap();
    assert_eq!(got, Value::Str("x".into()));
    let got = eval(r#"from_yaml("a: 'it''s' # note").a"#).unwrap();
    assert_eq!(got, Value::Str("it's".into()));
    // A `#` inside the quotes is data.
    let got = eval(r#"from_yaml("a: \"x # y\"").a"#).unwrap();
    assert_eq!(got, Value::Str("x # y".into()));
}

#[test]
fn db_csv_query_refuses_a_repeated_header() {
    let dir = temp_dir("dbcsv");
    let path = dir.join("dup.csv");
    std::fs::write(&path, "a,a\n1,2\n").unwrap();
    let result = eval(&format!(r#"db_csv_query("{}")"#, slash(&path)));
    let _ = std::fs::remove_dir_all(&dir);
    let text = result.expect_err("duplicate header").to_string();
    assert!(text.contains("duplicate CSV column 'a'"), "{text}");
}

#[test]
fn sql_over_json_lines_gives_the_file_line_and_the_column() {
    let dir = temp_dir("sqljsonl");
    let path = dir.join("rows.jsonl");
    std::fs::write(&path, "{\"a\": 1}\n\n{\"a\": \n").unwrap();
    let result = eval(&format!(
        r#"sql_value("{}", "select count(*) from t")"#,
        slash(&path)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let text = result.expect_err("bad record").to_string();
    assert!(text.contains("(line 3)"), "{text}");
    assert!(text.contains("column "), "{text}");
    assert!(!text.contains("at line 1"), "{text}");
}

//! Round 22: CSV column typing, to_csv's column union, csv_columns' file
//! check, sql's argument messages, enumerate, grep line numbers, JSON indent
//! and json_set, and did-you-mean spellings. Each test runs in process through
//! the parser and evaluator.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

fn s(v: &str) -> Value {
    Value::Str(v.to_string())
}

// ── item 1: whole-column CSV typing ─────────────────────────────────────

#[test]
fn a_leading_zero_code_makes_its_whole_column_text() {
    assert_eq!(
        eval(r#"from_csv("n\n007\nNA\n12\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![s("007"), s("NA"), s("12")])
    );
    assert_eq!(
        eval(r#"from_csv("n\n007\n12\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![s("007"), s("12")])
    );
}

#[test]
fn a_leading_plus_makes_its_column_text() {
    assert_eq!(
        eval(r#"from_csv("n\n+5\n6\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![s("+5"), s("6")])
    );
}

#[test]
fn zero_and_signed_decimals_are_numbers() {
    assert_eq!(
        eval(r#"from_csv("n\n0\n-0.5\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![Value::Float(0.0), Value::Float(-0.5)])
    );
    assert_eq!(
        eval(r#"from_csv("n\n-01\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![s("-01")])
    );
}

#[test]
fn a_column_of_integers_is_int_and_a_mixed_one_is_float() {
    assert_eq!(
        eval(r#"from_csv("n\n1\n2\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        eval(r#"from_csv("n\n1\n2.5\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![Value::Float(1.0), Value::Float(2.5)])
    );
}

#[test]
fn a_blank_in_a_text_column_stays_an_empty_string_and_in_a_numeric_one_is_null() {
    assert_eq!(
        eval(r#"from_csv("n,s\n007,\n8,x\n") | map(fn(r) => r.s)"#).unwrap(),
        Value::Array(vec![s(""), s("x")])
    );
    // The 007 is in another column: the blank and the 2 are a numeric column.
    assert_eq!(
        eval(r#"from_csv("n,m\n007,\n8,2\n") | map(fn(r) => r.n)"#).unwrap(),
        Value::Array(vec![s("007"), s("8")])
    );
    assert_eq!(
        eval(r#"from_csv("n,m\n1,\n8,2\n") | map(fn(r) => r.m)"#).unwrap(),
        Value::Array(vec![Value::Null, Value::Int(2)])
    );
}

// ── item 2: to_csv keeps every key ──────────────────────────────────────

#[test]
fn to_csv_keeps_a_key_that_only_a_later_row_has() {
    assert_eq!(
        eval(r#"[{id: 1}, {id: 2, customer: "Ada"}] | to_csv"#).unwrap(),
        s("customer,id\n,1\nAda,2\n")
    );
    assert_eq!(
        eval(r#"to_csv([{a: 1}, {b: 2}, {a: 3, c: 4}])"#).unwrap(),
        s("a,b,c\n1,,\n,2,\n3,,4\n")
    );
}

// ── item 3: a file name that is not a file ──────────────────────────────

#[test]
fn csv_columns_refuses_a_one_line_file_name_that_is_not_a_file() {
    for name in ["nofile.csv", "nofile.TSV", "nofile.txt"] {
        let e = eval(&format!(r#"csv_columns("{name}")"#)).expect_err(name);
        let text = e.to_string();
        assert!(text.contains("E_NOT_FOUND"), "{text}");
        assert!(text.contains(name), "{text}");
        assert!(
            text.contains("pass CSV text with a newline, or an existing file"),
            "{text}"
        );
    }
}

// ── item 4: sql's argument messages and rows as the first argument ──────

#[test]
fn sql_with_params_and_no_rows_runs_over_the_empty_database() {
    // Round 24 lets a lone query take params; the table `t` is absent, so the
    // refusal is SQLite's, not a demand for a source.
    let e = eval(r#"sql("select name from t where name = ?", ["Ann"])"#).expect_err("no table t");
    let text = e.to_string();
    assert!(!text.contains("needs a source first"), "{text}");
    assert!(text.contains("no such "), "{text}");
    assert!(!text.contains("source_or_query"), "{text}");
}

#[test]
fn sql_params_that_are_not_an_array_are_named_as_the_mistake() {
    let dir = std::env::temp_dir().join(format!("ae_round22_sql_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("people.csv");
    std::fs::write(&path, "name\nAnn\nBo\n").unwrap();
    let p = path.to_string_lossy().replace('\\', "/");
    let got = eval(&format!(
        r#"sql("{p}", "select name from people where name = ?", "Ann")"#
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let text = got.expect_err("params as a String").to_string();
    assert!(text.contains("params must be an Array"), "{text}");
    assert!(text.contains("pass the bind values as an Array"), "{text}");
}

#[test]
fn sql_and_sql_value_accept_rows_as_the_first_argument() {
    assert_eq!(
        eval(r#"sql([{n: 1}, {n: 2}, {n: 3}], "select n from t where n > ?", [1]) | len"#).unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        eval(r#"sql_value([{n: 1}, {n: 2}, {n: 3}], "select count(*) from t where n > ?", [1])"#)
            .unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        eval(r#"sql_value([{n: 1}, {n: 2}, {n: 3}], "select max(n) from t")"#).unwrap(),
        Value::Int(3)
    );
}

#[test]
fn csv_columns_still_reads_one_line_text_and_multi_line_text() {
    assert_eq!(
        eval(r#"csv_columns("id,customer")"#).unwrap(),
        Value::Array(vec![s("id"), s("customer")])
    );
    assert_eq!(
        eval(r#"csv_columns("name\nAda\n")"#).unwrap(),
        Value::Array(vec![s("name")])
    );
}

// ── item 5: enumerate and grep line numbers ─────────────────────────────

/// A fresh directory for one test. The tag keeps parallel tests apart.
fn fresh_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_round22_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A path as a string the source can hold without escapes.
fn fwd(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[test]
fn enumerate_pairs_each_element_with_its_index_from_zero() {
    assert_eq!(
        eval(r#"enumerate(["a", "b"]) | last | fn(r) => r.index"#).unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        eval(r#"["a", "b"] | enumerate | last | fn(r) => r.value"#).unwrap(),
        s("b")
    );
    assert_eq!(eval("enumerate([]) | len").unwrap(), Value::Int(0));
}

#[test]
fn enumerate_refuses_what_is_not_an_array() {
    let e = eval("enumerate(5)").expect_err("an Int");
    assert!(e.to_string().contains("E_BAD_ARG"), "{e}");
}

#[test]
fn grep_without_options_still_returns_lines() {
    assert_eq!(
        eval(r#""a\nb\na" | grep("a") | len"#).unwrap(),
        Value::Int(2)
    );
    assert_eq!(eval(r#""a\nb\na" | grep("a") | first"#).unwrap(), s("a"));
}

#[test]
fn grep_line_numbers_returns_records_numbered_from_one() {
    assert_eq!(
        eval(r#""a\nb\na" | grep("a", {line_numbers: true}) | map(fn(r) => r.line)"#).unwrap(),
        Value::Array(vec![Value::Int(1), Value::Int(3)])
    );
    assert_eq!(
        eval(r#""a\nb\na" | grep("a", {line_numbers: true}) | map(fn(r) => r.text)"#).unwrap(),
        Value::Array(vec![s("a"), s("a")])
    );
    // Piped text has no file.
    assert_eq!(
        eval(r#""a\nb\na" | grep("a", {line_numbers: true}) | first | fn(r) => r.file"#).unwrap(),
        Value::Null
    );
}

#[test]
fn grep_line_numbers_names_the_file_and_numbers_each_file_from_one() {
    let dir = fresh_dir("grep_lines");
    std::fs::write(dir.join("a.txt"), "x\ny\n").unwrap();
    std::fs::write(dir.join("b.txt"), "y\n").unwrap();
    let glob = format!("{}/*.txt", fwd(&dir));
    let got = eval(&format!(
        r#"grep("y", "{glob}", {{line_numbers: true}}) | map(fn(r) => r.line)"#
    ));
    let files = eval(&format!(
        r#"grep("y", "{glob}", {{line_numbers: true}}) | map(fn(r) => r.file)"#
    ));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        got.unwrap(),
        Value::Array(vec![Value::Int(2), Value::Int(1)])
    );
    let Value::Array(names) = files.unwrap() else {
        panic!("file names are an Array")
    };
    assert!(
        names
            .first()
            .is_some_and(|n| matches!(n, Value::Str(s) if s.ends_with("a.txt"))),
        "{names:?}"
    );
}

#[test]
fn grep_refuses_an_option_it_does_not_take() {
    let e = eval(r#""a" | grep("a", {colour: true})"#).expect_err("unknown option");
    assert!(e.to_string().contains("E_BAD_ARG"), "{e}");
    let e = eval(r#""a" | grep("a", {line_numbers: 1})"#).expect_err("not a Bool");
    assert!(e.to_string().contains("E_BAD_ARG"), "{e}");
}

// ── item 7: did-you-mean for delete spellings ───────────────────────────

#[test]
fn a_file_delete_spelling_is_suggested_as_rm() {
    let members = ["delete_lines", "copy", "move"];
    let got = iron_shell::builtins::module_member_candidates("file", "delete", &members);
    assert_eq!(got.first().map(String::as_str), Some("rm"), "{got:?}");
    for spelling in ["remove", "unlink", "file_delete", "delete_file"] {
        let got = iron_shell::builtins::did_you_mean(spelling);
        assert_eq!(
            got.first().map(String::as_str),
            Some("rm"),
            "{spelling}: {got:?}"
        );
    }
}

// ── item 6: JSON indent and json_set ────────────────────────────────────

#[test]
fn to_json_and_json_stringify_indent_by_the_given_spaces() {
    assert_eq!(eval(r#"to_json([1, 2], 2)"#).unwrap(), s("[\n  1,\n  2\n]"));
    assert_eq!(
        eval(r#"[1, 2] | to_json(2)"#).unwrap(),
        s("[\n  1,\n  2\n]")
    );
    assert_eq!(
        eval(r#"json_stringify({a: 1}, 4)"#).unwrap(),
        s("{\n    \"a\": 1\n}")
    );
    assert_eq!(eval(r#"to_json([1, 2], 0)"#).unwrap(), s("[1,2]"));
}

#[test]
fn an_indent_outside_zero_to_eight_is_refused() {
    for src in [r#"to_json([1], 9)"#, r#"json_stringify([1], -1)"#] {
        let e = eval(src).expect_err(src);
        assert!(e.to_string().contains("E_BAD_ARG"), "{src}: {e}");
    }
}

#[test]
fn json_set_keeps_key_order_and_four_space_indentation() {
    let dir = fresh_dir("json_four");
    let path = dir.join("c.json");
    let before = "{\n    \"name\": \"x\",\n    \"items\": [\n        {\n            \"name\": \"a\"\n        }\n    ]\n}\n";
    std::fs::write(&path, before).unwrap();
    let p = fwd(&path);
    let got = eval(&format!(r#"json_set("{p}", "items.0.name", "b")"#));
    let after = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    got.unwrap();
    assert_eq!(
        after,
        "{\n    \"name\": \"x\",\n    \"items\": [\n        {\n            \"name\": \"b\"\n        }\n    ]\n}\n"
    );
}

#[test]
fn json_set_keeps_a_tab_indented_file_tabbed() {
    let dir = fresh_dir("json_tab");
    let path = dir.join("c.json");
    std::fs::write(&path, "{\n\t\"a\": 1,\n\t\"b\": 2\n}\n").unwrap();
    let p = fwd(&path);
    eval(&format!(r#"json_set("{p}", "a", 5)"#)).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(after, "{\n\t\"a\": 5,\n\t\"b\": 2\n}\n");
}

#[test]
fn json_set_writes_a_minified_file_minified_and_creates_missing_keys() {
    let dir = fresh_dir("json_min");
    let path = dir.join("c.json");
    std::fs::write(&path, r#"{"z":1,"a":2}"#).unwrap();
    let p = fwd(&path);
    eval(&format!(r#"json_set("{p}", "c.d", 3)"#)).unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    // Key order is kept: z stays before a, and the new key goes last.
    assert_eq!(after, r#"{"z":1,"a":2,"c":{"d":3}}"#);
}

#[test]
fn json_set_refuses_to_create_through_a_non_object_and_leaves_the_file() {
    let dir = fresh_dir("json_refuse");
    let path = dir.join("c.json");
    let before = r#"{"a": "x"}"#;
    std::fs::write(&path, before).unwrap();
    let p = fwd(&path);
    let e = eval(&format!(r#"json_set("{p}", "a.b", 1)"#)).expect_err("a string");
    let after = std::fs::read_to_string(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(e.to_string().contains("E_BAD_ARG"), "{e}");
    assert!(e.to_string().contains("a.b"), "{e}");
    assert_eq!(after, before);
}

#[test]
fn json_set_refuses_a_missing_array_index_and_a_missing_file() {
    let dir = fresh_dir("json_index");
    let path = dir.join("c.json");
    std::fs::write(&path, r#"{"items": []}"#).unwrap();
    let p = fwd(&path);
    let e = eval(&format!(r#"json_set("{p}", "items.3", 1)"#)).expect_err("no index 3");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(e.to_string().contains("E_BAD_ARG"), "{e}");
    let e = eval(r#"json_set("/nonexistent_ae_round22/none.json", "a", 1)"#).expect_err("missing");
    assert!(e.to_string().contains("E_NOT_FOUND"), "{e}");
}

#[test]
fn a_column_of_true_and_false_is_bool() {
    let v = eval(r#"from_csv("a,b\ntrue,x\n,y\nfalse,z") | map(fn(r) => r.a)"#).unwrap();
    assert_eq!(
        v,
        Value::Array(vec![Value::Bool(true), Value::Null, Value::Bool(false)])
    );
    let mixed = eval(r#"from_csv("a\ntrue\nmaybe") | map(fn(r) => r.a)"#).unwrap();
    assert_eq!(
        mixed,
        Value::Array(vec![Value::Str("true".into()), Value::Str("maybe".into())])
    );
}

#[test]
fn an_unknown_path_module_does_not_suggest_math() {
    let e = eval(r#"path.ext("a.txt")"#).expect_err("no path module");
    let text = format!("{e:#}");
    assert!(!text.contains("math"), "{text}");
    assert!(text.contains("path_exists"), "{text}");
    let e = eval(r#"path.exists("a.txt")"#).expect_err("no path module");
    assert!(format!("{e:#}").contains("path_exists"), "{e:#}");
}

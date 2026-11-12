//! Round 23: the data builtins' error texts and behaviour: grep's options and
//! pattern errors, sql's argument and source messages, sql with no source,
//! to_csv's number and naming, date unit spellings, CSV Bool columns, and
//! csv_columns on a directory. Each test runs in process through the parser
//! and evaluator.

use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

fn s(v: &str) -> Value {
    Value::Str(v.to_string())
}

// ── item 1: grep's options name the problem ─────────────────────────────

#[test]
fn grep_names_a_line_numbers_value_that_is_not_a_bool() {
    let e = eval(r#"grep("one", "notes.txt", {line_numbers: "yes"})"#).expect_err("not a Bool");
    let text = e.to_string();
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(
        text.contains("line_numbers must be a Bool, got String"),
        "{text}"
    );
}

#[test]
fn grep_names_an_unknown_option_and_the_one_it_takes() {
    let e = eval(r#"grep("one", "notes.txt", {bogus: true})"#).expect_err("unknown option");
    let text = e.to_string();
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(
        text.contains("unknown option bogus; grep takes line_numbers"),
        "{text}"
    );
}

// ── item 2: sql's params errors name the params, not the query ──────────

#[test]
fn sql_names_a_non_array_params_value_after_the_query() {
    for src in [
        r#"sql([{id: 1}], "select id from t where id = ?", 5)"#,
        r#"sql([{id: 1}], "select id from t where id = ?", {a: 1})"#,
        r#"[{id: 1}] | sql("select id from t where id = ?", 5)"#,
    ] {
        let text = eval(src).expect_err(src).to_string();
        assert!(text.contains("E_BAD_ARG"), "{src}: {text}");
        assert!(
            text.contains("params must be an Array, e.g. [5], got "),
            "{src}: {text}"
        );
        assert!(!text.contains("expected query"), "{src}: {text}");
    }
    let text = eval(r#"sql([{id: 1}], "select id from t where id = ?", {a: 1})"#)
        .expect_err("a Record")
        .to_string();
    assert!(text.contains("got Record"), "{text}");
}

#[test]
fn sql_with_an_array_of_params_still_binds_them() {
    assert_eq!(
        eval(r#"sql([{id: 1}, {id: 2}], "select id from t where id = ?", [2]) | len"#).unwrap(),
        Value::Int(1)
    );
}

// ── item 3: a JSON object is named as a Record, not as a piped one ──────

#[test]
fn sql_on_a_json_object_says_the_file_holds_a_record() {
    let dir = std::env::temp_dir().join(format!("ae_round23_sql_obj_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");
    std::fs::write(&path, r#"{"host": "a", "port": 8080}"#).unwrap();
    let p = path.to_string_lossy().replace('\\', "/");
    let got = eval(&format!(r#"sql("{p}", "select * from config")"#));
    let _ = std::fs::remove_dir_all(&dir);
    let text = got.expect_err("an object, not rows").to_string();
    assert!(text.contains("holds a Record, not rows"), "{text}");
    assert!(text.contains("JSON Array of records"), "{text}");
    assert!(!text.contains("piped"), "{text}");
}

#[test]
fn sql_on_a_json_object_of_arrays_still_queries_each_field() {
    let dir = std::env::temp_dir().join(format!("ae_round23_sql_arr_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("data.json");
    std::fs::write(&path, r#"{"people": [{"n": 1}, {"n": 2}]}"#).unwrap();
    let p = path.to_string_lossy().replace('\\', "/");
    let got = eval(&format!(
        r#"sql("{p}", "select sum(n) as total from people")"#
    ));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(
        got.unwrap(),
        Value::Array(vec![Value::Record(
            [("total".to_string(), Value::Int(3))].into_iter().collect()
        )])
    );
}

// ── item 4: a lone query runs over an empty database ────────────────────

#[test]
fn a_lone_query_runs_over_an_empty_database() {
    assert_eq!(
        eval(r#"sql("select 1 as x")"#).unwrap(),
        Value::Array(vec![Value::Record(
            [("x".to_string(), Value::Int(1))].into_iter().collect()
        )])
    );
    assert_eq!(eval(r#"sql_value("select 1 + 1")"#).unwrap(), Value::Int(2));
}

// ── item 5: to_csv is named with an underscore, and one wording ─────────

#[test]
fn to_csv_refusals_use_the_underscore_name_and_one_wording() {
    for src in [r#"[1, 2] | to_csv"#, r#"to_csv(5)"#] {
        let text = eval(src).expect_err(src).to_string();
        assert!(
            text.contains("to_csv: input must be an array of records"),
            "{src}: {text}"
        );
        assert!(!text.contains("to-csv"), "{src}: {text}");
    }
}

// ── item 6: a whole Float is written with its point ─────────────────────

#[test]
fn to_csv_writes_a_whole_float_with_a_point_and_reads_it_back_as_a_float() {
    assert_eq!(
        eval(r#"[{n: 1.0, m: 2}] | to_csv | lines | last"#).unwrap(),
        s("2,1.0")
    );
    assert_eq!(
        eval(r#"[{n: 1.0}] | to_csv | from_csv | first | fn(r) => r.n"#).unwrap(),
        Value::Float(1.0)
    );
}

// ── item 7: date units are case-insensitive and take the singular ───────

#[test]
fn date_add_accepts_any_case_and_the_singular_unit() {
    assert_eq!(
        eval(r#"date_add("2024-01-01", 1, "DAYS")"#).unwrap(),
        s("2024-01-02")
    );
    assert_eq!(
        eval(r#"date_add("2024-01-31", 1, "Month")"#).unwrap(),
        s("2024-02-29")
    );
    assert_eq!(
        eval(r#"date_add("2024-01-01T10:00:00Z", 2, "hour")"#).unwrap(),
        s("2024-01-01T12:00:00Z")
    );
}

#[test]
fn date_diff_accepts_any_case_and_the_singular_unit() {
    assert_eq!(
        eval(r#"date_diff("2024-02-29", "2024-03-02", "day")"#).unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        eval(r#"date_diff("2024-01-01", "2024-01-15", "WEEKS")"#).unwrap(),
        Value::Int(2)
    );
}

#[test]
fn an_unknown_date_unit_is_still_refused() {
    for src in [
        r#"date_add("2024-01-01", 1, "fortnights")"#,
        r#"date_diff("2024-01-01", "2024-01-15", "fortnight")"#,
    ] {
        let text = eval(src).expect_err(src).to_string();
        assert!(text.contains("E_BAD_ARG"), "{src}: {text}");
    }
}

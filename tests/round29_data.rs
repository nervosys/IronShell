//! Round 29: the sort, aggregate, SQL and did-you-mean edges an agent hits.
//!
//! - `sort_by` refuses a key it cannot order (Null, a Bool, a mixed kind) and
//!   a row without the named field, rather than returning input order.
//! - `sort_by`'s order is asc or desc in any case; anything else is refused.
//! - `min` and `max` take an array of Strings, and refuse a mix.
//! - `avg` of an empty array is refused, not NaN.
//! - `first` and `last` refuse a negative count, as `take` does.
//! - `skip`, `drop`, `rest`, `has` and `has_key` have did-you-mean names.
//! - `sql` refuses a result with two columns of one name.
//! - `sql`'s write refusal names ATTACH only for an ATTACH.
//! - `sql` names a text where the parameters belong.
//! - `db_sqlite_exec` names a syntax error as the SQL's fault. This one needs
//!   sqlite3 on PATH and is skipped without it.

use iron_shell::value::Value;

fn eval(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn refused(src: &str) -> String {
    match eval(src) {
        Ok(v) => panic!("expected a refusal for {src:?}, got {v:?}"),
        Err(e) => e.to_string(),
    }
}

fn ints(v: &[i64]) -> Value {
    Value::Array(v.iter().map(|i| Value::Int(*i)).collect())
}

// ── item 1: sort_by refuses keys it cannot order ─────────────────────────

#[test]
fn sort_by_refuses_a_null_key_and_names_the_row() {
    let text = refused("[{a: 3, i: 1}, {a: null, i: 2}, {a: 1, i: 3}] | sort_by(fn(r) => r.a)");
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("row 1's key is Null"), "{text}");
    assert!(
        text.contains("keys must all be numbers or all strings"),
        "{text}"
    );
}

#[test]
fn sort_by_refuses_a_string_among_ints() {
    let text = refused("[3, \"x\", 1] | sort_by(fn(r) => r)");
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("row 1's key is"), "{text}");
}

#[test]
fn sort_by_refuses_a_missing_field_naming_row_and_field() {
    let text = refused("[{a: 1}, {b: 2}] | sort_by(\"a\")");
    assert!(text.contains("E_UNKNOWN_FIELD"), "{text}");
    assert!(text.contains("row 1 has no field 'a'"), "{text}");
    assert!(text.contains("the record has: b"), "{text}");
}

#[test]
fn sort_by_still_sorts_when_every_key_is_orderable() {
    assert_eq!(
        eval("[{a: 2}, {a: 1.5}, {a: 3}] | sort_by(\"a\") | map(fn(r) => r.a)").unwrap(),
        Value::Array(vec![Value::Float(1.5), Value::Int(2), Value::Int(3)])
    );
}

// ── item 2: the order argument ───────────────────────────────────────────

#[test]
fn sort_by_accepts_desc_in_any_letter_case() {
    assert_eq!(
        eval("[{n: 1}, {n: 2}] | sort_by(\"n\", \"DESC\") | first | fn(r) => r.n").unwrap(),
        Value::Int(2)
    );
    assert_eq!(
        eval("[{n: 1}, {n: 2}] | sort_by(\"n\", \"Asc\") | first | fn(r) => r.n").unwrap(),
        Value::Int(1)
    );
}

#[test]
fn sort_by_refuses_an_unknown_order_word() {
    let text = refused("[{n: 1}, {n: 2}] | sort_by(\"n\", \"sideways\")");
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("an order of"), "{text}");
    assert!(text.contains("asc"), "{text}");
    assert!(text.contains("desc"), "{text}");
}

// ── item 3: min and max over Strings ─────────────────────────────────────

#[test]
fn min_and_max_take_an_array_of_strings_by_code_point() {
    assert_eq!(
        eval("[\"pear\", \"apple\", \"fig\"] | min").unwrap(),
        Value::Str("apple".to_string())
    );
    assert_eq!(
        eval("[\"pear\", \"apple\", \"fig\"] | max").unwrap(),
        Value::Str("pear".to_string())
    );
    // Uppercase sorts before lowercase, as the code point order does.
    assert_eq!(
        eval("[\"b\", \"A\"] | min").unwrap(),
        Value::Str("A".to_string())
    );
}

#[test]
fn min_refuses_a_mix_of_strings_and_numbers() {
    let text = refused("[1, \"a\"] | min");
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("all be numbers or all Strings"), "{text}");
}

#[test]
fn min_and_max_of_numbers_are_unchanged() {
    assert_eq!(eval("[1, 5, 3] | max").unwrap(), Value::Int(5));
    assert_eq!(eval("[1, 5, 3] | min").unwrap(), Value::Int(1));
}

// ── item 4: avg of an empty array ────────────────────────────────────────

#[test]
fn avg_of_an_empty_array_is_refused_not_nan() {
    let text = refused("[] | avg");
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(
        text.contains("avg of an empty array is undefined"),
        "{text}"
    );
}

#[test]
fn mean_of_an_empty_array_is_refused_too() {
    let text = refused("[] | mean");
    assert!(
        text.contains("avg of an empty array is undefined"),
        "{text}"
    );
}

// ── item 5: first and last refuse a negative count ───────────────────────

#[test]
fn first_refuses_a_negative_count_as_take_does() {
    let text = refused("[1, 2] | first(-1)");
    assert!(
        text.contains("first count must be a nonnegative Int"),
        "{text}"
    );
    let take = refused("[1, 2] | take(-1)");
    assert!(
        take.contains("take count must be a nonnegative Int"),
        "{take}"
    );
}

#[test]
fn last_refuses_a_negative_count() {
    let text = refused("[1, 2] | last(-1)");
    assert!(
        text.contains("last count must be a nonnegative Int"),
        "{text}"
    );
}

#[test]
fn a_zero_count_is_still_an_empty_array() {
    assert_eq!(eval("[1, 2] | first(0)").unwrap(), ints(&[]));
    assert_eq!(eval("[1, 2] | last(0)").unwrap(), ints(&[]));
}

// ── item 6: did-you-mean for skip, drop, rest, has and has_key ───────────

#[test]
fn skip_drop_and_rest_are_suggested_as_slice() {
    for name in ["skip", "drop", "rest"] {
        assert_eq!(
            iron_shell::builtins::did_you_mean(name),
            vec!["slice".to_string()],
            "{name}"
        );
    }
}

#[test]
fn has_key_is_suggested_as_has() {
    // `has` itself exists now (round 33), so only the other spelling is mapped.
    assert_eq!(
        iron_shell::builtins::did_you_mean("has_key"),
        vec!["has".to_string()]
    );
}

#[test]
fn slice_with_one_index_drops_the_leading_elements() {
    assert_eq!(eval("[1, 2, 3] | slice(1)").unwrap(), ints(&[2, 3]));
}

// ── item 7: duplicate result column names are refused ────────────────────

#[test]
fn sql_refuses_two_columns_with_one_name() {
    let text = refused("sql(\"select 1 as a, 2 as a\")");
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("the result has two columns named"), "{text}");
    assert!(text.contains("give each a distinct name with AS"), "{text}");
}

#[test]
fn sql_refuses_a_self_join_that_repeats_a_column() {
    let text = refused(
        "[{name: \"x\"}] | sql(\"select a.name, b.name from t a join t b on a.name = b.name\")",
    );
    assert!(text.contains("two columns named"), "{text}");
}

#[test]
fn sql_with_distinct_aliases_keeps_both_columns() {
    assert_eq!(
        eval("[{name: \"x\"}] | sql(\"select a.name as left_name, b.name as right_name from t a join t b on a.name = b.name\") | first | fn(r) => r.right_name")
            .unwrap(),
        Value::Str("x".to_string())
    );
}

// ── item 8: the write refusal names ATTACH only for an ATTACH ────────────

#[test]
fn a_refused_write_gets_the_plain_read_only_hint() {
    let text = refused("[{name: \"x\"}] | sql(\"delete from t\")");
    assert!(
        text.contains("sql is read-only: use db_sqlite_exec to change a database file"),
        "{text}"
    );
    assert!(!text.contains("cannot ATTACH"), "{text}");
}

// ── item 9: parameters given as text are named as the mistake ────────────

#[test]
fn sql_names_text_given_where_the_parameters_belong() {
    let text = refused("sql(\"select ? as x\", \"notarray\")");
    assert!(text.contains("the parameters must be an Array"), "{text}");
    assert!(!text.contains("no database named"), "{text}");
}

#[test]
fn sql_with_an_array_of_parameters_still_binds_them() {
    assert_eq!(
        eval("sql(\"select ? as x\", [\"ok\"]) | first | fn(r) => r.x").unwrap(),
        Value::Str("ok".to_string())
    );
}

#[test]
fn sql_source_and_query_still_reads_as_a_source() {
    // A source that is not a query keeps the existing refusal.
    let text = refused("sql(\"no_such_file_round29.db\", \"select 1 as x\")");
    assert!(text.contains("no database named"), "{text}");
}

// ── item 10: a SQL syntax error from sqlite3 is named as the SQL's fault ─

fn sqlite3_present() -> bool {
    std::process::Command::new("sqlite3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn db_sqlite_exec_names_a_syntax_error_as_the_sql_s_fault() {
    if !sqlite3_present() {
        eprintln!("skipped: sqlite3 is not on PATH");
        return;
    }
    let dir = std::env::temp_dir().join(format!("ae_round29_sqlite_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("x.db").to_string_lossy().replace('\\', "/");
    let mut env = iron_shell::env::Env::new();
    let made = iron_shell::builtins::call(
        "db_sqlite_exec",
        vec![
            Value::Str(db.clone()),
            Value::Str("create table t(x)".to_string()),
        ],
        &mut env,
    );
    let refusal = iron_shell::builtins::call(
        "db_sqlite_exec",
        vec![Value::Str(db), Value::Str("selekt 1".to_string())],
        &mut env,
    );
    let _ = std::fs::remove_dir_all(&dir);
    // db_sqlite_exec is Exec: a direct call may stop at the approval gate
    // before sqlite3 runs. That is not the hint under test, so it is skipped.
    if let Err(e) = &made {
        if e.to_string().contains("E_NEEDS_APPROVAL") {
            eprintln!("skipped: db_sqlite_exec needs approval in this test context");
            return;
        }
    }
    assert!(made.is_ok(), "{made:?}");
    let text = refusal.expect_err("a syntax error is refused").to_string();
    assert!(text.contains("the SQL did not parse"), "{text}");
    assert!(!text.contains("installed but rejected"), "{text}");
}

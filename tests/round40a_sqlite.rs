//! Round 40a: the sqlite builtins' line endings, VACUUM, DELETE without a where,
//! column types, row shapes, identifiers, TSV and empty cells in SQL, and the
//! error messages.

use iron_shell::env::Env;
use iron_shell::value::Value;
use std::path::PathBuf;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn have_sqlite3() -> bool {
    std::process::Command::new("sqlite3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn call(name: &str, args: Vec<Value>) -> Result<Value, String> {
    let mut env = Env::new();
    iron_shell::builtins::call(name, args, &mut env).map_err(|e| e.to_string())
}

fn s(v: impl AsRef<str>) -> Value {
    Value::Str(v.as_ref().to_string())
}

fn p(path: &std::path::Path) -> Value {
    s(path.to_string_lossy())
}

/// A fresh scratch directory, with the undo journal pointed inside it so no test
/// reaches the real one.
fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("r40a_{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("IRONSHELL_SESSION_DIR", dir.join("session"));
    dir
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn to_csv_ends_each_row_with_one_crlf_in_text_and_file() {
    if !have_sqlite3() {
        eprintln!("skipping: sqlite3 not on PATH");
        return;
    }
    let _g = lock();
    let dir = scratch("csv");
    let db = dir.join("t.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE t(id INTEGER, note TEXT); INSERT INTO t VALUES (1, 'a' || char(10) || 'b'); INSERT INTO t VALUES (2, 'x');",
    )
    .unwrap();
    drop(conn);
    let Value::Str(text) = call("db_sqlite_to_csv", vec![p(&db), s("t")]).unwrap() else {
        panic!("expected text")
    };
    assert!(!text.contains("\r\r"), "{text:?}");
    assert!(text.starts_with("id,note\r\n1,\"a"), "{text:?}");
    assert!(text.ends_with("\"\r\n2,x\r\n"), "{text:?}");
    let out = dir.join("out.csv");
    call("db_sqlite_to_csv", vec![p(&db), s("t"), p(&out)]).unwrap();
    assert_eq!(std::fs::read_to_string(&out).unwrap(), text);
}

#[test]
fn vacuum_runs() {
    let _g = lock();
    let dir = scratch("vacuum");
    let db = dir.join("v.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE t(a); INSERT INTO t VALUES (1); DELETE FROM t;")
        .unwrap();
    drop(conn);
    assert_eq!(
        call("db_sqlite_vacuum", vec![p(&db)]).unwrap(),
        Value::Bool(true)
    );
}

#[test]
fn delete_without_a_where_is_refused() {
    let _g = lock();
    let dir = scratch("delete");
    let db = dir.join("d.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE t(a); INSERT INTO t VALUES (1);")
        .unwrap();
    drop(conn);
    for args in [
        vec![p(&db), s("t")],
        vec![p(&db), s("t"), s("")],
        vec![p(&db), s("t"), s("   ")],
    ] {
        let e = call("db_sqlite_delete", args).unwrap_err();
        assert!(e.contains("E_BAD_ARG"), "{e}");
        assert!(e.contains("a where clause is required"), "{e}");
        assert!(e.contains("1=1"), "{e}");
    }
    let conn = rusqlite::Connection::open(&db).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn csv_import_infers_types_and_loads_empty_cells_as_null() {
    let _g = lock();
    let dir = scratch("csvimport");
    let csv = dir.join("rows.csv");
    std::fs::write(&csv, "id,amt,name\n10,1.5,a\n2,,b\n3,2,\n").unwrap();
    let db = dir.join("r.db");
    assert_eq!(
        call("db_csv_to_sqlite", vec![p(&db), p(&csv), s("rows")]).unwrap(),
        Value::Bool(true)
    );
    let conn = rusqlite::Connection::open(&db).unwrap();
    let types: Vec<(String, String)> = conn
        .prepare("SELECT name, type FROM pragma_table_info('rows') ORDER BY cid")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        types,
        [
            ("id".to_string(), "INTEGER".to_string()),
            ("amt".to_string(), "REAL".to_string()),
            ("name".to_string(), "TEXT".to_string())
        ]
    );
    let first: i64 = conn
        .query_row("SELECT id FROM rows ORDER BY id LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(first, 2);
    let nulls: i64 = conn
        .query_row(
            "SELECT count(*) FROM rows WHERE amt IS NULL OR name IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(nulls, 2);
}

#[test]
fn json_import_types_columns_and_unions_the_keys_of_every_row() {
    if !have_sqlite3() {
        eprintln!("skipping: sqlite3 not on PATH");
        return;
    }
    let _g = lock();
    let dir = scratch("jsonimport");
    let json = dir.join("rows.json");
    std::fs::write(
        &json,
        r#"[{"id": 10, "ok": true}, {"id": 2, "price": 1.5, "tag": "x"}, {"id": 3}]"#,
    )
    .unwrap();
    let db = dir.join("j.db");
    assert_eq!(
        call("db_json_to_sqlite", vec![p(&db), p(&json)]).unwrap(),
        Value::Bool(true)
    );
    let conn = rusqlite::Connection::open(&db).unwrap();
    let types: Vec<(String, String)> = conn
        .prepare("SELECT name, type FROM pragma_table_info('imported') ORDER BY cid")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        types,
        [
            ("id".to_string(), "INTEGER".to_string()),
            ("ok".to_string(), "INTEGER".to_string()),
            ("price".to_string(), "REAL".to_string()),
            ("tag".to_string(), "TEXT".to_string())
        ]
    );
    let ids: Vec<i64> = conn
        .prepare("SELECT id FROM imported ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(ids, [2, 3, 10]);
    let missing: i64 = conn
        .query_row(
            "SELECT count(*) FROM imported WHERE price IS NULL AND tag IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(missing, 2);
}

#[test]
fn names_with_spaces_quotes_and_reserved_words_work() {
    if !have_sqlite3() {
        eprintln!("skipping: sqlite3 not on PATH");
        return;
    }
    let _g = lock();
    let dir = scratch("idents");
    let db = dir.join("i.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE \"my \"\"table\" (\"select\" TEXT, \"a b\" INTEGER);")
        .unwrap();
    drop(conn);
    let row = Value::Record(
        [
            ("select".to_string(), s("v")),
            ("a b".to_string(), Value::Int(7)),
        ]
        .into_iter()
        .collect(),
    );
    assert_eq!(
        call("db_sqlite_insert", vec![p(&db), s("my \"table"), row]).unwrap(),
        Value::Bool(true)
    );
    let conn = rusqlite::Connection::open(&db).unwrap();
    let got: (String, i64) = conn
        .query_row(
            "SELECT \"select\", \"a b\" FROM \"my \"\"table\"",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(got, ("v".to_string(), 7));
    // A control character is still refused, and the message says what is allowed.
    let e = call("db_sqlite_count", vec![p(&db), s("a\nb")]).unwrap_err();
    assert!(e.contains("control characters"), "{e}");
}

#[test]
fn sql_reads_tsv_and_loads_empty_csv_cells_as_null() {
    let _g = lock();
    let dir = scratch("tsv");
    let tsv = dir.join("o.tsv");
    std::fs::write(&tsv, "id\tqty\n1\t5\n2\t7\n").unwrap();
    assert_eq!(
        call("sql_value", vec![p(&tsv), s("select sum(qty) from o")]).unwrap(),
        Value::Int(12)
    );
    let csv = dir.join("c.csv");
    std::fs::write(&csv, "id,amt,note\n1,5,a\n2,,b\n3,7,\n").unwrap();
    assert_eq!(
        call(
            "sql_value",
            vec![p(&csv), s("select count(*) from c where amt is null")]
        )
        .unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        call(
            "sql_value",
            vec![p(&csv), s("select count(*) from c where note is null")]
        )
        .unwrap(),
        Value::Int(1)
    );
}

#[test]
fn messages_are_plain() {
    let _g = lock();
    let e = call("sql", vec![s("")]).unwrap_err();
    assert!(e.contains("the query is empty"), "{e}");
    assert!(!e.contains("sql: sql:"), "{e}");
    let e = call("sql", vec![s("select nope from nothing")]).unwrap_err();
    assert!(!e.contains("sql: sql:"), "{e}");
    if !have_sqlite3() {
        return;
    }
    let dir = scratch("messages");
    let db = dir.join("m.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE people(name TEXT);")
        .unwrap();
    drop(conn);
    let row = Value::Record([("nme".to_string(), s("x"))].into_iter().collect());
    let e = call("db_sqlite_insert", vec![p(&db), s("people"), row]).unwrap_err();
    assert!(e.contains("table people has no column named nme"), "{e}");
    assert!(e.contains("db_sqlite_insert"), "{e}");
    assert!(!e.contains("command line argument"), "{e}");
}

//! `db_sqlite_exec` takes an optional Array of bind values for its `?`s.

use iron_shell::value::Value;

fn try_run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn run(src: &str) -> Value {
    try_run(src).expect("runs")
}

fn db_path(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("ish_r47b_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("t.db").to_string_lossy().replace('\\', "/")
}

#[test]
fn exec_binds_params_and_keeps_the_bool_return() {
    let f = db_path("bind");
    let src = format!(
        "f = \"{f}\"\n\
         db_sqlite_exec(f, \"CREATE TABLE t(v INTEGER, n TEXT)\", [])\n\
         r = db_sqlite_exec(f, \"INSERT INTO t VALUES (?, ?)\", [7, \"it's\"])\n\
         [r, sql_value(f, \"select n from t where v = 7\")]"
    );
    assert_eq!(
        run(&src),
        Value::Array(vec![Value::Bool(true), Value::Str("it's".to_string())])
    );
}

#[test]
fn exec_refuses_a_param_count_mismatch_and_a_non_array() {
    let f = db_path("bad");
    let n = try_run(&format!(
        "db_sqlite_exec(\"{f}\", \"CREATE TABLE t(v INTEGER)\", [])\n\
         db_sqlite_exec(\"{f}\", \"INSERT INTO t VALUES (?)\", [1, 2])"
    ));
    assert!(n.expect_err("mismatch").to_string().contains("placeholder"));
    let s = try_run(&format!("db_sqlite_exec(\"{f}\", \"select 1\", \"x\")"));
    assert!(s.is_err());
}

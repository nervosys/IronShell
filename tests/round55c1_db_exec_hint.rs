//! `db_sqlite_exec` is Exec (arbitrary SQL through the sqlite3 CLI), so agent
//! mode asks for approval; the refusal names the approval-free ways to build a
//! database. help("db") describes every member, including the two that the
//! catalogue files under the shorter spelling of their dispatch row.

use iron_shell::value::Value;
use std::process::Command;

#[test]
fn help_db_describes_sqlite_exec_and_query() {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(r#"help("db").members"#).expect("parses");
    let members = match iron_shell::eval::eval_program(&stmts, &mut env).expect("runs") {
        Value::Array(a) => a,
        other => panic!("members is a {}", other.type_name()),
    };
    let mut seen = 0;
    for m in &members {
        let Value::Record(r) = m else { continue };
        let name = match r.get("name") {
            Some(Value::Str(s)) => s.clone(),
            _ => continue,
        };
        if name == "sqlite_exec" || name == "sqlite_query" {
            seen += 1;
            match r.get("description") {
                Some(Value::Str(d)) => assert!(!d.is_empty(), "{name} has an empty description"),
                _ => panic!("{name} has no description"),
            }
        }
    }
    assert_eq!(seen, 2);
}

#[test]
fn the_approval_refusal_names_csv_to_sqlite() {
    let root = std::env::temp_dir().join(format!("ish-r55c1-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let ws = root.join("ws");
    let session = root.join("session");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&session).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--agent", "--workspace"])
        .arg(&ws)
        .args(["-c", r#"db.sqlite_exec("out.db", "create table t(a int)")"#])
        .current_dir(&ws)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", &session)
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_APPROVE_ALL")
        .env_remove("IRONSHELL_APPROVE")
        .env_remove("IRONSHELL_WORKSPACE")
        .output()
        .expect("run ish");
    let shown = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
    assert!(shown.contains("E_NEEDS_APPROVAL"), "{shown}");
    assert!(shown.contains("csv_to_sqlite"), "{shown}");
}

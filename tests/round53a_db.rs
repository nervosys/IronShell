//! Round 53a: the db module's members and the db builtins' help.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    match iron_shell::eval::eval_program(&stmts, &mut env) {
        Ok(v) => v,
        Err(_) => panic!("the program failed"),
    }
}

#[test]
fn db_exposes_the_import_and_export_builtins_as_members() {
    for member in ["csv_to_sqlite", "json_to_sqlite", "sqlite_to_csv"] {
        let v = run(&format!("db.{member}"));
        assert!(
            matches!(v, Value::Builtin(_)),
            "db.{member} is not a builtin reference"
        );
    }
}

#[test]
fn db_sqlite_query_has_its_own_signature_and_names_itself_in_errors() {
    let sig = iron_shell::signature::signature_of("db_sqlite_query").expect("declared");
    assert_eq!(sig.name, "db_sqlite_query");
    let alias = iron_shell::signature::signature_of("sqlite_query").expect("declared");
    assert_eq!(alias.name, "db_sqlite_query");
    assert_eq!(
        iron_shell::signature::signature_of("sql")
            .expect("declared")
            .name,
        "sql"
    );
    let got =
        run("try { db_sqlite_query(\":memory:\", \"SELEC nonsense\") } catch e { e.message }");
    let Value::Str(message) = got else {
        panic!("not a String")
    };
    assert!(message.contains("db_sqlite_query"), "{message}");
    assert!(!message.starts_with("sql:"), "{message}");
}

#[test]
fn the_csv_import_doc_says_what_an_empty_cell_becomes() {
    let sig = iron_shell::signature::signature_of("db_csv_to_sqlite").expect("declared");
    assert!(sig.doc.contains("NULL in a numeric column"), "{}", sig.doc);
    assert!(sig.doc.contains("text column"), "{}", sig.doc);
    assert!(!sig.doc.contains("An empty cell is NULL."), "{}", sig.doc);
}

#[test]
fn db_json_to_sqlite_refuses_json_text_without_echoing_it() {
    let payload = "x".repeat(500);
    let src = format!(
        "try {{ db_json_to_sqlite(\"unused.db\", \"[{{\\\"a\\\": \\\"{payload}\\\"}}]\") }} catch e {{ [e.code, e.message] }}"
    );
    let Value::Array(parts) = run(&src) else {
        panic!("not an Array")
    };
    assert_eq!(parts[0], Value::Str("E_BAD_ARG".into()));
    let Value::Str(message) = &parts[1] else {
        panic!("not a String")
    };
    assert!(
        message.contains("takes a file path, not JSON text"),
        "{message}"
    );
    assert!(message.len() < 200, "{}", message.len());
}

#[test]
fn the_json_import_doc_says_it_takes_a_path() {
    let sig = iron_shell::signature::signature_of("db_json_to_sqlite").expect("declared");
    assert!(sig.doc.contains("not JSON text"), "{}", sig.doc);
}

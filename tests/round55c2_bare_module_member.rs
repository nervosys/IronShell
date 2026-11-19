//! A bare name that exactly one module has as a member is that member:
//! help("csv_to_sqlite") describes db.csv_to_sqlite, and calling the bare name
//! suggests it.

fn try_run(src: &str) -> anyhow::Result<iron_shell::value::Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env)
}

#[test]
fn help_of_a_bare_member_name_resolves() {
    match try_run(r#"help("csv_to_sqlite")"#) {
        Ok(v) => {
            let text = format!("{v:?}");
            assert!(text.contains("csv_to_sqlite"), "{}", text.len());
        }
        Err(e) => panic!("help failed: {e}"),
    }
}

#[test]
fn an_unknown_bare_member_name_suggests_the_module_call() {
    let err = try_run(r#"csv_to_sqlite("a.db", "a.csv", "t")"#).expect_err("not a builtin");
    let se = err
        .downcast_ref::<iron_shell::safety::SafetyError>()
        .expect("a structured refusal");
    assert!(
        se.did_you_mean.iter().any(|s| s == "db.csv_to_sqlite"),
        "{:?}",
        se.did_you_mean
    );
}

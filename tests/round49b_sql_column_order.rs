//! A sql row is a Record, and a Record sorts its keys: the columns of every
//! result (not only `select *`) come back in sorted key order. AGENTS.md says
//! so, and says how to control the order.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

#[test]
fn an_explicit_column_list_still_comes_back_sorted() {
    let v = run(r#"[{z: 1, a: 2}] | sql("select z, a from t") | first | keys"#);
    assert_eq!(
        v,
        Value::Array(vec![Value::Str("a".into()), Value::Str("z".into())])
    );
}

#[test]
fn to_csv_with_a_column_list_controls_the_order() {
    let v = run(r#"[{z: 1, a: 2}] | to_csv(["z", "a"]) | lines | first"#);
    assert_eq!(v, Value::Str("z,a".into()));
}

#[test]
fn agents_md_states_the_rule() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/AGENTS.md"))
        .expect("AGENTS.md");
    assert!(text.contains("every sql row (not only select *)"));
    assert!(text.contains("to_csv(rows, [cols])"));
}

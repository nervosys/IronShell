//! Round 42b: `sql` takes a Record of tables as its first argument, as it does
//! piped, and says what an empty CSV cell is.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

#[test]
fn a_record_of_tables_as_the_first_argument_joins() {
    let direct = run(
        r#"sql({orders: [{cid: 1, total: 5}, {cid: 2, total: 7}], cust: [{id: 1, name: "ann"}, {id: 2, name: "bo"}]}, "select cust.name as name, orders.total as total from orders join cust on orders.cid = cust.id order by total")"#,
    );
    let piped = run(
        r#"{orders: [{cid: 1, total: 5}, {cid: 2, total: 7}], cust: [{id: 1, name: "ann"}, {id: 2, name: "bo"}]} | sql("select cust.name as name, orders.total as total from orders join cust on orders.cid = cust.id order by total")"#,
    );
    assert_eq!(direct, piped);
    match direct {
        Value::Array(rows) => assert_eq!(rows.len(), 2),
        other => panic!("expected rows, got {other:?}"),
    }
}

#[test]
fn a_record_of_tables_takes_params_too() {
    let v = run(
        r#"sql({a: [{id: 1}, {id: 2}], b: [{id: 2, m: 9}]}, "select b.m as m from a join b on a.id = b.id where a.id = ?", [2]) | first | fn(r) => r.m"#,
    );
    assert_eq!(v, Value::Int(9));
}

#[test]
fn the_signature_states_the_empty_cell_rule() {
    let sig = iron_shell::signature::SIGNATURES
        .iter()
        .find(|s| s.name == "sql")
        .expect("sql has a signature");
    let text = format!("{sig:?}");
    assert!(
        text.contains("NULL in a numeric column and '' in a text column"),
        "{text}"
    );
}

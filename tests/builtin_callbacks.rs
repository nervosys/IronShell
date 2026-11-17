//! A builtin passed where a callback is expected is called with the element.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn strs(v: &[&str]) -> Value {
    Value::Array(v.iter().map(|s| Value::Str((*s).into())).collect())
}

#[test]
fn sort_by_takes_a_builtin() {
    assert_eq!(run("[\"bb\", \"a\"] | sort_by(len)"), strs(&["a", "bb"]));
}

#[test]
fn map_takes_a_builtin() {
    assert_eq!(
        run("[\"bb\", \"a\"] | map(len)"),
        Value::Array(vec![Value::Int(2), Value::Int(1)])
    );
}

#[test]
fn where_any_all_each_and_group_by_take_a_builtin() {
    // `path_is_absolute` answers a Bool from a String alone (text only, no I/O).
    assert_eq!(
        run("[\"/abs\", \"rel\"] | where(path_is_absolute)"),
        strs(&["/abs"]),
        "where"
    );
    assert_eq!(
        run("[\"/abs\", \"rel\"] | any(path_is_absolute)"),
        Value::Bool(true)
    );
    assert_eq!(
        run("[\"/abs\", \"rel\"] | all(path_is_absolute)"),
        Value::Bool(false)
    );
    assert_eq!(run("[\"a\", \"b\"] | each(len)"), strs(&["a", "b"]));
    assert_eq!(
        run("[\"a\", \"bb\", \"c\"] | group_by(len) | len"),
        Value::Int(2)
    );
}

//! A table row must never print as an empty or whitespace-only line: a client
//! that trims lines would lose the row.

use iron_shell::value::Value;
use std::collections::BTreeMap;

fn rec(pairs: &[(&str, Value)]) -> Value {
    let mut m = BTreeMap::new();
    for (k, v) in pairs {
        m.insert(k.to_string(), v.clone());
    }
    Value::Record(m)
}

fn call(name: &str, args: Vec<Value>) -> Value {
    let mut env = iron_shell::env::Env::new();
    iron_shell::builtins::call(name, args, &mut env).expect("builtin call")
}

fn check(arr: &Value) {
    let out = match call("aecon", vec![arr.clone()]) {
        Value::Str(s) => s,
        other => panic!("aecon should return a string, got {other:?}"),
    };
    for (i, line) in out.lines().enumerate() {
        assert!(!line.trim().is_empty(), "line {i} is blank:\n{out:?}");
    }
    let back = call("aecon_decode", vec![Value::Str(out.clone())]);
    assert_eq!(&back, arr, "round trip failed:\n{out}");
}

#[test]
fn a_cell_equal_to_the_prefix_keeps_its_row_visible() {
    let arr = Value::Array(
        ["/usr/bin/a", "/usr/bin/b", "/usr/bin", "/usr/bin"]
            .iter()
            .map(|s| rec(&[("p", Value::Str((*s).into()))]))
            .collect(),
    );
    check(&arr);
}

#[test]
fn a_cell_equal_to_the_suffix_keeps_its_row_visible() {
    let arr = Value::Array(
        (0..8)
            .map(|i| {
                let s = if i % 2 == 0 { "active" } else { "inactive" };
                rec(&[("status", Value::Str(s.into()))])
            })
            .collect(),
    );
    check(&arr);
    let arr2 = Value::Array(
        ["status_ok", "status_ok", "ok", "ok", "ok", "ok", "ok", "ok"]
            .iter()
            .map(|s| rec(&[("status", Value::Str((*s).into()))]))
            .collect(),
    );
    check(&arr2);
}

#[test]
fn a_row_with_every_cell_repeated_keeps_its_first_cell() {
    let long = ["alpha-long-value-here", "bravo-long-value-here"];
    let other = ["charlie-long-value-here", "delta-long-value-here"];
    let arr = Value::Array(
        (0..8)
            .map(|i| {
                let src = if i < 4 { &long } else { &other };
                rec(&[
                    ("a", Value::Str(src[0].into())),
                    ("b", Value::Str(src[1].into())),
                ])
            })
            .collect(),
    );
    check(&arr);
}

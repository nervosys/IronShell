use iron_shell::{env::Env, eval::eval_program, parser::parse_program, value::Value};
fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

/// Runs `src` in an existing environment, so a test can look at bindings
/// after an error.
fn eval_in(env: &mut Env, src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, env)
}

// ---------- record field assignment ----------

#[test]
fn field_assignment_sets_an_existing_field() {
    let v = eval("mut r = {a: 1}\nr.a = 2\nr.a").unwrap();
    assert_eq!(v, Value::Int(2));
}

#[test]
fn field_assignment_creates_a_missing_field() {
    let v = eval("mut r = {a: 1}\nr.b = 3\nr.b").unwrap();
    assert_eq!(v, Value::Int(3));
    let kept = eval("mut r = {a: 1}\nr.b = 3\nr.a").unwrap();
    assert_eq!(kept, Value::Int(1));
}

#[test]
fn field_assignment_into_a_nested_record() {
    let v = eval("mut r = {a: {b: 1, c: 2}}\nr.a.b = 7\nr.a.b + r.a.c").unwrap();
    assert_eq!(v, Value::Int(9));
}

#[test]
fn field_assignment_evaluates_the_new_value_from_the_old_record() {
    let v = eval("mut r = {a: 1}\nr.a = r.a + 10\nr.a").unwrap();
    assert_eq!(v, Value::Int(11));
}

#[test]
fn field_assignment_leaves_copies_alone() {
    // Records are values: `s` keeps the record it was copied from.
    let s = eval("mut r = {a: 1}\ns = r\nr.a = 9\ns.a").unwrap();
    assert_eq!(s, Value::Int(1));
    let r = eval("mut r = {a: 1}\ns = r\nr.a = 9\nr.a").unwrap();
    assert_eq!(r, Value::Int(9));
}

#[test]
fn field_assignment_through_a_non_record_intermediate_is_refused() {
    let mut env = Env::new();
    eval_in(&mut env, "mut r = {a: 1}").unwrap();
    let err = eval_in(&mut env, "r.a.b = 2").unwrap_err().to_string();
    assert!(err.contains("r.a"), "error should name the path: {err}");
    // The refused assignment left `r` as it was.
    assert_eq!(eval_in(&mut env, "r.a").unwrap(), Value::Int(1));
}

#[test]
fn field_assignment_through_a_missing_intermediate_is_refused() {
    let mut env = Env::new();
    eval_in(&mut env, "mut r = {a: 1}").unwrap();
    let err = eval_in(&mut env, "r.x.y = 2").unwrap_err().to_string();
    assert!(err.contains("r.x"), "error should name the path: {err}");
}

#[test]
fn field_assignment_on_a_non_record_root_is_refused() {
    let err = eval("mut n = 5\nn.a = 1").unwrap_err().to_string();
    assert!(
        err.contains("Record"),
        "error should say a Record is needed: {err}"
    );
}

#[test]
fn field_assignment_on_an_unbound_name_is_an_unknown_name_error() {
    let err = eval("q.a = 1").unwrap_err().to_string();
    assert!(
        err.contains("q"),
        "error should name the unknown name: {err}"
    );
}

#[test]
fn field_assignment_on_an_immutable_binding_is_refused() {
    // Field assignment rebinds the record, so it needs `mut`, as `x = ...` does.
    let mut env = Env::new();
    eval_in(&mut env, "r = {a: 1}").unwrap();
    assert!(eval_in(&mut env, "r.a = 2").is_err());
    assert_eq!(eval_in(&mut env, "r.a").unwrap(), Value::Int(1));
}

#[test]
fn lambdas_after_a_field_assignment_read_the_new_field() {
    let v = eval(
        "mut r = {k: 1}\n\
         r.k = 3\n\
         f = fn(x) => x + r.k\n\
         f(1)",
    )
    .unwrap();
    assert_eq!(v, Value::Int(4));
    let rows = eval(
        "mut r = {k: 1}\n\
         r.k = 3\n\
         [1, 2] | map(fn(x) => x + r.k)",
    )
    .unwrap();
    assert_eq!(rows, Value::Array(vec![Value::Int(4), Value::Int(5)]));
}

// ---------- `x = x + e` appends in place ----------

#[test]
fn string_built_by_reduce_keeps_every_piece() {
    let v = eval(r#"range(20000) | reduce(fn(acc, i) => acc + "x", "")"#).unwrap();
    assert_eq!(v, Value::Str("x".repeat(20000)));
}

#[test]
fn string_built_by_repeated_assignment_keeps_every_piece() {
    let mut src = String::from("mut s = \"\"\n");
    for _ in 0..20000 {
        src.push_str("s = s + \"x\"\n");
    }
    src.push('s');
    assert_eq!(eval(&src).unwrap(), Value::Str("x".repeat(20000)));
}

#[test]
fn int_reassignment_with_addition_still_works() {
    let v = eval("mut x = 1\nx = x + 1\nx = x + 2\nx").unwrap();
    assert_eq!(v, Value::Int(4));
}

#[test]
fn array_reassignment_with_addition_appends() {
    let v = eval("mut a = [1]\na = a + [2]\na = a + [3, 4]\na").unwrap();
    assert_eq!(
        v,
        Value::Array(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Int(4)
        ])
    );
}

#[test]
fn failing_operand_leaves_the_string_unchanged() {
    let mut env = Env::new();
    eval_in(&mut env, "mut x = \"a\"").unwrap();
    assert!(eval_in(&mut env, "x = x + undefined_name").is_err());
    assert_eq!(eval_in(&mut env, "x").unwrap(), Value::Str("a".into()));
}

#[test]
fn reassignment_of_an_immutable_string_is_still_refused() {
    let mut env = Env::new();
    eval_in(&mut env, "x = \"a\"").unwrap();
    assert!(eval_in(&mut env, "x = x + \"b\"").is_err());
    assert_eq!(eval_in(&mut env, "x").unwrap(), Value::Str("a".into()));
}

#[test]
fn string_past_the_size_limit_is_refused_and_leaves_the_binding() {
    // 2^23 bytes is the largest string a doubling can reach under the 8 MiB limit.
    let mut env = Env::new();
    eval_in(&mut env, "mut s = \"a\"").unwrap();
    for _ in 0..23 {
        eval_in(&mut env, "s = s + s").unwrap();
    }
    assert!(eval_in(&mut env, "s = s + s").is_err());
    assert_eq!(eval_in(&mut env, "len(s)").unwrap(), Value::Int(1 << 23));
}

// ---------- a lambda body `acc + e` appends to its parameter ----------

#[test]
fn reduce_appending_to_its_accumulator_keeps_every_value() {
    let v = eval(r#"range(5) | reduce(fn(acc, i) => acc + [i * 2], [])"#).unwrap();
    assert_eq!(v, Value::Array((0..5).map(|i| Value::Int(i * 2)).collect()));
    let n = eval(r#"range(5) | reduce(fn(acc, i) => acc + i, 10)"#).unwrap();
    assert_eq!(n, Value::Int(20));
}

#[test]
fn a_body_that_reads_its_accumulator_twice_still_sees_it_whole() {
    let v = eval(r#"range(3) | reduce(fn(acc, i) => acc + acc, "a")"#).unwrap();
    assert_eq!(v, Value::Str("aaaaaaaa".into()));
}

#[test]
fn a_reduce_body_that_fails_reports_the_error() {
    let e = eval(r#"range(3) | reduce(fn(acc, i) => acc + nope, "a")"#).unwrap_err();
    assert!(e.to_string().contains("nope"), "{e}");
}

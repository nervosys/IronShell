//! Round 22 language fixes: `&&` and `||` take Bool operands, `where` and the
//! pipe precedence hint say what to write, assignment errors name the fix, a
//! long append loop is linear, and string literals decode their escapes.

use iron_shell::{
    env::Env,
    eval::{eval_program, eval_program_statement},
    parser::parse_program,
    value::Value,
};

fn eval(src: &str) -> anyhow::Result<Value> {
    let stmts = parse_program(src).expect("parse");
    eval_program(&stmts, &mut Env::new())
}

/// The text of a parse error, for a source the parser must refuse.
fn parse_refusal(src: &str) -> String {
    match parse_program(src) {
        Ok(_) => panic!("expected a parse error for {src:?}"),
        Err(e) => format!("{e}"),
    }
}

/// The text of a run-time error, for a program that must fail.
fn run_refusal(src: &str) -> String {
    match eval(src) {
        Ok(v) => panic!("expected an error for {src:?}, got {v:?}"),
        Err(e) => format!("{e}"),
    }
}

// ---- 1. `&&` and `||` take Bool operands only ----

#[test]
fn and_refuses_a_non_bool_left_operand() {
    let text = run_refusal("1 && 2");
    assert!(
        text.contains("operator And: expected Bool operands, got Int"),
        "{text}"
    );
    assert!(
        text.contains("compare explicitly, e.g. x != 0"),
        "no hint: {text}"
    );
}

#[test]
fn or_refuses_a_non_bool_left_operand() {
    let text = run_refusal("[] || true");
    assert!(
        text.contains("operator Or: expected Bool operands"),
        "{text}"
    );
}

#[test]
fn a_string_is_not_true_under_and() {
    let text = run_refusal("\"false\" && true");
    assert!(
        text.contains("operator And: expected Bool operands"),
        "{text}"
    );
}

#[test]
fn a_null_left_operand_is_refused_too() {
    assert!(run_refusal("null || false").contains("operator Or: expected Bool operands"));
}

#[test]
fn a_non_bool_right_operand_is_refused_when_it_is_evaluated() {
    let text = run_refusal("true && 2");
    assert!(
        text.contains("operator And: expected Bool operands, got Int"),
        "{text}"
    );
    let text = run_refusal("false || 2");
    assert!(
        text.contains("operator Or: expected Bool operands, got Int"),
        "{text}"
    );
}

#[test]
fn a_short_circuited_non_bool_right_operand_is_not_checked() {
    // The right side is never evaluated, so its type does not matter.
    assert!(matches!(eval("false && 2").unwrap(), Value::Bool(false)));
    assert!(matches!(eval("true || 2").unwrap(), Value::Bool(true)));
}

#[test]
fn bool_operands_still_combine() {
    assert!(matches!(eval("true && false").unwrap(), Value::Bool(false)));
    assert!(matches!(eval("false || true").unwrap(), Value::Bool(true)));
    assert!(matches!(
        eval("(1 > 0) && (2 > 1)").unwrap(),
        Value::Bool(true)
    ));
}

// ---- 2. where's non-Bool predicate ----

#[test]
fn where_names_the_value_it_got_and_how_to_fix_the_predicate() {
    let text = run_refusal("[0] | where(fn(x) => x)");
    assert!(
        text.contains("where predicate must return Bool, got Int 0"),
        "{text}"
    );
    assert!(
        text.contains("return a Bool from the predicate, e.g. fn(x) => x != 0"),
        "{text}"
    );
    assert!(!text.contains("Int(0)"), "Debug form leaked: {text}");
}

// ---- 3. a comparison on a function names the pipe precedence ----

#[test]
fn a_comparison_on_a_function_says_the_pipe_binds_loosest() {
    let text = run_refusal("[\"ab\"] | where(fn(f) => f | len > 0)");
    assert!(text.contains("`|` binds loosest"), "{text}");
    assert!(text.contains("(a | f) > 0"), "{text}");
    assert!(!text.contains("convert with int(x)"), "{text}");
}

// ---- 4. assignment in the wrong place ----

#[test]
fn an_indexed_element_cannot_be_assigned() {
    let text = parse_refusal("rows = [{n: 1}]\nrows[0].n = 99\n");
    assert!(
        text.contains("an indexed element cannot be assigned; rebuild the array"),
        "{text}"
    );
    assert!(!text.contains("use `==`"), "{text}");
}

#[test]
fn a_field_assignment_in_a_block_gets_the_one_expression_hint() {
    let text = parse_refusal("r = {a: 1}\nif true { r.a = 7 }\n");
    assert!(
        text.contains("a block holds one expression; bind values before it"),
        "{text}"
    );
}

#[test]
fn a_field_assignment_in_parentheses_gets_the_one_expression_hint() {
    let text = parse_refusal("r = {a: 1}\nf = fn(v) => (r.a = v)\n");
    assert!(
        text.contains("a block holds one expression; bind values before it"),
        "{text}"
    );
}

#[test]
fn a_lambda_comparison_still_says_use_double_equals() {
    let text = parse_refusal("where([{a: 1}], fn(r) => r.a = 1)\n");
    assert!(text.contains("use `==` to compare; `=` assigns"), "{text}");
}

#[test]
fn a_nested_field_assignment_through_an_int_names_the_replacement() {
    let text = run_refusal("mut r = {a: 1}\nr.a.b = 5\n");
    assert!(text.contains("r.a: expected a Record, got Int"), "{text}");
    assert!(
        text.contains("r.a is an Int; assign r.a = {b: 5} to replace it"),
        "{text}"
    );
}

// ---- 5. appending in a loop is linear ----

#[test]
fn appending_50000_elements_one_statement_at_a_time() {
    // The REPL runs each top-level statement on its own, with `discard` set for
    // all but the last, which is the path the quadratic copy used to take.
    let mut src = String::from("mut a = []\n");
    for _ in 0..50_000 {
        src.push_str("a = a + [1]\n");
    }
    src.push_str("a\n");
    let stmts = parse_program(&src).expect("parse");
    let mut env = Env::new();
    let n = stmts.len();
    let mut last = Value::Null;
    for (i, s) in stmts.iter().enumerate() {
        last = eval_program_statement(s, i + 1 < n, &mut env).expect("statement");
    }
    match last {
        Value::Array(items) => assert_eq!(items.len(), 50_000),
        other => panic!("expected an Array, got {other:?}"),
    }
}

#[test]
fn appending_50000_elements_in_one_program() {
    let mut src = String::from("mut a = []\n");
    for _ in 0..50_000 {
        src.push_str("a = a + [1]\n");
    }
    src.push_str("a\n");
    match eval(&src).expect("program") {
        Value::Array(items) => assert_eq!(items.len(), 50_000),
        other => panic!("expected an Array, got {other:?}"),
    }
}

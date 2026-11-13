//! Round 31: the declared shape of pow, assert, unique, uniq, pick, chunk and lines.
//!
//! Each item was observed with `ish -c`: pow saturated where `*` refused; assert
//! raised a String where error() raises a coded record; unique and pick took
//! subjects their declarations did not admit; `chunk` was advertised and absent.
//! The lines examples read the repo's working directory, so they now use text.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

/// The refusal a program raises, as its JSON-rendered error.
fn refusal(src: &str) -> String {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    match iron_shell::eval::eval_program(&stmts, &mut env) {
        Ok(v) => panic!("{src} was accepted and answered {v:?}"),
        Err(e) => e.to_string(),
    }
}

fn ints(v: &[i64]) -> Value {
    Value::Array(v.iter().map(|i| Value::Int(*i)).collect())
}

fn strs(v: &[&str]) -> Value {
    Value::Array(v.iter().map(|s| Value::Str((*s).to_string())).collect())
}

// ---- item 1: pow never saturates an Int result ----

#[test]
fn pow_refuses_an_int_overflow_as_the_operators_do() {
    for src in [
        "pow(2, 100)",
        "pow(2, 63)",
        "pow(-2, 64)",
        "pow(10, 19)",
        "math.pow(2, 100)",
    ] {
        let e = refusal(src);
        assert!(e.contains("E_BAD_VALUE"), "{src}: {e}");
        assert!(e.contains("does not fit in a 64-bit Int"), "{src}: {e}");
        assert!(
            e.contains("pow(2.0, 100)"),
            "the hint must name the Float form: {e}"
        );
    }
}

#[test]
fn pow_keeps_exact_int_results_at_the_edge() {
    assert_eq!(run("pow(2, 62)"), Value::Int(4611686018427387904));
    assert_eq!(run("pow(-2, 63)"), Value::Int(i64::MIN));
    assert_eq!(run("pow(10, 18)"), Value::Int(1_000_000_000_000_000_000));
    assert_eq!(run("math.pow(2, 10)"), Value::Int(1024));
}

#[test]
fn pow_keeps_float_and_negative_exponent_results() {
    assert_eq!(run("pow(2, -1)"), Value::Float(0.5));
    assert_eq!(run("pow(2.0, 100) > 1.0e30"), Value::Bool(true));
    // The operator and the builtin agree on the refusal.
    assert!(refusal("2 * 4611686018427387904").contains("E_BAD_VALUE"));
}

// ---- item 2: assert raises a coded error, like error() ----

#[test]
fn a_failed_assert_is_caught_as_a_coded_record() {
    assert_eq!(
        run(r#"try { assert(false, "msg") } catch e { e.code }"#),
        Value::Str("E_ASSERT".to_string())
    );
    assert_eq!(
        run(r#"try { assert(false, "msg") } catch e { e.message }"#),
        Value::Str("assertion failed: msg".to_string())
    );
    assert_eq!(
        run(r#"try { assert(0) } catch e { e.message }"#),
        Value::Str("assertion failed".to_string())
    );
}

#[test]
fn a_piped_failed_assert_raises_the_same_error() {
    assert_eq!(
        run(r#"try { false | assert("piped") } catch e { e.code }"#),
        Value::Str("E_ASSERT".to_string())
    );
    assert_eq!(run(r#"true | assert("never shown")"#), Value::Bool(true));
}

#[test]
fn assert_declares_its_condition_and_its_message() {
    let sig = iron_shell::signature::signature_of("assert").expect("assert is declared");
    let names: Vec<&str> = sig.params.iter().map(|p| p.name).collect();
    assert_eq!(names, vec!["condition", "message"]);
}

// ---- item 3: unique takes an Array; uniq takes an Array or a String ----

#[test]
fn unique_refuses_a_string_in_both_forms_with_a_hint() {
    let direct = refusal(r#"unique("aab")"#);
    assert!(direct.contains("E_BAD_ARG"), "{direct}");
    assert!(
        direct.contains("split"),
        "the hint names the character split: {direct}"
    );
    let piped = refusal(r#""aab" | unique"#);
    assert!(piped.contains("E_BAD_ARG"), "{piped}");
    assert!(piped.contains("lines"), "{piped}");
}

#[test]
fn unique_takes_an_array_piped_or_direct() {
    assert_eq!(run("[1, 2, 1, 3] | unique"), ints(&[1, 2, 3]));
    assert_eq!(run(r#"unique(["b", "a", "b"])"#), strs(&["b", "a"]));
}

#[test]
fn uniq_takes_its_array_directly_as_it_does_piped() {
    assert_eq!(run("uniq([1, 1, 2, 1])"), ints(&[1, 2, 1]));
    assert_eq!(run("[1, 1, 2, 1] | uniq"), ints(&[1, 2, 1]));
}

#[test]
fn uniq_splits_a_string_into_lines_and_drops_adjacent_duplicates_only() {
    assert_eq!(run(r#""a\na\na\nb" | uniq"#), strs(&["a", "b"]));
    assert_eq!(run(r#"uniq("a\na\nb")"#), strs(&["a", "b"]));
    // Adjacent only: the second "a" comes back after the "b".
    assert_eq!(run(r#""a\nb\na" | uniq"#), strs(&["a", "b", "a"]));
}

// ---- item 4: pick takes an Array of Records or a Record ----

#[test]
fn pick_refuses_a_non_record_subject() {
    let e = refusal(r#"pick("a", "b", "c")"#);
    assert!(e.contains("E_BAD_ARG"), "{e}");
    let e = refusal(r#"pick(5, "a")"#);
    assert!(e.contains("E_BAD_ARG"), "{e}");
    let e = refusal(r#"[1, 2] | pick("a")"#);
    assert!(e.contains("E_BAD_ARG"), "{e}");
}

#[test]
fn pick_keeps_only_the_named_fields_of_a_record() {
    assert_eq!(run(r#"pick({a: 1, b: 2}, "a").a"#), Value::Int(1));
    assert_eq!(
        run(r#"pick({a: 1, b: 2}, "a") | keys | len"#),
        Value::Int(1)
    );
}

#[test]
fn pick_keeps_each_record_of_an_array_and_leaves_out_missing_fields() {
    assert_eq!(
        run(r#"[{a: 1, b: 2}, {b: 3}] | pick("a") | len"#),
        Value::Int(2)
    );
    assert_eq!(
        run(r#"pick({a: 1, b: 2}, "a", "zz") | keys | len"#),
        Value::Int(1)
    );
}

// ---- item 5: chunk splits an Array into pieces ----

#[test]
fn chunk_splits_an_array_piped_or_direct() {
    let want = Value::Array(vec![ints(&[1, 2]), ints(&[3])]);
    assert_eq!(run("[1, 2, 3] | chunk(2)"), want);
    assert_eq!(run("chunk([1, 2, 3], 2)"), want);
    assert_eq!(run("chunk([1, 2, 3, 4], 2) | len"), Value::Int(2));
    assert_eq!(run("arr.chunk([1, 2, 3], 3) | len"), Value::Int(1));
    assert_eq!(run("chunk([], 2)"), Value::Array(vec![]));
}

#[test]
fn chunk_refuses_a_size_below_one_or_not_an_int() {
    for src in [
        "chunk([1, 2], 0)",
        "chunk([1, 2], -1)",
        r#"chunk([1, 2], "2")"#,
        "chunk([1, 2])",
    ] {
        let e = refusal(src);
        assert!(e.contains("E_BAD_ARG"), "{src}: {e}");
    }
    let e = refusal("chunk(5, 2)");
    assert!(e.contains("E_BAD_ARG"), "{e}");
}

// ---- item 6: the lines examples need no working directory ----

#[test]
fn lines_splits_text_without_its_terminators() {
    assert_eq!(run(r#""a\nb" | lines"#), strs(&["a", "b"]));
    assert_eq!(run(r#"lines("a\nb\n") | len"#), Value::Int(2));
    assert_eq!(
        run(r#""x\r\ny" | lines | last"#),
        Value::Str("y".to_string())
    );
}

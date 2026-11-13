//! Round 38a: numbers that leave the Int range or are not finite are refused,
//! max/min keep the winner's type, the statistics builtins, and clean refusals
//! from `open` of YAML.

use iron_shell::value::Value;
use std::process::Command;

fn try_run(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn run(src: &str) -> Value {
    try_run(src).unwrap_or_else(|e| panic!("{src}: {e}"))
}

fn refusal(src: &str) -> String {
    try_run(src).expect_err(src).to_string()
}

fn float(v: Value) -> f64 {
    match v {
        Value::Float(f) => f,
        other => panic!("expected a Float, got {other:?}"),
    }
}

// 1. floor, ceil and round refuse a result outside the Int range.

#[test]
fn floor_ceil_and_round_refuse_what_int_refuses() {
    for src in [
        "floor(1e20)",
        "ceil(-1e20)",
        "round(1e19, 0)",
        "round(1e19)",
        "math.floor(1e20)",
        "math.ceil(-1e20)",
        "math.round(1e19)",
    ] {
        let text = refusal(src);
        assert!(text.contains("has no Int value"), "{src}: {text}");
        assert!(text.contains("E_BAD_VALUE"), "{src}: {text}");
    }
}

#[test]
fn floor_ceil_and_round_still_answer_in_range() {
    assert_eq!(run("floor(2.7)"), Value::Int(2));
    assert_eq!(run("ceil(-2.7)"), Value::Int(-2));
    assert_eq!(run("round(2.5)"), Value::Int(3));
    assert_eq!(run("floor(-9223372036854775808.0)"), Value::Int(i64::MIN));
}

// 3. sql refuses a non-finite result cell, naming the column.

#[test]
fn sql_refuses_an_infinite_result_cell() {
    let text = refusal(r#"[{a: 1}] | sql("select 1e999 as x from t")"#);
    assert!(text.contains("float overflow"), "{text}");
    assert!(text.contains("column x"), "{text}");
    let text = refusal(r#"[{a: 1e300}] | sql("select a * 1e10 as big from t")"#);
    assert!(text.contains("column big"), "{text}");
    assert_eq!(
        run(r#"[{a: 2}] | sql("select a * 1.5 as x from t") | first | fn(r) => r.x"#),
        Value::Float(3.0)
    );
}

// 4. max and min return the winning element unchanged.

#[test]
fn max_and_min_keep_the_winner_s_type() {
    assert_eq!(run("max(3, 2.5)"), Value::Int(3));
    assert_eq!(run("min(1, 2.5)"), Value::Int(1));
    assert_eq!(run("max(2.5, 3)"), Value::Int(3));
    assert_eq!(float(run("max(3.5, 2)")), 3.5);
    assert_eq!(float(run("min(2.5, 3)")), 2.5);
    assert_eq!(run("[3, 2.5] | max"), Value::Int(3));
    assert_eq!(run("max([1, 2.5, 2])"), Value::Float(2.5));
    assert_eq!(run("[1, 2.5, 2] | min"), Value::Int(1));
    assert_eq!(run("max(1, 7, 3.5)"), Value::Int(7));
    assert_eq!(run("min(4, 1.5, 3)"), Value::Float(1.5));
}

// 6. A JSON -0 is the integer 0.

#[test]
fn json_negative_zero_integer_is_int_zero() {
    assert_eq!(
        run(r#"from_json("[-0, 1]")"#),
        Value::Array(vec![Value::Int(0), Value::Int(1)])
    );
    assert_eq!(run(r#"json_parse("-0")"#), Value::Int(0));
    assert_eq!(
        run(r#"from_json("{\"a\": -0}").a"#),
        Value::Int(0),
        "inside an object"
    );
    // A fraction or an exponent keeps it a Float; a string is untouched.
    let Value::Array(items) = run(r#"from_json("[-0.0]")"#) else {
        panic!("array")
    };
    assert!(matches!(items[0], Value::Float(f) if f == 0.0 && f.is_sign_negative()));
    assert_eq!(
        run(r#"from_json("[\"-0\"]")"#),
        Value::Array(vec![Value::Str("-0".into())])
    );
}

// 7. median, variance, stddev and percentile.

#[test]
fn statistics_builtins() {
    assert_eq!(run("[1, 2, 3] | median"), Value::Float(2.0));
    assert_eq!(run("median([1, 2, 3, 4])"), Value::Float(2.5));
    assert_eq!(run("median([3, 1, 2])"), Value::Float(2.0));
    assert_eq!(run("[1, 2, 3, 4, 5] | variance"), Value::Float(2.5));
    assert_eq!(run("[1, 2, 3] | stddev"), Value::Float(1.0));
    assert_eq!(
        run("math.variance([1.0, 2.0, 3.0, 4.0, 5.0])"),
        Value::Float(2.5)
    );
    assert_eq!(run("[1, 2, 3, 4, 5] | percentile(50)"), Value::Float(3.0));
    assert_eq!(run("percentile([1, 2, 3, 4], 25)"), Value::Float(1.75));
    assert_eq!(run("percentile([5, 1, 3], 0)"), Value::Float(1.0));
    assert_eq!(run("math.percentile([5, 1, 3], 100)"), Value::Float(5.0));
    assert_eq!(run("math.median([4, 1])"), Value::Float(2.5));
    assert_eq!(
        run("math.stddev([2, 4])").clone(),
        Value::Float(2f64.sqrt())
    );
}

#[test]
fn statistics_builtins_refuse_what_has_no_answer() {
    for src in [
        "[1] | variance",
        "stddev([])",
        "[] | median",
        "percentile([], 50)",
        "percentile([1, 2], 101)",
        "percentile([1, 2], -1)",
        r#"[1, "a"] | median"#,
    ] {
        let text = refusal(src);
        assert!(text.contains("E_BAD_ARG"), "{src}: {text}");
    }
}

// 8. An unknown module member does not suggest a short global by chance.

#[test]
fn a_module_member_suggestion_ignores_a_short_substring_global() {
    let got = iron_shell::builtins::module_member_candidates("math", "variancx", &["sqrt", "abs"]);
    assert!(got.iter().all(|c| c != "nc"), "{got:?}");
    let text = refusal("math.nosuchthing_nc(1)");
    assert!(!text.contains("\"nc\""), "{text}");
}

// 2 and 5. YAML: non-finite numbers and wide integers are refused cleanly.

fn open_yaml(name: &str, body: &str) -> (bool, String) {
    let dir = std::env::temp_dir().join(format!("ish_round38a_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("y.yaml"), body).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", r#"open("y.yaml")"#])
        .current_dir(&dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .output()
        .expect("runs ish");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
    (out.status.success(), text)
}

#[test]
fn open_of_yaml_with_a_wide_integer_is_one_clean_error() {
    let (ok, text) = open_yaml("wide", "a: 1\nh: 9223372036854775808\n");
    assert!(!ok, "{text}");
    assert!(text.contains("integer overflow"), "{text}");
    assert!(text.contains("line 2"), "{text}");
    assert!(text.contains("quote it as a string"), "{text}");
    assert!(
        !text.contains(r#"{"error""#) && !text.contains(r#"\"error\""#),
        "no structured error stringified into another: {text}"
    );
    assert!(!text.contains("is not valid YAML"), "{text}");
}

#[test]
fn open_of_yaml_with_inf_or_nan_is_refused() {
    for word in [".inf", "-.inf", ".nan"] {
        let (ok, text) = open_yaml("inf", &format!("a: 1\nb: {word}\n"));
        assert!(!ok, "{word}: {text}");
        assert!(text.contains("not a finite number"), "{word}: {text}");
        assert!(text.contains("line 2"), "{word}: {text}");
        assert!(text.contains("quote it as a string"), "{word}: {text}");
    }
}

#[test]
fn from_yaml_refuses_inf_and_nan_but_reads_a_quoted_one() {
    let text = refusal(r#"from_yaml("a: .inf")"#);
    assert!(text.contains("not a finite number"), "{text}");
    assert_eq!(
        run(r#"from_yaml("a: \".inf\"").a"#),
        Value::Str(".inf".into())
    );
}

//! The shipped standard library (`lib/*.ish`), imported by absolute path and run
//! in-process. Each test pins one defect from the round-30 stdlib review: the
//! value an import gives, or the refusal it must raise.

use std::path::Path;

use iron_shell::env::Env;
use iron_shell::value::Value;

/// The absolute path of a `lib/` file, with forward slashes so it reads as an
/// IronShell string literal on every platform.
fn lib(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("lib")
        .join(name)
        .display()
        .to_string()
        .replace('\\', "/")
        .trim_start_matches("//?/")
        .to_string()
}

/// Imports each of `files`, then evaluates `expr`, in one environment.
fn run_with(files: &[&str], expr: &str) -> anyhow::Result<Value> {
    let (mut env, _) = fresh(files)?;
    let stmts = iron_shell::parser::parse_program(expr)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

/// A fresh environment with `files` imported; returns it for further checks.
fn fresh(files: &[&str]) -> anyhow::Result<(Env, Value)> {
    let mut env = iron_shell::modules::env_with_modules();
    let mut src = String::new();
    for f in files {
        src.push_str(&format!("import \"{}\"\n", lib(f)));
    }
    src.push_str("null");
    let stmts = iron_shell::parser::parse_program(&src)?;
    let v = iron_shell::eval::eval_program(&stmts, &mut env)?;
    Ok((env, v))
}

fn strs(v: &[&str]) -> Value {
    Value::Array(v.iter().map(|s| Value::Str(s.to_string())).collect())
}

fn text(s: &str) -> Value {
    Value::Str(s.to_string())
}

fn ok(files: &[&str], expr: &str) -> Value {
    run_with(files, expr).unwrap_or_else(|e| panic!("`{expr}` failed: {e:#}"))
}

fn refused(files: &[&str], expr: &str) -> String {
    match run_with(files, expr) {
        Ok(v) => panic!("`{expr}` should be refused, got {v:?}"),
        Err(e) => format!("{e:#}"),
    }
}

// ===== string.ish =====

#[test]
fn words_splits_on_any_whitespace_run() {
    assert_eq!(
        ok(&["string.ish"], "words(\"a\\tb\\nc\")"),
        strs(&["a", "b", "c"])
    );
    assert_eq!(
        ok(&["string.ish"], "words(\"  one \\r\\n two\\t\\t three  \")"),
        strs(&["one", "two", "three"])
    );
    assert_eq!(ok(&["string.ish"], "words(\" \\t\\n \")"), strs(&[]));
}

#[test]
fn case_transforms_inherit_the_whitespace_split() {
    assert_eq!(
        ok(&["string.ish"], "title_case(\"hello\\tworld\")"),
        text("Hello World")
    );
    assert_eq!(ok(&["string.ish"], "snake_case(\"a\\nb\")"), text("a_b"));
    assert_eq!(ok(&["string.ish"], "kebab_case(\"a\\r\\nb\")"), text("a-b"));
}

#[test]
fn is_upper_needs_a_cased_letter() {
    assert_eq!(ok(&["string.ish"], "is_upper(\"ABC\")"), Value::Bool(true));
    assert_eq!(ok(&["string.ish"], "is_upper(\"A1\")"), Value::Bool(true));
    assert_eq!(ok(&["string.ish"], "is_upper(\"123\")"), Value::Bool(false));
    assert_eq!(ok(&["string.ish"], "is_upper(\"\")"), Value::Bool(false));
    assert_eq!(ok(&["string.ish"], "is_upper(\"AbC\")"), Value::Bool(false));
}

#[test]
fn is_lower_needs_a_cased_letter() {
    assert_eq!(ok(&["string.ish"], "is_lower(\"abc\")"), Value::Bool(true));
    assert_eq!(ok(&["string.ish"], "is_lower(\"123\")"), Value::Bool(false));
    assert_eq!(ok(&["string.ish"], "is_lower(\"\")"), Value::Bool(false));
    assert_eq!(ok(&["string.ish"], "is_lower(\"aB\")"), Value::Bool(false));
}

#[test]
fn count_substr_of_an_empty_needle_is_python_count() {
    // Python: "abc".count("") is 4 (len + 1 positions).
    assert_eq!(
        ok(&["string.ish"], "count_substr(\"abc\", \"\")"),
        Value::Int(4)
    );
    assert_eq!(
        ok(&["string.ish"], "count_substr(\"abcabc\", \"bc\")"),
        Value::Int(2)
    );
}

#[test]
fn repeat_refuses_a_negative_count() {
    assert_eq!(ok(&["string.ish"], "repeat(\"ab\", 2)"), text("abab"));
    let msg = refused(&["string.ish"], "repeat(\"ab\", -1)");
    assert!(msg.contains("must not be negative"), "{msg}");
}

#[test]
fn importing_string_keeps_the_builtin_lines() {
    // The builtin drops the final terminator and a CR before LF; the lib no
    // longer shadows it with a plain split.
    assert_eq!(
        ok(&["string.ish"], "lines(\"a\\nb\\n\")"),
        strs(&["a", "b"])
    );
    assert_eq!(
        ok(&["string.ish"], "lines(\"a\\r\\nb\")"),
        strs(&["a", "b"])
    );
    assert_eq!(ok(&["string.ish"], "lines(\"\")"), strs(&[]));
}

// ===== io.ish =====

#[test]
fn file_ext_reads_only_the_last_component() {
    assert_eq!(ok(&["io.ish"], "file_ext(\"dir/file\")"), text(""));
    assert_eq!(ok(&["io.ish"], "file_ext(\"a.b/file\")"), text(""));
    assert_eq!(ok(&["io.ish"], "file_ext(\".bashrc\")"), text(""));
    assert_eq!(ok(&["io.ish"], "file_ext(\"x.tar.gz\")"), text("gz"));
    assert_eq!(
        ok(&["io.ish"], "file_ext(\"C:\\\\dir\\\\f.txt\")"),
        text("txt")
    );
}

#[test]
fn file_dir_of_a_root_file_is_the_root() {
    assert_eq!(ok(&["io.ish"], "file_dir(\"/x\")"), text("/"));
    assert_eq!(ok(&["io.ish"], "file_dir(\"a/b/c.txt\")"), text("a/b"));
    assert_eq!(ok(&["io.ish"], "file_dir(\"c.txt\")"), text(""));
}

#[test]
fn file_name_ignores_a_trailing_slash() {
    assert_eq!(ok(&["io.ish"], "file_name(\"a/b/\")"), text("b"));
    assert_eq!(
        ok(&["io.ish"], "file_name(\"C:\\\\x\\\\y.txt\")"),
        text("y.txt")
    );
}

#[test]
fn path_join_puts_one_slash_at_each_joint() {
    assert_eq!(ok(&["io.ish"], "path_join([\"a/\", \"b\"])"), text("a/b"));
    assert_eq!(ok(&["io.ish"], "path_join([\"a\", \"/b\"])"), text("a/b"));
    assert_eq!(ok(&["io.ish"], "path_join([\"a\", \"b\"])"), text("a/b"));
}

#[test]
fn to_json_pretty_is_indented() {
    let v = ok(&["io.ish"], "to_json_pretty({a: 1})");
    match v {
        Value::Str(s) => {
            assert!(s.contains('\n'), "not pretty: {s:?}");
            assert!(s.contains("\n  \"a\": 1"), "not indented: {s:?}");
        }
        other => panic!("expected a String, got {other:?}"),
    }
    assert_eq!(ok(&["io.ish"], "to_json({a: 1})"), text("{\"a\":1}"));
}

// ===== functional.ish =====

#[test]
fn drop_refuses_a_negative_count() {
    assert_eq!(
        ok(&["functional.ish"], "drop([1, 2, 3], 1)"),
        Value::Array(vec![Value::Int(2), Value::Int(3)])
    );
    let msg = refused(&["functional.ish"], "drop([1, 2, 3], -1)");
    assert!(msg.contains("must not be negative"), "{msg}");
}

#[test]
fn importing_functional_does_not_shadow_the_file_find_builtin() {
    let (env, _) = fresh(&["functional.ish"]).expect("import");
    assert!(
        env.get_var("find").is_none(),
        "lib find shadows the builtin"
    );
}

// ===== collection.ish =====

#[test]
fn union_treats_one_and_one_point_zero_as_one_element() {
    assert_eq!(
        ok(&["collection.ish"], "len(union([1, 1.0], [1.0]))"),
        Value::Int(1)
    );
    assert_eq!(
        ok(&["collection.ish"], "union([1, 2], [2, 3]) == [1, 2, 3]"),
        Value::Bool(true)
    );
    assert_eq!(
        ok(&["collection.ish"], "len(intersect([1, 1.0], [1.0]))"),
        Value::Int(1)
    );
}

// ===== math.ish =====

#[test]
fn factorial_and_fib_refuse_negative_input() {
    assert_eq!(ok(&["math.ish"], "factorial(5)"), Value::Int(120));
    assert_eq!(ok(&["math.ish"], "fib(10)"), Value::Int(55));
    let msg = refused(&["math.ish"], "factorial(-1)");
    assert!(msg.contains("must not be negative"), "{msg}");
    let msg = refused(&["math.ish"], "fib(-1)");
    assert!(msg.contains("must not be negative"), "{msg}");
}

#[test]
fn importing_math_keeps_the_builtin_mean() {
    // The builtin averages to a Float; the removed lib version gave integer division.
    assert_eq!(ok(&["math.ish"], "[1, 2] | mean"), Value::Float(1.5));
}

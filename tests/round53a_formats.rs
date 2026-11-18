//! Round 53a: data-format fixes (from_yaml error code, from_jsonl wording).

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    match iron_shell::eval::eval_program(&stmts, &mut env) {
        Ok(v) => v,
        Err(_) => panic!("the program failed"),
    }
}

fn s(text: &str) -> Value {
    Value::Str(text.to_string())
}

#[test]
fn from_yaml_and_open_report_a_parse_failure_with_one_code() {
    let dir = std::env::temp_dir().join(format!("ish_r53a_yaml_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bad.yaml");
    std::fs::write(&path, "a: [1, 2\nb: &x 3\n").unwrap();
    let p = path
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    let src = format!(
        "a = try {{ from_yaml(\"a: &x 1\nb: *x\n\") }} catch e {{ e.code }}\n\
         b = try {{ open(\"{p}\") }} catch e {{ e.code }}\n\
         [a, b]"
    );
    let got = run(&src);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(got, Value::Array(vec![s("E_BAD_VALUE"), s("E_BAD_VALUE")]));
}

#[test]
fn from_jsonl_names_the_line_and_its_start() {
    let got = run("try { from_jsonl(\"{\\\"a\\\": 1}\n\n{nope}\n\") } catch e { e.message }");
    let Value::Str(text) = got else {
        panic!("not a String")
    };
    assert!(
        text.starts_with("from_jsonl: line 3 is not JSON: {nope}: "),
        "{text}"
    );
}

#[test]
fn a_hash_that_starts_a_yaml_value_is_a_comment() {
    assert_eq!(
        run("from_yaml(\"c: #only\nd: x #y\n\")"),
        run("{c: null, d: \"x\"}")
    );
    assert_eq!(run("from_yaml(\"- #c\n- 1\n\")"), run("[null, 1]"));
    assert_eq!(run("from_yaml(\"a: #note\n  b: 1\n\")"), run("{a: {b: 1}}"));
}

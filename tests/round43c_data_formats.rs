//! Round 43c: numbers stay finite, from_yaml reads common YAML and refuses
//! what YAML forbids, from_toml names a duplicate table, from_jsonl and
//! to_jsonl exist, unique compares as `==` does, archive_test reports an unsafe
//! entry, and an empty file name is not "no file named ''".

use iron_shell::value::Value;
use std::io::Write as _;
use std::process::Command;

fn eval(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src)?;
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn run(src: &str) -> Value {
    eval(src).unwrap_or_else(|e| panic!("`{src}` failed: {e}"))
}

fn refusal(src: &str) -> String {
    match eval(src) {
        Ok(v) => panic!("`{src}` should be refused, but answered {v}"),
        Err(e) => format!("{e:#}"),
    }
}

fn text(s: &str) -> Value {
    Value::Str(s.to_string())
}

fn ints(v: &[i64]) -> Value {
    Value::Array(v.iter().map(|i| Value::Int(*i)).collect())
}

// ---- 1: finite numbers ----------------------------------------------------

#[test]
fn a_float_literal_that_overflows_is_a_parse_error() {
    assert!(iron_shell::parser::parse_program("print(1e400)").is_err());
    assert!(iron_shell::parser::parse_program("x = -1e999").is_err());
    assert!(iron_shell::parser::parse_program("print(1e308)").is_ok());
}

#[test]
fn from_yaml_refuses_a_float_that_overflows() {
    for src in ["a: 1e400", "a: -1e999", "a: [1e400]", "a: {b: 1e400}"] {
        let msg = refusal(&format!("from_yaml({src:?})"));
        assert!(msg.contains("not a finite number"), "{src}: {msg}");
    }
}

#[test]
fn to_yaml_and_to_toml_refuse_a_non_finite_float() {
    // No literal spells infinity, so the value is built by the Rust API.
    let rec = Value::Record(
        [("x".to_string(), Value::Float(f64::INFINITY))]
            .into_iter()
            .collect(),
    );
    let mut env = iron_shell::modules::env_with_modules();
    env.set_var("v", rec).unwrap();
    for call in ["to_yaml(v)", "to_toml(v)"] {
        let stmts = iron_shell::parser::parse_program(call).unwrap();
        let err = iron_shell::eval::eval_program(&stmts, &mut env)
            .expect_err("a non-finite float must be refused");
        assert!(format!("{err:#}").contains("infinity"), "{call}: {err:#}");
    }
}

// ---- 2 and 3: from_yaml ---------------------------------------------------

#[test]
fn a_block_sequence_may_sit_at_its_keys_indent() {
    assert_eq!(run(r#"from_yaml("a:\n- 1\n- 2\n").a"#), ints(&[1, 2]));
    let doc = r#"from_yaml("a:\n- x: 1\n  y: 2\nb: 3\n")"#;
    assert_eq!(run(&format!("{doc}.b")), Value::Int(3));
    assert_eq!(run(&format!("{doc}.a | len")), Value::Int(1));
    assert_eq!(
        run(&format!("{doc}.a | first | fn(m) => m.y")),
        Value::Int(2)
    );
}

#[test]
fn flow_collections_take_plain_scalars() {
    assert_eq!(
        run(r#"from_yaml("a: {b: 1, c: [x, y]}").a.b"#),
        Value::Int(1)
    );
    assert_eq!(
        run(r#"from_yaml("a: {b: 1, c: [x, y]}").a.c | last"#),
        text("y")
    );
    assert_eq!(
        run(r#"from_yaml("a: [1, true, ~, 2.5]").a | len"#),
        Value::Int(4)
    );
    // JSON-style flow still reads, and a malformed one is still refused.
    assert_eq!(run(r#"from_yaml("a: {\"b\": 1}").a.b"#), Value::Int(1));
    assert!(refusal(r#"from_yaml("a: [1, 2\n")"#).contains("flow collection"));
}

#[test]
fn unsupported_features_keep_their_messages() {
    assert!(refusal(r#"from_yaml("a: &x 1\n")"#).contains("anchors"));
    assert!(refusal(r#"from_yaml("a: {b: *x}\n")"#).contains("aliases"));
}

#[test]
fn from_yaml_refuses_a_mapping_value_inside_a_value() {
    let msg = refusal(r#"from_yaml("a: x: y")"#);
    assert!(msg.contains("mapping value is not allowed"), "{msg}");
    // A colon with no space after it is still part of the scalar.
    assert_eq!(run(r#"from_yaml("a: 12:30").a"#), text("12:30"));
}

#[test]
fn from_yaml_refuses_tab_indentation() {
    let msg = refusal(r#"from_yaml("a:\n\tb: 1")"#);
    assert!(msg.contains("tab") && msg.contains("spaces"), "{msg}");
}

#[test]
fn from_yaml_strips_a_leading_bom() {
    let mut env = iron_shell::modules::env_with_modules();
    env.set_var("t", text("\u{feff}a: 1\n")).unwrap();
    let stmts = iron_shell::parser::parse_program("from_yaml(t).a").unwrap();
    assert_eq!(
        iron_shell::eval::eval_program(&stmts, &mut env).unwrap(),
        Value::Int(1)
    );
}

// ---- 4: from_toml ---------------------------------------------------------

#[test]
fn from_toml_names_a_duplicate_table() {
    let msg = refusal(r#"from_toml("[a]\n[a]\n")"#);
    assert!(msg.contains("duplicate table `a`"), "{msg}");
}

// ---- 7: JSON lines --------------------------------------------------------

#[test]
fn from_jsonl_and_to_jsonl_round_trip() {
    assert_eq!(
        run(r#"to_jsonl([{a: 1}, {a: 2}])"#),
        text("{\"a\":1}\n{\"a\":2}\n")
    );
    assert_eq!(
        run(r#"from_jsonl("{\"a\": 1}\n\n{\"a\": 2}\n") | len"#),
        Value::Int(2)
    );
    assert_eq!(
        run(r#"[{a: 1}, {a: 2}] | to_jsonl | from_jsonl | last | fn(r) => r.a"#),
        Value::Int(2)
    );
    assert_eq!(run(r#"to_jsonl([])"#), text(""));
}

#[test]
fn from_jsonl_names_the_bad_line() {
    let msg = refusal(r#"from_jsonl("{\"a\": 1}\n\n{oops}\n")"#);
    assert!(msg.contains("line 3"), "{msg}");
}

#[test]
fn to_jsonl_wants_an_array() {
    assert!(refusal(r#"to_jsonl({a: 1})"#).contains("not an Array"));
}

// ---- 8: unique ------------------------------------------------------------

#[test]
fn unique_compares_as_equals_does() {
    assert_eq!(run("[1, 1.0, 2] | unique | len"), Value::Int(2));
    assert_eq!(run("[1.0, 1] | unique | first"), Value::Float(1.0));
    assert_eq!(run("[1, 1.0] | unique | first"), Value::Int(1));
    assert_eq!(run("arr.unique([1, 1.0, 2.5, 2.5]) | len"), Value::Int(2));
    assert_eq!(run("[[1], [1.0], [2]] | unique | len"), Value::Int(2));
    assert_eq!(run(r#"["1", 1] | unique | len"#), Value::Int(2));
}

// ---- 6: archive_test ------------------------------------------------------

#[test]
fn archive_test_is_false_for_a_zip_with_an_escaping_entry() {
    let root = std::env::temp_dir().join(format!("ae-round43c-zip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let make = |name: &str, entry: &str| {
        let file = std::fs::File::create(root.join(name)).unwrap();
        let mut w = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        w.start_file(entry, opts).unwrap();
        w.write_all(b"x").unwrap();
        w.finish().unwrap();
    };
    make("evil.zip", "../escape.txt");
    make("fine.zip", "ok/inside.txt");
    let session = root.join("session");
    std::fs::create_dir_all(&session).unwrap();
    let ish = |expr: String| {
        let out = Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-c", &expr])
            .current_dir(&root)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &session)
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_AGENT")
            .env_remove("IRONSHELL_WORKSPACE")
            .output()
            .expect("run ish");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    assert_eq!(ish("archive_test(\"evil.zip\")".to_string()), "false");
    assert_eq!(ish("archive_test(\"fine.zip\")".to_string()), "true");
    let _ = std::fs::remove_dir_all(&root);
}

// ---- 9: an empty file name -------------------------------------------------

#[test]
fn an_empty_path_is_named_as_empty_not_as_a_missing_file() {
    let msg = refusal(r#"file_write("", "x")"#);
    assert!(!msg.contains("no file named ''"), "{msg}");
    assert!(msg.contains("empty"), "{msg}");
}

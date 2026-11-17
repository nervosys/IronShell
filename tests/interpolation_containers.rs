//! A String inside an Array or Record is quoted when the container is turned
//! into text by interpolation, so `[1, "a"]` is not read back as `[1, a]`. A top-level String
//! stays raw.

use iron_shell::value::Value;

fn text(src: &str) -> String {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    match iron_shell::eval::eval_program(&stmts, &mut env).expect("runs") {
        Value::Str(s) => s,
        other => panic!("expected a String, got {other:?}"),
    }
}

#[test]
fn interpolated_containers_quote_their_strings() {
    assert_eq!(text(r#""${[1, "a"]}""#), r#"[1, "a"]"#);
    assert_eq!(text(r#""${{k: "v"}}""#), r#"{k: "v"}"#);
    assert_eq!(text(r#""${[["x"], {a: "y"}]}""#), r#"[["x"], {a: "y"}]"#);
}

#[test]
fn a_top_level_string_stays_unquoted() {
    assert_eq!(text(r#""${"a"}!""#), "a!");
}

#[test]
fn str_keeps_its_plain_form() {
    assert_eq!(text(r#"str([1, "a"])"#), "[1, a]");
}

//! Round 41a: an error from a module member names what the caller wrote
//! (`file.read`), not the builtin the member runs (`cat`); the code is unchanged.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn text(v: Value) -> String {
    match v {
        Value::Str(s) => s,
        other => panic!("expected a String, got {other:?}"),
    }
}

#[test]
fn a_module_member_error_names_the_member() {
    let msg = text(run(
        "try { file.read(\"ae-round41a-no-such-file.txt\") } catch e { e.error.message }",
    ));
    assert!(msg.starts_with("file.read:"), "{msg}");
    assert!(!msg.contains("cat:"), "{msg}");
    let builtin = text(run(
        "try { file.read(\"ae-round41a-no-such-file.txt\") } catch e { e.error.builtin }",
    ));
    assert_eq!(builtin, "file.read");
}

#[test]
fn the_builtin_called_by_its_own_name_is_unchanged() {
    let msg = text(run(
        "try { cat(\"ae-round41a-no-such-file.txt\") } catch e { e.error.message }",
    ));
    assert!(msg.starts_with("cat:"), "{msg}");
}

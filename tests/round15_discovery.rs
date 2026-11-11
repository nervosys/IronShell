//! Round 15: discovery and error messages a cold-start agent hit. `help("name")`
//! describes one builtin, the shell-flag shape is named at parse time, a
//! directory is refused as a directory rather than as a permission failure,
//! mv names the missing destination folder and the directory target, the
//! unknown-module hint names ontology_manifest(), and the mkdir signature takes
//! its path.

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
fn help_with_a_name_is_the_ontology_entry_for_that_builtin() {
    let via_help = run("help(\"ls\")");
    let via_ontology = run("ontology_describe(\"ls\")");
    assert_eq!(via_help, via_ontology);
    match &via_help {
        Value::Record(r) => assert_eq!(r.get("builtin"), Some(&Value::Str("ls".into()))),
        other => panic!("help(\"ls\") should be a Record, got {other:?}"),
    }
}

#[test]
fn help_with_an_unknown_name_refuses_as_ontology_describe_does() {
    let code = text(run(
        "try { help(\"no_such_builtin_r15\") } catch e { e.error.code }",
    ));
    let ont = text(run(
        "try { ontology_describe(\"no_such_builtin_r15\") } catch e { e.error.code }",
    ));
    assert_eq!(code, ont);
    assert_eq!(code, "E_UNKNOWN_BUILTIN");
}

#[test]
fn bare_help_is_still_the_short_reference_and_points_at_help_name() {
    let short = text(run("help()"));
    assert!(short.contains("help(\"name\")"), "{short}");
    assert!(short.contains("ontology_manifest()"), "{short}");
}

//! Import refusals from round 30, in-process. Each fixture is a fresh temp
//! directory, and each module has its own name, so the process-wide module cache
//! cannot make one test see another's file.

use std::path::{Path, PathBuf};

use iron_shell::env::Env;
use iron_shell::parser::ParseError;
use iron_shell::safety::{ErrorCode, SafetyError};

fn scratch(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("ae_r30_{tag}_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn write(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).expect("write fixture");
}

/// Runs `code` as a script in `dir`, the way `ish FILE` does.
fn run_in(dir: &Path, code: &str, env: &mut Env) -> anyhow::Result<iron_shell::value::Value> {
    let script = dir.join("main.ish");
    let _entry = iron_shell::module_loader::enter_entry_file(&script);
    let stmts = iron_shell::parser::parse_program(code)?;
    iron_shell::eval::eval_program(&stmts, env)
}

fn fresh_env() -> Env {
    iron_shell::modules::env_with_modules()
}

// ===== item 9: a syntax error inside an imported module =====

#[test]
fn a_syntax_error_in_an_imported_module_is_an_e_parse_naming_the_module() {
    let dir = scratch("parse");
    write(&dir, "bad.ish", "pub let x = 1\npub let y = (1 +\n");
    let mut env = fresh_env();
    let err = run_in(&dir, "import \"bad.ish\"", &mut env).expect_err("must not import");

    let pe = err
        .downcast_ref::<ParseError>()
        .unwrap_or_else(|| panic!("not a ParseError: {err:#}"));
    assert!(
        pe.message.contains("bad.ish"),
        "no module named: {}",
        pe.message
    );
    assert!(pe.line.is_some(), "no line: {}", pe.message);
    // The JSON form is the error itself, not a string inside another message.
    assert!(
        format!("{err}").starts_with(r#"{"error":{"code":"E_PARSE""#),
        "{err}"
    );
    assert_eq!(iron_shell::repl::exit_status_for(&err), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

// ===== item 10: a name the module defines without `pub` =====

#[test]
fn a_private_name_in_a_named_import_says_mark_it_pub() {
    let dir = scratch("private");
    write(&dir, "a.ish", "let hidden = 1\npub let shown = 2\n");
    let mut env = fresh_env();
    let err = run_in(&dir, "import { hidden } from \"a.ish\"", &mut env)
        .expect_err("a private name must not import");
    let se = err
        .downcast_ref::<SafetyError>()
        .unwrap_or_else(|| panic!("not coded: {err:#}"));
    assert_eq!(se.code, ErrorCode::NotFound);
    assert_eq!(
        se.hint,
        "`hidden` is defined in a.ish but not exported; mark it `pub`"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_name_in_a_named_import_lists_the_exports() {
    let dir = scratch("missing");
    write(&dir, "a.ish", "pub let shown = 2\n");
    let mut env = fresh_env();
    let err = run_in(&dir, "import { nope } from \"a.ish\"", &mut env)
        .expect_err("an unknown name must not import");
    let se = err
        .downcast_ref::<SafetyError>()
        .unwrap_or_else(|| panic!("not coded: {err:#}"));
    assert_eq!(se.code, ErrorCode::NotFound);
    assert!(se.hint.contains("exports of"), "{}", se.hint);
    assert!(se.hint.contains("shown"), "{}", se.hint);
    assert!(!se.hint.contains("list the available"), "{}", se.hint);
    let _ = std::fs::remove_dir_all(&dir);
}

// ===== item 11: assigning a field of an import alias =====

#[test]
fn assigning_a_field_of_an_import_alias_is_refused_as_immutable() {
    let dir = scratch("alias_set");
    write(&dir, "m.ish", "pub let x = 1\n");
    let mut env = fresh_env();
    let err = run_in(&dir, "import \"m.ish\" as a\na.x = 7", &mut env)
        .expect_err("an alias is immutable");
    let msg = format!("{err:#}");
    assert!(msg.contains("immutable"), "{msg}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reading_a_field_of_an_import_alias_still_works() {
    let dir = scratch("alias_read");
    write(&dir, "m.ish", "pub let x = 1\n");
    let mut env = fresh_env();
    let v = run_in(&dir, "import \"m.ish\" as a\na.x", &mut env).expect("reads");
    assert_eq!(v, iron_shell::value::Value::Int(1));
    let _ = std::fs::remove_dir_all(&dir);
}

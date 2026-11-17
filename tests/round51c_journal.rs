//! Round 51c: undo({step: n}) and its option errors.

fn try_run(src: &str) -> anyhow::Result<iron_shell::value::Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env)
}

#[test]
fn undo_unknown_option_lists_the_valid_keys() {
    let e = try_run("undo({bogus: 1})").unwrap_err().to_string();
    assert!(e.contains("id, step and force"), "{e}");
    assert!(e.contains("bogus"), "{e}");
}

#[test]
fn undo_step_must_be_a_positive_int() {
    let e = try_run("undo({step: 0})").unwrap_err().to_string();
    assert!(e.contains("step"), "{e}");
}

#[test]
fn undo_takes_id_or_step_not_both() {
    let e = try_run("undo({id: 1, step: 1})").unwrap_err().to_string();
    assert!(e.contains("not both"), "{e}");
}

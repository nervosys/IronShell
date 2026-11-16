//! `rm(path, {recursive: true})` names the real fix: rm has no recursive
//! option, rmdir(path, true) does.

fn run_err(src: &str) -> String {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    let e = iron_shell::eval::eval_program(&stmts, &mut env).expect_err("must fail");
    format!("{e:#}")
}

#[test]
fn rm_with_a_record_second_argument_points_at_rmdir() {
    let msg = run_err("rm(\"round47c-no-such-dir\", {recursive: true})");
    assert!(msg.contains("rmdir(path, true)"), "got: {msg}");
    assert!(msg.contains("no recursive option"), "got: {msg}");
}

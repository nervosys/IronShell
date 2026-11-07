//! Rebinding a name that is taken gives a hint that is about that name.
//!
//! `make = fn(x) => x` where `make` is a module used to report
//! "call `use a different name, or `let mut` to rebind` first, then retry this
//! unchanged": the statement's advice had been passed to a template that is
//! meant for a builtin to call first. The hint now says what the name is and
//! what to do about it.

use iron_shell::env::Env;
use iron_shell::safety::{ErrorCode, SafetyError};
use iron_shell::{eval, parser};

fn run(env: &mut Env, src: &str) -> anyhow::Result<iron_shell::value::Value> {
    let stmts = parser::parse_program(src)?;
    eval::eval_program(&stmts, env)
}

fn refusal(e: &anyhow::Error) -> SafetyError {
    e.downcast_ref::<SafetyError>()
        .cloned()
        .unwrap_or_else(|| panic!("structured error expected: {e:#}"))
}

#[test]
fn rebinding_a_module_name_says_it_is_a_module() {
    let mut env = iron_shell::modules::env_with_modules();
    let e = run(&mut env, "file = 5").expect_err("a module name cannot be rebound");
    let se = refusal(&e);
    assert_eq!(se.code, ErrorCode::BadState);
    assert!(
        se.hint
            .contains("`file` is a module name; use a different name"),
        "{}",
        se.hint
    );
    assert!(
        !se.hint.contains("call `"),
        "no builtin to call first: {}",
        se.hint
    );
    assert!(!se.hint.contains("to rebind"), "{}", se.hint);
    assert!(
        !se.expected.contains("the state left by"),
        "the expected field must not name a builtin: {}",
        se.expected
    );
    assert!(se.message.contains("built-in module"), "{}", se.message);
}

#[test]
fn rebinding_an_immutable_variable_gives_the_let_mut_advice() {
    let mut env = Env::new();
    run(&mut env, "x = 1").unwrap();
    let e = run(&mut env, "x = 2").expect_err("an immutable binding is not reassigned");
    let se = refusal(&e);
    assert_eq!(se.code, ErrorCode::BadState);
    assert!(
        se.message.contains("Cannot reassign immutable variable"),
        "{}",
        se.message
    );
    assert!(se.hint.contains("let mut x"), "{}", se.hint);
    assert!(!se.hint.contains("call `"), "{}", se.hint);
}

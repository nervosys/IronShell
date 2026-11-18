//! Round 53a: nested Array literals and concatenations are charged to the memory
//! budget before they are built.
//!
//! The test binary does not install the counting allocator, so live bytes read
//! zero and only a single request larger than the budget is refused. The
//! budget is process-wide, so every test holds `LOCK` and restores it.

use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

fn code_of(src: &str) -> String {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    match iron_shell::eval::eval_program(&stmts, &mut env) {
        Ok(_) => "ok".to_string(),
        Err(e) => match e.downcast_ref::<iron_shell::safety::SafetyError>() {
            Some(se) => format!("{:?}", se.code),
            None => "other".to_string(),
        },
    }
}

fn with_limit<T>(mb: usize, f: impl FnOnce() -> T) -> T {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    iron_shell::memory::set_limit_mb(mb);
    let out = f();
    iron_shell::memory::set_limit_mb(0);
    out
}

#[test]
fn doubling_an_array_literal_is_refused_before_it_is_built() {
    let mut src = String::from("mut a = [1]\n");
    for _ in 0..22 {
        src.push_str("a = [a, a]\n");
    }
    src.push_str("len(a)");
    assert_eq!(with_limit(1, || code_of(&src)), "BudgetExceeded");
}

#[test]
fn doubling_by_concatenation_is_refused_before_it_is_built() {
    let mut src = String::from("mut a = [1]\n");
    for _ in 0..24 {
        src.push_str("a = a + a\n");
    }
    src.push_str("len(a)");
    assert_eq!(with_limit(1, || code_of(&src)), "BudgetExceeded");
}

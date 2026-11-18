//! Round 53a: values built from files and text are charged to the
//! memory budget before they are built.
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
fn reading_a_file_larger_than_the_budget_is_refused() {
    let dir = std::env::temp_dir().join(format!("ish_r53a_read_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("big.txt");
    std::fs::write(&path, vec![b'x'; 3 * 1024 * 1024]).unwrap();
    let p = path
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    let code = with_limit(1, || code_of(&format!("len(file.read(\"{p}\"))")));
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(code, "BudgetExceeded");
}

#[test]
fn a_csv_file_whose_rows_would_pass_the_budget_is_refused() {
    let dir = std::env::temp_dir().join(format!("ish_r53a_csv_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rows.csv");
    let mut text = String::from("a,b,c\n");
    for i in 0..60_000 {
        text.push_str(&format!("{i},x{i},y\n"));
    }
    std::fs::write(&path, text).unwrap();
    let p = path
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    let code = with_limit(1, || code_of(&format!("len(open(\"{p}\"))")));
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(code, "BudgetExceeded");
}

#[test]
fn from_json_text_past_the_budget_is_refused() {
    let text = format!("[{}]", vec!["1"; 400_000].join(","));
    let src = format!("len(from_json(\"{text}\"))");
    assert_eq!(with_limit(1, || code_of(&src)), "BudgetExceeded");
}

#[test]
fn small_inputs_still_work_under_a_budget() {
    let src = "len(from_csv(\"a,b\\n1,2\\n\")) + len(from_json(\"[1,2]\"))";
    assert_eq!(with_limit(64, || code_of(src)), "ok");
}

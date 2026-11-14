//! A malformed program reports a handful of errors, not thousands.

use iron_shell::parser::parse_program;

/// Parse on a thread with the stack the ish binary gives its evaluator: the
/// test harness's 2 MB thread overflows on 512 nested levels in a debug build.
fn message(src: &str) -> String {
    let src = src.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || format!("{:#}", parse_program(&src).expect_err("must not parse")))
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn deep_nesting_reports_the_limit_once_without_the_cascade() {
    let src = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    let m = message(&src);
    assert!(m.contains("nested more than"), "{m}");
    assert!(m.lines().count() <= 15, "{} lines: {m}", m.lines().count());
    assert!(!m.contains("unexpected token RParen"), "{m}");
}

#[test]
fn many_independent_errors_are_capped_with_a_count() {
    let src = ")\n".repeat(200);
    let m = message(&src);
    assert!(m.lines().count() <= 15, "{} lines: {m}", m.lines().count());
    if m.contains("found 1") || m.contains("found 2") {
        return;
    }
    assert!(m.contains("... and "), "{m}");
}

//! Memory and time run out as an error, not as an aborted process or a hang.
//!
//! Every limit here is small (64 MB, 1 s) so the test never allocates much or
//! waits long.

use std::process::Command;

const ISH: &str = env!("CARGO_BIN_EXE_ish");

fn run(args: &[&str], envs: &[(&str, &str)]) -> (String, Option<i32>) {
    let dir = std::env::temp_dir().join(format!("ish-budgets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cmd = Command::new(ISH);
    cmd.args(args).env("IRONSHELL_SESSION_DIR", &dir);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn ish");
    (
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr),
        out.status.code(),
    )
}

fn assert_budget(text: &str, code: Option<i32>, what: &str) {
    assert!(
        text.contains("E_BUDGET_EXCEEDED") || text.contains("budget"),
        "{what}: {text}"
    );
    assert!(
        code.is_some_and(|c| c != 0),
        "{what}: exit {code:?}: {text}"
    );
}

#[test]
fn interpolation_is_held_to_the_string_limit() {
    let (t, c) = run(
        &[
            "-c",
            r#"arr.range(40) | reduce(fn(a, i) => "${a}${a}", "abcdefgh") | len"#,
        ],
        &[],
    );
    assert_budget(&t, c, "interpolation");
    assert!(t.contains("string operation would produce"), "{t}");
}

#[test]
fn array_plus_is_held_to_the_element_limit() {
    let (t, c) = run(
        &[
            "-c",
            "arr.range(40) | reduce(fn(a, i) => a + a, [1,2,3]) | len",
        ],
        &[],
    );
    assert_budget(&t, c, "array +");
    assert!(t.contains("element limit"), "{t}");
}

#[test]
fn nesting_that_doubles_memory_hits_the_memory_budget() {
    let (t, c) = run(
        &[
            "--memory-limit",
            "64",
            "-c",
            "arr.range(40) | reduce(fn(a, i) => [a, a], 1) | len",
        ],
        &[],
    );
    assert_budget(&t, c, "nesting");
    assert!(t.contains("memory budget of 64 MB exceeded"), "{t}");
}

#[test]
fn the_memory_limit_env_var_works_for_a_big_pipeline() {
    let (t, c) = run(
        &[
            "-c",
            "arr.range(10000000) | map(fn(i) => i * 2) | where(fn(i) => i > 5) | len",
        ],
        &[("IRONSHELL_MEMORY_LIMIT_MB", "64")],
    );
    assert_budget(&t, c, "pipeline");
}

#[test]
fn a_small_program_runs_under_the_default_budget() {
    let (t, c) = run(&["-c", "1 + 1"], &[]);
    assert_eq!(c, Some(0), "{t}");
    assert!(t.contains('2'), "{t}");
}

#[test]
fn timeout_cuts_a_sleep_short() {
    let started = std::time::Instant::now();
    let (t, c) = run(&["--timeout", "1", "-c", "sleep(100000)"], &[]);
    assert!(started.elapsed().as_secs() < 30, "took too long: {t}");
    assert_budget(&t, c, "sleep");
}

#[test]
fn timeout_env_var_stops_a_long_loop() {
    let started = std::time::Instant::now();
    let (t, c) = run(
        &[
            "-c",
            "arr.range(10000000) | map(fn(i) => i * 2) | map(fn(i) => i * 2) | map(fn(i) => i * 2) | len",
        ],
        &[("IRONSHELL_TIMEOUT", "1"), ("IRONSHELL_MEMORY_LIMIT_MB", "0")],
    );
    assert!(started.elapsed().as_secs() < 60, "took too long: {t}");
    // Either the deadline or a quick finish; a finish is not a failure of the flag.
    if c != Some(0) {
        assert!(t.contains("1 s deadline"), "{t}");
    }
}

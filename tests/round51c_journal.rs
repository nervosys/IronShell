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

#[test]
fn agent_output_that_was_redacted_says_so_on_stderr() {
    let dir = std::env::temp_dir().join(format!("ish_r51c_redact_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--agent", "-c", r#""password=hunter2hunter2""#])
        .env("IRONSHELL_SESSION_DIR", &dir)
        .env_remove("IRONSHELL_REDACT")
        .output()
        .expect("runs ish");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stdout.contains("hunter2hunter2"), "{stdout}");
    assert!(stdout.contains("[REDACTED]"), "{stdout}");
    assert!(stderr.contains("redacted from the output"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_brace_glob_sorts_across_alternatives() {
    let dir = std::env::temp_dir().join(format!("ish_r51c_glob_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["a.py", "b.rs", "c.py", "d.rs"] {
        std::fs::write(dir.join(name), "x").unwrap();
    }
    let pattern = format!(
        "{}/*.{{rs,py}}",
        dir.to_string_lossy().replace(char::from(92), "/")
    );
    let v = try_run(&format!("glob(\"{pattern}\")")).expect("glob runs");
    let iron_shell::value::Value::Array(items) = v else {
        panic!("not an array");
    };
    let names: Vec<String> = items
        .iter()
        .map(|i| {
            i.to_display_string()
                .rsplit('/')
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(names, ["a.py", "b.rs", "c.py", "d.rs"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ls_with_options_hints_at_glob() {
    let v =
        try_run("try { ls(\"src\", {recursive: true}) } catch e { e.error.hint }").expect("caught");
    let text = v.to_display_string();
    assert!(text.contains("glob("), "{text}");
}

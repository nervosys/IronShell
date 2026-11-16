//! Round 47a: error hints that pointed the wrong way, run through the real binary.

use std::process::Command;

/// Runs `ish -c src` with `extra` flags first; returns (stdout, stderr, exit code).
fn run(extra: &[&str], src: &str) -> (String, String, i32) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ae_r47a_{}_{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::create_dir_all(&dir);
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(extra)
        .arg("-c")
        .arg(src)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .current_dir(&dir)
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn unknown_module_member_does_not_suggest_none() {
    let (out, err, code) = run(&["--agent"], "file.nope()");
    let all = format!("{out}{err}");
    assert_ne!(code, 0, "{all}");
    assert!(all.contains("E_UNKNOWN_BUILTIN"), "{all}");
    assert!(!all.contains("did you mean: None"), "{all}");
    assert!(!all.contains("\"None\""), "{all}");
    assert!(all.contains("\"retryable\":false"), "{all}");
}

#[test]
fn unterminated_single_quote_says_use_double_quotes() {
    let (out, err, code) = run(&[], r"'a\'b");
    let all = format!("{out}{err}");
    assert_ne!(code, 0, "{all}");
    assert!(all.contains("takes no escapes"), "{all}");
    assert!(all.contains("double-quoted"), "{all}");
    assert!(!all.contains("or escape the quote inside it"), "{all}");
    // A double-quoted one still may escape.
    let (out, err, _) = run(&[], "\"abc");
    assert!(
        format!("{out}{err}").contains("escape the quote"),
        "{out}{err}"
    );
}

#[test]
fn unclosed_hole_in_single_quotes_points_to_double_quotes() {
    let (out, err, code) = run(&[], "'a ${x'");
    let all = format!("{out}{err}");
    assert_ne!(code, 0, "{all}");
    assert!(all.contains("double-quoted string with"), "{all}");
}

#[test]
fn match_arms_need_a_separator() {
    let (out, err, code) = run(&[], "match 1 { 1 => 2 _ => 3 }");
    let all = format!("{out}{err}");
    assert_ne!(code, 0, "{all}");
    assert!(all.contains("E_PARSE"), "{all}");
    assert!(
        all.contains("separate match arms with a comma or a newline"),
        "{all}"
    );
    let (out, _, code) = run(&[], "match 1 { 1 => 2, _ => 3 }");
    assert_eq!((code, out.as_str()), (0, "2"));
    let (out, _, code) = run(&[], "match 1 {\n 1 => 2\n _ => 3\n}");
    assert_eq!((code, out.as_str()), (0, "2"));
}

#[test]
fn piping_into_a_literal_is_refused() {
    for src in [
        "1 | 2",
        "[1] | 5",
        "1 | \"s\"",
        "1 | [2]",
        "1 | true",
        "1 | null",
    ] {
        let (out, err, code) = run(&[], src);
        let all = format!("{out}{err}");
        assert_ne!(code, 0, "{src}: {all}");
        assert!(all.contains("cannot pipe into"), "{src}: {all}");
        assert!(
            all.contains("the right side of | must be a function call or builtin"),
            "{src}: {all}"
        );
    }
    let (out, _, code) = run(&[], "[1, 2] | len");
    assert_eq!((code, out.as_str()), (0, "2"));
}

#[test]
fn empty_array_extremum_hint_names_the_empty_array() {
    for src in ["[] | max", "[] | min", "max([])"] {
        let (out, err, code) = run(&[], src);
        let all = format!("{out}{err}");
        assert_ne!(code, 0, "{src}: {all}");
        assert!(
            all.contains("the array is empty; check len(xs) > 0 first"),
            "{src}: {all}"
        );
        assert!(!all.contains("max takes Array"), "{src}: {all}");
    }
}

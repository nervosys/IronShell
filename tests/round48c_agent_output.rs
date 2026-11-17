//! Round 48c: agent-mode output and error text.
//!
//! Each test runs the `ish` binary with a private session directory.

use std::path::PathBuf;
use std::process::Command;

fn session(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ish_r48c_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("session dir");
    dir
}

/// Run `ish` with `args`, returning stdout and stderr with CRLF folded to LF.
fn ish(dir: &PathBuf, args: &[&str]) -> (String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env_remove("IRONSHELL_HANDLE_BYTES")
        .env_remove("IRONSHELL_DETERMINISTIC")
        .output()
        .expect("run ish");
    (
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n"),
    )
}

fn agent(dir: &PathBuf, code: &str) -> (String, String) {
    ish(dir, &["--agent", "-c", code])
}

#[test]
fn a_zero_pid_error_says_it_needs_an_id_above_zero() {
    let dir = session("pid0");
    for code in ["proc.info(0)", "proc.exists(0)"] {
        let (out, err) = agent(&dir, code);
        let all = format!("{out}{err}");
        assert!(all.contains("greater than 0"), "{code}: {all}");
        assert!(all.contains("process groups"), "{code}: {all}");
        assert!(!all.contains("addresses groups"), "{code}: {all}");
    }
}

#[test]
fn a_handle_missing_from_this_session_says_so() {
    let dir = session("nohandle");
    let (out, err) = agent(&dir, r#"handle("h1")"#);
    let all = format!("{out}{err}");
    assert!(all.contains("no handles live in this session"), "{all}");
    assert!(all.contains("IRONSHELL_SESSION_DIR="), "{all}");
    assert!(
        all.contains("readable only from the session that made it"),
        "{all}"
    );
    assert!(!all.contains("do not survive the process"), "{all}");
}

#[test]
fn deterministic_floats_keep_their_point_in_exponent_form() {
    let dir = session("detfloat");
    for (code, want) in [
        ("1e16", "1.0e16"),
        ("1e21", "1.0e21"),
        ("1e-7", "1.0e-7"),
        ("1.5e300", "1.5e300"),
        ("5e-324", "5.0e-324"),
        ("-2e-9", "-2.0e-9"),
        ("1.5", "1.5"),
        ("2.0", "2.0"),
    ] {
        let (out, err) = ish(&dir, &["--deterministic", "-c", code]);
        assert_eq!(out.trim(), want, "{code}: {err}");
    }
}

#[test]
fn a_big_map_near_the_memory_limit_is_refused_not_aborted() {
    let dir = session("memlimit");
    let (out, err) = ish(
        &dir,
        &[
            "--agent",
            "--memory-limit",
            "300",
            "-c",
            "arr.range(4000000) | map(fn(i) => i * 2) | len",
        ],
    );
    let all = format!("{out}{err}");
    assert!(all.contains("E_BUDGET_EXCEEDED"), "{all}");
    assert!(!all.contains("memory allocation of"), "{all}");
}

fn lines_of(s: &str) -> Vec<&str> {
    s.lines().collect()
}

#[test]
fn a_table_cell_that_reads_as_another_type_is_quoted() {
    let dir = session("cells");
    let (out, err) = agent(
        &dir,
        r#"[{v: "null"}, {v: null}, {v: "true"}, {v: true}, {v: "1"}, {v: 1}, {v: ""}, {v: "@x"}, {v: " pad"}, {v: "a\tb"}, {v: "ok"}]"#,
    );
    assert_eq!(
        lines_of(&out),
        vec![
            "v", "\"null\"", "null", "\"true\"", "true", "\"1\"", "1", "\"\"", "\"@x\"",
            "\" pad\"", "\"a\tb\"", "ok"
        ],
        "stderr: {err}"
    );
}

#[test]
fn an_ordinary_string_cell_stays_bare() {
    let dir = session("bare");
    let (out, err) = agent(&dir, r#"[{v: "abc"}, {v: "x y"}]"#);
    assert_eq!(lines_of(&out), vec!["v", "abc", "x y"], "stderr: {err}");
}

#[test]
fn a_first_row_cell_that_spells_a_directive_is_quoted() {
    let dir = session("directive");
    let (out, err) = agent(&dir, r#"[{a: "@same"}, {a: "@const"}]"#);
    assert_eq!(
        lines_of(&out),
        vec!["a", "\"@same\"", "\"@const\""],
        "stderr: {err}"
    );
    assert!(!out.lines().any(|l| l.starts_with("@same")), "{out}");
    let (out, err) = agent(
        &dir,
        r#"([{a: "@same"}, {a: "@const"}] | aecon | aecon_decode) == [{a: "@same"}, {a: "@const"}]"#,
    );
    assert_eq!(out.trim(), "true", "stderr: {err}");
}

#[test]
fn a_record_key_that_needs_it_is_quoted() {
    let dir = session("reckeys");
    let (out, err) = agent(
        &dir,
        r#"json.parse('{"a b":1,"c=d":"x y","":2,"plain":3}')"#,
    );
    assert_eq!(
        out.trim(),
        r#"{""=2 "a b"=1 "c=d"="x y" plain=3}"#,
        "stderr: {err}"
    );
}

#[test]
fn a_table_header_key_that_needs_it_is_quoted_and_round_trips() {
    let dir = session("hdrkeys");
    let rows = r#"json.parse('[{"a b":1,"c=d":2,"k z":7},{"a b":3,"c=d":4,"k z":7}]')"#;
    let (out, err) = agent(&dir, rows);
    assert_eq!(
        lines_of(&out),
        vec!["\"a b\"\t\"c=d\"", "@const \"k z\"=7", "1\t2", "3\t4"],
        "stderr: {err}"
    );
    let (out, err) = agent(&dir, &format!("({rows} | aecon | aecon_decode) == {rows}"));
    assert_eq!(out.trim(), "true", "stderr: {err}");
}

#[test]
fn top_level_null_and_empty_values_are_distinguishable() {
    let dir = session("toplevel");
    for (code, want) in [
        ("null", "null"),
        (r#""""#, r#""""#),
        ("[{}]", "[{}]"),
        ("[{}, {}]", "[{},{}]"),
        ("[]", "[]"),
        ("{}", "{}"),
        (r#""hello""#, "hello"),
        (r#""42""#, r#""42""#),
        (r#""null""#, r#""null""#),
        (r#""@handle h1""#, r#""@handle h1""#),
    ] {
        let (out, err) = agent(&dir, code);
        assert_eq!(out.trim_end_matches('\n'), want, "{code}: {err}");
    }
}

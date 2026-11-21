//! Round 61b: findings from the round 61 usability run.
//! Semicolon CSV, parse-error hints, temp-directory cleanup in agent mode,
//! `ish agent serve` on a taken port, and the API's unknown-builtin suggestions.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ish-r61b-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cwd")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Scratch { root }
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.root.join("cwd").join(name), text).unwrap();
    }

    fn run(&self, flags: &[&str], expr: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(flags)
            .args(["-c", expr])
            .current_dir(self.root.join("cwd"))
            .env("NO_COLOR", "1")
            .env_remove("IRONSHELL_ALLOW_SH")
            .env_remove("IRONSHELL_APPROVE")
            .env_remove("IRONSHELL_WORKSPACE")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .output()
            .expect("run ish")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).trim().to_string()
}

// 1. Semicolon CSV

#[test]
fn a_semicolon_csv_with_three_columns_names_the_separator() {
    let s = Scratch::new("csv3");
    s.write("s.csv", "id;customer;amount\n1;Acme;1.234,50\n");
    let o = s.run(&["--deterministic"], r#"open("s.csv")"#);
    assert!(!o.status.success());
    let e = err(&o);
    assert!(e.contains("E_BAD_ARG"), "{e}");
    assert!(e.contains("semicolon-separated"), "{e}");
    assert!(e.contains("delimiter"), "{e}");
    assert!(e.contains("from_csv(cat("), "{e}");
}

#[test]
fn a_two_column_semicolon_csv_opens_as_read_with_a_stderr_note() {
    let s = Scratch::new("csv2");
    s.write("t.csv", "a;b\n1;2\n");
    let o = s.run(&["--deterministic"], r#"open("t.csv")"#);
    assert!(o.status.success(), "{}", err(&o));
    // stdout is the one-column result, unchanged by the note
    assert_eq!(out(&o), r#"[{"a;b":"1;2"}]"#);
    let e = err(&o);
    assert!(e.contains("semicolon-separated"), "{e}");
    assert!(e.contains(r#"{delimiter: ";"}"#), "{e}");
}

#[test]
fn the_delimiter_option_reads_the_semicolon_file_and_prints_no_note() {
    let s = Scratch::new("csvopt");
    s.write("t.csv", "a;b\n1;2\n");
    let o = s.run(
        &["--deterministic"],
        r#"from_csv(cat("t.csv"), {delimiter: ";"})"#,
    );
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), r#"[{"a":1,"b":2}]"#);
    assert!(!err(&o).contains("separated"), "{}", err(&o));
}

#[test]
fn a_plain_single_column_file_prints_no_note() {
    let s = Scratch::new("csv1");
    s.write("u.csv", "a\n1\n2\n");
    let o = s.run(&["--deterministic"], r#"open("u.csv")"#);
    assert!(o.status.success());
    assert_eq!(out(&o), r#"[{"a":1},{"a":2}]"#);
    assert_eq!(err(&o), "");
}

#[test]
fn a_single_column_whose_cells_hold_semicolons_but_not_the_header_prints_no_note() {
    let s = Scratch::new("csvcell");
    s.write("v.csv", "note\na;b\nc;d\n");
    let o = s.run(&["--deterministic"], r#"open("v.csv")"#);
    assert!(o.status.success());
    assert_eq!(err(&o), "");
}

// 2. Parse-error hints

fn parse_error(expr: &str) -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let s = Scratch::new(&format!("parse{n}"));
    let o = s.run(&[], expr);
    assert!(!o.status.success(), "ran: {}", out(&o));
    let e = err(&o);
    assert!(e.contains("E_PARSE"), "{e}");
    e
}

#[test]
fn a_bare_arrow_lambda_hints_fn() {
    let e = parse_error("[1,2] | map(x => x*2)");
    assert!(e.contains("fn(x) => x * 2"), "{e}");
}

#[test]
fn a_braced_lambda_body_hints_the_arrow_expression() {
    let e = parse_error("[1,2] | map(fn(x) { x*2 })");
    assert!(e.contains("one expression after `=>`"), "{e}");
}

#[test]
fn a_record_bodied_lambda_still_parses() {
    let s = Scratch::new("recbody");
    let o = s.run(&["--deterministic"], "[1] | map(fn(x) {a: x})");
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), r#"[{"a":1}]"#);
}

#[test]
fn a_compound_assignment_hints_the_long_form() {
    let e = parse_error("a = 1\na += 1");
    assert!(e.contains("a = a + 1"), "{e}");
    assert!(e.contains("mut"), "{e}");
}

#[test]
fn import_of_a_bare_name_says_modules_are_built_in() {
    let e = parse_error("import os");
    assert!(e.contains("no `import os`"), "{e}");
    assert!(e.contains("file.read"), "{e}");
    assert!(e.contains("sys.hostname"), "{e}");
}

#[test]
fn a_guard_that_ends_in_a_name_before_the_arrow_still_parses() {
    let s = Scratch::new("guard");
    let o = s.run(
        &["--deterministic"],
        r#"flag = true; match 1 { n if flag => "yes", _ => "no" }"#,
    );
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), r#""yes""#);
}

// 3. Temp-directory cleanup in agent mode

#[test]
fn rmdir_of_a_tempdir_made_in_the_same_process_needs_no_approval() {
    let s = Scratch::new("tmp");
    let o = s.run(
        &["--agent"],
        r#"d = fs_tempdir("x"); file.write(path_join(d, "f.txt"), "hi"); rmdir(d, true); file_exists(d).exists"#,
    );
    assert!(o.status.success(), "{} / {}", out(&o), err(&o));
    assert_eq!(out(&o), "false");
}

#[test]
fn rmdir_of_a_directory_fs_tempdir_did_not_make_still_needs_approval() {
    let s = Scratch::new("other");
    let o = s.run(
        &["--agent"],
        r#"mkdir("plain"); file.write("plain/f.txt", "hi"); rmdir("plain", true)"#,
    );
    assert_eq!(o.status.code(), Some(75), "{} / {}", out(&o), err(&o));
    assert!(err(&o).contains("E_NEEDS_APPROVAL"), "{}", err(&o));
    assert!(s.root.join("cwd/plain/f.txt").exists());
}

#[test]
fn rmdir_of_a_tempdir_another_process_made_still_needs_approval() {
    let s = Scratch::new("proc");
    let made = s.run(&["--agent"], r#"fs_tempdir("keep")"#);
    assert!(made.status.success(), "{}", err(&made));
    let dir = out(&made);
    let o = s.run(&["--agent"], &format!("rmdir({dir:?}, true)"));
    assert_eq!(o.status.code(), Some(75), "{} / {}", out(&o), err(&o));
    assert!(PathBuf::from(&dir).exists());
}

#[test]
fn rmdir_of_the_temp_root_or_a_subdirectory_still_needs_approval() {
    let s = Scratch::new("root");
    let o = s.run(
        &["--agent"],
        r#"d = fs_tempdir("x"); mkdir(path_join(d, "sub")); file.write(path_join(d, "sub/f"), "1"); rmdir(path_join(d, "sub"), true)"#,
    );
    assert_eq!(o.status.code(), Some(75), "{} / {}", out(&o), err(&o));
    let o = s.run(
        &["--agent"],
        r#"d = fs_tempdir("x"); rmdir(path_dirname(d), true)"#,
    );
    // refused outright (the temp root sits under the protected .ironshell
    // directory) or sent to approval; never run
    assert!(
        matches!(o.status.code(), Some(75) | Some(77)),
        "{} / {}",
        out(&o),
        err(&o)
    );
    assert!(s.root.join("cwd/.ironshell/tmp").exists());
}

// 4. Port in use

#[test]
fn agent_serve_on_a_taken_port_fails_before_the_banner() {
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let s = Scratch::new("serve");
    let o = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["agent", "serve", "--port", &port.to_string()])
        .current_dir(s.root.join("cwd"))
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", s.root.join("session"))
        .output()
        .expect("run ish");
    assert!(!o.status.success());
    let stdout = out(&o);
    assert!(!stdout.contains("Auth token"), "{stdout}");
    assert!(!stdout.contains("starting on"), "{stdout}");
    let e = err(&o);
    assert!(e.contains(&format!("port {port} is already in use")), "{e}");
    assert!(e.contains("--port"), "{e}");
}

// 5. API suggestions

#[test]
fn the_api_unknown_builtin_error_has_the_cli_suggestions() {
    let r = iron_shell::agent_api::unknown_builtin_refusal("file_read").expect("unknown");
    let near = r
        .metadata
        .as_ref()
        .and_then(|m| m.get("did_you_mean"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let names: Vec<String> = near
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    assert!(names.iter().any(|n| n == "file.read"), "{names:?}");
    assert!(names.iter().any(|n| n == "read_text"), "{names:?}");
    let hint = r.hint.unwrap_or_default();
    assert!(hint.contains("/api/v1/call/suggest"), "{hint}");
    assert!(hint.contains(r#"["<words>"]"#), "{hint}");
    assert!(!hint.contains("suggest(\"<words>\")"), "{hint}");
}

// 6. git_log count

#[test]
fn git_log_count_is_an_alias_of_n_and_other_keys_are_refused() {
    let s = Scratch::new("gitlog");
    let o = s.run(&["--deterministic"], r#"git_log({bogus: 1})"#);
    assert!(!o.status.success());
    assert!(err(&o).contains("E_BAD_ARG"), "{}", err(&o));
}

#[test]
fn text_repeats_with_an_int() {
    let s = Scratch::new("repeat");
    let o = s.run(&["--deterministic"], r#""5" * 3"#);
    assert_eq!(out(&o), r#""555""#);
}

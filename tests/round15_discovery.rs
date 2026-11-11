//! Round 15: discovery and error messages a cold-start agent hit. `help("name")`
//! describes one builtin, the shell-flag shape is named at parse time, a
//! directory is refused as a directory rather than as a permission failure,
//! mv names the missing destination folder and the directory target, the
//! unknown-module hint names ontology_manifest(), and the mkdir signature takes
//! its path.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn text(v: Value) -> String {
    match v {
        Value::Str(s) => s,
        other => panic!("expected a String, got {other:?}"),
    }
}

#[test]
fn help_with_a_name_is_the_ontology_entry_for_that_builtin() {
    let via_help = run("help(\"ls\")");
    let via_ontology = run("ontology_describe(\"ls\")");
    assert_eq!(via_help, via_ontology);
    match &via_help {
        Value::Record(r) => assert_eq!(r.get("builtin"), Some(&Value::Str("ls".into()))),
        other => panic!("help(\"ls\") should be a Record, got {other:?}"),
    }
}

#[test]
fn help_with_an_unknown_name_refuses_as_ontology_describe_does() {
    let code = text(run(
        "try { help(\"no_such_builtin_r15\") } catch e { e.error.code }",
    ));
    let ont = text(run(
        "try { ontology_describe(\"no_such_builtin_r15\") } catch e { e.error.code }",
    ));
    assert_eq!(code, ont);
    assert_eq!(code, "E_UNKNOWN_BUILTIN");
}

#[test]
fn bare_help_is_still_the_short_reference_and_points_at_help_name() {
    let short = text(run("help()"));
    assert!(short.contains("help(\"name\")"), "{short}");
    assert!(short.contains("ontology_manifest()"), "{short}");
}

// ── shell-style flags ───────────────────────────────────────────────────

/// The E_PARSE hint for a source that must not parse.
fn parse_refusal_hint(src: &str) -> String {
    let err = iron_shell::parser::parse_program(src)
        .expect_err(&format!("{src:?} must be refused as a shell flag"));
    assert!(
        err.to_string().contains("\"code\":\"E_PARSE\""),
        "{src:?} is not E_PARSE: {err}"
    );
    err.downcast_ref::<iron_shell::parser::ParseError>()
        .unwrap_or_else(|| panic!("{src:?} is not a ParseError: {err}"))
        .hint
        .clone()
}

#[test]
fn a_shell_flag_at_the_start_of_a_statement_is_named_as_a_parse_error() {
    let hint = parse_refusal_hint("ls --help");
    assert!(hint.contains("help(\"ls\")"), "{hint}");
    assert!(hint.contains("has no -flags"), "{hint}");
    assert!(hint.contains("ish -b"), "{hint}");

    let hint = parse_refusal_hint("mkdir -p tree/a/b");
    assert!(hint.contains("help(\"mkdir\")"), "{hint}");

    let hint = parse_refusal_hint("grep -r x");
    assert!(hint.contains("help(\"grep\")"), "{hint}");

    // The second statement of a program is a statement start too.
    let hint = parse_refusal_hint("x = 1; ls -la");
    assert!(hint.contains("help(\"ls\")"), "{hint}");
}

#[test]
fn subtraction_negative_numbers_and_mid_statement_words_are_not_flags() {
    for src in [
        "a - b",
        "a-b",
        "x = a -b",
        "x = a - -1",
        "-1",
        "n - -1",
        "n -1",
        "[1, -1]",
        "ls -1",
    ] {
        assert!(
            iron_shell::parser::parse_program(src).is_ok(),
            "{src:?} is not a shell flag and must still parse"
        );
    }
}

// ── directories are named as directories ────────────────────────────────

#[test]
fn rm_of_a_directory_is_refused_as_a_directory_with_the_rmdir_call() {
    let out = text(run("let d = fs_tempdir(\"ae_r15_rm_dir\"); \
         let r = try { rm(d) } catch e { e.error.code + \"|\" + e.error.hint }; \
         rmdir(d, true); r"));
    assert!(out.starts_with("E_BAD_ARG|"), "{out}");
    assert!(!out.contains("E_OS_DENIED"), "{out}");
    assert!(
        out.contains("is a directory; remove it with rmdir("),
        "{out}"
    );
    assert!(out.contains(", true)"), "{out}");
}

#[test]
fn rm_still_removes_a_file() {
    let out = text(run("let d = fs_tempdir(\"ae_r15_rm_file\"); \
         file_write(d + \"/a.txt\", \"x\"); \
         let r = rm(d + \"/a.txt\"); \
         rmdir(d, true); r"));
    assert!(out.starts_with("removed:"), "{out}");
}

#[test]
fn crypto_hash_file_of_a_directory_says_so_and_names_the_glob() {
    let out = text(run("let d = fs_tempdir(\"ae_r15_hash_dir\"); \
         let r = try { crypto_hash_file(d) } catch e { e.error.code + \"|\" + e.error.hint }; \
         rmdir(d, true); r"));
    assert!(out.starts_with("E_BAD_ARG|"), "{out}");
    assert!(!out.contains("E_OS_DENIED"), "{out}");
    assert!(
        out.contains("is a directory; hash its files with glob("),
        "{out}"
    );
}

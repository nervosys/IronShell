//! Round 9: the agent-tester findings that each have one decided behaviour.
//!
//! One test group per item. In-process tests evaluate source through the same
//! entry points the other contract tests use; CLI tests spawn the binary with a
//! scratch session directory, for the behaviour that lives in `main`.

use iron_shell::value::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

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

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ae-round9-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cwd")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Scratch { root }
    }

    fn cwd(&self) -> PathBuf {
        self.root.join("cwd")
    }

    fn ish(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(self.cwd())
            .env("NO_COLOR", "1")
            .env_remove("IRONSHELL_ALLOW_SH")
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

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

// 1. The declared call forms are the underscore names; the hyphen is an alias.

#[test]
fn declared_conversions_render_their_underscore_call_form() {
    for (name, call) in [
        ("from-csv", "from_csv("),
        ("to-csv", "to_csv("),
        ("from-yaml", "from_yaml("),
        ("to-yaml", "to_yaml("),
        ("from-json", "from_json("),
        ("to-json", "to_json("),
    ] {
        let v = iron_shell::agent_api::ontology_describe_json(name);
        let sig = v["signature"].as_str().unwrap_or_default();
        assert!(sig.contains(call), "{name}: {v}");
        // `->` is the return arrow; only the call itself must be hyphen-free.
        let head = sig.split("->").next().unwrap_or_default();
        assert!(!head.contains('-'), "{name} renders a hyphen: {sig}");
    }
}

#[test]
fn powershell_style_builtins_are_catalogued_by_their_callable_name() {
    // `select`, `measure` and `foreach` are callable; their PowerShell
    // spellings are aliases, so the catalogue renders no hyphen for them.
    for name in ["select", "measure", "foreach"] {
        let v = iron_shell::agent_api::ontology_describe_json(name);
        let sig = v["signature"].as_str().unwrap_or_default();
        assert!(sig.contains(&format!("{name}(")), "{name}: {v}");
        let head = sig.split("->").next().unwrap_or_default();
        assert!(!head.contains('-'), "{name} renders a hyphen: {sig}");
    }
}

// 2. Mapped spellings lead the suggestions.

#[test]
fn mapped_date_and_raise_spellings_lead_did_you_mean() {
    assert_eq!(iron_shell::builtins::did_you_mean("date")[0], "now_iso");
    assert_eq!(iron_shell::builtins::did_you_mean("raise")[0], "error");
}

#[test]
fn an_unknown_bare_name_suggests_the_mapped_spelling_first() {
    let s = Scratch::new("dym");
    let out = s.ish(&["-c", "date"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("did you mean: now_iso"),
        "{}",
        stderr(&out)
    );
}

// 3. Module members first; unrelated globals are not suggested.

#[test]
fn a_missing_module_member_suggests_its_own_members_first() {
    let got = iron_shell::builtins::module_member_candidates(
        "str",
        "padd",
        &["pad_left", "pad_right", "upper"],
    );
    assert_eq!(
        got.first().map(String::as_str),
        Some("str.pad_left"),
        "{got:?}"
    );
    assert!(got.iter().all(|c| c != "pwd"), "{got:?}");
}

#[test]
fn unrelated_globals_are_not_suggested_for_a_module_member() {
    for field in ["padd", "lpad", "ljust"] {
        let got = iron_shell::builtins::module_member_candidates("str", field, &["upper"]);
        for bad in ["pwd", "head", "lnav", "last", "list"] {
            assert!(
                got.iter().all(|c| c != bad),
                "str.{field} suggested {bad}: {got:?}"
            );
        }
    }
}

// 4. The implicit row is not named.

#[test]
fn a_field_of_the_implicit_row_is_named_as_a_field() {
    let msg = text(run("try { [1, 2] | .a } catch e { e.error.message }"));
    assert!(msg.contains(".a: expected a Record"), "{msg}");
    assert!(!msg.contains("__"), "{msg}");
}

// 5. args() under -c.

#[test]
fn args_index_under_c_says_c_takes_no_script_arguments() {
    let s = Scratch::new("argsidx");
    let out = s.ish(&["-c", "args()[0]"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("-c takes no script arguments"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn arguments_after_c_get_one_warning_line() {
    let s = Scratch::new("argswarn");
    let out = s.ish(&["-c", "args()", "x", "y"]);
    assert!(
        stderr(&out).contains(
            "ish: arguments after -c are ignored (args() is empty under -c); use a script file"
        ),
        "{}",
        stderr(&out)
    );
}

// 6. file_insert lines start at 1; file_replace names itself.

#[test]
fn file_insert_refuses_a_line_below_one() {
    for line in ["-1", "0"] {
        let src = format!(
            "let d = fs_tempdir(\"ae_round9_ins\"); let f = d + \"/n.txt\"; \
             file_write(f, \"a\\nb\"); \
             let m = try {{ file_insert(f, {line}, \"x\") }} catch e {{ e.error.message }}; \
             rmdir(d, true); m"
        );
        let msg = text(run(&src));
        assert!(
            msg.contains("file_insert: lines are numbered from 1"),
            "{line}: {msg}"
        );
    }
}

#[test]
fn file_replace_on_a_directory_names_file_replace() {
    let src = "let d = fs_tempdir(\"ae_round9_repl\"); \
               let m = try { file_replace(d, \"a\", \"b\") } catch e { e.error.message }; \
               rmdir(d, true); m";
    let msg = text(run(src));
    assert!(!msg.contains("file_patch"), "{msg}");
    assert!(msg.starts_with("file_replace"), "{msg}");
}

// 8. The bare first and last of an empty array are null.

#[test]
fn bare_first_and_last_of_an_empty_array_are_null() {
    assert_eq!(run("[] | first"), Value::Null);
    assert_eq!(run("[] | last"), Value::Null);
    // An explicit count still answers an array.
    assert_eq!(run("[] | first(1)"), Value::Array(vec![]));
}

// 9. An empty error message is refused.

#[test]
fn an_empty_error_message_is_refused() {
    let msg = text(run("try { error(\"\") } catch e { e.error.message }"));
    assert!(msg.contains("error: the message is empty"), "{msg}");
    let code = text(run("try { error(\"\") } catch e { e.error.code }"));
    assert_eq!(code, "E_BAD_ARG");
}

// 10. --explain agrees with the sh() guard.

#[test]
fn explain_denies_sh_when_the_guard_refuses_it() {
    let s = Scratch::new("explain");
    let out = s.ish(&["--explain", "-c", "sh(\"ls\")"]);
    let t = stdout(&out);
    assert!(t.contains("\"decision\": \"deny\""), "{t}");
    assert!(!t.contains("\"approve\""), "{t}");
}

// 11. Nested values in CSV are JSON, not placeholders.

#[test]
fn to_csv_writes_a_nested_array_as_json() {
    assert_eq!(
        run("[{a: 1, b: [1, 2]}] | to_csv | contains(\"1,\\\"[1,2]\\\"\")"),
        Value::Bool(true)
    );
}

// 12. A tab or newline in a path is named as the escape it probably is.

#[test]
fn a_path_with_a_tab_says_how_backslashes_were_read() {
    let s = Scratch::new("tabpath");
    let out = s.ish(&["-c", "file.read(\"C:\\temp\\new\")"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("the path contains a tab, newline or carriage return"),
        "{}",
        stderr(&out)
    );
}

// 7. $0 and set --.

#[test]
fn dollar_zero_is_ish_under_c_and_the_script_path_for_a_file() {
    let s = Scratch::new("dollar0");
    let out = s.ish(&["-b", "-c", "echo $0"]);
    assert_eq!(stdout(&out), "ish", "{}", stderr(&out));

    std::fs::write(s.cwd().join("s.sh"), "echo $0\n").unwrap();
    let out = s.ish(&["-b", "s.sh"]);
    assert_eq!(stdout(&out), "s.sh", "{}", stderr(&out));
}

#[test]
fn set_double_dash_replaces_and_clears_positional_parameters() {
    let s = Scratch::new("setdd");
    let out = s.ish(&["-b", "-c", "set -- a b; echo $1 $2 $#"]);
    assert_eq!(stdout(&out), "a b 2", "{}", stderr(&out));
    let out = s.ish(&["-b", "-c", "set -- a; set --; echo $#"]);
    assert_eq!(stdout(&out), "0", "{}", stderr(&out));
}

// 13. A hyphenated query names the underscore builtin.

#[test]
fn ontology_describe_maps_a_hyphenated_name() {
    let v = iron_shell::agent_api::ontology_describe_json("to-csv");
    assert_eq!(v["builtin"], "to_csv", "{v}");
    assert!(v.get("error").is_none(), "{v}");
}

// 14. import is discoverable.

#[test]
fn ontology_describes_import() {
    let v = iron_shell::agent_api::ontology_describe_json("import");
    let form = v["form"].as_str().unwrap_or_default();
    assert!(form.contains("import \"lib/x.ish\""), "{v}");
    assert!(form.contains("pub"), "{v}");
}

// 15. Import cycle and missing-module hints.

#[test]
fn an_import_cycle_names_the_import_that_closes_it() {
    let s = Scratch::new("cycle");
    std::fs::create_dir_all(s.cwd().join("lib")).unwrap();
    std::fs::write(s.cwd().join("lib/a.ish"), "import \"b.ish\"\npub x = 1\n").unwrap();
    std::fs::write(s.cwd().join("lib/b.ish"), "import \"a.ish\"\npub y = 2\n").unwrap();
    std::fs::write(s.cwd().join("main.ish"), "import \"lib/a.ish\"\n").unwrap();
    let out = s.ish(&["main.ish"]);
    let t = stderr(&out);
    assert!(!out.status.success(), "{t}");
    assert!(
        t.contains("b.ish imports") && t.contains("a.ish, which is already being loaded"),
        "{t}"
    );
    assert!(t.contains("remove one of the two imports"), "{t}");
}

#[test]
fn a_missing_import_says_paths_are_relative_to_the_importer() {
    let s = Scratch::new("missing");
    std::fs::write(s.cwd().join("main.ish"), "import \"nope.ish\"\n").unwrap();
    let out = s.ish(&["main.ish"]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("paths are relative to the importing file:"),
        "{}",
        stderr(&out)
    );
}

// 16. sql with no input runs its query over an empty database.

#[test]
fn sql_with_no_input_runs_over_an_empty_database() {
    let s = Scratch::new("sql");
    let out = s.ish(&["-c", "sql(\"select 1 as x\") | len"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim(), "1");
}

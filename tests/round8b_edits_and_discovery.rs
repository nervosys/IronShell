//! Round 8b: a file edit names a missing file as missing, a file_insert past the
//! end is refused, the help text and the suggestions name the real spellings,
//! the schema lists underscore names only, and map and where say what their
//! two-parameter lambda receives.

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
fn a_missing_file_is_named_as_missing_not_as_a_missing_pattern() {
    let msg = text(run(
        "try { file_replace(\"ae-round8b-no-such-file.txt\", \"a\", \"b\") } catch e { e.error.message }",
    ));
    assert!(
        msg.starts_with("file_replace: no file at ae-round8b-no-such-file.txt"),
        "{msg}"
    );
    assert!(!msg.contains("pattern not found"), "{msg}");
    let code = text(run(
        "try { file_replace(\"ae-round8b-no-such-file.txt\", \"a\", \"b\") } catch e { e.error.code }",
    ));
    assert_eq!(code, "E_NOT_FOUND");
}

#[test]
fn an_insert_past_the_end_is_refused_and_the_line_after_it_appends() {
    let refused = text(run(
        "let d = fs_tempdir(\"ae_round8b_insert\"); let f = d + \"/note.txt\"; \
         file_write(f, \"a\\nb\\nc\\nd\"); \
         let r = try { file_insert(f, 99, \"x\") } catch e { e.error.message }; \
         rmdir(d, true); r",
    ));
    assert!(refused.contains("is past the end of"), "{refused}");
    assert!(refused.contains("(4 lines)"), "{refused}");

    let at = run(
        "let d = fs_tempdir(\"ae_round8b_append\"); let f = d + \"/note.txt\"; \
         file_write(f, \"a\\nb\\nc\\nd\"); \
         let r = file_insert(f, 5, \"x\"); rmdir(d, true); r.at_line",
    );
    assert_eq!(at, Value::Int(5));
}

#[test]
fn help_names_grep_in_the_order_it_takes() {
    let help = text(run("help()"));
    assert!(help.contains("grep       <pattern> [file]"), "{help}");
    assert!(!help.contains("<file|array> <pattern>"), "{help}");
}

#[test]
fn map_and_where_pass_the_index_to_a_two_parameter_lambda() {
    assert_eq!(
        run("[10, 20] | map(fn(x, i) => i)"),
        Value::Array(vec![Value::Int(0), Value::Int(1)])
    );
    assert_eq!(
        run("[10, 20] | where(fn(x, i) => i > 0)"),
        Value::Array(vec![Value::Int(20)])
    );
}

#[test]
fn now_iso_names_the_zone_it_was_given() {
    let msg = text(run(
        "try { now_iso(\"Europe/Paris\") } catch e { e.error.message }",
    ));
    assert!(msg.contains("got \"Europe/Paris\""), "{msg}");
    assert!(msg.contains("\"utc\" or \"local\""), "{msg}");
}

#[test]
fn the_names_people_guess_for_the_clock_and_for_raising_point_at_the_real_ones() {
    for guess in ["date", "datetime", "timestamp_iso", "date_iso"] {
        let found = iron_shell::builtins::did_you_mean(guess);
        assert!(found.contains(&"now_iso".to_string()), "{guess}: {found:?}");
    }
    for guess in ["raise", "throw", "fail"] {
        let found = iron_shell::builtins::did_you_mean(guess);
        assert!(found.contains(&"error".to_string()), "{guess}: {found:?}");
    }
}

#[test]
fn a_candidate_offered_twice_is_listed_once() {
    let found = iron_shell::builtins::nearest_names("yq", ["yq", "yq", "y"].into_iter());
    assert_eq!(found, vec!["yq".to_string(), "y".to_string()]);
    let module_guess = iron_shell::builtins::did_you_mean("file_read");
    let mut seen = module_guess.clone();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), module_guess.len(), "{module_guess:?}");
}

#[test]
fn the_schema_advertises_underscore_names_only() {
    let schema = iron_shell::agent_api::generate_schema("compact").expect("schema");
    assert!(!schema.contains("\"ai-detect\""), "hyphenated name listed");
    assert!(
        !schema.contains("\"ai-detect-format\""),
        "hyphenated name listed"
    );
    assert!(
        schema.contains("ai_detect_format"),
        "underscore name missing"
    );
}

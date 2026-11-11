//! Round 9: the string, padding, date and TOML builtins, in-process.
//!
//! Each builtin is run in both of its forms where it has two (a direct call
//! with the text first, and a piped text), and each refusal is checked by its
//! code and by the part of the message that names what was wrong.

use iron_shell::value::Value;
use std::collections::BTreeMap;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

/// The full text of the error a program ends with. It must fail.
fn refusal(src: &str) -> String {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    match iron_shell::eval::eval_program(&stmts, &mut env) {
        Ok(v) => panic!("`{src}` should be refused, but answered {v}"),
        Err(e) => e.to_string(),
    }
}

fn text(s: &str) -> Value {
    Value::Str(s.to_string())
}

fn ints(v: &[i64]) -> Value {
    Value::Array(v.iter().map(|i| Value::Int(*i)).collect())
}

fn record(v: Value) -> BTreeMap<String, Value> {
    match v {
        Value::Record(map) => map,
        other => panic!("expected a record, got {other}"),
    }
}

// --- regex_match -----------------------------------------------------------

#[test]
fn regex_match_answers_in_both_forms() {
    assert_eq!(run(r#"regex_match("abc", "b")"#), Value::Bool(true));
    assert_eq!(run(r#""abc" | regex_match("b")"#), Value::Bool(true));
    assert_eq!(run(r#"regex_match("abc", "^b")"#), Value::Bool(false));
    assert_eq!(run(r#""abc" | regex_match("^b")"#), Value::Bool(false));
}

#[test]
fn regex_match_refuses_an_invalid_pattern_and_names_it() {
    let msg = refusal(r#"regex_match("abc", "(")"#);
    assert!(msg.contains("E_BAD_ARG"), "{msg}");
    assert!(msg.contains("regex_match"), "{msg}");
    // The refusal is rendered as JSON, so the quotes around the pattern are escaped.
    assert!(msg.contains(r#"\"(\""#), "the pattern is named: {msg}");
}

#[test]
fn regex_match_refuses_a_pattern_over_the_size_cap() {
    // A counted repetition of a repetition: far past the 1 MiB compile cap,
    // whichever of the parser or the size check refuses it first.
    let msg = refusal(r#"regex_match("a", "(.{1000}){1000}")"#);
    assert!(msg.contains("E_BAD_ARG"), "{msg}");
}

#[test]
fn regex_match_refuses_a_number_as_text_and_a_missing_pattern() {
    assert!(refusal(r#"regex_match(5, "a")"#).contains("E_BAD_ARG"));
    assert!(refusal(r#"5 | regex_match("a")"#).contains("E_BAD_ARG"));
    assert!(refusal(r#"regex_match("a")"#).contains("E_BAD_ARG"));
}

// --- regex_find --------------------------------------------------------------

#[test]
fn regex_find_answers_the_first_match_as_a_record() {
    let m = record(run(r#"regex_find("a1b22", "[0-9]+")"#));
    assert_eq!(m.get("match"), Some(&text("1")));
    assert_eq!(m.get("groups"), Some(&Value::Array(vec![])));
    assert_eq!(m.get("start"), Some(&Value::Int(1)));
    assert_eq!(m.get("end"), Some(&Value::Int(2)));
    assert_eq!(
        run(r#"("a1b22" | regex_find("[0-9]+")).end"#),
        Value::Int(2)
    );
}

#[test]
fn regex_find_answers_null_when_nothing_matches() {
    assert_eq!(run(r#"regex_find("abc", "z")"#), Value::Null);
    assert_eq!(run(r#""abc" | regex_find("z")"#), Value::Null);
}

#[test]
fn regex_find_groups_are_strings_and_null_for_a_group_that_did_not_take_part() {
    let m = record(run(r#"regex_find("a", "(a)|(b)")"#));
    assert_eq!(
        m.get("groups"),
        Some(&Value::Array(vec![text("a"), Value::Null]))
    );
    let m = record(run(r#"regex_find("key=val", "([a-z]+)=([a-z]+)")"#));
    assert_eq!(
        m.get("groups"),
        Some(&Value::Array(vec![text("key"), text("val")]))
    );
    assert_eq!(m.get("match"), Some(&text("key=val")));
}

#[test]
fn regex_find_offsets_are_characters_not_bytes() {
    // "é" is two bytes; the "w" is the seventh byte and the sixth character.
    let m = record(run(r#"regex_find("héllo wörld", "w")"#));
    assert_eq!(m.get("start"), Some(&Value::Int(6)));
    assert_eq!(m.get("end"), Some(&Value::Int(7)));
}

#[test]
fn regex_find_refuses_an_invalid_pattern() {
    let msg = refusal(r#"regex_find("abc", "[")"#);
    assert!(msg.contains("E_BAD_ARG"), "{msg}");
    assert!(msg.contains(r#"\"[\""#), "the pattern is named: {msg}");
}

// --- regex_find_all ----------------------------------------------------------

#[test]
fn regex_find_all_answers_every_match_left_to_right() {
    assert_eq!(
        run(r#"regex_find_all("a1b22", "[0-9]+") | len"#),
        Value::Int(2)
    );
    assert_eq!(
        run(r#""a1b22" | regex_find_all("[0-9]+") | len"#),
        Value::Int(2)
    );
    let second = match run(r#"regex_find_all("a1b22", "[0-9]+")"#) {
        Value::Array(items) => record(items[1].clone()),
        other => panic!("expected an array, got {other}"),
    };
    assert_eq!(second.get("match"), Some(&text("22")));
    assert_eq!(second.get("start"), Some(&Value::Int(3)));
    assert_eq!(second.get("end"), Some(&Value::Int(5)));
}

#[test]
fn regex_find_all_answers_an_empty_array_when_nothing_matches() {
    assert_eq!(run(r#"regex_find_all("abc", "z")"#), Value::Array(vec![]));
}

#[test]
fn regex_find_all_refuses_an_invalid_pattern() {
    assert!(refusal(r#"regex_find_all("abc", "a{")"#).contains("E_BAD_ARG"));
}

// --- regex_replace -----------------------------------------------------------

#[test]
fn regex_replace_expands_numbered_and_named_groups() {
    assert_eq!(
        run(r#"regex_replace("a1b2", "([0-9])", "<$1>")"#),
        text("a<1>b<2>")
    );
    assert_eq!(
        run(r#"regex_replace("2024-02-29", "([0-9]+)-([0-9]+)-([0-9]+)", "$3.$2.$1")"#),
        text("29.02.2024")
    );
    assert_eq!(
        run(r#"regex_replace("ab", "(?P<y>a)", "[$y]")"#),
        text("[a]b")
    );
}

#[test]
fn regex_replace_replaces_every_match_in_both_forms() {
    assert_eq!(run(r##""a1" | regex_replace("[0-9]", "#")"##), text("a#"));
    assert_eq!(
        run(r#"regex_replace("1 2 3", "[0-9]", "x")"#),
        text("x x x")
    );
    assert_eq!(run(r#"regex_replace("abc", "z", "x")"#), text("abc"));
}

#[test]
fn regex_replace_refuses_an_invalid_pattern_and_a_missing_replacement() {
    assert!(refusal(r#"regex_replace("abc", "(", "x")"#).contains("E_BAD_ARG"));
    assert!(refusal(r#"regex_replace("abc", "b")"#).contains("E_BAD_ARG"));
}

// --- the str module members --------------------------------------------------

#[test]
fn the_str_module_names_the_regex_and_padding_builtins() {
    assert_eq!(run(r#"str.regex_match("abc", "b")"#), Value::Bool(true));
    assert_eq!(
        run(r##"str.regex_replace("a1", "[0-9]", "#")"##),
        text("a#")
    );
    assert_eq!(run(r#"str.pad_left("7", 3, "0")"#), text("007"));
    assert_eq!(run(r#"str.pad_right("7", 3, "0")"#), text("700"));
}

// --- pad_left and pad_right --------------------------------------------------

#[test]
fn pad_left_fills_on_the_left_in_both_forms() {
    assert_eq!(run(r#"pad_left("7", 3, "0")"#), text("007"));
    assert_eq!(run(r#""7" | pad_left(3)"#), text("  7"));
    assert_eq!(run(r#"pad_left("7", 3)"#), text("  7"));
}

#[test]
fn pad_right_fills_on_the_right_in_both_forms() {
    assert_eq!(run(r#"pad_right("ab", 4, ".")"#), text("ab.."));
    assert_eq!(run(r#""ab" | pad_right(4)"#), text("ab  "));
}

#[test]
fn padding_counts_characters_and_returns_a_text_at_or_over_the_width_unchanged() {
    assert_eq!(run(r#"pad_left("é", 3, "*")"#), text("**é"));
    assert_eq!(run(r#"pad_right("abcd", 2)"#), text("abcd"));
    assert_eq!(run(r#"pad_left("ab", 2)"#), text("ab"));
    assert_eq!(run(r#"pad_left("", 0)"#), text(""));
    assert_eq!(run(r#"pad_left("x", 0)"#), text("x"));
}

#[test]
fn padding_refuses_a_fill_that_is_not_one_character() {
    assert!(refusal(r#"pad_left("a", 3, "xy")"#).contains("E_BAD_ARG"));
    assert!(refusal(r#"pad_right("a", 3, "")"#).contains("E_BAD_ARG"));
}

#[test]
fn padding_refuses_a_bad_width() {
    assert!(refusal(r#"pad_left("a", -1)"#).contains("E_BAD_ARG"));
    assert!(refusal(r#"pad_left("a", 1000001)"#).contains("E_BAD_ARG"));
    assert!(refusal(r#"pad_right("a", "3")"#).contains("E_BAD_ARG"));
    assert!(refusal(r#"pad_left("a")"#).contains("E_BAD_ARG"));
}

// --- date_parse --------------------------------------------------------------

#[test]
fn date_parse_answers_a_date_as_a_date() {
    assert_eq!(run(r#"date_parse("2024-02-29")"#), text("2024-02-29"));
    assert_eq!(run(r#""2024-02-29" | date_parse"#), text("2024-02-29"));
}

#[test]
fn date_parse_answers_a_datetime_in_utc_with_z() {
    assert_eq!(
        run(r#"date_parse("2024-02-29T10:00:00Z")"#),
        text("2024-02-29T10:00:00Z")
    );
    assert_eq!(
        run(r#"date_parse("2024-02-29T10:00:00+02:00")"#),
        text("2024-02-29T08:00:00Z")
    );
    assert_eq!(
        run(r#"date_parse("2024-02-29T10:00:00.5Z")"#),
        text("2024-02-29T10:00:00.500Z")
    );
}

#[test]
fn date_parse_reads_a_strftime_layout_when_one_is_given() {
    assert_eq!(
        run(r#"date_parse("29/02/2024", "%d/%m/%Y")"#),
        text("2024-02-29")
    );
    assert_eq!(
        run(r#"date_parse("2024-02-29 10:30", "%Y-%m-%d %H:%M")"#),
        text("2024-02-29T10:30:00Z")
    );
    assert_eq!(
        run(r#""29/02/2024" | date_parse("%d/%m/%Y")"#),
        text("2024-02-29")
    );
}

#[test]
fn date_parse_refuses_text_that_is_not_a_date_and_names_it() {
    let msg = refusal(r#"date_parse("someday")"#);
    assert!(msg.contains("E_BAD_VALUE"), "{msg}");
    assert!(msg.contains("someday"), "{msg}");
    assert!(refusal(r#"date_parse("2024-02-30")"#).contains("E_BAD_VALUE"));
    assert!(refusal(r#"date_parse("29/02/2024", "%Y-%m-%d")"#).contains("E_BAD_VALUE"));
}

#[test]
fn date_parse_refuses_an_invalid_layout() {
    assert!(refusal(r#"date_parse("2024-02-29", "%Q")"#).contains("E_BAD_ARG"));
}

// --- date_diff ---------------------------------------------------------------

#[test]
fn date_diff_counts_whole_days_by_default() {
    assert_eq!(
        run(r#"date_diff("2024-02-29", "2026-11-09")"#),
        Value::Int(984)
    );
    assert_eq!(
        run(r#"date_diff("2026-11-09", "2024-02-29")"#),
        Value::Int(-984)
    );
    assert_eq!(
        run(r#""2024-02-29" | date_diff("2024-03-01")"#),
        Value::Int(1)
    );
    assert_eq!(
        run(r#"date_diff("2024-02-29", "2024-02-29")"#),
        Value::Int(0)
    );
}

#[test]
fn date_diff_counts_hours_minutes_and_seconds() {
    assert_eq!(
        run(r#"date_diff("2024-02-29", "2026-11-09", "hours")"#),
        Value::Int(23616)
    );
    assert_eq!(
        run(r#"date_diff("2024-02-29", "2026-11-09", "minutes")"#),
        Value::Int(1416960)
    );
    assert_eq!(
        run(r#"date_diff("2024-02-29", "2026-11-09", "seconds")"#),
        Value::Int(85017600)
    );
    assert_eq!(
        run(r#""2024-02-29" | date_diff("2024-03-01", "days")"#),
        Value::Int(1)
    );
}

#[test]
fn date_diff_default_drops_a_partial_day_but_hours_count_it() {
    assert_eq!(
        run(r#"date_diff("2026-11-01T23:00:00Z", "2026-11-02T01:00:00Z")"#),
        Value::Int(0)
    );
    assert_eq!(
        run(r#"date_diff("2026-11-01T23:00:00Z", "2026-11-02T01:00:00Z", "hours")"#),
        Value::Int(2)
    );
    assert_eq!(
        run(r#"date_diff("2026-11-02T01:00:00Z", "2026-11-01T23:00:00Z", "hours")"#),
        Value::Int(-2)
    );
}

#[test]
fn date_diff_truncates_a_partial_day_toward_zero() {
    assert_eq!(
        run(r#"date_diff("2024-02-29T00:00:00Z", "2024-03-01T12:00:00Z")"#),
        Value::Int(1)
    );
    assert_eq!(
        run(r#"date_diff("2024-03-01T12:00:00Z", "2024-02-29T00:00:00Z")"#),
        Value::Int(-1)
    );
}

#[test]
fn date_diff_refuses_a_bad_unit_and_a_bad_date() {
    assert!(refusal(r#"date_diff("2024-02-29", "2026-11-09", "fortnights")"#).contains("E_BAD_ARG"));
    let msg = refusal(r#"date_diff("2024-02-29", "nope")"#);
    assert!(msg.contains("E_BAD_VALUE"), "{msg}");
    assert!(msg.contains("nope"), "{msg}");
}

// --- date_format -------------------------------------------------------------

#[test]
fn date_format_writes_the_layout_in_both_forms() {
    assert_eq!(
        run(r#"date_format("2024-02-29T10:00:00Z", "%d %B %Y")"#),
        text("29 February 2024")
    );
    assert_eq!(run(r#""2024-02-29" | date_format("%A")"#), text("Thursday"));
    assert_eq!(
        run(r#"date_format("2024-02-29T10:05:00Z", "%H:%M")"#),
        text("10:05")
    );
}

#[test]
fn date_format_refuses_an_invalid_layout_and_a_bad_date() {
    assert!(refusal(r#"date_format("2024-02-29", "%Q")"#).contains("E_BAD_ARG"));
    assert!(refusal(r#"date_format("someday", "%Y")"#).contains("E_BAD_VALUE"));
    assert!(refusal(r#"date_format("2024-02-29")"#).contains("E_BAD_ARG"));
}

// --- date_add ----------------------------------------------------------------

#[test]
fn date_add_moves_a_date_by_days_and_answers_a_date() {
    assert_eq!(
        run(r#"date_add("2024-02-28", 1, "days")"#),
        text("2024-02-29")
    );
    assert_eq!(
        run(r#"date_add("2024-02-28", 2, "days")"#),
        text("2024-03-01")
    );
    assert_eq!(
        run(r#""2024-02-29" | date_add(-1, "days")"#),
        text("2024-02-28")
    );
}

#[test]
fn date_add_answers_a_utc_datetime_for_hours_minutes_and_seconds() {
    assert_eq!(
        run(r#"date_add("2024-02-28T23:00:00Z", 2, "hours")"#),
        text("2024-02-29T01:00:00Z")
    );
    assert_eq!(
        run(r#"date_add("2024-02-28", 90, "minutes")"#),
        text("2024-02-28T01:30:00Z")
    );
    assert_eq!(
        run(r#"date_add("2024-02-28", 1, "hours")"#),
        text("2024-02-28T01:00:00Z")
    );
    assert_eq!(
        run(r#"date_add("2024-02-28T00:00:00Z", 30, "seconds")"#),
        text("2024-02-28T00:00:30Z")
    );
}

#[test]
fn date_add_refuses_a_bad_unit_amount_or_date() {
    assert!(refusal(r#"date_add("2024-02-28", 1, "fortnights")"#).contains("E_BAD_ARG"));
    assert!(refusal(r#"date_add("2024-02-28", "1", "days")"#).contains("E_BAD_ARG"));
    assert!(refusal(r#"date_add("2024-02-28", 1)"#).contains("E_BAD_ARG"));
    assert!(refusal(r#"date_add("someday", 1, "days")"#).contains("E_BAD_VALUE"));
    let msg = refusal(r#"date_add("2024-02-28", 9223372036854775807, "days")"#);
    assert!(msg.contains("E_BAD_ARG"), "{msg}");
}

// --- to_toml and from_toml ---------------------------------------------------

#[test]
fn to_toml_writes_a_record_as_a_document() {
    let Value::Str(doc) = run(r#"to_toml({a: 1, b: "x"})"#) else {
        panic!("to_toml answers a String");
    };
    assert!(doc.contains("a = 1"), "{doc}");
    assert!(doc.contains("b = \"x\""), "{doc}");
    let Value::Str(piped) = run("r = {a: 1}\nr | to_toml()") else {
        panic!("to_toml answers a String");
    };
    assert!(piped.contains("a = 1"), "{piped}");
}

#[test]
fn to_toml_and_from_toml_round_trip_nested_values() {
    let back = record(run(
        r#"from_toml(to_toml({name: "ae", t: {n: [1, 2]}, flag: true, f: 1.5}))"#,
    ));
    assert_eq!(back.get("name"), Some(&text("ae")));
    assert_eq!(back.get("flag"), Some(&Value::Bool(true)));
    assert_eq!(back.get("f"), Some(&Value::Float(1.5)));
    let t = record(back.get("t").cloned().expect("t survives"));
    assert_eq!(t.get("n"), Some(&ints(&[1, 2])));
}

#[test]
fn to_toml_refuses_a_top_level_that_is_not_a_record() {
    let msg = refusal("to_toml([1])");
    assert!(msg.contains("E_BAD_ARG"), "{msg}");
    assert!(msg.contains("top level"), "{msg}");
    assert!(refusal(r#"to_toml("x")"#).contains("E_BAD_ARG"));
}

#[test]
fn to_toml_refuses_a_null_and_names_its_path() {
    let msg = refusal(r#"to_toml({a: from_json("null")})"#);
    assert!(msg.contains("E_BAD_ARG"), "{msg}");
    assert!(msg.contains("null at a "), "{msg}");

    let msg = refusal(r#"to_toml({app: {servers: [{note: from_json("null")}]}})"#);
    assert!(msg.contains("E_BAD_ARG"), "{msg}");
    assert!(msg.contains("app.servers[0].note"), "{msg}");
}

#[test]
fn from_toml_reads_a_document_and_refuses_text_that_is_not_toml() {
    assert_eq!(run(r#"from_toml("a = 1").a"#), Value::Int(1));
    assert_eq!(run(r#"("a = 1" | from_toml).a"#), Value::Int(1));
    let msg = refusal(r#"from_toml("a = ")"#);
    assert!(msg.contains("E_BAD_VALUE"), "{msg}");
    assert!(msg.contains("not TOML"), "{msg}");
    assert!(refusal(r#"from_toml("a = 1", "b")"#).contains("E_BAD_ARG"));
}

#[test]
fn from_toml_answers_a_datetime_as_its_text() {
    let d = record(run(r#"from_toml("d = 1979-05-27")"#));
    assert_eq!(d.get("d"), Some(&text("1979-05-27")));
}

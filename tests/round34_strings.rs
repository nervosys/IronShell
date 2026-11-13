//! Round 34: text and number builtins that were missing or too narrow.
//!
//! - `trim(text, chars?)` strips a set of characters from both ends.
//! - `title` and `capitalize` case text; `repeat` and `words` are the builtins
//!   of those names (no longer a `pad_left` guess).
//! - `log`, `ln`, `log10`, `log2` and `exp` answer Floats, and refuse what has no
//!   finite Float answer with E_BAD_VALUE.
//! - `from_epoch` and `to_epoch` convert between epoch seconds and ISO 8601 UTC
//!   text, for the years 0001 to 9999 only.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

/// The error code a refused call answers, read through try/catch.
fn code(call: &str) -> String {
    match run(&format!("try {{ {call} }} catch e {{ e.error.code }}")) {
        Value::Str(s) => s,
        other => panic!("`{call}` should be refused, but answered {other}"),
    }
}

/// The message a refused call answers.
fn message(call: &str) -> String {
    match run(&format!("try {{ {call} }} catch e {{ e.error.message }}")) {
        Value::Str(s) => s,
        other => panic!("`{call}` should be refused, but answered {other}"),
    }
}

fn text(s: &str) -> Value {
    Value::Str(s.to_string())
}

fn float_near(v: Value, want: f64) {
    match v {
        Value::Float(f) => assert!((f - want).abs() < 1e-9, "{f} is not {want}"),
        other => panic!("expected a Float near {want}, got {other}"),
    }
}

// --- trim ------------------------------------------------------------------

#[test]
fn trim_with_no_set_strips_unicode_whitespace() {
    assert_eq!(run(r#"trim("  hello  ")"#), text("hello"));
    let src = format!("trim(\"{}x{}\")", '\u{3000}', '\u{a0}');
    assert_eq!(run(&src), text("x"));
    assert_eq!(run(r#""  x  " | trim"#), text("x"));
}

#[test]
fn trim_strips_every_character_of_the_set_from_both_ends() {
    assert_eq!(run(r#"trim("xxhixx", "x")"#), text("hi"));
    assert_eq!(run(r#""xxhixx" | trim("x")"#), text("hi"));
    assert_eq!(run(r#"trim("abcxyz", "xyz")"#), text("abc"));
    // Only the ends: the x inside the text stays.
    assert_eq!(run(r#"trim("xaxbx", "x")"#), text("axb"));
}

#[test]
fn trim_set_with_multi_byte_characters() {
    assert_eq!(run(r#"trim("éaéxéé", "é")"#), text("aéx"));
    assert_eq!(run(r#"trim("ünïcödé", "üé")"#), text("nïcöd"));
    assert_eq!(run(r#"trim("ééé", "é")"#), text(""));
}

#[test]
fn trim_with_an_empty_set_strips_nothing() {
    assert_eq!(run(r#"trim("  x  ", "")"#), text("  x  "));
}

#[test]
fn trim_refuses_a_set_that_is_not_text_and_extra_arguments() {
    assert_eq!(code(r#"trim("a", 1)"#), "E_BAD_ARG");
    assert_eq!(code(r#"trim("a", "b", "c")"#), "E_BAD_ARG");
    assert_eq!(code("trim()"), "E_BAD_ARG");
}

// --- title and capitalize --------------------------------------------------

#[test]
fn title_upper_cases_the_first_letter_of_each_word_and_lower_cases_the_rest() {
    assert_eq!(run(r#"title("hello WORLD")"#), text("Hello World"));
    assert_eq!(run(r#""well-known fact" | title"#), text("Well-Known Fact"));
    let spaced = format!("title(\"a  b{}c\")", '\t');
    assert_eq!(run(&spaced), text(&format!("A  B{}C", '\t')));
    assert_eq!(run(r#"title("")"#), text(""));
}

#[test]
fn title_words_are_runs_of_letters_digits_and_apostrophes() {
    // An apostrophe is inside a word: they're is one word.
    assert_eq!(run(r#"title("they're here")"#), text("They're Here"));
    assert_eq!(run(r#"title("o'neil")"#), text("O'neil"));
    // A digit does not start a word: the first letter after it is the upper one.
    assert_eq!(run(r#"title("3rd place")"#), text("3Rd Place"));
    assert_eq!(run(r#"title("42 ITEMS")"#), text("42 Items"));
}

#[test]
fn title_maps_unicode_letters() {
    assert_eq!(run(r#"title("élan vital")"#), text("Élan Vital"));
    assert_eq!(run(r#""ÉLAN VITAL" | title"#), text("Élan Vital"));
}

#[test]
fn capitalize_upper_cases_the_first_character_and_lower_cases_the_rest() {
    assert_eq!(run(r#"capitalize("hELLO wORLD")"#), text("Hello world"));
    assert_eq!(run(r#""élan" | capitalize"#), text("Élan"));
    assert_eq!(run(r#"capitalize("")"#), text(""));
    // The first character is a digit: nothing to upper-case, the rest is lower.
    assert_eq!(run(r#"capitalize("1ABC")"#), text("1abc"));
    // A hyphen does not start a new word for capitalize.
    assert_eq!(run(r#"capitalize("hyphen-WORD")"#), text("Hyphen-word"));
}

// --- repeat and words --------------------------------------------------------

#[test]
fn repeat_repeats_the_text_in_both_forms() {
    assert_eq!(run(r#"repeat("ab", 3)"#), text("ababab"));
    assert_eq!(run(r#""-" | repeat(5)"#), text("-----"));
    assert_eq!(run(r#"repeat("x", 0)"#), text(""));
    assert_eq!(run(r#""é" | repeat(3)"#), text("ééé"));
}

#[test]
fn repeat_of_an_empty_text_is_empty_whatever_the_count() {
    assert_eq!(run(r#"repeat("", 1000000000000)"#), text(""));
}

#[test]
fn repeat_refuses_a_negative_or_missing_or_non_int_count() {
    assert_eq!(code(r#"repeat("a", -1)"#), "E_BAD_ARG");
    assert_eq!(code(r#"repeat("a", 1.5)"#), "E_BAD_ARG");
    assert_eq!(code(r#"repeat("a", "3")"#), "E_BAD_ARG");
    assert_eq!(code(r#"repeat("a")"#), "E_BAD_ARG");
}

#[test]
fn repeat_refuses_a_result_over_100_megabytes_before_building_it() {
    // 2 bytes times 100000000 is 200 MB, refused without allocating it.
    assert_eq!(code(r#"repeat("ab", 100000000)"#), "E_BAD_VALUE");
    assert_eq!(code(r#"repeat("a", 100000001)"#), "E_BAD_VALUE");
    // A count whose byte total overflows is refused too, not wrapped.
    assert_eq!(code(r#"repeat("ab", 9223372036854775807)"#), "E_BAD_VALUE");
}

#[test]
fn words_splits_on_any_whitespace_run_and_drops_the_empties() {
    let src = format!("words(\"  the  quick{}fox{}\")", '\t', '\n');
    assert_eq!(
        run(&src),
        Value::Array(vec![text("the"), text("quick"), text("fox")])
    );
    assert_eq!(run(r#"words("")"#), Value::Array(vec![]));
    assert_eq!(run(r#"words("   ")"#), Value::Array(vec![]));
    assert_eq!(run(r#""a b" | words | len"#), Value::Int(2));
}

#[test]
fn words_counts_a_no_break_space_as_whitespace() {
    let src = format!("words(\"a{}b\")", '\u{a0}');
    assert_eq!(run(&src), Value::Array(vec![text("a"), text("b")]));
}

// --- logarithms and exp ------------------------------------------------------

#[test]
fn logarithms_answer_floats_and_promote_ints() {
    assert_eq!(run("ln(1)"), Value::Float(0.0));
    assert_eq!(run("log(1)"), Value::Float(0.0));
    assert_eq!(run("log10(1)"), Value::Float(0.0));
    assert_eq!(run("log2(1)"), Value::Float(0.0));
    float_near(run("log2(8)"), 3.0);
    float_near(run("log10(1000)"), 3.0);
    float_near(run("8 | log2"), 3.0);
    float_near(run("log(8, 2)"), 3.0);
    float_near(run("log(1000, 10)"), 3.0);
}

#[test]
fn log_with_a_base_is_the_logarithm_in_that_base() {
    assert_eq!(run("log(1, 2)"), Value::Float(0.0));
    float_near(run("log(0.5, 2)"), -1.0);
}

#[test]
fn logarithm_of_zero_or_less_is_refused_as_e_bad_value() {
    for call in [
        "log(0)",
        "log(-1)",
        "ln(0)",
        "log10(-5)",
        "log2(0)",
        "log(0, 2)",
    ] {
        assert_eq!(code(call), "E_BAD_VALUE", "{call}");
    }
    assert!(message("log(0)").contains("log of a non-positive number"));
    assert!(message("ln(-2)").contains("log of a non-positive number"));
}

#[test]
fn log_refuses_a_base_of_one_or_less_as_e_bad_value() {
    assert_eq!(code("log(8, 1)"), "E_BAD_VALUE");
    assert_eq!(code("log(8, 0)"), "E_BAD_VALUE");
    assert_eq!(code("log(8, -2)"), "E_BAD_VALUE");
}

#[test]
fn logarithm_refuses_a_text_argument() {
    assert_eq!(code(r#"ln("2")"#), "E_BAD_ARG");
}

#[test]
fn exp_answers_a_float_and_refuses_an_overflow() {
    assert_eq!(run("exp(0)"), Value::Float(1.0));
    // A result that underflows is still finite, so it is an answer.
    assert_eq!(run("exp(-1000)"), Value::Float(0.0));
    assert_eq!(code("exp(1000)"), "E_BAD_VALUE");
}

#[test]
fn math_module_members_name_the_logarithms_and_exp() {
    assert_eq!(run("math.ln(1)"), Value::Float(0.0));
    assert_eq!(run("math.exp(0)"), Value::Float(1.0));
    float_near(run("math.log2(8)"), 3.0);
    float_near(run("math.log(8, 2)"), 3.0);
}

#[test]
fn str_module_members_name_the_text_builtins() {
    assert_eq!(run(r#"str.title("ab cd")"#), text("Ab Cd"));
    assert_eq!(run(r#"str.capitalize("ABC")"#), text("Abc"));
    assert_eq!(run(r#"str.repeat("ab", 2)"#), text("abab"));
    assert_eq!(
        run(r#"str.words("a b")"#),
        Value::Array(vec![text("a"), text("b")])
    );
    assert_eq!(run(r#"str.trim("xhx", "x")"#), text("h"));
}

// --- epoch seconds -----------------------------------------------------------

#[test]
fn from_epoch_answers_iso_8601_utc_to_the_second() {
    assert_eq!(run("from_epoch(0)"), text("1970-01-01T00:00:00Z"));
    assert_eq!(run("86400 | from_epoch"), text("1970-01-02T00:00:00Z"));
    assert_eq!(run("from_epoch(1234567890)"), text("2009-02-13T23:31:30Z"));
}

#[test]
fn from_epoch_takes_the_range_ends() {
    assert_eq!(
        run("from_epoch(-62135596800)"),
        text("0001-01-01T00:00:00Z")
    );
    assert_eq!(
        run("from_epoch(253402300799)"),
        text("9999-12-31T23:59:59Z")
    );
    assert_eq!(code("from_epoch(-62135596801)"), "E_BAD_VALUE");
    assert_eq!(code("from_epoch(253402300800)"), "E_BAD_VALUE");
    assert!(message("from_epoch(253402300800)").contains("0001 to 9999"));
}

#[test]
fn from_epoch_drops_a_fraction_of_a_second_to_the_second_containing_it() {
    assert_eq!(run("from_epoch(86400.5)"), text("1970-01-02T00:00:00Z"));
    assert_eq!(run("from_epoch(-0.5)"), text("1969-12-31T23:59:59Z"));
    assert_eq!(code("from_epoch(1000000000000000.0)"), "E_BAD_VALUE");
    assert_eq!(code("from_epoch(\"0\")"), "E_BAD_ARG");
}

#[test]
fn to_epoch_reads_a_utc_datetime_or_a_date() {
    assert_eq!(
        run(r#"to_epoch("1970-01-02T00:00:00Z")"#),
        Value::Int(86400)
    );
    assert_eq!(run(r#""1970-01-01" | to_epoch"#), Value::Int(0));
    assert_eq!(
        run(r#"to_epoch("2009-02-13T23:31:30Z")"#),
        Value::Int(1234567890)
    );
}

#[test]
fn to_epoch_applies_an_offset_and_drops_a_fraction() {
    assert_eq!(
        run(r#"to_epoch("1970-01-01T01:00:00+01:00")"#),
        Value::Int(0)
    );
    assert_eq!(run(r#"to_epoch("1970-01-01T00:00:00.9Z")"#), Value::Int(0));
}

#[test]
fn to_epoch_takes_the_range_ends_and_refuses_the_rest() {
    assert_eq!(
        run(r#"to_epoch("0001-01-01T00:00:00Z")"#),
        Value::Int(-62135596800)
    );
    assert_eq!(
        run(r#"to_epoch("9999-12-31T23:59:59Z")"#),
        Value::Int(253402300799)
    );
    assert_eq!(code(r#"to_epoch("0000-12-31T23:59:59Z")"#), "E_BAD_VALUE");
    assert_eq!(code(r#"to_epoch("10000-01-01T00:00:00Z")"#), "E_BAD_VALUE");
    assert_eq!(code(r#"to_epoch("not a date")"#), "E_BAD_VALUE");
    assert_eq!(code("to_epoch(5)"), "E_BAD_ARG");
}

#[test]
fn to_epoch_and_from_epoch_round_trip() {
    assert_eq!(
        run("to_epoch(from_epoch(1234567890))"),
        Value::Int(1234567890)
    );
    assert_eq!(
        run("from_epoch(to_epoch(\"2024-02-29\"))"),
        text("2024-02-29T00:00:00Z")
    );
}

// --- registration ------------------------------------------------------------

#[test]
fn every_new_name_is_a_served_builtin() {
    for name in [
        "trim",
        "title",
        "capitalize",
        "repeat",
        "words",
        "log",
        "ln",
        "log10",
        "log2",
        "exp",
        "from_epoch",
        "to_epoch",
    ] {
        assert!(
            iron_shell::builtins::is_served(name),
            "{name} is not served"
        );
    }
}

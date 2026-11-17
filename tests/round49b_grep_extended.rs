//! `grep` is BRE; `{extended: true}` reads the pattern as an extended regex,
//! and the refusal of `recursive` names the recursive alternatives.

fn run(src: &str) -> iron_shell::value::Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn int(n: i64) -> iron_shell::value::Value {
    iron_shell::value::Value::Int(n)
}

#[test]
fn a_bare_bar_is_literal_in_bre() {
    assert_eq!(
        run(r#""TODO a\nFIXME b\nok" | grep("TODO|FIXME") | len"#),
        int(0)
    );
}

#[test]
fn a_backslashed_bar_alternates() {
    assert_eq!(
        run(r#""TODO a\nFIXME b\nok" | grep("TODO\\|FIXME") | len"#),
        int(2)
    );
}

#[test]
fn extended_true_alternates_with_a_bare_bar() {
    assert_eq!(
        run(r#""TODO a\nFIXME b\nok" | grep("TODO|FIXME", {extended: true}) | len"#),
        int(2)
    );
    assert_eq!(
        run(r#""aaa\nb" | grep("a+", {extended: true}) | len"#),
        int(1)
    );
}

#[test]
fn recursive_is_refused_with_a_hint_naming_search_grep() {
    let v = run(r#"(try { "x" | grep("x", {recursive: true}) } catch e { e.error.hint })"#);
    let text = format!("{v:?}");
    assert!(text.contains("search"), "{text}");
}

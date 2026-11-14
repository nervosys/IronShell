//! The first letter of `capitalize` and `title` is titlecase, not uppercase, for
//! the Unicode digraphs. `capitalize("ǆ")` gave Ǆ; Python and Unicode give ǅ.
//! Only the digraph letters change: everything else keeps its uppercase form.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn text(src: &str) -> String {
    match run(src) {
        Value::Str(s) => s,
        other => panic!("{src}: expected a String, got {other:?}"),
    }
}

#[test]
fn capitalize_gives_titlecase_for_each_digraph_family() {
    // Dž: U+01C4..U+01C6 -> U+01C5
    assert_eq!(text("capitalize(\"\u{01C4}\")"), "\u{01C5}");
    assert_eq!(text("capitalize(\"\u{01C6}\")"), "\u{01C5}");
    // Lj: U+01C7..U+01C9 -> U+01C8
    assert_eq!(text("capitalize(\"\u{01C7}\")"), "\u{01C8}");
    assert_eq!(text("capitalize(\"\u{01C9}\")"), "\u{01C8}");
    // Nj: U+01CA..U+01CC -> U+01CB
    assert_eq!(text("capitalize(\"\u{01CA}\")"), "\u{01CB}");
    assert_eq!(text("capitalize(\"\u{01CC}\")"), "\u{01CB}");
    // Dz: U+01F1..U+01F3 -> U+01F2
    assert_eq!(text("capitalize(\"\u{01F1}\")"), "\u{01F2}");
    assert_eq!(text("capitalize(\"\u{01F3}\")"), "\u{01F2}");
}

#[test]
fn capitalize_titlecases_the_digraph_and_lowercases_the_rest() {
    // ǆungla -> ǅungla; the rest of an all-caps word is lower case.
    assert_eq!(text("capitalize(\"\u{01C6}ungla\")"), "\u{01C5}ungla");
    assert_eq!(text("capitalize(\"\u{01C4}UNGLA\")"), "\u{01C5}ungla");
}

#[test]
fn capitalize_keeps_uppercase_for_other_letters() {
    assert_eq!(text("capitalize(\"élan\")"), "Élan");
    assert_eq!(text("capitalize(\"hELLO wORLD\")"), "Hello world");
    // A letter whose upper case is two characters keeps that behaviour.
    assert_eq!(text("capitalize(\"ßa\")"), "SSa");
}

#[test]
fn title_uses_titlecase_at_each_word_start() {
    assert_eq!(
        text("title(\"\u{01C6}ungla and \u{01F1}ULU\")"),
        "\u{01C5}ungla And \u{01F2}ulu"
    );
    // A digraph in the middle of a word is lower case, as any letter there is.
    assert_eq!(text("title(\"a\u{01C6}b\")"), "A\u{01C6}b");
}

#[test]
fn title_keeps_uppercase_for_other_letters() {
    assert_eq!(text("title(\"hello WORLD\")"), "Hello World");
    assert_eq!(text("title(\"well-known fact\")"), "Well-Known Fact");
}

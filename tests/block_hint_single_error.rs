//! A block with two statements is one fault, and it is reported as one. It used
//! to give five parse errors: an unclosed '{' first, the one-expression hint
//! last, and the statements after the block as knock-on errors.

use iron_shell::parser::{parse_program, ParseError};

fn parse_error(src: &str) -> (String, ParseError) {
    let err = parse_program(src).expect_err("a two-statement block is refused");
    let pe = err
        .downcast_ref::<ParseError>()
        .unwrap_or_else(|| panic!("not an E_PARSE error: {err}"))
        .clone();
    (err.to_string(), pe)
}

#[test]
fn a_two_statement_block_reports_one_error_with_the_block_hint() {
    let (text, pe) = parse_error("if c { a = 1\n b = 2 }");
    assert!(
        pe.hint
            .contains("a block holds one expression; bind values before it"),
        "{}",
        pe.hint
    );
    assert!(!text.contains("found"), "more than one error: {text}");
    assert!(
        !text.contains("unclosed") && !text.contains("is not closed"),
        "{text}"
    );
    assert!(text.contains("at line 1"), "{text}");
}

#[test]
fn the_error_is_not_buried_by_statements_after_the_block() {
    let (text, pe) = parse_error("if c { a = 1\n b = 2 }\nz = 3\nw = 4\n");
    assert!(
        pe.hint.contains("a block holds one expression"),
        "{}",
        pe.hint
    );
    assert!(!text.contains("found"), "{text}");
}

#[test]
fn a_block_with_a_semicolon_is_one_error_too() {
    let (text, pe) = parse_error("if true { x = 1; x + 1 }");
    assert!(
        pe.hint.contains("a block holds one expression"),
        "{}",
        pe.hint
    );
    assert!(!text.contains("found"), "{text}");
}

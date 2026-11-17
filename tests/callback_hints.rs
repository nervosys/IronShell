//! Hints for the two commonest callback mistakes.

fn hint_of(src: &str) -> String {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    let err = iron_shell::eval::eval_program(&stmts, &mut env).expect_err("must fail");
    err.downcast_ref::<iron_shell::safety::SafetyError>()
        .map(|se| se.hint.clone())
        .unwrap_or_else(|| panic!("not a coded error: {err:#}"))
}

#[test]
fn a_field_read_of_an_int_says_what_it_is() {
    let hint = hint_of("[1, 2] | map(fn(x) => x.foo)");
    assert!(
        hint.contains("`x` is an Int; .foo reads a field of a Record"),
        "{hint}"
    );
}

#[test]
fn a_lambda_with_no_parameter_says_to_take_the_element() {
    let hint = hint_of("[1, 2] | map(fn() => 1)");
    assert!(
        hint.contains("the lambda passed to map takes one argument (the element): fn(x) => ..."),
        "{hint}"
    );
}

//! There is no ontology_search; the refusal names what lists the ontology.

fn refusal(src: &str) -> iron_shell::safety::SafetyError {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    let err = iron_shell::eval::eval_program(&stmts, &mut env).expect_err("must be refused");
    err.downcast_ref::<iron_shell::safety::SafetyError>()
        .expect("a structured refusal")
        .clone()
}

#[test]
fn calling_ontology_search_suggests_the_manifest() {
    let se = refusal(r#"ontology_search("file")"#);
    assert!(
        se.did_you_mean.iter().any(|s| s == "ontology_manifest"),
        "{:?}",
        se.did_you_mean
    );
}

#[test]
fn help_of_ontology_search_suggests_the_manifest() {
    let se = refusal(r#"help("ontology_search")"#);
    assert!(
        se.did_you_mean.iter().any(|s| s == "ontology_manifest"),
        "{:?}",
        se.did_you_mean
    );
}

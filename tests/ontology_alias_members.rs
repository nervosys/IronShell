//! The ontology answers for the names a user types: a session verb looked up
//! bare (`redo`) has a description, an alias says which builtin it runs, and a
//! module lists the editing builtins it calls (`file.delete_lines`) even though
//! their catalogue entry is named by the alias.

use iron_shell::agent_api::{generate_schema_filtered, ontology_describe_json};

#[test]
fn a_bare_session_verb_is_described_by_its_builtin() {
    let redo = ontology_describe_json("redo");
    assert_eq!(redo["builtin"], "session_redo", "{redo}");
    assert!(
        redo["description"].as_str().is_some_and(|d| !d.is_empty()),
        "{redo}"
    );
    assert!(redo.get("error").is_none(), "{redo}");
}

#[test]
fn undo_and_journal_have_descriptions() {
    for name in ["undo", "journal"] {
        let v = ontology_describe_json(name);
        assert!(
            v["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{name}: {v}"
        );
    }
}

#[test]
fn an_alias_names_the_builtin_it_runs() {
    let v = ontology_describe_json("delete_lines");
    assert_eq!(v["builtin"], "delete_lines", "{v}");
    assert_eq!(v["implemented_by"], "file_delete_lines", "{v}");
    assert!(
        v["signature"]
            .as_str()
            .is_some_and(|s| s.starts_with("file_delete_lines(")),
        "{v}"
    );
}

#[test]
fn the_file_module_lists_delete_lines() {
    let schema = generate_schema_filtered("json", Some("file"), None).expect("schema");
    assert!(
        schema.contains("\"delete_lines\""),
        "file module lacks delete_lines"
    );
    assert!(schema.contains("\"file_insert\"") || schema.contains("\"insert\""));
}

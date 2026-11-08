//! `ontology_describe` takes the dotted spelling a call uses, in process.
//!
//! Reproduced on 14.0.0: `ontology_describe("file.replace")`, `("file.write")` and
//! `("crypto.hash")` said the name was unknown, although `file.replace(...)` and
//! `crypto.hash(...)` run. The dotted name now resolves to the builtin the call
//! runs, and describes that same entry. Plain builtin names are unchanged, and a
//! dotted name no module has is still unknown.

fn describe(query: &str) -> serde_json::Value {
    iron_shell::agent_api::ontology_describe_json(query)
}

fn builtin_of(query: &str) -> String {
    let v = describe(query);
    assert!(v.get("error").is_none(), "{query} is unknown: {v}");
    v["builtin"].as_str().unwrap_or_default().to_string()
}

#[test]
fn file_replace_by_its_dotted_name() {
    assert_eq!(builtin_of("file.replace"), "file_replace");
}

#[test]
fn file_write_by_its_dotted_name() {
    assert_eq!(builtin_of("file.write"), "file_write");
}

#[test]
fn crypto_hash_by_its_dotted_name() {
    // The ontology lists this entry as `hash`, with `crypto_hash` as an alias,
    // so the test compares against the plain spelling rather than a name.
    assert_eq!(builtin_of("crypto.hash"), builtin_of("crypto_hash"));
}

#[test]
fn the_dotted_name_describes_the_same_entry_as_the_builtin_name() {
    assert_eq!(describe("crypto.hash"), describe("crypto_hash"));
}

#[test]
fn a_plain_builtin_name_is_unchanged() {
    assert_eq!(builtin_of("file_replace"), "file_replace");
}

#[test]
fn a_dotted_name_no_module_has_is_still_unknown() {
    let v = describe("file.nosuchmember");
    assert!(v.get("error").is_some(), "{v}");
}

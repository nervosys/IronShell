//! help("MODULE") always answers the module record, whatever else shares the name.

use iron_shell::agent_api::{build_language_ontology, ontology_describe_json};
use iron_shell::safety::SafetyError;

#[test]
fn every_schema_module_lists_members_that_resolve() {
    let ontology = build_language_ontology();
    assert!(!ontology.modules.is_empty());
    let mut bad = Vec::new();
    for m in &ontology.modules {
        let record = ontology_describe_json(&m.name);
        if record.get("module").and_then(|v| v.as_str()) != Some(m.name.as_str()) {
            bad.push(format!("help({}) is not a module record: {record}", m.name));
            continue;
        }
        let members = record
            .get("members")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if members.is_empty() {
            bad.push(format!("help({}) lists no members", m.name));
        }
        for member in &members {
            let name = member.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let q = format!("{}.{name}", m.name);
            let described = ontology_describe_json(&q);
            if described.get("error").is_some() {
                bad.push(format!("help({q}) does not resolve"));
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

#[test]
fn a_module_that_shares_a_builtin_name_keeps_both() {
    for name in ["str", "platform", "ai", "jq", "shell"] {
        let record = ontology_describe_json(name);
        assert_eq!(
            record.get("module").and_then(|v| v.as_str()),
            Some(name),
            "{name}: {record}"
        );
        let members = record["members"].as_array().expect("members");
        assert!(!members.is_empty(), "{name} lists no members");
        let builtin = record
            .get("builtin")
            .unwrap_or_else(|| panic!("{name} should carry its same-named builtin: {record}"));
        assert_eq!(
            builtin.get("builtin").and_then(|v| v.as_str()),
            Some(name),
            "{name}: {builtin}"
        );
    }
}

#[test]
fn crypto_and_math_answer_the_module_not_the_category() {
    let math = ontology_describe_json("math");
    assert!(math.get("category").is_none(), "{math}");
    let names: Vec<&str> = math["members"]
        .as_array()
        .expect("members")
        .iter()
        .filter_map(|m| m["name"].as_str())
        .collect();
    assert!(!names.contains(&"ps_add"), "{names:?}");
    for want in ["sqrt", "pi"] {
        assert!(names.contains(&want), "math lacks {want}: {names:?}");
    }
    let crypto = ontology_describe_json("crypto");
    assert!(crypto.get("category").is_none(), "{crypto}");
    assert!(crypto["members"].as_array().is_some_and(|m| !m.is_empty()));
}

#[test]
fn a_top_level_builtin_named_as_a_member_gets_the_call_hint() {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program("help(\"math.ps_add\")").expect("parses");
    let e = iron_shell::eval::eval_program(&stmts, &mut env).expect_err("not a member");
    let s = e.downcast_ref::<SafetyError>().expect("a coded error");
    assert!(s.hint.contains("is a top-level builtin"), "{}", s.hint);
    assert!(!s.hint.contains("ontology_manifest"), "{}", s.hint);
}

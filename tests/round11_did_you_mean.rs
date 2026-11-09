//! Round 11: the did-you-mean suggestions for names other languages and
//! libraries use. A library spelling (`re.findall`, `json.loads`, `filter`)
//! is named for the builtin it means, rather than for the nearest module or
//! builtin by edit distance.

use iron_shell::env::Env;
use iron_shell::safety::SafetyError;

fn refusal(src: &str) -> SafetyError {
    let mut env = Env::new();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    let err =
        iron_shell::eval::eval_program(&stmts, &mut env).expect_err("the program must be refused");
    err.downcast_ref::<SafetyError>()
        .expect("a structured refusal")
        .clone()
}

#[test]
fn re_findall_is_regex_find_all_not_the_rg_module() {
    let se = refusal(r#"re.findall("a", "banana")"#);
    assert_eq!(
        se.did_you_mean.first().map(String::as_str),
        Some("regex_find_all")
    );
    assert!(
        !se.did_you_mean.iter().any(|s| s == "rg" || s == "rl"),
        "{:?}",
        se.did_you_mean
    );
}

#[test]
fn regex_module_spellings_name_the_regex_builtins() {
    let cases = [
        (r#"regex.match("a", "a")"#, "regex_match"),
        (r#"regex.search("a", "a")"#, "regex_find"),
        (r#"re.match("a", "a")"#, "regex_match"),
        (r#"re.sub("a", "b", "a")"#, "regex_replace"),
    ];
    for (src, want) in cases {
        let se = refusal(src);
        assert_eq!(se.did_you_mean, vec![want.to_string()], "{src}");
    }
}

#[test]
fn json_loads_and_dumps_name_the_json_module_members() {
    let se = refusal(r#"json.loads("[1]")"#);
    assert_eq!(
        se.did_you_mean.first().map(String::as_str),
        Some("json.parse")
    );
    let se = refusal(r#"json.dumps([1])"#);
    assert_eq!(
        se.did_you_mean.first().map(String::as_str),
        Some("json.stringify")
    );
}

#[test]
fn toml_parse_and_dumps_name_the_toml_builtins() {
    let se = refusal(r#"toml.parse("a = 1")"#);
    assert_eq!(se.did_you_mean, vec!["from_toml".to_string()]);
    let se = refusal(r#"toml.dumps({a: 1})"#);
    assert_eq!(se.did_you_mean, vec!["to_toml".to_string()]);
}

#[test]
fn filter_is_where() {
    let se = refusal("filter(fn(x) => x > 1, [1, 2])");
    assert_eq!(se.did_you_mean.first().map(String::as_str), Some("where"));
}

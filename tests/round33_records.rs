//! Round 33: the record and row edges a data-wrangling agent hits.
//!
//! - `group_by` takes its subject as the first argument as well as piped, and
//!   its errors do not speak of `Group-Object`.
//! - `get`, `has`, `to_entries` and `from_entries` read and rebuild records
//!   whose rows are not all alike; a missing field's hint names `get`.
//! - A quoted record key interpolates `${...}` like any string.
//! - `open` parses `.tsv` like `.csv` with a tab delimiter.
//! - `from_yaml` names the unsupported construct in its hint.
//! - `sql`'s "no such table" hint names the Record-of-tables form.

use iron_shell::value::Value;
use std::collections::BTreeMap;

fn eval(src: &str) -> anyhow::Result<Value> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env)
}

fn run(src: &str) -> Value {
    eval(src).unwrap_or_else(|e| panic!("{src:?} failed: {e}"))
}

fn refused(src: &str) -> String {
    match eval(src) {
        Ok(v) => panic!("expected a refusal for {src:?}, got {v:?}"),
        Err(e) => e.to_string(),
    }
}

fn ints(v: &[i64]) -> Value {
    Value::Array(v.iter().map(|i| Value::Int(*i)).collect())
}

fn strs(v: &[&str]) -> Value {
    Value::Array(v.iter().map(|s| Value::Str((*s).to_string())).collect())
}

fn rec(pairs: &[(&str, Value)]) -> Value {
    Value::Record(
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect::<BTreeMap<_, _>>(),
    )
}

/// A fresh temp directory, as a forward-slash path an ish string can hold.
fn temp_dir(tag: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("round33_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let text = dir.to_string_lossy().replace('\\', "/");
    (dir, text)
}

// ---- item 1: group_by takes its subject as the first argument ----

#[test]
fn group_by_takes_the_rows_as_the_first_argument() {
    assert_eq!(
        run(r#"group_by([{a: 1}, {a: 2}, {a: 1}], "a") | map(fn(g) => g.Count)"#),
        ints(&[2, 1])
    );
    assert_eq!(
        run(r#"group_by([{k: "x"}, {k: "y"}, {k: "x"}], fn(r) => r.k) | map(fn(g) => g.Count)"#),
        ints(&[2, 1])
    );
}

#[test]
fn group_by_still_works_piped() {
    assert_eq!(
        run(r#"[{a: 1}, {a: 2}, {a: 1}] | group_by("a") | map(fn(g) => g.Count)"#),
        ints(&[2, 1])
    );
    assert_eq!(
        run(r#"[{k: "x"}, {k: "y"}, {k: "x"}] | group_by(fn(r) => r.k) | len"#),
        Value::Int(2)
    );
}

#[test]
fn group_by_errors_do_not_say_group_object_or_demand_a_pipe() {
    for src in [
        r#"group_by("a")"#,
        "group_by([{a: 1}])",
        "group_by()",
        "group_by(5, \"a\")",
    ] {
        let text = refused(src);
        assert!(!text.contains("Group-Object"), "{src}: {text}");
        assert!(!text.contains("requires pipeline input"), "{src}: {text}");
    }
}

// ---- item 6: group_by's Name and order, as documented ----

#[test]
fn group_by_names_are_strings_sorted_as_text() {
    assert_eq!(
        run(r#"[{n: 10}, {n: 9}, {n: 10}] | group_by("n") | map(fn(g) => g.Name)"#),
        strs(&["10", "9"])
    );
    assert_eq!(
        run(r#"typeof(([{n: 1}] | group_by("n") | first).Name)"#),
        Value::Str("String".into())
    );
    let doc = iron_shell::signature::signature_of("group_by")
        .expect("declared")
        .doc;
    assert!(doc.contains("Name is always a String"), "{doc}");
    assert!(doc.contains("sorted by Name as text"), "{doc}");
    assert!(doc.contains("group_by(rows, \"a\")"), "{doc}");
}

// ---- item 2: optional fields ----

#[test]
fn get_returns_the_default_for_a_missing_or_null_step() {
    assert_eq!(run(r#"{u: {n: 5}} | get("u.n", 0)"#), Value::Int(5));
    assert_eq!(run(r#"get({u: {n: 5}}, "u.n")"#), Value::Int(5));
    assert_eq!(run(r#"{a: 1} | get("b", 0)"#), Value::Int(0));
    assert_eq!(run(r#"get({a: 1}, "b")"#), Value::Null);
    assert_eq!(
        run(r#"get({u: null}, "u.n", "none")"#),
        Value::Str("none".into())
    );
    assert_eq!(run(r#"get({u: {n: null}}, "u.n", 7)"#), Value::Int(7));
    assert_eq!(run(r#"get({xs: [10, 20]}, ["xs", 1])"#), Value::Int(20));
    assert_eq!(run(r#"get({xs: [10, 20]}, "xs.1")"#), Value::Int(20));
    assert_eq!(run(r#"get({xs: [10, 20]}, "xs.5", -1)"#), Value::Int(-1));
}

#[test]
fn where_over_rows_missing_a_field_works_with_get() {
    let (dir, path) = temp_dir("het");
    std::fs::write(
        dir.join("het.jsonl"),
        "{\"a\": 1, \"b\": 2}\n{\"b\": 3}\n{\"a\": 1, \"b\": 4}\n",
    )
    .expect("write");
    // The strict form still refuses the row without the field ...
    let text = refused(&format!(r#"open("{path}/het.jsonl") | where(.a == 1)"#));
    assert!(text.contains("E_UNKNOWN_FIELD"), "{text}");
    // ... and get is the form for an optional one.
    assert_eq!(
        run(&format!(
            r#"open("{path}/het.jsonl") | where(fn(r) => get(r, "a", 0) == 1) | map(fn(r) => r.b)"#
        )),
        ints(&[2, 4])
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn has_tests_for_a_key() {
    assert_eq!(run(r#"{a: 1} | has("a")"#), Value::Bool(true));
    assert_eq!(run(r#"has({a: 1}, "b")"#), Value::Bool(false));
    assert_eq!(run(r#"{a: null} | has("a")"#), Value::Bool(true));
    assert_eq!(run("has([1, 2], 1)"), Value::Bool(true));
    assert_eq!(run("has([1, 2], 2)"), Value::Bool(false));
}

#[test]
fn has_is_a_builtin_and_has_key_points_at_it() {
    assert_eq!(
        iron_shell::builtins::did_you_mean("has_key"),
        vec!["has".to_string()]
    );
    assert!(iron_shell::builtins::BUILTIN_LOOKUP.contains_key("has"));
    assert!(iron_shell::builtins::BUILTIN_LOOKUP.contains_key("get"));
}

#[test]
fn to_entries_and_from_entries_round_trip() {
    assert_eq!(
        run("{a: 1, b: 2} | to_entries"),
        Value::Array(vec![
            rec(&[("key", Value::Str("a".into())), ("value", Value::Int(1))]),
            rec(&[("key", Value::Str("b".into())), ("value", Value::Int(2))]),
        ])
    );
    assert_eq!(
        run(r#"[["a", 1], ["b", 2]] | from_entries"#),
        rec(&[("a", Value::Int(1)), ("b", Value::Int(2))])
    );
    assert_eq!(
        run(r#"from_entries([{key: "a", value: 1}, {key: "a", value: 2}])"#),
        rec(&[("a", Value::Int(2))])
    );
    assert_eq!(
        run("{a: 1, b: 2} | to_entries | from_entries"),
        rec(&[("a", Value::Int(1)), ("b", Value::Int(2))])
    );
    assert_eq!(run("from_entries([])"), rec(&[]));
}

#[test]
fn from_entries_refuses_a_non_string_key_naming_the_index() {
    let text = refused(r#"from_entries([["a", 1], [2, 3]])"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("entry 1"), "{text}");
}

#[test]
fn a_missing_field_hint_names_get_and_suggests_only_close_keys() {
    // One-letter names are not slips of each other.
    let text = refused("r = {b: 1}; r.a");
    assert!(text.contains("E_UNKNOWN_FIELD"), "{text}");
    assert!(text.contains("this record has no field `a`"), "{text}");
    assert!(text.contains("use get(r, \"a\")"), "{text}");
    assert!(!text.contains("did you mean: b"), "{text}");
    // A close key is still offered.
    let text = refused("r = {name: 1}; r.nmae");
    assert!(text.contains("did you mean: name"), "{text}");
}

// ---- item 3: interpolated record keys ----

#[test]
fn a_quoted_record_key_interpolates() {
    assert_eq!(
        run(r#"r = {customer: "acme", region: "eu"}; {"${r.customer}": r.region}"#),
        rec(&[("acme", Value::Str("eu".into()))])
    );
    assert_eq!(
        run(r#"r = {customer: "acme"}; {'${r.customer}': 1}"#),
        rec(&[("acme", Value::Int(1))])
    );
    assert_eq!(
        run(r#"n = 3; {"k${n}": n, "x_${n + 1}": 0}"#),
        rec(&[("k3", Value::Int(3)), ("x_4", Value::Int(0))])
    );
}

#[test]
fn a_plain_record_key_stays_constant_and_a_later_duplicate_wins() {
    assert_eq!(
        run(r#"{"plain key": 1, bare: 2}"#),
        rec(&[("plain key", Value::Int(1)), ("bare", Value::Int(2))])
    );
    assert_eq!(
        run(r#"a = "k"; {"${a}": 1, "k": 2}"#),
        rec(&[("k", Value::Int(2))])
    );
    assert_eq!(
        run(r#"a = "k"; {"k": 1, "${a}": 2}"#),
        rec(&[("k", Value::Int(2))])
    );
}

#[test]
fn an_interpolated_key_works_in_a_lambda_over_rows() {
    assert_eq!(
        run(
            r#"[{c: "a", v: 1}, {c: "b", v: 2}] | map(fn(r) => {"${r.c}": r.v}) | map(fn(x) => keys(x) | first)"#
        ),
        strs(&["a", "b"])
    );
}

// ---- item 4: .tsv ----

#[test]
fn open_parses_a_tsv_file_like_a_csv_one() {
    let (dir, path) = temp_dir("tsv");
    std::fs::write(dir.join("x.tsv"), "a\tb\n1\tx y\n2\tz\n").expect("write");
    assert_eq!(
        run(&format!(r#"open("{path}/x.tsv") | map(fn(r) => r.a)"#)),
        ints(&[1, 2])
    );
    assert_eq!(
        run(&format!(r#"open("{path}/x.tsv") | map(fn(r) => r.b)"#)),
        strs(&["x y", "z"])
    );
    // It was the raw text before.
    assert_eq!(
        run(&format!(r#"typeof(open("{path}/x.tsv"))"#)),
        Value::Str("Array".into())
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn open_documents_tsv() {
    let sig = iron_shell::signature::signature_of("open").expect("declared");
    assert!(sig.doc.contains("TSV"), "{}", sig.doc);
    assert!(sig.params[0].doc.contains(".tsv"), "{}", sig.params[0].doc);
}

// ---- item 5: from_yaml hint ----

#[test]
fn from_yaml_names_the_unsupported_construct_in_its_hint() {
    for (src, construct) in [
        (r#"from_yaml("a: |\n  block\n")"#, "block scalars"),
        (r#"from_yaml("a: &x 1\nb: 2\n")"#, "anchors"),
        (r#"from_yaml("a: !!str 1\n")"#, "explicit tags"),
        (r#"from_yaml("a: {b: 1}\n")"#, "flow collection"),
    ] {
        let text = refused(src);
        assert!(
            text.contains("outside IronShell's YAML subset"),
            "{src}: {text}"
        );
        assert!(text.contains(construct), "{src}: {text}");
        assert!(text.contains("from_json"), "{src}: {text}");
        assert!(!text.contains("from_yaml takes String"), "{src}: {text}");
    }
}

// ---- item 7: a join over two tables ----

#[test]
fn sql_names_the_record_of_tables_form_for_a_missing_table() {
    let text = refused(r#"[{id: 1}] | sql("select * from t o join customers c on o.id = c.id")"#);
    assert!(text.contains("no such table"), "{text}");
    assert!(text.contains("Record of tables"), "{text}");
    // And the form it names works.
    assert_eq!(
        run(
            r#"{t: [{id: 1, n: "x"}], customers: [{id: 1, name: "acme"}]} | sql("select c.name as name from t o join customers c on o.id = c.id") | map(fn(r) => r.name)"#
        ),
        strs(&["acme"])
    );
}

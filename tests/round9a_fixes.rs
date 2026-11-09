//! Round 9, agent A: empty find strings, negative slice indices, `len` in
//! characters, `\uHHHH` escapes, and a module's private helpers.

use iron_shell::value::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn run_err(src: &str) -> String {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    match iron_shell::eval::eval_program(&stmts, &mut env) {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.to_string(),
    }
}

fn ints(v: &[i64]) -> Value {
    Value::Array(v.iter().map(|i| Value::Int(*i)).collect())
}

fn s(text: &str) -> Value {
    Value::Str(text.to_string())
}

/// A scratch directory of this test's own, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae_r9a_fixes_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        Scratch(dir)
    }

    fn file(&self, name: &str, text: &str) -> String {
        let p = self.0.join(name);
        std::fs::write(&p, text).expect("seed");
        p.to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn call(name: &str, args: Vec<Value>) -> Result<Value, String> {
    let mut env = iron_shell::env::Env::new();
    iron_shell::builtins::call(name, args, &mut env).map_err(|e| e.to_string())
}

// ---- 1. An empty find string is refused before anything is written ----

#[test]
fn file_replace_with_an_empty_find_is_refused_and_writes_nothing() {
    let dir = Scratch::new("empty_replace");
    let p = dir.file("f.txt", "abc");
    let err = call("file_replace", vec![s(&p), s(""), s("x")]).expect_err("refused");
    assert!(
        err.contains("file_replace: the text to find is empty"),
        "{err}"
    );
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "abc");
}

#[test]
fn file_patch_with_an_empty_find_is_refused_and_writes_nothing() {
    let dir = Scratch::new("empty_patch");
    let p = dir.file("f.txt", "abc");
    let mut patch = BTreeMap::new();
    patch.insert("find".to_string(), s(""));
    patch.insert("replace".to_string(), s("x"));
    let err = call(
        "file_patch",
        vec![s(&p), Value::Array(vec![Value::Record(patch)])],
    )
    .expect_err("refused");
    assert!(err.contains("the text to find is empty"), "{err}");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "abc");
}

#[test]
fn file_insert_with_an_empty_anchor_is_refused_and_writes_nothing() {
    let dir = Scratch::new("empty_anchor");
    let p = dir.file("f.txt", "one\ntwo\n");
    let err = call("file_insert", vec![s(&p), s(""), s("new")]).expect_err("refused");
    assert!(
        err.contains("file_insert: the text to find is empty"),
        "{err}"
    );
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "one\ntwo\n");
}

#[test]
fn file_delete_lines_with_an_empty_pattern_is_refused_and_deletes_nothing() {
    let dir = Scratch::new("empty_delete");
    let p = dir.file("f.txt", "one\ntwo\n");
    let mut selector = BTreeMap::new();
    selector.insert("contains".to_string(), s(""));
    let err = call("file_delete_lines", vec![s(&p), Value::Record(selector)]).expect_err("refused");
    assert!(err.contains("the text to find is empty"), "{err}");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "one\ntwo\n");
}

#[test]
fn a_non_empty_find_still_replaces() {
    let dir = Scratch::new("nonempty");
    let p = dir.file("f.txt", "abc");
    call("file_replace", vec![s(&p), s("b"), s("X")]).expect("replace");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "aXc");
}

// ---- 4. Negative slice indices count from the end ----

#[test]
fn slice_with_a_negative_start_counts_from_the_end() {
    assert_eq!(run(r#""hello" | slice(-3)"#), s("llo"));
    assert_eq!(run("[1, 2, 3, 4, 5] | slice(-2)"), ints(&[4, 5]));
}

#[test]
fn slice_with_a_negative_end_drops_from_the_end() {
    assert_eq!(run(r#""hello" | slice(0, -1)"#), s("hell"));
    assert_eq!(run("[1, 2, 3, 4, 5] | slice(1, -1)"), ints(&[2, 3, 4]));
}

#[test]
fn slice_clamps_and_a_start_past_the_end_is_empty() {
    assert_eq!(run(r#""hello" | slice(-10)"#), s("hello"));
    assert_eq!(run(r#""hello" | slice(3, 1)"#), s(""));
    assert_eq!(run("[1, 2, 3] | slice(5)"), ints(&[]));
    assert_eq!(run("[1, 2, 3] | slice(-1, -2)"), ints(&[]));
}

// ---- 5. len counts characters ----

#[test]
fn len_of_a_string_counts_characters_not_bytes() {
    assert_eq!(run(r#"len("日本語")"#), Value::Int(3));
    assert_eq!(run(r#"len("😀")"#), Value::Int(1));
    assert_eq!(run(r#"len("héllo")"#), Value::Int(5));
    assert_eq!(run(r#""héllo" | len"#), Value::Int(5));
}

// ---- 6. \uHHHH escapes ----

#[test]
fn a_four_digit_unicode_escape_is_one_character() {
    assert_eq!(run(r#""é""#), s("é"));
    assert_eq!(run(r#""aAb""#), s("aAb"));
}

#[test]
fn the_braced_unicode_escape_still_works() {
    assert_eq!(run(r#""\u{e9}""#), s("é"));
}

#[test]
fn an_unknown_escape_keeps_its_backslash() {
    assert_eq!(run(r#""\q""#), s("\\q"));
}

// ---- 8. An exported lambda reaches private helpers defined after it ----

/// A module file in its own directory, and the entry file that imports it.
fn module_run(tag: &str, module: &str, main: &str) -> Value {
    let dir = Scratch::new(tag);
    std::fs::write(dir.0.join("lib.ish"), module).expect("module");
    let entry = dir.0.join("main.ish");
    std::fs::write(&entry, main).expect("entry");
    let _guard = iron_shell::module_loader::enter_entry_file(Path::new(&entry));
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(main).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

#[test]
fn an_export_calls_a_helper_defined_after_it() {
    let v = module_run(
        "fwd_helper",
        "pub let f = fn(x) => h(x)\nlet h = fn(x) => x + 1\n",
        "import \"lib.ish\"\nf(1)\n",
    );
    assert_eq!(v, Value::Int(2));
}

#[test]
fn an_export_reaches_a_chain_of_helpers_defined_after_it() {
    let v = module_run(
        "fwd_chain",
        "pub let f = fn(x) => h(x)\nlet h = fn(x) => k(x) * 2\nlet k = fn(x) => x + 1\n",
        "import \"lib.ish\"\nf(1)\n",
    );
    assert_eq!(v, Value::Int(4));
}

#[test]
fn an_export_reaches_a_later_value_it_reads() {
    let v = module_run(
        "fwd_value",
        "pub let f = fn(x) => x + offset\nlet offset = 10\n",
        "import \"lib.ish\"\nf(1)\n",
    );
    assert_eq!(v, Value::Int(11));
}

#[test]
fn mutually_recursive_module_functions_work_from_outside() {
    let v = module_run(
        "mutual",
        "pub let is_even = fn(n) => if n == 0 { true } else { is_odd(n - 1) }\n\
         let is_odd = fn(n) => if n == 0 { false } else { is_even(n - 1) }\n",
        "import \"lib.ish\"\nis_even(4)\n",
    );
    assert_eq!(v, Value::Bool(true));
}

#[test]
fn a_helper_is_still_not_visible_by_name_to_the_importer() {
    let err = module_run_err(
        "private_name",
        "pub let f = fn(x) => h(x)\nlet h = fn(x) => x + 1\n",
        "import \"lib.ish\"\nh(1)\n",
    );
    assert!(err.contains("not exported") || err.contains("E_"), "{err}");
}

fn module_run_err(tag: &str, module: &str, main: &str) -> String {
    let dir = Scratch::new(tag);
    std::fs::write(dir.0.join("lib.ish"), module).expect("module");
    let entry = dir.0.join("main.ish");
    std::fs::write(&entry, main).expect("entry");
    let _guard = iron_shell::module_loader::enter_entry_file(Path::new(&entry));
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(main).expect("parses");
    match iron_shell::eval::eval_program(&stmts, &mut env) {
        Ok(v) => panic!("expected an error, got {v:?}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn run_err_reports_an_unknown_name() {
    assert!(run_err("no_such_name_zz").contains("E_"));
}

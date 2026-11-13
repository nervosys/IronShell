//! The text-only path builtins, and the `path` module that names them.
//!
//! Every answer is computed from the text, so each expectation below holds on
//! Linux, macOS and Windows alike. Windows-style inputs (`C:\a\b.txt`) are
//! included on purpose: `\` separates components on every platform, and every
//! result uses `/`.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn text(src: &str) -> String {
    match run(src) {
        Value::Str(s) => s,
        other => panic!("{src} gave {other:?}, not a String"),
    }
}

fn strs(items: &[&str]) -> Value {
    Value::Array(items.iter().map(|s| Value::Str(s.to_string())).collect())
}

fn code_of(src: &str) -> String {
    text(&format!("(try {{ {src} }} catch e {{ e.error.code }})"))
}

#[test]
fn join_puts_one_slash_between_parts() {
    assert_eq!(text(r#"path_join("a", "b")"#), "a/b");
    assert_eq!(text(r#"path_join("a", "b", "c.txt")"#), "a/b/c.txt");
    assert_eq!(text(r#"path_join("a/", "b")"#), "a/b");
}

#[test]
fn join_takes_an_array_or_a_piped_first_part() {
    assert_eq!(text(r#"path_join(["data", "c.txt"])"#), "data/c.txt");
    assert_eq!(text(r#""data" | path_join("c.txt")"#), "data/c.txt");
}

#[test]
fn an_absolute_part_replaces_the_parts_before_it() {
    assert_eq!(text(r#"path_join("a", "/etc", "b")"#), "/etc/b");
    assert_eq!(text(r#"path_join("a", "C:\\data")"#), "C:/data");
}

#[test]
fn join_turns_windows_separators_into_slashes() {
    assert_eq!(text(r#"path_join("C:\\data", "x.txt")"#), "C:/data/x.txt");
    assert_eq!(text(r#"path_join("a", "b\\c")"#), "a/b/c");
}

#[test]
fn basename_is_the_last_component() {
    assert_eq!(text(r#"path_basename("a/b/c.txt")"#), "c.txt");
    assert_eq!(text(r#"path_basename("C:\\a\\b.txt")"#), "b.txt");
    assert_eq!(text(r#""C:\\a\\b.txt" | path_basename()"#), "b.txt");
}

#[test]
fn basename_ignores_a_trailing_separator() {
    assert_eq!(text(r#"path_basename("a/b/")"#), "b");
    assert_eq!(text(r#"path_basename("a\\b\\")"#), "b");
}

#[test]
fn basename_of_a_root_or_nothing_is_empty() {
    assert_eq!(text(r#"path_basename("/")"#), "");
    assert_eq!(text(r#"path_basename("")"#), "");
}

#[test]
fn dirname_is_everything_above_the_last_component() {
    assert_eq!(text(r#"path_dirname("a/b/c.txt")"#), "a/b");
    assert_eq!(text(r#"path_dirname("C:\\a\\b.txt")"#), "C:/a");
    assert_eq!(text(r#"path_dirname("a/b/")"#), "a");
}

#[test]
fn dirname_of_one_relative_component_is_dot() {
    assert_eq!(text(r#"path_dirname("c.txt")"#), ".");
    assert_eq!(text(r#"path_dirname("")"#), ".");
}

#[test]
fn dirname_of_a_root_is_the_root() {
    assert_eq!(text(r#"path_dirname("/a")"#), "/");
    assert_eq!(text(r#"path_dirname("/")"#), "/");
    assert_eq!(text(r#"path_dirname("C:\\a")"#), "C:/");
}

#[test]
fn extension_is_the_last_suffix_with_its_dot() {
    assert_eq!(text(r#"path_extension("c.txt")"#), ".txt");
    assert_eq!(text(r#"path_extension("archive.tar.gz")"#), ".gz");
    assert_eq!(text(r#"path_extension("C:\\x\\y.TXT")"#), ".TXT");
}

#[test]
fn extension_is_empty_when_the_name_has_no_suffix() {
    assert_eq!(text(r#"path_extension(".bashrc")"#), "");
    assert_eq!(text(r#"path_extension("Makefile")"#), "");
    assert_eq!(text(r#"path_extension("a.")"#), "");
    assert_eq!(text(r#"path_extension("dir.d/file")"#), "");
}

#[test]
fn stem_is_the_name_without_its_suffix() {
    assert_eq!(text(r#"path_stem("c.txt")"#), "c");
    assert_eq!(text(r#"path_stem("archive.tar.gz")"#), "archive.tar");
    assert_eq!(text(r#"path_stem(".bashrc")"#), ".bashrc");
    assert_eq!(text(r#"path_stem("C:\\a\\b.txt")"#), "b");
}

#[test]
fn normalize_collapses_dots_separators_and_parent_pairs() {
    assert_eq!(text(r#"path_normalize("a/./b/../c//d/")"#), "a/c/d");
    assert_eq!(text(r#"path_normalize("a/..")"#), ".");
    assert_eq!(text(r#"path_normalize("")"#), ".");
    assert_eq!(text(r#"path_normalize(".")"#), ".");
}

#[test]
fn normalize_keeps_the_leading_parents_of_a_relative_path() {
    assert_eq!(text(r#"path_normalize("../x/../../y")"#), "../../y");
    assert_eq!(text(r#"path_normalize("../..")"#), "../..");
}

#[test]
fn normalize_drops_parents_above_a_root() {
    assert_eq!(text(r#"path_normalize("/../a")"#), "/a");
    assert_eq!(text(r#"path_normalize("C:\\a\\..\\..\\b")"#), "C:/b");
}

#[test]
fn normalize_reads_windows_separators_as_separators() {
    assert_eq!(text(r#"path_normalize("a\\.\\b\\..\\c")"#), "a/c");
}

#[test]
fn is_absolute_sees_a_root_or_a_drive() {
    assert_eq!(run(r#"path_is_absolute("/etc")"#), Value::Bool(true));
    assert_eq!(run(r#"path_is_absolute("C:\\x")"#), Value::Bool(true));
    assert_eq!(run(r#"path_is_absolute("C:/x")"#), Value::Bool(true));
    assert_eq!(run(r#"path_is_absolute("a/b")"#), Value::Bool(false));
    assert_eq!(run(r#"path_is_absolute("")"#), Value::Bool(false));
}

#[test]
fn split_lists_the_components_root_first() {
    assert_eq!(
        run(r#"path_split("a/b/c.txt")"#),
        strs(&["a", "b", "c.txt"])
    );
    assert_eq!(run(r#"path_split("/a/b")"#), strs(&["/", "a", "b"]));
    assert_eq!(run(r#"path_split("C:\\a\\b")"#), strs(&["C:/", "a", "b"]));
    assert_eq!(run(r#"path_split("")"#), strs(&[]));
}

#[test]
fn split_drops_dot_parts_and_keeps_parents() {
    assert_eq!(run(r#"path_split("./a/../b")"#), strs(&["a", "..", "b"]));
}

#[test]
fn a_non_string_is_refused_with_bad_arg() {
    assert_eq!(code_of("path_basename(1)"), "E_BAD_ARG");
    assert_eq!(code_of("path_join(1, \"a\")"), "E_BAD_ARG");
    assert_eq!(code_of("path_join()"), "E_BAD_ARG");
    assert_eq!(code_of("path_normalize(\"a\", \"b\")"), "E_BAD_ARG");
}

#[test]
fn the_module_names_the_same_builtins() {
    assert_eq!(text(r#"path.join("a", "b")"#), "a/b");
    assert_eq!(text(r#"path.join(["a", "b", "c.txt"])"#), "a/b/c.txt");
    assert_eq!(text(r#"path.basename("C:\\a\\b.txt")"#), "b.txt");
    assert_eq!(text(r#"path.dirname("a/b/c.txt")"#), "a/b");
    assert_eq!(text(r#"path.extension("archive.tar.gz")"#), ".gz");
    assert_eq!(text(r#"path.stem("c.txt")"#), "c");
    assert_eq!(text(r#"path.normalize("a/./b/../c")"#), "a/c");
    assert_eq!(run(r#"path.is_absolute("/etc")"#), Value::Bool(true));
    assert_eq!(run(r#"path.split("a/b")"#), strs(&["a", "b"]));
}

#[test]
fn the_module_and_the_builtin_agree_on_a_piped_path() {
    assert_eq!(
        text(r#""a/b/c.txt" | path.basename()"#),
        text(r#"path_basename("a/b/c.txt")"#)
    );
}

#[test]
fn path_is_a_module_name_and_a_member_list() {
    assert!(iron_shell::modules::is_module_name("path"));
    for member in [
        "join",
        "basename",
        "dirname",
        "extension",
        "stem",
        "normalize",
        "is_absolute",
        "split",
    ] {
        assert!(
            iron_shell::modules::is_module_member("path", member),
            "path.{member} is missing"
        );
    }
}

#[test]
fn the_path_builtins_are_pure() {
    for name in [
        "path_join",
        "path_basename",
        "path_dirname",
        "path_extension",
        "path_stem",
        "path_normalize",
        "path_is_absolute",
        "path_split",
    ] {
        assert_eq!(
            format!("{:?}", iron_shell::safety::effect_of(name)),
            "Pure",
            "{name} must be a pure builtin"
        );
    }
}

#[test]
fn the_pathlib_and_os_path_names_point_at_the_new_builtins() {
    use iron_shell::builtins::did_you_mean;
    assert!(did_you_mean("basename").contains(&"path_basename".to_string()));
    assert!(did_you_mean("dirname").contains(&"path_dirname".to_string()));
    assert!(did_you_mean("extname").contains(&"path_extension".to_string()));
    assert!(did_you_mean("file_stem").contains(&"path_stem".to_string()));
    assert!(did_you_mean("normpath").contains(&"path_normalize".to_string()));
    assert!(did_you_mean("splitext").contains(&"path_stem".to_string()));
    assert!(did_you_mean("abspath").contains(&"path_is_absolute".to_string()));
    assert_eq!(
        iron_shell::builtins::mapped_module_spelling("path", "normpath"),
        vec!["path.normalize".to_string()]
    );
}

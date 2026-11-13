//! Round 28: the file and format writers keep the bytes they were given.
//!
//! - `file_insert` at line 1 of a UTF-8 BOM file keeps the BOM as the first bytes.
//! - `file_edit` keeps a leading BOM and writes CRLF when every line break is
//!   CRLF; its refusals name the edit and the key that is wrong.
//! - `to_csv` refuses a repeated name in an explicit column list, and a NaN or
//!   infinite Float cell, as `to_json` refuses them.
//! - `open` on a directory says it is a directory, not a file.
//! - `to_yaml` double-quotes and escapes control characters; `from_yaml` reads
//!   `0o` and `0x` integers, and `to_yaml` quotes strings that look like them.
//! - `file_write` names the element of a mixed Array that is not a String.
//!
//! In process, against real files in a temp directory. Serialised, since the
//! journal is process-global.

use iron_shell::journal;
use iron_shell::safety::SafetyError;
use iron_shell::value::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

struct Session {
    dir: PathBuf,
}

impl Session {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ae_r28_files_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::env::set_var("IRONSHELL_JOURNAL", "on");
        std::env::set_var("IRONSHELL_SESSION_DIR", dir.join("state"));
        journal::clear();
        Self { dir }
    }
    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        journal::clear();
        std::env::remove_var("IRONSHELL_JOURNAL");
        std::env::remove_var("IRONSHELL_SESSION_DIR");
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn call(name: &str, args: Vec<Value>) -> Result<Value, anyhow::Error> {
    let mut env = iron_shell::env::Env::new();
    iron_shell::builtins::call(name, args, &mut env)
}

/// The structured refusal a call answers with; panics if it succeeded or was
/// not structured.
fn refusal(name: &str, args: Vec<Value>) -> SafetyError {
    let err = call(name, args).expect_err("the call is refused");
    err.downcast_ref::<SafetyError>()
        .expect("a structured refusal")
        .clone()
}

fn record(pairs: &[(&str, Value)]) -> Value {
    let mut m = BTreeMap::new();
    for (k, v) in pairs {
        m.insert((*k).to_string(), v.clone());
    }
    Value::Record(m)
}

fn s(text: &str) -> Value {
    Value::Str(text.to_string())
}

fn text(v: Value) -> String {
    match v {
        Value::Str(t) => t,
        other => panic!("expected a String, got {other:?}"),
    }
}

#[test]
fn file_insert_at_line_one_keeps_a_bom_first() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let ses = Session::new("bom_insert");
    let path = ses.path("bom.txt");
    std::fs::write(&path, b"\xEF\xBB\xBFx=1\r\ny=2\r\n").unwrap();
    call("file_insert", vec![s(&path), Value::Int(1), s("ins")]).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"\xEF\xBB\xBFins\r\nx=1\r\ny=2\r\n"
    );
    std::fs::write(&path, b"\xEF\xBB\xBFx=1\r\ny=2\r\n").unwrap();
    call("file_insert", vec![s(&path), Value::Int(2), s("ins")]).unwrap();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"\xEF\xBB\xBFx=1\r\nins\r\ny=2\r\n"
    );
}

#[test]
fn file_edit_keeps_a_bom_and_writes_crlf_only_when_every_break_is_crlf() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let ses = Session::new("bom_edit");
    let path = ses.path("f.txt");
    let replace = |line: i64, content: &str| {
        record(&[
            ("op", s("replace")),
            ("line", Value::Int(line)),
            ("content", s(content)),
        ])
    };
    std::fs::write(&path, b"\xEF\xBB\xBFa=1\r\nb=2\r\n").unwrap();
    call("file_edit", vec![s(&path), replace(2, "c=3")]).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"\xEF\xBB\xBFa=1\r\nc=3\r\n");
    // A BOM over an LF file: the BOM stays, and the lines stay LF.
    std::fs::write(&path, b"\xEF\xBB\xBFa\nb\n").unwrap();
    call("file_edit", vec![s(&path), replace(1, "z")]).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"\xEF\xBB\xBFz\nb\n");
    // Mixed line breaks: the file keeps the LF form, as before.
    std::fs::write(&path, "a\r\nb\n").unwrap();
    call("file_edit", vec![s(&path), replace(1, "z")]).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"z\nb\n");
}

#[test]
fn file_edit_refusals_name_the_edit_and_the_key() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let ses = Session::new("edit_errors");
    let path = ses.path("f.txt");
    std::fs::write(&path, "a\nb\n").unwrap();
    let cases = [
        (
            record(&[("op", s("replace")), ("content", s("x"))]),
            "has no \"start_line\" or \"line\"",
        ),
        (
            record(&[("op", s("insert")), ("content", s("x"))]),
            "edit 0 has no \"line\"",
        ),
        (
            record(&[
                ("op", s("replace")),
                ("line", Value::Int(0)),
                ("content", s("x")),
            ]),
            "is 0; line numbers count from 1",
        ),
        (
            record(&[("op", s("x")), ("line", Value::Int(1)), ("content", s("x"))]),
            "unknown op \"x\": use replace, insert or delete",
        ),
        (
            record(&[
                ("op", s("replace")),
                ("line", Value::Int(1)),
                ("content", Value::Int(2)),
            ]),
            "\"content\" must be a String, got Int",
        ),
    ];
    for (edit, want) in cases {
        let se = refusal("file_edit", vec![s(&path), edit]);
        assert_eq!(se.code.as_str(), "E_BAD_ARG", "{}", se.message);
        assert!(
            se.message.contains(want),
            "wanted {want:?} in {}",
            se.message
        );
        assert!(!se.hint.contains("takes String"), "{}", se.hint);
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\nb\n");
}

#[test]
fn to_csv_refuses_a_repeated_column_name() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let rows = Value::Array(vec![record(&[("a", Value::Int(1))])]);
    let se = refusal("to_csv", vec![rows, Value::Array(vec![s("a"), s("a")])]);
    assert_eq!(se.code.as_str(), "E_BAD_ARG");
    assert!(
        se.message.contains("duplicate CSV column 'a'"),
        "{}",
        se.message
    );
}

#[test]
fn to_csv_refuses_a_nan_or_infinite_float_cell_with_its_path() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let nan = Value::Array(vec![record(&[
        ("a", Value::Float(1.5)),
        ("b", Value::Float(f64::NAN)),
    ])]);
    let se = refusal("to_csv", vec![nan]);
    assert_eq!(se.code.as_str(), "E_BAD_VALUE", "{}", se.message);
    assert!(se.message.contains("rows[0].b is NaN"), "{}", se.message);

    let inf = Value::Array(vec![
        record(&[("a", Value::Float(1.0))]),
        record(&[("a", Value::Float(f64::NEG_INFINITY))]),
    ]);
    let se = refusal("to_csv", vec![inf]);
    assert_eq!(se.code.as_str(), "E_BAD_VALUE", "{}", se.message);
    assert!(
        se.message.contains("rows[1].a is negative infinity"),
        "{}",
        se.message
    );
}

#[test]
fn open_names_a_directory_as_a_directory_not_as_missing() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let ses = Session::new("open_dir");
    let dir = ses.path("adir");
    std::fs::create_dir_all(&dir).unwrap();
    let se = refusal("open", vec![s(&dir)]);
    assert_eq!(se.code.as_str(), "E_BAD_ARG", "{}", se.message);
    assert!(
        se.message.contains("is a directory, not a file"),
        "{}",
        se.message
    );
}

#[test]
fn to_yaml_double_quotes_control_characters_and_round_trips() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let original = record(&[
        ("c", s("\u{1}x")),
        ("nul", s("a\0b")),
        ("del", s("x\u{7f}y")),
        ("cr", s("p\rq")),
        ("esc", s("a\u{b}b\u{c}c\u{1b}d\u{7}e\u{8}f")),
        ("seps", s("\u{85}\u{2028}\u{2029}")),
    ]);
    let out = text(call("to_yaml", vec![original.clone()]).unwrap());
    assert!(out.contains(r#"c: "\x01x""#), "{out}");
    for raw in [
        '\u{1}', '\0', '\r', '\u{7f}', '\u{85}', '\u{2028}', '\u{2029}',
    ] {
        assert!(!out.contains(raw), "raw {raw:?} in {out:?}");
    }
    assert_eq!(call("from_yaml", vec![s(&out)]).unwrap(), original, "{out}");
}

#[test]
fn from_yaml_reads_octal_and_hex_integers_and_to_yaml_quotes_them() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let read = call("from_yaml", vec![s("k: 0o17\nh: 0x1F\nn: 1_000\n")]).unwrap();
    assert_eq!(
        read,
        record(&[
            ("k", Value::Int(15)),
            ("h", Value::Int(31)),
            ("n", s("1_000")),
        ])
    );
    let se = refusal("from_yaml", vec![s("k: 0xFFFFFFFFFFFFFFFFF\n")]);
    assert_eq!(se.code.as_str(), "E_BAD_VALUE", "{}", se.message);

    let strings = record(&[("k", s("0o17")), ("h", s("0x1F"))]);
    let out = text(call("to_yaml", vec![strings.clone()]).unwrap());
    assert!(out.contains(r#"k: "0o17""#), "{out}");
    assert!(out.contains(r#"h: "0x1F""#), "{out}");
    assert_eq!(call("from_yaml", vec![s(&out)]).unwrap(), strings);
}

#[test]
fn file_write_names_the_element_that_is_not_a_string() {
    let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let ses = Session::new("write_mixed");
    let path = ses.path("w.txt");
    let se = refusal(
        "file_write",
        vec![s(&path), Value::Array(vec![s("a"), Value::Int(2)])],
    );
    assert_eq!(se.code.as_str(), "E_BAD_ARG", "{}", se.message);
    assert!(
        se.message
            .contains("content: an Array must hold only Strings (lines), element 1 is Int"),
        "{}",
        se.message
    );
    assert!(!std::path::Path::new(&path).exists());
    // The Array is joined with a newline between elements, and none after the last.
    call(
        "file_write",
        vec![s(&path), Value::Array(vec![s("a"), s("b")])],
    )
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"a\nb");
}

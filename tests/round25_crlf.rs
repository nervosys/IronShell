//! Line endings in the file edit builtins (round 25).
//!
//! A CRLF file must stay CRLF: `file_insert` used to rewrite the whole file with
//! LF. A file whose line breaks are not all CRLF, and a file with none, keeps the
//! LF behaviour. Runs in process, against real files in a temp directory;
//! serialised, and the session directory is a temp directory so the repo's real
//! journal is never touched.

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
        let dir = std::env::temp_dir().join(format!("ae_crlf_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::env::set_var("IRONSHELL_SESSION_DIR", dir.join("state"));
        Self { dir }
    }

    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_string_lossy().into_owned()
    }

    /// Seed `name` with `text` exactly as given, line breaks included.
    fn seed(&self, name: &str, text: &str) -> String {
        let p = self.path(name);
        std::fs::write(&p, text.as_bytes()).expect("seed");
        p
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.path(name)).expect("read back")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        std::env::remove_var("IRONSHELL_SESSION_DIR");
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn call(name: &str, args: Vec<Value>) -> Result<Value, String> {
    let mut env = iron_shell::env::Env::new();
    iron_shell::builtins::call(name, args, &mut env).map_err(|e| e.to_string())
}

fn insert_at_line(s: &Session, file: &str, line: i64, content: &str) {
    call(
        "file_insert",
        vec![
            Value::Str(s.path(file)),
            Value::Int(line),
            Value::Str(content.into()),
        ],
    )
    .expect("file_insert");
}

fn insert_after(s: &Session, file: &str, anchor: &str, content: &str) {
    let mut position = BTreeMap::new();
    position.insert("after".to_string(), Value::Str(anchor.into()));
    call(
        "file_insert",
        vec![
            Value::Str(s.path(file)),
            Value::Record(position),
            Value::Str(content.into()),
        ],
    )
    .expect("file_insert");
}

#[test]
fn insert_at_line_one_of_a_crlf_file_writes_crlf() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("ins_line1");
    s.seed("i.txt", "a\r\nb\r\n");

    insert_at_line(&s, "i.txt", 1, "z");

    assert_eq!(s.read("i.txt"), "z\r\na\r\nb\r\n");
}

#[test]
fn insert_after_an_anchor_in_a_crlf_file_writes_crlf() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("ins_after");
    s.seed("i.txt", "a\r\nb\r\n");

    insert_after(&s, "i.txt", "a", "c");

    assert_eq!(s.read("i.txt"), "a\r\nc\r\nb\r\n");
}

#[test]
fn insert_in_the_middle_of_a_crlf_file_without_a_final_line_break_adds_none() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("ins_mid_nofinal");
    s.seed("e.txt", "x\r\ny");

    insert_at_line(&s, "e.txt", 2, "z");

    assert_eq!(s.read("e.txt"), "x\r\nz\r\ny");
}

#[test]
fn append_after_the_unterminated_last_line_of_a_crlf_file_breaks_the_line_first() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("ins_append_nofinal");
    s.seed("e.txt", "x\r\ny");

    insert_at_line(&s, "e.txt", 3, "z");

    assert_eq!(s.read("e.txt"), "x\r\ny\r\nz\r\n");
}

#[test]
fn append_to_a_crlf_file_with_a_final_line_break_writes_crlf() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("ins_append");
    s.seed("i.txt", "a\r\nb\r\n");

    insert_at_line(&s, "i.txt", 3, "z");

    assert_eq!(s.read("i.txt"), "a\r\nb\r\nz\r\n");
}

#[test]
fn insert_into_an_lf_file_is_unchanged_in_behaviour() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("ins_lf");
    s.seed("a.txt", "a\nb\n");
    s.seed("b.txt", "x\ny");
    s.seed("c.txt", "x\ny");

    insert_after(&s, "a.txt", "a", "c");
    insert_at_line(&s, "b.txt", 2, "z");
    insert_at_line(&s, "c.txt", 3, "z");

    assert_eq!(s.read("a.txt"), "a\nc\nb\n");
    assert_eq!(s.read("b.txt"), "x\nz\ny");
    assert_eq!(s.read("c.txt"), "x\ny\nz\n");
}

#[test]
fn insert_into_a_mixed_file_keeps_its_bytes_and_writes_lf() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("ins_mixed");
    s.seed("m.txt", "a\r\nb\n");

    insert_at_line(&s, "m.txt", 1, "z");

    assert_eq!(s.read("m.txt"), "z\na\r\nb\n");
}

// ---- file_replace and file_patch write CRLF into a CRLF file ----

fn replace(s: &Session, file: &str, old: &str, new: &str) -> Value {
    call(
        "file_replace",
        vec![
            Value::Str(s.path(file)),
            Value::Str(old.into()),
            Value::Str(new.into()),
        ],
    )
    .expect("file_replace")
}

fn patch(s: &Session, file: &str, find: &str, replacement: &str) -> Value {
    let mut m = BTreeMap::new();
    m.insert("find".to_string(), Value::Str(find.into()));
    m.insert("replace".to_string(), Value::Str(replacement.into()));
    call(
        "file_patch",
        vec![
            Value::Str(s.path(file)),
            Value::Array(vec![Value::Record(m)]),
        ],
    )
    .expect("file_patch")
}

fn field(v: &Value, key: &str) -> Value {
    match v {
        Value::Record(m) => m.get(key).cloned().unwrap_or(Value::Null),
        other => panic!("expected a record, got {other:?}"),
    }
}

#[test]
fn replace_with_a_multi_line_text_in_a_crlf_file_writes_crlf() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("rep_multi");
    s.seed("c.txt", "one\r\ntwo\r\n");

    let r = replace(&s, "c.txt", "two", "x\ny");

    assert_eq!(s.read("c.txt"), "one\r\nx\r\ny\r\n");
    assert_eq!(field(&r, "success"), Value::Bool(true));
}

#[test]
fn patch_with_a_multi_line_text_in_a_crlf_file_writes_crlf() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("pat_multi");
    s.seed("p.txt", "one\r\ntwo\r\n");

    let r = patch(&s, "p.txt", "two", "2\nzwei");

    assert_eq!(s.read("p.txt"), "one\r\n2\r\nzwei\r\n");
    assert_eq!(field(&r, "success"), Value::Bool(true));
}

#[test]
fn replace_and_patch_in_an_lf_file_write_lf() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("lf_repl");
    s.seed("r.txt", "one\ntwo\n");
    s.seed("p.txt", "one\ntwo\n");

    replace(&s, "r.txt", "two", "x\ny");
    patch(&s, "p.txt", "two", "2\nzwei");

    assert_eq!(s.read("r.txt"), "one\nx\ny\n");
    assert_eq!(s.read("p.txt"), "one\n2\nzwei\n");
}

#[test]
fn replace_in_a_mixed_file_writes_the_text_as_given() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("rep_mixed");
    s.seed("m.txt", "one\r\ntwo\n");

    replace(&s, "m.txt", "two", "x\ny");

    assert_eq!(s.read("m.txt"), "one\r\nx\ny\n");
}

// ---- a multi-line find matches across CRLF ----

#[test]
fn replace_with_a_multi_line_find_matches_a_crlf_file() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("find_multi");
    s.seed("c.txt", "one\r\ntwo\r\n");

    let r = replace(&s, "c.txt", "one\ntwo", "X");

    assert_eq!(s.read("c.txt"), "X\r\n");
    assert_eq!(field(&r, "success"), Value::Bool(true));
    assert_eq!(field(&r, "patches_applied"), Value::Int(1));
}

#[test]
fn patch_with_a_multi_line_find_matches_a_crlf_file() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("find_pat");
    s.seed("p.txt", "one\r\ntwo\r\nthree\r\n");

    let r = patch(&s, "p.txt", "one\ntwo", "X");

    assert_eq!(s.read("p.txt"), "X\r\nthree\r\n");
    assert_eq!(field(&r, "success"), Value::Bool(true));
}

#[test]
fn a_multi_line_find_that_is_not_in_a_crlf_file_still_writes_nothing() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("find_miss");
    s.seed("c.txt", "one\r\ntwo\r\n");

    let err = call(
        "file_replace",
        vec![
            Value::Str(s.path("c.txt")),
            Value::Str("two\nthree".into()),
            Value::Str("X".into()),
        ],
    )
    .expect_err("the find is not in the file");

    assert!(err.contains("pattern not found"), "{err}");
    assert_eq!(s.read("c.txt"), "one\r\ntwo\r\n");
}

#[test]
fn a_multi_line_find_in_an_lf_file_is_unchanged() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("find_lf");
    s.seed("l.txt", "one\ntwo\n");

    replace(&s, "l.txt", "one\ntwo", "X");

    assert_eq!(s.read("l.txt"), "X\n");
}

#[test]
fn a_replacement_that_already_has_crlf_is_not_doubled() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let s = Session::new("rep_crlf_given");
    s.seed("c.txt", "one\r\ntwo\r\n");

    replace(&s, "c.txt", "two", "x\r\ny");

    assert_eq!(s.read("c.txt"), "one\r\nx\r\ny\r\n");
}

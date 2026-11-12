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

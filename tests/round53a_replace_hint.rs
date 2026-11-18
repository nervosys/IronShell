//! Round 53a: file_replace explains a miss that is only a line-ending mismatch.

use iron_shell::env::Env;
use iron_shell::value::Value;

fn hint_of_miss(contents: &[u8], old_text: &str) -> String {
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("r53a_replace_{}", contents.len()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("IRONSHELL_SESSION_DIR", dir.join("session"));
    let file = dir.join("f.txt");
    std::fs::write(&file, contents).unwrap();
    let mut env = Env::new();
    let result = iron_shell::builtins::call(
        "file_replace",
        vec![
            Value::Str(file.to_string_lossy().into_owned()),
            Value::Str(old_text.to_string()),
            Value::Str("x".to_string()),
        ],
        &mut env,
    );
    match result {
        Ok(_) => panic!("the replacement unexpectedly succeeded"),
        Err(e) => match e.downcast_ref::<iron_shell::safety::SafetyError>() {
            Some(se) => se.hint.clone(),
            None => panic!("not a coded error"),
        },
    }
}

#[test]
fn a_mixed_ending_file_says_so_when_only_the_endings_differ() {
    let hint = hint_of_miss(b"a\r\nb\nc\r\n", "a\nb");
    assert!(hint.contains("mixes CRLF and LF"), "{hint}");
}

#[test]
fn a_plain_miss_has_no_line_ending_note() {
    let hint = hint_of_miss(b"a\r\nb\nc\r\n", "zzz\nyyy");
    assert!(!hint.contains("CRLF"), "{hint}");
}

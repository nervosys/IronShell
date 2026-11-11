//! Round 22: `tee` of a non-String value writes plain text to the file, not
//! the coloured terminal rendering, and a short String preview in `vars`-style
//! records is plain too.

use std::process::Command;

#[test]
fn tee_of_a_record_writes_no_escape_codes() {
    let dir = std::env::temp_dir().join(format!("ish_r22p_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", r#"{a: 1, b: "x"} | tee("out.txt")"#])
        .current_dir(&dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("CLICOLOR_FORCE", "1")
        .env("FORCE_COLOR", "1")
        .env_remove("NO_COLOR")
        .output()
        .expect("run ish");
    let written = std::fs::read_to_string(dir.join("out.txt"))
        .unwrap_or_else(|e| panic!("tee wrote no file ({e}): {out:?}"));
    assert!(!written.contains('\u{1b}'), "{written:?}");
    assert!(
        written.contains('a') && written.contains('1'),
        "{written:?}"
    );
}

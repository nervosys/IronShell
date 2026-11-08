//! `sort -k N[,M]`, `-t C` and the per-key `n`/`r` options run in-process, and
//! print what GNU coreutils `sort` prints (LANG=C).
//!
//! Before this, `-k` and `-t` were not modelled: the subset declined, and on
//! Windows the program that ran was `System32\sort.exe`, which printed
//! "Input file specified two times." and nothing else. The expected outputs
//! below were recorded from GNU sort on these exact fixtures; they are literals
//! so the test needs no GNU tools.

use std::path::PathBuf;
use std::process::Command;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let root =
            std::env::temp_dir().join(format!("ae-posix-sortkeys-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("session")).unwrap();
        std::fs::create_dir_all(root.join("empty-path")).unwrap();
        Dir(root)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PEOPLE: &str =
    "alice,30,london\nbob,25,paris\ncarol,35,berlin\ndave,25,rome\neve,40,oslo\nfrank,25,athens\n";
const BLANKS: &str = "  b  2\na   1\n c 3\n";
const COLON: &str = "a:30:x\nb:4:y\nc:100:z\n";
const WORDS: &str = "pear\napple\n\nfig\napple\n";

/// Run `script` under `ish -b -c` in a folder holding the fixtures, and return
/// stdout and the exit status. Stderr must be empty.
///
/// PATH is an empty folder and LC_ALL is C. With no GNU program on PATH, a
/// declined `sort` cannot be answered by a fallback: it fails with stderr, so
/// these cases prove the in-process sort ran. LC_ALL=C is what the expected
/// outputs were recorded under: a UTF-8 locale such as en_US collates the
/// blanks away (`a   1` sorts first), and that is the locale's order, not a bug.
fn sort_run(tag: &str, script: &str) -> (String, i32) {
    let dir = Dir::new(tag);
    std::fs::write(dir.0.join("people.csv"), PEOPLE).unwrap();
    std::fs::write(dir.0.join("blanks.txt"), BLANKS).unwrap();
    std::fs::write(dir.0.join("colon.txt"), COLON).unwrap();
    std::fs::write(dir.0.join("words.txt"), WORDS).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", script])
        .current_dir(&dir.0)
        .env("NO_COLOR", "1")
        .env("LC_ALL", "C")
        .env("PATH", dir.0.join("empty-path"))
        .env("IRONSHELL_SESSION_DIR", dir.0.join("session"))
        .output()
        .expect("run ish");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.is_empty(), "{script}: stderr was {stderr:?}");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn key_range_with_comma_matches_gnu() {
    // GNU: sort -t, -k2,2n people.csv
    assert_eq!(
        sort_run("a", "sort -t, -k2,2n people.csv"),
        (
            "bob,25,paris\ndave,25,rome\nfrank,25,athens\nalice,30,london\ncarol,35,berlin\neve,40,oslo\n"
                .into(),
            0
        )
    );
    // GNU: sort -t, -k2,3 people.csv (the key runs to the end of field 3)
    assert_eq!(
        sort_run("b", "sort -k2,3 -t, people.csv"),
        (
            "frank,25,athens\nbob,25,paris\ndave,25,rome\nalice,30,london\ncarol,35,berlin\neve,40,oslo\n"
                .into(),
            0
        )
    );
}

#[test]
fn per_key_numeric_and_reverse_options_match_gnu() {
    // GNU: sort -t, -k2,2nr people.csv
    assert_eq!(
        sort_run("c", "sort -t, -k2,2nr people.csv"),
        (
            "eve,40,oslo\ncarol,35,berlin\nalice,30,london\nbob,25,paris\ndave,25,rome\nfrank,25,athens\n"
                .into(),
            0
        )
    );
    // GNU: sort -t: -k2,2rn colon.txt
    assert_eq!(
        sort_run("d", "sort -t: -k2,2rn colon.txt"),
        ("c:100:z\na:30:x\nb:4:y\n".into(), 0)
    );
    // GNU: sort -r -t, -k2,2 people.csv (global -r reverses the whole order)
    assert_eq!(
        sort_run("e", "sort -r -t, -k2,2 people.csv"),
        (
            "eve,40,oslo\ncarol,35,berlin\nalice,30,london\nfrank,25,athens\ndave,25,rome\nbob,25,paris\n"
                .into(),
            0
        )
    );
}

#[test]
fn separator_flag_forms_match_gnu() {
    // GNU: sort -rn -t: -k2 colon.txt
    assert_eq!(
        sort_run("f", "sort -rn -t: -k2 colon.txt"),
        ("c:100:z\na:30:x\nb:4:y\n".into(), 0)
    );
    // GNU: sort -k 2 -t: colon.txt (no -n: lexical on the key)
    assert_eq!(
        sort_run("g", "sort -k 2 -t: colon.txt"),
        ("c:100:z\na:30:x\nb:4:y\n".into(), 0)
    );
    // GNU: sort -k3,3 -t: -n colon.txt
    assert_eq!(
        sort_run("h", "sort -k3,3 -t: -n colon.txt"),
        ("a:30:x\nb:4:y\nc:100:z\n".into(), 0)
    );
}

#[test]
fn default_field_separation_matches_gnu() {
    // Without -t a field is blanks plus a run of non-blanks, so the leading
    // blanks belong to the field: "  b" sorts before " c" and "a".
    // GNU: sort -k1,1 blanks.txt
    assert_eq!(
        sort_run("i", "sort -k1,1 blanks.txt"),
        ("  b  2\n c 3\na   1\n".into(), 0)
    );
    // GNU: sort -k2,2n blanks.txt
    assert_eq!(
        sort_run("j", "sort -k2,2n blanks.txt"),
        ("a   1\n  b  2\n c 3\n".into(), 0)
    );
}

#[test]
fn unique_and_stable_keys_match_gnu() {
    // GNU: sort -t, -k2,2 -u people.csv keeps the first of each equal run in
    // input order.
    assert_eq!(
        sort_run("k", "sort -t, -k2,2 -u people.csv"),
        (
            "bob,25,paris\nalice,30,london\ncarol,35,berlin\neve,40,oslo\n".into(),
            0
        )
    );
    // GNU: sort -s -k2,2 -t, people.csv
    assert_eq!(
        sort_run("l", "sort -s -k2,2 -t, people.csv"),
        (
            "bob,25,paris\ndave,25,rome\nfrank,25,athens\nalice,30,london\ncarol,35,berlin\neve,40,oslo\n"
                .into(),
            0
        )
    );
    // GNU: sort -u words.txt (no keys: whole lines, equal lines dropped)
    assert_eq!(
        sort_run("m", "sort -u words.txt"),
        ("\napple\nfig\npear\n".into(), 0)
    );
}

#[test]
fn a_key_with_no_such_field_keeps_input_order_as_gnu_does() {
    // Field 2 does not exist in these lines, so every key is empty, and with
    // -k there is no whole-line tie-break: the input order is kept.
    // GNU: sort -k2 people.csv
    assert_eq!(sort_run("n", "sort -k2 people.csv"), (PEOPLE.into(), 0));
}

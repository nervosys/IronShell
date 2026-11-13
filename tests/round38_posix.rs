//! Round 38: the POSIX subset (`ish -b`) against bash 5.2 and GNU coreutils:
//! IFS field splitting, `tr` and `cut` usage errors, `wc` column width.
//!
//! Each case runs the binary with `-b -c` in a fresh temp folder, with
//! IRONSHELL_SESSION_DIR set to a folder of its own and an empty PATH.

use std::path::PathBuf;
use std::process::Command;

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("ae-round38-posix-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("session")).unwrap();
        std::fs::create_dir_all(root.join("empty-path")).unwrap();
        Sandbox(root)
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-b", "-c", script])
            .current_dir(&self.0)
            .env("NO_COLOR", "1")
            .env("LC_ALL", "C")
            .env("PATH", self.0.join("empty-path"))
            .env("IRONSHELL_SESSION_DIR", self.0.join("session"))
            .output()
            .expect("run ish");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn ifs_comma_splits_an_unquoted_expansion() {
    let sb = Sandbox::new("ifs1");
    let (out, err, code) = sb.run("IFS=,\ny=\"p,q,r\"\nfor i in $y; do echo \"<$i>\"; done");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "<p>\n<q>\n<r>\n");
}

#[test]
fn ifs_non_white_delimiters_keep_empty_fields() {
    let sb = Sandbox::new("ifs2");
    let (out, err, code) = sb.run("IFS=,\ny=\"a,,b\"\nfor i in $y; do echo \"<$i>\"; done");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "<a>\n<>\n<b>\n");
}

#[test]
fn ifs_white_space_collapses_beside_a_delimiter() {
    let sb = Sandbox::new("ifs3");
    let (out, err, code) =
        sb.run("IFS=' ,'\ny=\" a , b  c \"\nfor i in $y; do echo \"<$i>\"; done");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "<a>\n<b>\n<c>\n");
}

#[test]
fn empty_ifs_splits_nothing_and_quotes_never_split() {
    let sb = Sandbox::new("ifs4");
    let (out, err, code) =
        sb.run("y=\"a b\"\nIFS=\nfor i in $y; do echo \"<$i>\"; done\nIFS=,\nfor i in \"p,q\"; do echo \"<$i>\"; done");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "<a b>\n<p,q>\n");
}

#[test]
fn default_ifs_still_splits_on_blanks() {
    let sb = Sandbox::new("ifs5");
    let (out, err, code) = sb.run("y=\"  a   b \"\nfor i in $y; do echo \"<$i>\"; done");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "<a>\n<b>\n");
}

#[test]
fn tr_without_operands_is_a_missing_operand() {
    let sb = Sandbox::new("tr0");
    let (_, err, code) = sb.run("echo hello | tr");
    assert_eq!(code, 1, "{err}");
    assert_eq!(
        err,
        "tr: missing operand\nTry 'tr --help' for more information.\n"
    );
}

#[test]
fn tr_with_one_operand_asks_for_two_strings() {
    let sb = Sandbox::new("tr1");
    let (_, err, code) = sb.run("echo hello | tr a");
    assert_eq!(code, 1, "{err}");
    assert_eq!(
        err,
        "tr: missing operand after 'a'\nTwo strings must be given when translating.\nTry 'tr --help' for more information.\n"
    );
}

#[test]
fn wc_pads_to_seven_when_an_operand_is_not_a_regular_file() {
    let sb = Sandbox::new("wc");
    let (out, err, code) = sb.run("echo data > f\nwc /dev/null f");
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "      0       0       0 /dev/null\n      1       1       5 f\n      1       1       5 total\n"
    );
    let (out, err, code) = sb.run("echo data | wc");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "      1       1       5\n");
}

#[test]
fn wc_pads_to_the_widest_count_for_regular_files() {
    let sb = Sandbox::new("wc2");
    let (out, err, code) = sb.run("echo data > f\necho data > g\nwc f\nwc f g");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "1 1 5 f\n 1  1  5 f\n 1  1  5 g\n 2  2 10 total\n");
}

#[test]
fn usage_errors_carry_the_try_line() {
    let sb = Sandbox::new("try");
    for (script, first, tool) in [
        (
            "echo a | cut",
            "cut: you must specify a list of bytes, characters, or fields",
            "cut",
        ),
        ("mkdir", "mkdir: missing operand", "mkdir"),
        ("touch", "touch: missing file operand", "touch"),
        ("grep", "Usage: grep [OPTION]... PATTERNS [FILE]...", "grep"),
    ] {
        let (_, err, _) = sb.run(script);
        assert_eq!(
            err,
            format!("{first}\nTry '{tool} --help' for more information.\n"),
            "{script}"
        );
    }
}

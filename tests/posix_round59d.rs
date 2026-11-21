//! Round 59d: GNU compatibility of the in-process `sed`, `printf`, `sort`,
//! `cut`, `grep` and `test` / `[` of the POSIX subset (`ish -b`).
//!
//! Every expected output and status below was produced by GNU sed 4.9, sort,
//! cut and grep 3.11 (coreutils 9.x) and bash 5.2 on Debian, with the same
//! input. Known differences left alone: `printf %q`, and `cut -c` counting
//! characters where GNU counts bytes.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_r59d_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `ish -b -c <script>` in `dir`. Returns (stdout, stderr, exit code).
fn ish_bash(script: &str, dir: &Path) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", script])
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("state"))
        .env("IRONSHELL_WORKSPACE", dir)
        .env_remove("IRONSHELL_MODE")
        .output()
        .expect("run ish");
    (
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n"),
        out.status.code().unwrap_or(-1),
    )
}

/// Each case is (script, stdout, stderr, status), all exact.
fn check(dir: &Path, cases: &[(&str, &str, &str, i32)]) {
    for (cmd, want_out, want_err, want_code) in cases {
        let (out, err, code) = ish_bash(cmd, dir);
        assert_eq!(
            (out.as_str(), err.as_str(), code),
            (*want_out, *want_err, *want_code),
            "{cmd}"
        );
    }
}

#[test]
fn sed_blocks_keep_quiet_and_run_their_commands() {
    let dir = scratch("sedblock");
    check(
        &dir,
        &[
            (r"printf 'a\nb\nc\n' | sed -n '2{p;p}'", "b\nb\n", "", 0),
            (r"printf 'a\nb\nc\n' | sed -n '/a/{p;p;}'", "a\na\n", "", 0),
            (
                r"printf 'a\nb\n' | sed -n -e '/a/{' -e p -e '}'",
                "a\n",
                "",
                0,
            ),
            (
                r"printf 'a\nb\nc\nd\n' | sed -n '2,3{p;s/b/X/p}'",
                "b\nX\nc\n",
                "",
                0,
            ),
            (
                r"printf 'a\nb\nc\nd\n' | sed '2,3{/b/d}'",
                "a\nc\nd\n",
                "",
                0,
            ),
            // A `;` inside an s command is not a separator.
            (r"printf 'a;b\n' | sed 's/a;b/c/'", "c\n", "", 0),
            (r"printf 'a\nb\n' | sed 's/a/X/;s/b/Y/'", "X\nY\n", "", 0),
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sed_unbalanced_braces_fail_with_status_1() {
    let dir = scratch("sedbrace");
    for script in [r"sed '2{'", r"sed '2}'"] {
        let (out, _, code) = ish_bash(&format!(r"printf 'a\nb\nc\n' | {script}"), &dir);
        assert_eq!((out.as_str(), code), ("", 1), "{script}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sed_append_insert_change_take_text_from_the_next_line() {
    let dir = scratch("sedtext");
    check(
        &dir,
        &[
            (
                "printf 'a\\nb\\nc\\n' | sed '2a\\\nadded'",
                "a\nb\nadded\nc\n",
                "",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '2i\\\nins'",
                "a\nins\nb\nc\n",
                "",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '2c\\\nchg'",
                "a\nchg\nc\n",
                "",
                0,
            ),
            // Over a range, `c` prints its text once, at the end.
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2,3c\\\nchg'",
                "a\nchg\nd\n",
                "",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '2a\\\nl1\\\nl2'",
                "a\nb\nl1\nl2\nc\n",
                "",
                0,
            ),
            // The one-line forms.
            (
                "printf 'a\\nb\\nc\\n' | sed '2a hello'",
                "a\nb\nhello\nc\n",
                "",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '2a\\  indented'",
                "a\nb\n  indented\nc\n",
                "",
                0,
            ),
            // The text is printed under -n as well.
            (
                "printf 'a\\nb\\nc\\n' | sed -n '2a hello'",
                "hello\n",
                "",
                0,
            ),
            // A last line without a newline gets one before the text.
            ("printf 'a\\nb' | sed '$a end'", "a\nb\nend\n", "", 0),
            (
                "printf 'a\\nb\\nc\\n' | sed -e '2a\\' -e 'x y'",
                "a\nb\nx y\nc\n",
                "",
                0,
            ),
            // `q` flushes the text queued by earlier commands of the cycle.
            ("printf 'a\\nb\\nc\\n' | sed '2q;a x'", "a\nx\nb\n", "", 0),
            (
                "printf 'a\\nb\\nc\\n' | sed '2{a x\nd}'",
                "a\nx\nc\n",
                "",
                0,
            ),
            // A newline in the replacement of s.
            (
                "printf 'a x\\nb\\n' | sed 's/x/a\\\nb/'",
                "a a\nb\nb\n",
                "",
                0,
            ),
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn printf_star_takes_width_and_precision_from_the_arguments() {
    let dir = scratch("pstar");
    check(
        &dir,
        &[
            (r"printf '%*d|\n' 5 3", "    3|\n", "", 0),
            (r"printf '%-*d|\n' 5 3", "3    |\n", "", 0),
            // A negative width is `-` and its absolute value.
            (r"printf '%*d|\n' -5 3", "3    |\n", "", 0),
            (r"printf '%.*f|\n' 2 3.14159", "3.14|\n", "", 0),
            (r"printf '%*.*f|\n' 8 2 3.14159", "    3.14|\n", "", 0),
            // A negative precision is none: %f keeps its six decimals.
            (r"printf '%.*f|\n' -1 3.14159", "3.141590|\n", "", 0),
            (r"printf '%*s|\n' 6 ab", "    ab|\n", "", 0),
            // A missing value is 0.
            (r"printf '%*d|\n' 5", "    0|\n", "", 0),
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn printf_decodes_octal_escapes_in_the_format() {
    let dir = scratch("poct");
    check(
        &dir,
        &[
            (r"printf 'a\101b\n'", "aAb\n", "", 0),
            // `\0101` is the three digits 010 (a backspace), then a `1`.
            (r"printf '\0101|\n'", "\u{8}1|\n", "", 0),
            (r"printf '\60\061\n'", "01\n", "", 0),
            // 8 is no octal digit: the backslash stays.
            (r"printf '\8\n'", "\\8\n", "", 0),
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sort_global_options_skip_keys_that_have_their_own() {
    let dir = scratch("sortkey");
    let abc = r"printf 'a 2\nb 1\nc 3\n' | ";
    let ten = r"printf 'a 10\nb 9\nc 3\n' | ";
    let tie = r"printf 'b 1\na 1\nc 0\n' | ";
    let cases: Vec<(String, &str)> = vec![
        (format!("{abc}sort -k2,2n -r"), "b 1\na 2\nc 3\n"),
        (format!("{abc}sort -r -k2n"), "b 1\na 2\nc 3\n"),
        (format!("{abc}sort -r -k2"), "c 3\na 2\nb 1\n"),
        (format!("{abc}sort -n -k2r"), "c 3\na 2\nb 1\n"),
        (format!("{abc}sort -r -k2,2r"), "c 3\na 2\nb 1\n"),
        (format!("{ten}sort -n -k2"), "c 3\nb 9\na 10\n"),
        // `-k2r` has an option of its own, so the global -n is not applied.
        (format!("{ten}sort -n -k2r"), "b 9\nc 3\na 10\n"),
        (format!("{ten}sort -n -k2,2"), "c 3\nb 9\na 10\n"),
        (format!("{tie}sort -r -k2n"), "c 0\nb 1\na 1\n"),
        (format!("{tie}sort -k2n -k1r"), "c 0\nb 1\na 1\n"),
        // The second key has none, so it takes -r: b before a.
        (format!("{tie}sort -r -k2n -k1"), "c 0\nb 1\na 1\n"),
    ];
    for (cmd, want) in &cases {
        let (out, err, code) = ish_bash(cmd, &dir);
        assert_eq!((out.as_str(), code), (*want, 0), "{cmd}: {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn grep_on_a_directory_exits_2() {
    let dir = scratch("grepdir");
    std::fs::create_dir_all(dir.join("d")).unwrap();
    std::fs::write(dir.join("d").join("f"), "x\n").unwrap();
    std::fs::write(dir.join("f"), "pat\n").unwrap();
    let msg = "grep: d: Is a directory\n";
    check(
        &dir,
        &[
            ("grep pat d", "", msg, 2),
            ("grep -s pat d", "", "", 2),
            ("grep pat d f", "f:pat\n", msg, 2),
            ("grep pat f d", "f:pat\n", msg, 2),
            // -q stops at the match in f and exits 0, but d was named first.
            ("grep -q pat d f", "", msg, 0),
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cut_refuses_a_decreasing_range() {
    let dir = scratch("cutdec");
    let msg = "cut: invalid decreasing range\nTry 'cut --help' for more information.\n";
    check(
        &dir,
        &[
            ("echo a,b,c | cut -d, -f2-1", "", msg, 1),
            ("echo a,b,c | cut -d, -f3-1,1", "", msg, 1),
            ("echo abc | cut -c3-2", "", msg, 1),
            ("echo abc | cut -b2-1", "", msg, 1),
            ("echo a,b,c | cut -d, -f1-2", "a,b\n", "", 0),
            ("echo a,b,c | cut -d, -f-2", "a,b\n", "", 0),
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_reads_parentheses_and_the_extra_unary_operators() {
    let dir = scratch("testops");
    std::fs::write(dir.join("f"), "x\n").unwrap();
    check(
        &dir,
        &[
            (r"[ \( 1 -eq 1 \) ]", "", "", 0),
            (r"[ \( 1 -eq 2 \) ]", "", "", 1),
            (r"[ \( -n a \) -a \( -n b \) ]", "", "", 0),
            (r"[ \( -n a \) -a \( -z b \) ]", "", "", 1),
            (r"[ \( -z a \) -o \( -n b \) ]", "", "", 0),
            (r"[ ! \( -z a \) ]", "", "", 0),
            (r"[ \( \( 1 -eq 1 \) \) ]", "", "", 0),
            (r"[ \( a \) ]", "", "", 0),
            // One word is a string test, even when it is a parenthesis.
            (r"[ \( ]", "", "", 0),
            ("[ '(' = ')' ]", "", "", 1),
            // Three words with an operator in the middle are a comparison.
            ("[ ! = ! ]", "", "", 0),
            ("[ ! -n ]", "", "", 1),
            // Standard input is not a terminal here.
            ("[ -t 0 ]", "", "", 1),
            ("[ -t 0 ] < /dev/null", "", "", 1),
            ("[ -t abc ]", "", "", 1),
            ("[ -t ]", "", "", 0),
            // None of these is true of a plain file (or of a missing one).
            ("[ -b f ]", "", "", 1),
            ("[ -p f ]", "", "", 1),
            ("[ -S f ]", "", "", 1),
            ("[ -u f ]", "", "", 1),
            ("[ -g f ]", "", "", 1),
            ("[ -k f ]", "", "", 1),
            ("[ -N f ]", "", "", 1),
            ("[ -b nofile ]", "", "", 1),
            ("[ -N ]", "", "", 0),
        ],
    );
    let _ = std::fs::remove_dir_all(&dir);
}

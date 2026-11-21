//! Round 61c: differences between the in-process POSIX engine (`ish -b`) and
//! bash 5.2 and GNU sed 4.9 found in round 61: sed `0,/re/`, the empty regex,
//! the `\0` `\xHH` `\oNNN` `\dNNN` `\cX` escapes, the text of `a`, `i` and `c`,
//! the newline of a last line that has none, and scripts GNU sed rejects;
//! `printf -v`, `--`, bad options and the `%b` escapes `\NNN` `\uHHHH`
//! `\UHHHHHHHH`; `read -n`, `-d` and the options it refuses; `[ -a FILE ]`;
//! names `export` and `read` refuse; `$LINENO`, `$IFS` and `$OPTIND`;
//! `cut -d` of more than one byte; the statuses of `sed` on a missing input;
//! the status of a failed `${x:?}` or `set -u` in a substitution, a pipeline
//! and under `set -e`; and the constructs refused before anything runs
//! (descriptors above 2, array assignments, `read -t`, `sed -f /dev/stdin`).
//!
//! Every expected stdout and exit status below was produced by GNU bash 5.2 and
//! GNU sed 4.9 / coreutils on Debian with the same script. The bytes are
//! asserted as they are, not through `od`, whose output differs on macOS.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_r61c_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `ish -b -c <script>` in `dir`. Returns (raw stdout, stderr, exit code).
fn ish_raw(script: &str, dir: &Path) -> (Vec<u8>, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", script])
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("state"))
        .env("IRONSHELL_WORKSPACE", dir)
        .env_remove("IRONSHELL_MODE")
        .env_remove("OLDPWD")
        .env_remove("IFS")
        .output()
        .expect("run ish");
    (
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// Each case runs in a fresh directory, so files one case makes do not reach
/// the next. The output is compared byte for byte.
fn check(tag: &str, cases: &[(&str, &[u8], i32)]) {
    for (n, (script, want, want_code)) in cases.iter().enumerate() {
        let dir = scratch(&format!("{tag}{n}"));
        let (out, err, code) = ish_raw(script, &dir);
        assert_eq!(
            (out.as_slice(), code),
            (*want, *want_code),
            "{script}: stdout {:?}, stderr {err}",
            String::from_utf8_lossy(&out)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn sed_text_of_a_i_c_reads_the_gnu_escapes() {
    // GNU sed 4.9: `a`, `i` and `c` text takes \n, \t, \a, \f, \r, \v, \cX, \dNNN, \oNNN and \xHH; any other backslash is dropped.
    check(
        "aic",
        &[
            ("echo X | sed 'a hello\\nworld'", b"X\nhello\nworld\n", 0),
            ("echo X | sed 'a hello\\tworld'", b"X\nhello\tworld\n", 0),
            ("echo X | sed 'a\\\\thello'", b"X\n\thello\n", 0),
            ("echo X | sed 'a\\thello'", b"X\nthello\n", 0),
            ("echo X | sed 'a\\\n\\thello'", b"X\n\thello\n", 0),
            ("echo X | sed 'a hello\\\\nworld'", b"X\nhello\\nworld\n", 0),
            ("echo X | sed 'a h\\qx'", b"X\nhqx\n", 0),
            ("echo X | sed 'a h\\x41y'", b"X\nhAy\n", 0),
            ("echo X | sed 'a h\\cAy'", b"X\nh\x01y\n", 0),
            ("echo X | sed 'i hello\\nworld'", b"hello\nworld\nX\n", 0),
            ("echo X | sed 'c hello\\nworld'", b"hello\nworld\n", 0),
            ("echo X | sed 'a hello\\rworld'", b"X\nhello\rworld\n", 0),
            ("echo X | sed 'a  \\  hello'", b"X\n  hello\n", 0),
            ("echo X | sed 'a hello\\'", b"X\nhello\n", 0),
            (
                "echo X | sed -e 'a hello\\' -e 'world'",
                b"X\nhello\nworld\n",
                0,
            ),
            (
                "echo X | sed 'a\\\n  hello\\nworld'",
                b"X\n  hello\nworld\n",
                0,
            ),
            ("echo X | sed 'a hello\\n'", b"X\nhello\n\n", 0),
            ("echo X | sed 'a \\nhello'", b"X\nnhello\n", 0),
        ],
    );
}

#[test]
fn sed_replacement_and_pattern_escapes() {
    // GNU sed 4.9: `\0` is `&`; `\xHH`, `\oNNN`, `\dNNN` and `\cX` name a character in the pattern and in the replacement.
    check(
        "sesc",
        &[
            ("echo abc | sed 's/b/\\0/'", b"abc\n", 0),
            ("echo abc | sed 's/b/[\\0]/'", b"a[b]c\n", 0),
            ("echo abc | sed 's/a/\\x41/'", b"Abc\n", 0),
            ("echo abc | sed 's/\\x61/X/'", b"Xbc\n", 0),
            ("echo abc | sed 's/a/\\o101/'", b"Abc\n", 0),
            ("echo abc | sed 's/a/\\d065/'", b"Abc\n", 0),
            ("echo abc | sed 's/\\o141/X/'", b"Xbc\n", 0),
            ("echo abc | sed 's/\\d097/X/'", b"Xbc\n", 0),
            ("echo a.b | sed 's/\\x2e/X/'", b"X.b\n", 0),
            ("echo a.b | sed 's/a/\\x2e/'", b"..b\n", 0),
            ("echo 'a&b' | sed 's/a/\\x26/'", b"&&b\n", 0),
            ("echo 'a*b' | sed 's/\\x2a/X/'", b"aXb\n", 0),
            ("echo 'a+b' | sed 's/\\x2b/X/'", b"aXb\n", 0),
            ("echo 'a^b' | sed 's/\\x5e/X/'", b"Xa^b\n", 0),
            ("echo 'a/b' | sed 's/\\x2f/X/'", b"aXb\n", 0),
            ("echo abc | sed 's/b/\\x/'", b"axc\n", 0),
            ("echo abc | sed 's/b/\\xZ/'", b"axZc\n", 0),
            ("echo abc | sed 's/b/\\x4/'", b"a\x04c\n", 0),
            ("echo abc | sed 's/b/\\x414/'", b"aA4c\n", 0),
            ("echo abc | sed 's/b/\\o1010/'", b"aA0c\n", 0),
            ("printf 'a\\rb\\n' | sed 's/\\r/X/'", b"aXb\n", 0),
            ("printf 'a\\fb\\n' | sed 's/\\f/X/'", b"aXb\n", 0),
            ("printf 'a\\vb\\n' | sed 's/\\v/X/'", b"aXb\n", 0),
            ("printf 'a\\ab\\n' | sed 's/\\a/X/'", b"aXb\n", 0),
            ("echo abc | sed 'y/a/\\x41/'", b"Abc\n", 0),
        ],
    );
}

#[test]
fn sed_rejects_what_gnu_rejects() {
    // GNU sed 4.9 exits 1 with nothing printed for these scripts.
    check(
        "sebad",
        &[
            ("echo abc | sed 's/b/\\1/'", b"", 1),
            ("echo abc | sed 's/\\(b\\)/\\2/'", b"", 1),
            ("echo abc | sed -E 's/(b)/\\2/'", b"", 1),
            ("echo abc | sed 's/b/\\9/'", b"", 1),
            ("printf 'a\\nb\\n' | sed '2{p}3{p}'", b"", 1),
            ("printf 'a\\nb\\n' | sed '2{p} 3{p}'", b"", 1),
            ("printf 'a\\nb\\n' | sed '2{p}}'", b"", 1),
            ("printf 'a\\nb\\n' | sed '{p}p'", b"", 1),
            ("printf 'a\\nb\\n' | sed 'p;}'", b"", 1),
            ("printf 'a\\nb\\n' | sed '0p'", b"", 1),
            ("printf 'a\\nb\\n' | sed '0,3p'", b"", 1),
        ],
    );
}

#[test]
fn sed_zero_address_starts_a_range_at_the_first_line() {
    // GNU sed 4.9: `0,/re/` ends at the first line that matches, the first one included.
    check(
        "sezero",
        &[
            ("printf 'a\\nb\\n' | sed '0,/a/p'", b"a\na\nb\n", 0),
            ("printf 'a\\nb\\n' | sed -n '0,/a/p'", b"a\n", 0),
            ("printf 'a\\nb\\n' | sed -n '0!p'", b"", 1),
            ("seq 1 5 | sed -n '0,/2/p'", b"1\n2\n", 0),
            ("seq 1 5 | sed '0,/1/d'", b"2\n3\n4\n5\n", 0),
            ("seq 1 5 | sed -n '0,/1/p'", b"1\n", 0),
            ("seq 1 5 | sed -n '0,/9/p'", b"1\n2\n3\n4\n5\n", 0),
            ("seq 1 5 | sed '0,/3/s/^/x/'", b"x1\nx2\nx3\n4\n5\n", 0),
            ("seq 1 5 | sed '0,/[24]/!d'", b"1\n2\n", 0),
            ("seq 1 5 | sed -n '0,/2/{p;p}'", b"1\n1\n2\n2\n", 0),
            ("seq 1 5 | sed -n '0,/2/!p'", b"3\n4\n5\n", 0),
            ("seq 1 5 | sed -s -n '0,/2/p'", b"1\n2\n", 0),
        ],
    );
}

#[test]
fn sed_empty_regex_is_the_last_regex_used() {
    // GNU sed 4.9: `//` and `s//X/` use the last regex used when the program runs; with none the run stops with status 1.
    check(
        "seempty",
        &[
            (
                "printf 'foo\\nbar\\nfoo\\n' | sed '/foo/s//X/'",
                b"X\nbar\nX\n",
                0,
            ),
            ("echo foofoo | sed '/foo/s//X/g'", b"XX\n", 0),
            ("printf 'abc\\n' | sed '/b/s//[&]/p'", b"a[b]c\na[b]c\n", 0),
            ("printf 'a\\nb\\nc\\nb\\n' | sed '/b/,//d'", b"a\n", 0),
            (
                "printf 'a\\nb\\nc\\nb\\n' | sed -n '/b/{//p}'",
                b"b\nb\n",
                0,
            ),
            ("printf 'a\\nb\\nc\\nb\\n' | sed -n '/b/!{//p}'", b"", 0),
            ("printf 'a\\nb\\nc\\nb\\n' | sed '//d'", b"", 1),
            (
                "printf 'a\\nb\\nc\\nb\\n' | sed 's//X/'; echo rc=$?",
                b"rc=1\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nb\\n' | sed -n '1p;//p'; echo rc=$?",
                b"a\nrc=1\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nb\\n' | sed 's/b/X/;s//Y/'",
                b"a\nX\nc\nX\n",
                0,
            ),
            ("printf 'aXbXc\\n' | sed 's/X/-/;s//+/'", b"a-b+c\n", 0),
            (
                "printf 'aXbXc\\n' | sed 's//+/;s/X/-/'; echo rc=$?",
                b"rc=1\n",
                0,
            ),
            (
                "printf 'a1\\n' | sed -E 's/([0-9])/<\\1>/;s//{\\1}/'",
                b"a<{1}>\n",
                0,
            ),
            (
                "printf 'a\\nb\\n' | sed -n '1{//p}'; echo rc=$?",
                b"rc=1\n",
                0,
            ),
            (
                "printf 'a\\nb\\n' | sed -n '$!{//p}'; echo rc=$?",
                b"rc=1\n",
                0,
            ),
            (
                "printf 'one two\\n' | sed 's/one/1/;s//2/;s/two/3/;s//4/'",
                b"1 3\n",
                0,
            ),
        ],
    );
}

#[test]
fn sed_missing_final_newline_through_hold_space_and_q() {
    // GNU sed 4.9: the newline of a pattern space is printed unless it came from a last line that had none; `x`, `g`, `G`, `h` and `H` carry that with the text, and `q` ends a line left without a newline.
    check(
        "senl",
        &[
            ("printf 'a' | sed G", b"a\n\n", 0),
            ("printf 'a' | sed 'x;G'", b"\na", 0),
            ("printf 'a' | sed x", b"\n", 0),
            ("printf 'a\\nb' | sed -n 'H;${x;p}'", b"\na\nb", 0),
            ("printf 'a\\nb' | sed -n 'H;${x;G;p}'", b"\na\nb\nb", 0),
            ("printf 'a\\nb' | sed '2q'", b"a\nb\n", 0),
            ("printf 'a\\nb' | sed -n '2{p;q}'", b"b\n", 0),
            ("printf 'a\\nb' | sed q", b"a\n", 0),
            ("printf 'a\\nb' | sed '$q'", b"a\nb\n", 0),
            ("printf 'a\\nb' | sed 'a X'", b"a\nX\nb\nX\n", 0),
            ("printf 'a\\nb' | sed '2{a X\nq}'", b"a\nb\nX\n", 0),
            ("printf 'a\\nb\\nc' | sed 'N;P;D'", b"a\nb\nc", 0),
            ("printf 'a\\nb' | sed 'G;G'", b"a\n\n\nb\n\n\n", 0),
            ("printf 'a\\nb' | sed 'x'", b"\na\n", 0),
            ("printf 'a\\nb' | sed '$!{h;d};x;G'", b"a\nb", 0),
            ("printf 'a\\nb' | sed 'n;n'", b"a\nb", 0),
            ("printf 'a\\nb' | sed -n 'h;$!d;x;p'", b"b", 0),
            ("printf 'a\\nb' | sed -n '1h;2{G;p}'", b"b\na\n", 0),
            ("printf 'a\\nb' | sed 'y/ab/xy/;G'", b"x\n\ny\n\n", 0),
            ("printf 'a\\nb' | sed -n '1{h;d};2{x;G;p}'", b"a\nb", 0),
        ],
    );
}

#[test]
fn printf_percent_b_escapes_options_and_v() {
    // bash 5.2: `%b` takes \NNN, \0NNN, \uHHHH and \UHHHHHHHH; `printf -v` sets a variable; `--` ends the options; an unknown option is status 2.
    check(
        "pf",
        &[
            (
                "printf '%b|' '\\101' '\\7' '\\1' '\\12' '\\18' '\\777' '\\400' '\\10'; echo",
                b"A|\x07|\x01|\n|\x018|\xff|\x00|\x08|\n",
                0,
            ),
            (
                "printf '%b|' '\\0101' '\\00' '\\08' '\\x41' '\\x4' '\\xZ' 'é' '\\U0001F600' '\\u41' '\\U41' '\\u' '\\U' '\\uZZ'; echo",
                b"A|\x00|\x008|A|\x04|\\xZ|\xc3\xa9|\xf0\x9f\x98\x80|A|A|\\u|\\U|\\uZZ|\n",
                0,
            ),
            (
                "printf '\\101 é \\U0001F600 \\x41\\n'",
                b"A \xc3\xa9 \xf0\x9f\x98\x80 A\n",
                0,
            ),
            (
                "echo -e '\\101|\\0101|é|\\U0001F600|\\x41'",
                b"\\101|A|\xc3\xa9|\xf0\x9f\x98\x80|A\n",
                0,
            ),
            (
                "printf -v v '%s' a; echo \"[$v]\"",
                b"[a]\n",
                0,
            ),
            (
                "printf -- '%s\\n' z",
                b"z\n",
                0,
            ),
            (
                "printf -x; echo rc=$?",
                b"rc=2\n",
                0,
            ),
            (
                "printf -x '%s\\n' a; echo rc=$?",
                b"rc=2\n",
                0,
            ),
            (
                "printf; echo rc=$?",
                b"rc=2\n",
                0,
            ),
            (
                "printf -v; echo rc=$?",
                b"rc=2\n",
                0,
            ),
            (
                "printf -- -x; echo rc=$?",
                b"-xrc=0\n",
                0,
            ),
            (
                "printf -- ; echo rc=$?",
                b"rc=2\n",
                0,
            ),
            (
                "printf '%s\\n' -x; echo rc=$?",
                b"-x\nrc=0\n",
                0,
            ),
            (
                "printf '%b\\n' 'a\\cb'; echo rc=$?",
                b"arc=0\n",
                0,
            ),
            (
                "printf '%b|' 'éx' 'é' '€'; echo",
                b"\xc3\xa9x|\xc3\xa9|\xe2\x82\xac|\n",
                0,
            ),
            (
                "printf '%5b|' '\\101'; echo",
                b"    A|\n",
                0,
            ),
            (
                "printf '%b|' '\\1010' '\\0101x' '\\x414'; echo",
                b"A0|Ax|A4|\n",
                0,
            ),
            (
                "printf -v v '%s-%s' a b; echo \"[$v]\"",
                b"[a-b]\n",
                0,
            ),
            (
                "printf -v 1x '%s' a; echo rc=$?",
                b"rc=2\n",
                0,
            ),
            (
                "printf -v v; echo \"[$v] rc=$?\"",
                b"[] rc=2\n",
                0,
            ),
            (
                "printf -vv '%s' q; echo \"[$v]\"",
                b"[q]\n",
                0,
            ),
            (
                "x=old; printf -v x '%d' 42; echo \"[$x]\"",
                b"[42]\n",
                0,
            ),
            (
                "printf -v v '%b' 'a\\nb'; echo \"[$v]\"",
                b"[a\nb]\n",
                0,
            ),
            (
                "printf -v v '%s\\n' a; echo \"[$v]\"",
                b"[a\n]\n",
                0,
            ),
            (
                "printf -v v '' ; echo \"[$v]\"",
                b"[]\n",
                0,
            ),
            (
                "printf -v 'v w' '%s' a; echo rc=$?",
                b"rc=2\n",
                0,
            ),
        ],
    );
}

#[test]
fn sed_bre_star_after_anchor_and_buffer_anchors() {
    // GNU sed 4.9: a `*` at the start of a BRE, after `\(` or `\|`, or after a leading `^` is literal; `` \` `` and `\'` are the ends of the pattern space.
    check(
        "sebre",
        &[
            ("echo abc | sed 's/^*/_/'", b"abc\n", 0),
            ("echo '*abc' | sed 's/^*/_/'", b"_abc\n", 0),
            ("echo '*abc' | sed 's/*/_/'", b"_abc\n", 0),
            ("echo 'a*bc' | sed 's/\\(*\\)/_/'", b"a_bc\n", 0),
            ("echo 'a*bc' | sed 's/a\\|*/_/'", b"_*bc\n", 0),
            (
                "echo 'abc def' | sed 's/\\`a/X/;s/f\\'\"'\"'/Y/'",
                b"Xbc deY\n",
                0,
            ),
            ("printf 'a\\nb\\n' | sed 'N;s/\\`a/X/'", b"X\nb\n", 0),
            ("printf 'a\\nb\\n' | sed -n 'N;s/^b/X/p;s/\\`b/Y/p'", b"", 0),
            (
                "printf 'a\\nb\\n' | sed 'N;s/a$/X/;s/b\\'\"'\"'/Y/'",
                b"a\nY\n",
                0,
            ),
            (
                "printf 'a\\nb\\n' | sed -E 'N;s/\\`a/X/;s/b\\'\"'\"'/Y/'",
                b"X\nY\n",
                0,
            ),
        ],
    );
}

#[test]
fn test_unary_a_is_exists() {
    // bash 5.2: `[ -a FILE ]` is true when FILE exists, and a doubled `-a` is a test and then the operator.
    check(
        "testa",
        &[
            ("[ -a nonexistent ]; echo \"rc=$?\"", b"rc=1\n", 0),
            ("[ -a . ]; echo \"rc=$?\"", b"rc=0\n", 0),
            ("[ -e . -a -a . ]; echo \"rc=$?\"", b"rc=0\n", 0),
            ("[ -a nonexistent -o -d . ]; echo \"rc=$?\"", b"rc=0\n", 0),
            ("[ -d . -a -a . -a -e . ]; echo \"rc=$?\"", b"rc=0\n", 0),
            ("[ -d . -a -f . ]; echo \"rc=$?\"", b"rc=1\n", 0),
            ("test -a .; echo \"rc=$?\"", b"rc=0\n", 0),
            ("[ -a . -a -a nonexistent ]; echo \"rc=$?\"", b"rc=1\n", 0),
        ],
    );
}

#[test]
fn export_and_read_refuse_names_that_are_no_identifier() {
    // bash 5.2: `export 1x=3`, `export x-y=3`, `export =3` and `read 1x` fail with status 1 and set nothing.
    check(
        "names",
        &[
            ("export =3; echo \"rc=$?\"", b"rc=1\n", 0),
            ("export ''; echo \"rc=$?\"", b"rc=1\n", 0),
            ("export 'a b=3'; echo \"rc=$?\"", b"rc=1\n", 0),
            ("export a.b=3; echo \"rc=$?\"", b"rc=1\n", 0),
            ("export 1x ok=1; echo \"rc=$? [$ok]\"", b"rc=1 [1]\n", 0),
            ("export ok=1 1x; echo \"rc=$? [$ok]\"", b"rc=1 [1]\n", 0),
            (
                "export a=1; export a; echo \"rc=$? [$a]\"",
                b"rc=0 [1]\n",
                0,
            ),
            ("export a+=x; echo \"rc=$? [$a]\"", b"rc=0 [x]\n", 0),
            ("echo hi | read 1x; echo \"rc=$?\"", b"rc=1\n", 0),
            ("echo hi | { read 1x; echo \"rc=$?\"; }", b"rc=1\n", 0),
            ("echo hi | { read a-b; echo \"rc=$?\"; }", b"rc=1\n", 0),
            ("echo hi | { read -r a b-c; echo \"rc=$?\"; }", b"rc=1\n", 0),
        ],
    );
}

#[test]
fn read_takes_n_d_and_p_with_r() {
    // bash 5.2: `read -rn N`, `-rd DELIM`, `-p`, `-s`, `-e` and `-u 0`.
    check(
        "readopt",
        &[
            (
                "printf 'xyz\\n' | { read -rn 1 v; echo \"[$v] rc=$?\"; }",
                b"[x] rc=0\n",
                0,
            ),
            (
                "printf 'xyz\\n' | { read -rn2 v; echo \"[$v] rc=$?\"; }",
                b"[xy] rc=0\n",
                0,
            ),
            (
                "printf 'xyz' | { read -rn3 v; echo \"[$v] rc=$?\"; }",
                b"[xyz] rc=0\n",
                0,
            ),
            (
                "printf 'ab\\ncd\\n' | { read -rn5 v; echo \"[$v] rc=$?\"; read -r w; echo \"[$w]\"; }",
                b"[ab] rc=0\n[cd]\n",
                0,
            ),
            (
                "printf 'é\\n' | { read -rn1 v; echo \"[$v] rc=$?\"; }",
                b"[\xc3\xa9] rc=0\n",
                0,
            ),
            (
                "printf 'a,b;c' | { read -r -d ';' v; echo \"[$v] rc=$?\"; }",
                b"[a,b] rc=0\n",
                0,
            ),
            (
                "printf 'a,b;c' | { read -rd , v; echo \"[$v] rc=$?\"; }",
                b"[a] rc=0\n",
                0,
            ),
            (
                "printf 'a\\nb\\0c' | { read -r -d '' v; echo \"[$v] rc=$?\"; }",
                b"[a\nb] rc=0\n",
                0,
            ),
            (
                "printf 'a\\nb;' | { read -r -d ';' v; echo \"[$v] rc=$?\"; }",
                b"[a\nb] rc=0\n",
                0,
            ),
            (
                "printf 'x' | { read -r -d ';' v; echo \"[$v] rc=$?\"; }",
                b"[x] rc=1\n",
                0,
            ),
            (
                "printf 'a\\\\;b;' | { read -r -d ';' v; echo \"[$v] rc=$?\"; }",
                b"[a\\] rc=0\n",
                0,
            ),
            (
                "printf 'hello\\n' | { read -r -s v; echo \"[$v] rc=$?\"; }",
                b"[hello] rc=0\n",
                0,
            ),
            (
                "printf 'hello\\n' | { read -- v; echo \"[$v] rc=$?\"; }",
                b"[hello] rc=0\n",
                0,
            ),
            (
                "printf '' | { read -rn1 v; echo \"[$v] rc=$?\"; }",
                b"[] rc=1\n",
                0,
            ),
            (
                "printf '\\nabc' | { read -rn3 v; echo \"[$v] rc=$?\"; }",
                b"[] rc=0\n",
                0,
            ),
        ],
    );
}

#[test]
fn lineno_ifs_optind_and_cut_delimiter_and_sed_statuses() {
    // bash 5.2 and GNU coreutils/sed 4.9: `$LINENO`, `$IFS`, `$OPTIND`; `cut -d` takes one byte; sed exits 2 for an unreadable input and 4 for `-i` with no file.
    check(
        "misc",
        &[
            ("echo $LINENO", b"1\n", 0),
            (
                "echo a\necho $LINENO\necho b\necho $LINENO",
                b"a\n2\nb\n4\n",
                0,
            ),
            ("echo \"[$OPTIND]\"", b"[1]\n", 0),
            ("echo \"[${LINENO:-x}]\"", b"[1]\n", 0),
            ("x=$LINENO; echo $x", b"1\n", 0),
            ("set -- a b; echo \"$*\"", b"a b\n", 0),
            ("IFS=; set -- a b; echo \"$*\"", b"ab\n", 0),
            (
                "printf 'a:b\\n' | cut -d:: -f1; echo \"rc=$?\"",
                b"rc=1\n",
                0,
            ),
            (
                "printf 'a\\303\\251b\\n' | cut -d'é' -f2; echo \"rc=$?\"",
                b"rc=1\n",
                0,
            ),
            (
                "printf 'a:b\\n' | cut -d '::' -f2; echo \"rc=$?\"",
                b"rc=1\n",
                0,
            ),
            ("sed p nonexistent; echo \"rc=$?\"", b"rc=2\n", 0),
            (
                "printf 'a\\n' | sed p nonexistent; echo \"rc=$?\"",
                b"rc=2\n",
                0,
            ),
            ("sed -i 1d; echo \"rc=$?\"", b"rc=4\n", 0),
            (
                "printf 'a\\nb\\n' > f; sed -i 1d f nonexistent; echo \"rc=$?\"; cat f",
                b"rc=2\nb\n",
                0,
            ),
            (
                "printf 'a\\n' > f; sed p f nonexistent; echo \"rc=$?\"",
                b"a\na\nrc=2\n",
                0,
            ),
        ],
    );
}

#[test]
fn sed_q_q_and_hold_flags_with_a_missing_newline() {
    // GNU sed 4.9: `q` writes the queued `a` text and ends the line; `Q` does neither; `H` and `G` take the newline flag from the space they copy.
    check(
        "seq",
        &[
            ("printf 'a\\nb\\n' | sed '1{a X\nq}'", b"a\nX\n", 0),
            ("printf 'a\\nb\\n' | sed '1{a X\nQ}'", b"", 0),
            ("printf 'a\\nb\\n' | sed -n '1{a X\nQ}'", b"", 0),
            ("printf 'a\\nb\\n' | sed '1{i X\nQ}'", b"X\n", 0),
            ("printf 'a\\nb' | sed -n '2{x;H;x;p}'", b"b\n\n", 0),
            ("printf 'a\\nb' | sed -n '2{x;H;x;G;p}'", b"b\n\n\n", 0),
            ("printf 'a\\nb' | sed -n '1{h;d};2{x;H;x;p}'", b"b\na\n", 0),
            ("printf 'a\\nb' | sed -n '$!{h;d};H;x;p'", b"a\nb", 0),
            ("printf 'a\\nb' | sed -n '$!{x;d};x;H;g;p'", b"b\na\n", 0),
            ("printf 'a' | sed 'H;h;x;G'", b"a\na", 0),
        ],
    );
}

#[test]
fn expansion_errors_in_substitutions_pipelines_and_errexit() {
    // bash 5.2: a failed `${x:?}` or `set -u` ends a command substitution or a pipeline stage with status 1, a `-c` string with 127, and under `set -e` with 1.
    check(
        "experr",
        &[
            ("set -eu; echo \"$y\"; echo after", b"", 1),
            ("set -e; echo \"${y:?msg}\"; echo after", b"", 1),
            (
                "f=$(echo \"${y:?msg}\"); echo \"rc=$? [$f]\"",
                b"rc=1 []\n",
                0,
            ),
            (
                "set -u; f=$(echo \"$y\"); echo \"rc=$? [$f]\"",
                b"rc=1 []\n",
                0,
            ),
            (
                "set -e; f=$(echo \"${y:?msg}\"); echo \"rc=$? [$f]\"",
                b"",
                1,
            ),
            ("set -eu; f=$(echo \"$y\"); echo \"rc=$? [$f]\"", b"", 1),
            (
                "echo \"${y:?msg}\" | cat; echo \"after rc=$?\"",
                b"after rc=0\n",
                0,
            ),
            ("if true; then echo \"${y:?msg}\"; fi; echo after", b"", 127),
            (
                "set -e; if true; then echo \"${y:?msg}\"; fi; echo after",
                b"",
                1,
            ),
            (
                "set -e; for i in 1 2; do echo \"$i\"; echo \"${y:?msg}\"; done; echo after",
                b"1\n",
                1,
            ),
            ("echo $(echo \"${y:?msg}\"); echo after", b"\nafter\n", 0),
            (
                "echo \"$(echo a; echo \"${y:?msg}\"; echo b)\"; echo after",
                b"a\nafter\n",
                0,
            ),
            (
                "set -u; echo \"$(echo \"$y\")\"; echo after rc=$?",
                b"\nafter rc=0\n",
                0,
            ),
            (
                "set -e; echo \"$(echo \"${y:?msg}\")\"; echo after rc=$?",
                b"\nafter rc=0\n",
                0,
            ),
            (
                "x=$(echo \"${y:?msg}\" 2>/dev/null); echo \"rc=$?\"",
                b"rc=1\n",
                0,
            ),
            (
                "set -u; x=$(echo \"$y\" 2>/dev/null); echo \"rc=$?\"",
                b"rc=1\n",
                0,
            ),
            (
                "set -u; echo \"$y\" | cat; echo \"after rc=$?\"",
                b"after rc=0\n",
                0,
            ),
            ("set -u; echo \"$1\"; echo after", b"", 127),
            ("set -e; x=${y:?msg}; echo after", b"", 1),
            ("x=${y:?msg}; echo after", b"", 127),
            ("set -u; x=$y; echo after", b"", 127),
            ("set -eu; x=$y; echo after", b"", 1),
            ("echo \"${y:?msg}\" > f; echo \"after rc=$?\"; ls", b"", 127),
            ("cat < \"${y:?msg}\"; echo after", b"after\n", 0),
            (
                "f=$(set -u; echo \"$y\"); echo \"rc=$? [$f]\"",
                b"rc=1 []\n",
                0,
            ),
            ("f=$(echo a; false; echo b); echo \"rc=$?\"", b"rc=0\n", 0),
            (
                "set -e; f=$(echo a; false; echo b); echo \"rc=$? [$f]\"",
                b"rc=0 [a\nb]\n",
                0,
            ),
            (
                "f=$(echo \"${y:?msg}\"); echo \"rc=$? [$f]\"; echo more",
                b"rc=1 []\nmore\n",
                0,
            ),
            (
                "echo hi | { echo \"${y:?msg}\"; echo inpipe; }; echo \"rc=$?\"",
                b"rc=1\n",
                0,
            ),
        ],
    );
}

#[test]
fn constructs_outside_the_subset_are_refused_before_anything_runs() {
    // bash runs each of these; the subset refuses it (exit 77) naming the
    // construct, and nothing runs, the `echo before` on line 1 included.
    let cases: &[(&str, &str)] = &[
        ("echo before\nread -t 1 v", "read -t"),
        ("echo before\nread -N 2 v", "read -N"),
        ("echo before\nread -a arr", "read -a"),
        ("echo before\nread -n 1 v", "read -n"),
        ("echo before\nread -d , v", "read -d"),
        ("echo before\nread -r -u 3 v", "read -u"),
        ("echo before\nread -r 'a[0]'", "read"),
        ("echo before\necho hi >&3", "descriptor above 2"),
        ("echo before\necho hi 2>&3", "descriptor above 2"),
        ("echo before\necho hi >&9", "descriptor above 2"),
        ("echo before\necho hi >&10", "descriptor above 2"),
        ("echo before\necho hi 3>&1", "descriptor above 2"),
        ("echo before\necho hi >&0", "descriptor 0"),
        ("echo before\nexport a=(1 2)", "array assignment"),
        ("echo before\nexport a+=(1 2)", "array assignment"),
        ("echo before\ndeclare -a a=(1 2)", "array assignment"),
        (
            "echo before\nprintf 'p\\n' | sed -f /dev/stdin x",
            "sed -f /dev/stdin",
        ),
        ("echo before\nsed --file=- x", "sed -f /dev/stdin"),
    ];
    for (script, needle) in cases {
        let dir = scratch("refuse");
        let (out, err, code) = ish_raw(script, &dir);
        assert_eq!((out.as_slice(), code), (&b""[..], 77), "{script}: {err}");
        assert!(err.contains(needle), "{script}: stderr was {err}");
        assert!(err.contains("line 2"), "{script}: stderr was {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn printf_v_with_an_array_element_is_refused_and_a_bad_name_is_status_2() {
    let dir = scratch("pfv");
    let (out, err, code) = ish_raw("printf -v 'a[0]' %s x", &dir);
    assert_eq!((out.as_slice(), code), (&b""[..], 77), "{err}");
    let (out, _, code) = ish_raw("printf -v 1x %s x; echo rc=$?", &dir);
    assert_eq!((out.as_slice(), code), (&b"rc=2\n"[..], 0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sed_runtime_error_stops_the_program_with_what_it_printed() {
    // `//` with no regex before it is an error when it runs: line 1 was already
    // printed, then status 1, as GNU sed does.
    let dir = scratch("sedrt");
    let (out, err, code) = ish_raw("printf 'a\\nb\\n' | sed -n '1p;//p'", &dir);
    assert_eq!((out.as_slice(), code), (&b"a\n"[..], 1), "{err}");
    assert!(err.contains("no previous regular expression"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_of_an_assignment_does_not_split_its_value() {
    // bash: `export V=$x` takes the whole value, spaces and all.
    let dir = scratch("exsplit");
    let (out, err, code) = ish_raw(
        "x='a b  c'; export V=$x; echo \"[$V]\"; export PATH=$PATH; echo rc=$?",
        &dir,
    );
    assert_eq!(
        (out.as_slice(), code),
        (
            &b"[a b  c]
rc=0
"[..],
            0
        ),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

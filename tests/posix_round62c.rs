//! Round 62c: differences between the in-process POSIX engine (`ish -b`) and
//! bash 5.2 and GNU coreutils / sed 4.9 found in round 62: `read` without
//! `-r` (a backslash quotes the next character), quoted parts of the `${x#pat}`
//! trims, the argument-count rules of `test` and `[` (and `-v`, `-o`, `-R`),
//! `printf` of floating-point numbers (the exact value of the long double bash
//! reads) and of integers past 64 bits, length modifiers, `%c` of an empty
//! argument and `inf` / `nan`; `wc` totals and the name `-`, `tr` class
//! rules, `basename` and `dirname` options, `exit` and `cd ''` and `pwd -x`;
//! and sed's `M` flag, `\u\L`, and which words are files once `-e` is given.
//!
//! Every expected stdout and exit status below was produced by GNU bash 5.2
//! and GNU coreutils / sed on Debian (glibc, x87 long double) with the same
//! script. The bytes are asserted as they are, not through `od`, whose output
//! differs on macOS.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_r62c_{tag}_{}", std::process::id()));
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
            (String::from_utf8_lossy(&out).into_owned(), code),
            (String::from_utf8_lossy(want).into_owned(), *want_code),
            "{script}: stderr {err}"
        );
        assert_eq!(out.as_slice(), *want, "{script}: bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn test_v_o_and_r() {
    // bash 5.2: `[ -v NAME ]`, `[ -o OPT ]`, `[ -R x ]` and the three-word `-a`/`-o` forms.
    check(
        "tvo",
        &[
            (
                r#"X=1; [ -v X ]; echo $?; [ -v Y ]; echo $?; Z=; [ -v Z ]; echo $?; [ -v 1 ]; echo $?"#,
                b"0\n1\n0\n1\n",
                0,
            ),
            (
                r#"set -- a b; [ -v 1 ]; echo $?; [ -v 3 ]; echo $?; [ -v HOME ]; echo $?; [ -v IFS ]; echo $?; [ -v 'a b' ]; echo $?"#,
                b"0\n1\n0\n0\n1\n",
                0,
            ),
            (
                r#"[ -o noglob ]; echo $?; [ -o errexit ]; echo $?; set -e; [ -o errexit ]; echo $?; set +e; [ -o errexit ]; echo $?; [ -o nounset ]; echo $?; set -u; [ -o nounset ]; echo $?; [ -o pipefail ]; echo $?; set -o pipefail; [ -o pipefail ]; echo $?"#,
                b"1\n1\n0\n1\n1\n0\n1\n0\n",
                0,
            ),
            (
                r#"[ -o braceexpand ]; echo $?; [ -o hashall ]; echo $?; [ -o interactive-comments ]; echo $?; [ -o nosuch ]; echo $?; [ -o allexport ]; echo $?; [ -o xtrace ]; echo $?; [ -o noclobber ]; echo $?; [ -o emacs ]; echo $?; [ -o posix ]; echo $?; [ -o history ]; echo $?"#,
                b"0\n0\n0\n1\n1\n1\n1\n1\n1\n1\n",
                0,
            ),
            (
                r#"[ -R x ]; echo $?; [ -R ]; echo $?; test -R x; echo $?"#,
                b"1\n0\n1\n",
                0,
            ),
            (
                r#"[ -n -a x ]; echo $?; [ -a -a -a ]; echo $?; [ -o -o -o ]; echo $?; [ ! a -o b ]; echo $?; [ ! a -a b ]; echo $?; [ ! '' -a '' ]; echo $?"#,
                b"0\n0\n0\n1\n1\n0\n",
                0,
            ),
        ],
    );
}

#[test]
fn exit_status_rules() {
    // bash 5.2: `exit` takes a number modulo 256; anything else ends the shell with 2; a second operand ends it with 1.
    check(
        "exit",
        &[
            (r#"exit -1"#, b"", 255),
            (r#"echo a; exit abc; echo b"#, b"a\n", 2),
            (r#"echo a; exit ''; echo b"#, b"a\n", 2),
            (r#"echo a; exit 1x; echo b"#, b"a\n", 2),
            (r#"exit 256"#, b"", 0),
            (r#"exit 300"#, b"", 44),
            (r#"exit 1 2; echo after $?"#, b"", 1),
            (r#"exit abc 2; echo after $?"#, b"", 2),
            (r#"exit +3"#, b"", 3),
            (r#"exit ' 7 '"#, b"", 7),
            (r#"exit -256"#, b"", 0),
            (r#"exit 9223372036854775808"#, b"", 2),
            (r#"true; false; exit"#, b"", 1),
        ],
    );
}

#[test]
fn cd_empty_and_pwd_options() {
    // bash 5.2: `cd ''` stays put; `pwd -x` is an invalid option.
    check(
        "cdpwd",
        &[
            (r#"cd ''; echo $?; pwd | wc -l"#, b"0\n1\n", 0),
            (
                r#"mkdir d; cd d; cd ''; echo $?; basename "$PWD"; echo "$OLDPWD" | grep -c ."#,
                b"0\nd\n1\n",
                0,
            ),
            (r#"pwd -x; echo $?"#, b"2\n", 0),
            (
                r#"pwd -L >/dev/null; echo $?; pwd -P >/dev/null; echo $?; pwd -LP >/dev/null; echo $?; pwd -- >/dev/null; echo $?; pwd a b >/dev/null; echo $?"#,
                b"0\n0\n0\n0\n0\n",
                0,
            ),
        ],
    );
}

#[test]
fn quoted_pattern_in_trims() {
    // bash 5.2: the quoted part of a `${x#pat}` pattern is literal.
    check(
        "trim",
        &[
            (
                r#"x='a*b'; echo "${x#"a*"}" "${x%"*b"}" "${x##"a*"}""#,
                b"b a b\n",
                0,
            ),
            (
                r#"x='a*b'; echo "${x#"a"*}" "${x%*"b"}" "${x##a\*}" "${x%%\*b}""#,
                b"*b a* b a\n",
                0,
            ),
            (
                r#"x='a*b'; echo ${x#"a*"} ${x%"*b"} ${x##"a*"} ${x%%"*b"}"#,
                b"b a b a\n",
                0,
            ),
            (
                r#"x='aab'; echo "${x#"a"*}" "${x##"a"*}" "${x%"b"}" "${x#a*}" "${x##a*}""#,
                b"ab  aa ab \n",
                0,
            ),
            (
                r#"x='a*b'; p='a*'; echo "${x#$p}" "${x#"$p"}" "${x##"$p"}" "${x%"$p"}""#,
                b"*b b b a*b\n",
                0,
            ),
            (
                r#"x='abc'; echo "${x#'a'}" "${x#'a*'}" "${x%'c'}""#,
                b"bc abc ab\n",
                0,
            ),
            (
                r#"x='a?b'; echo "${x#"a?"}" "${x#a?}" "${x%"?b"}" "${x%?b}""#,
                b"b b a a\n",
                0,
            ),
            (
                r#"x='a[b]c'; echo "${x#"a[b]"}" "${x#a[b]}" "${x#a\[b\]}""#,
                b"c a[b]c c\n",
                0,
            ),
            (
                r#"x="a b c"; echo "${x#"a "}" "${x%" c"}" "${x#a b }" "${x##* }""#,
                b"b c a b c c\n",
                0,
            ),
            (
                r#"x=foo.tar.gz; echo "${x%.*}" "${x%%.*}" "${x#*.}" "${x##*.}" "${x#"foo."}""#,
                b"foo.tar foo tar.gz gz tar.gz\n",
                0,
            ),
        ],
    );
}

#[test]
fn read_backslash_without_r() {
    // bash 5.2: without -r, a backslash quotes the next character, separators included, and is removed.
    check(
        "readbs",
        &[
            (
                r#"echo 'a\ b c' | { read a b; echo "[$a][$b]"; }"#,
                b"[a b][c]\n",
                0,
            ),
            (
                r#"printf 'a\\:b\n' | { IFS=: read x y; echo "[$x][$y]"; }"#,
                b"[a:b][]\n",
                0,
            ),
            (
                r#"echo 'a\ b c' | { read -r a b; echo "[$a][$b]"; }"#,
                b"[a\\][b c]\n",
                0,
            ),
            (
                r#"printf 'a\\\nb c\n' | { read a b; echo "[$a][$b]"; }"#,
                b"[ab][c]\n",
                0,
            ),
            (
                r#"printf 'a\\\nb c\n' | { read -r a b; echo "[$a][$b]"; }"#,
                b"[a\\][]\n",
                0,
            ),
            (
                r#"printf '\\ a b\n' | { read a b; echo "[$a][$b]"; }"#,
                b"[ a][b]\n",
                0,
            ),
            (
                r#"printf 'a b\\ \n' | { read a b; echo "[$a][$b]"; }"#,
                b"[a][b ]\n",
                0,
            ),
            (
                r#"printf 'a b \n' | { read a b; echo "[$a][$b]"; }"#,
                b"[a][b]\n",
                0,
            ),
            (
                r#"printf 'a\\tb\\\\c\n' | { read a; echo "[$a]"; }"#,
                b"[atb\\c]\n",
                0,
            ),
            (
                r#"printf 'a\\tb\\\\c\n' | { read -r a; echo "[$a]"; }"#,
                b"[a\\tb\\\\c]\n",
                0,
            ),
            (
                r#"printf 'a\\ b\\ c\n' | { read a; echo "[$a]"; }"#,
                b"[a b c]\n",
                0,
            ),
            (
                r#"printf 'a\\ b\\ c d\n' | { read a b; echo "[$a][$b]"; }"#,
                b"[a b c][d]\n",
                0,
            ),
            (
                r#"printf 'a:b\\:c:d\n' | { IFS=: read x y z; echo "[$x][$y][$z]"; }"#,
                b"[a][b:c][d]\n",
                0,
            ),
            (
                r#"printf 'a:b\\:c:d\n' | { IFS=: read x; echo "[$x]"; }"#,
                b"[a:b:c:d]\n",
                0,
            ),
            (
                r#"printf 'a\\' | { read a; echo "[$a]"; echo $?; }"#,
                b"[a]\n0\n",
                0,
            ),
            (
                r#"printf 'a\\\\\n' | { read a; echo "[$a]"; }"#,
                b"[a\\]\n",
                0,
            ),
            (
                r#"printf 'one\\ two three\n' | { read a b c; echo "[$a][$b][$c]"; }"#,
                b"[one two][three][]\n",
                0,
            ),
            (
                r#"printf 'x\\\n' | { read a; echo "[$a]"; echo $?; }"#,
                b"[x]\n0\n",
                0,
            ),
            (
                r#"printf '\\\\ \\q\n' | { read a b; echo "[$a][$b]"; }"#,
                b"[\\][q]\n",
                0,
            ),
            (
                r#"printf 'a  b\\ \\  c\n' | { read a b c; echo "[$a][$b][$c]"; }"#,
                b"[a][b  ][c]\n",
                0,
            ),
            (
                r#"printf 'a\\\n\\\nb\n' | { read a; echo "[$a]"; }"#,
                b"[ab]\n",
                0,
            ),
            (
                r#"printf 'é\\ ü z\n' | { read a b; echo "[$a][$b]"; }"#,
                b"[\xc3\xa9 \xc3\xbc][z]\n",
                0,
            ),
            (
                r#"printf 'a\\:b:c\n' | { IFS=: read x y; echo "[$x][$y]"; }"#,
                b"[a:b][c]\n",
                0,
            ),
            (
                r#"printf ' a\\ \n' | { read a b; echo "[$a][$b]"; }"#,
                b"[a ][]\n",
                0,
            ),
            (
                r#"printf 'a\\ b\n' | { IFS= read a b; echo "[$a][$b]"; }"#,
                b"[a b][]\n",
                0,
            ),
            (
                r#"printf 'a\\ b\n' | { IFS=' ' read a; echo "[$a]"; }"#,
                b"[a b]\n",
                0,
            ),
        ],
    );
}

#[test]
fn test_argument_count_rules() {
    // bash 5.2: a sample of 360 of the 15105 `[ ... ]` forms of up to seven words checked against bash.
    check(
        "tcount",
        &[
            (r#"[ '-o' '!' '=' '-o' ] 2>/dev/null; echo $?
[ '-n' '' 'x' ')' ] 2>/dev/null; echo $?
[ '=' '=' '!' '(' ] 2>/dev/null; echo $?
[ '!' '-a' 'x' '=' ] 2>/dev/null; echo $?
[ '(' '!' '' '!' ] 2>/dev/null; echo $?
[ 'x' '(' '-o' 'x' ] 2>/dev/null; echo $?
[ '(' '' '=' '-n' ] 2>/dev/null; echo $?
[ '1' '-eq' '(' '-a' '=' ] 2>/dev/null; echo $?
[ ')' '1' '-a' '' '!' ] 2>/dev/null; echo $?
[ '-z' '(' '-z' '-a' ] 2>/dev/null; echo $?
[ 'x' '-o' '-n' '(' ] 2>/dev/null; echo $?
[ '-z' '-n' '1' '(' ')' '1' ] 2>/dev/null; echo $?
[ ')' '(' '-z' ')' ] 2>/dev/null; echo $?
[ '-e' '!' '1' '-o' '-e' ] 2>/dev/null; echo $?
[ '!' '' '-o' '=' '-z' ] 2>/dev/null; echo $?
[ ')' '-o' '(' '1' '' '1' '=' ] 2>/dev/null; echo $?
[ ')' '-a' ')' '(' ] 2>/dev/null; echo $?
[ 'x' '' '!' '-o' ] 2>/dev/null; echo $?
[ '-a' 'x' '(' '-n' ] 2>/dev/null; echo $?
[ ')' '=' '-o' ')' ] 2>/dev/null; echo $?
[ '(' '-n' '=' '-n' ] 2>/dev/null; echo $?
[ 'x' '' '(' ')' ] 2>/dev/null; echo $?
[ '!' ')' '-a' ')' ] 2>/dev/null; echo $?
[ '=' '-o' '-n' 'x' ] 2>/dev/null; echo $?
[ '(' '' '!' '(' ] 2>/dev/null; echo $?
[ ')' '(' ')' '-eq' '-z' ] 2>/dev/null; echo $?
[ '=' '!' '-n' '-z' '1' ] 2>/dev/null; echo $?
[ 'x' '-o' '1' '' '(' ] 2>/dev/null; echo $?
[ 'x' '' '(' '(' ] 2>/dev/null; echo $?
[ ')' '-n' '-z' '=' ] 2>/dev/null; echo $?
[ ')' ')' '!' '(' '' ] 2>/dev/null; echo $?
[ '(' '1' '-z' '-a' '' '-n' '-o' ] 2>/dev/null; echo $?
[ 'x' '' 'x' '-n' ] 2>/dev/null; echo $?
[ '-a' '-n' '-o' '-a' ] 2>/dev/null; echo $?
[ 'x' '=' '-n' '-z' ] 2>/dev/null; echo $?
[ '-o' '=' 'x' '-a' '1' ] 2>/dev/null; echo $?
[ 'x' 'x' '' '' ] 2>/dev/null; echo $?
[ ')' ')' '=' '-eq' '' ] 2>/dev/null; echo $?
[ '' '-o' '-z' '-n' ] 2>/dev/null; echo $?
[ '(' '' '-n' '1' '!' ')' '=' ] 2>/dev/null; echo $?
[ '!' '-z' '-n' '=' ] 2>/dev/null; echo $?
[ '-a' '(' '=' '-a' ] 2>/dev/null; echo $?
[ '(' '=' '=' '-n' ] 2>/dev/null; echo $?
[ '-z' ')' '-n' '-o' ] 2>/dev/null; echo $?
[ '' '-a' '=' '-n' ] 2>/dev/null; echo $?
[ '' 'x' '-z' 'x' ] 2>/dev/null; echo $?
[ 'x' ')' '-n' '-a' ] 2>/dev/null; echo $?
[ '-a' '!' '-o' '-a' ')' 'x' '!' ] 2>/dev/null; echo $?
[ 'x' '=' ')' '!' ] 2>/dev/null; echo $?
[ '(' '-n' '-o' '-o' ] 2>/dev/null; echo $?
[ '=' '1' ')' ')' ')' ] 2>/dev/null; echo $?
[ ')' '!' 'x' '=' ] 2>/dev/null; echo $?
[ '' '-a' '' '-z' ] 2>/dev/null; echo $?
[ '-e' '-n' '-z' '(' '' ] 2>/dev/null; echo $?
[ ')' '=' '(' '-o' ] 2>/dev/null; echo $?
[ '!' ')' '=' '-o' ] 2>/dev/null; echo $?
[ '-o' '(' '(' '!' ] 2>/dev/null; echo $?
[ '=' '!' '-a' ')' ] 2>/dev/null; echo $?
[ 'x' '-a' '-n' '-n' ] 2>/dev/null; echo $?
[ '=' '!' '!' 'x' ] 2>/dev/null; echo $?
[ '-z' '=' 'x' ] 2>/dev/null; echo $?
[ '=' '-n' '-o' '-o' ] 2>/dev/null; echo $?
[ 'x' '-z' '-n' 'x' ] 2>/dev/null; echo $?
[ '-n' '-z' '-z' '-z' ] 2>/dev/null; echo $?
[ '' 'x' '1' ')' '' '-e' '-o' ] 2>/dev/null; echo $?
[ '-a' '=' '-z' '' ] 2>/dev/null; echo $?
[ '!' '-z' '-a' '-o' ] 2>/dev/null; echo $?
[ '-z' 'x' '' '-a' ] 2>/dev/null; echo $?
[ '=' '!' '-a' '' ] 2>/dev/null; echo $?
[ '-e' 'x' '-n' '1' '!' ] 2>/dev/null; echo $?
[ '' '-a' '-z' '=' ] 2>/dev/null; echo $?
[ 'x' '=' '!' '(' ] 2>/dev/null; echo $?
[ ')' '-n' '!' '-o' ] 2>/dev/null; echo $?
[ '-n' '-e' '(' '=' '(' '!' ] 2>/dev/null; echo $?
[ 'x' '-z' '!' '!' ] 2>/dev/null; echo $?
[ '=' '-e' '-a' '(' 'x' '-a' '-z' ] 2>/dev/null; echo $?
[ '-o' '-o' '!' '-n' ] 2>/dev/null; echo $?
[ '(' '=' '1' '1' '-e' ] 2>/dev/null; echo $?
[ '=' ')' '-z' '-n' ] 2>/dev/null; echo $?
[ '-o' '-o' '(' '-o' ] 2>/dev/null; echo $?
[ '-e' '-e' '!' ')' '=' '-z' '-e' ] 2>/dev/null; echo $?
[ '-a' '!' '-o' '-a' ] 2>/dev/null; echo $?
[ '1' '-e' '' '(' '-z' '-eq' ] 2>/dev/null; echo $?
[ '=' '-e' '!' '!' '-n' ] 2>/dev/null; echo $?
[ '-z' '!' '(' '-z' ] 2>/dev/null; echo $?
[ '(' '-e' '1' 'x' '(' '-z' '-e' ] 2>/dev/null; echo $?
[ '=' '(' ')' '-n' ] 2>/dev/null; echo $?
[ '-eq' '-eq' '-z' '-z' '-n' '' '-n' ] 2>/dev/null; echo $?
[ 'x' '' 'x' '-o' 'x' '=' ] 2>/dev/null; echo $?
[ '-eq' '-a' '-e' '1' '-a' ] 2>/dev/null; echo $?
[ 'x' '-n' ')' '-n' ] 2>/dev/null; echo $?
[ ')' '-o' '-o' 'x' '(' '-n' 'x' ] 2>/dev/null; echo $?
[ '-n' '' '-o' '(' ] 2>/dev/null; echo $?
[ 'x' '' '-o' '' ] 2>/dev/null; echo $?
[ '-n' '(' '(' '-a' ] 2>/dev/null; echo $?
[ '(' '' '-n' '=' ] 2>/dev/null; echo $?
[ '(' '(' '(' '=' ] 2>/dev/null; echo $?
[ '!' '-n' '!' '-o' ] 2>/dev/null; echo $?
[ '-n' '!' '-n' ] 2>/dev/null; echo $?
[ ')' '-a' '!' 'x' ] 2>/dev/null; echo $?
[ '(' 'x' '' '(' ] 2>/dev/null; echo $?
[ '-o' '!' '-o' '-a' ] 2>/dev/null; echo $?
[ '!' '-z' '-z' '1' '-o' '-n' '(' ] 2>/dev/null; echo $?
[ '-a' '-e' '' '-z' '(' ] 2>/dev/null; echo $?
[ 'x' '=' '-n' '-o' ] 2>/dev/null; echo $?
[ '-a' '-o' '!' '=' ] 2>/dev/null; echo $?
[ '=' '-z' '!' '-a' ] 2>/dev/null; echo $?
[ ')' '(' '-a' ] 2>/dev/null; echo $?
[ '=' '' '-o' '=' ] 2>/dev/null; echo $?
[ '-a' 'x' 'x' 'x' ] 2>/dev/null; echo $?
[ '!' '-o' '-o' '-e' '-n' '-e' ] 2>/dev/null; echo $?
[ '' '-o' '(' ')' ] 2>/dev/null; echo $?
[ '-a' ')' '-o' '' ] 2>/dev/null; echo $?
[ '-a' '' '(' '=' ] 2>/dev/null; echo $?
[ '-a' '(' '-n' 'x' ] 2>/dev/null; echo $?
[ 'x' '' '-z' '-o' ] 2>/dev/null; echo $?
[ '(' '-a' '(' ] 2>/dev/null; echo $?
[ '' '-a' '-z' '!' ] 2>/dev/null; echo $?
[ 'x' '=' '=' '-z' ] 2>/dev/null; echo $?
[ '!' '(' '-z' '-n' ] 2>/dev/null; echo $?
[ '=' '-o' '-o' '' ] 2>/dev/null; echo $?
[ '-o' '-o' '-o' '-eq' '-n' '=' ] 2>/dev/null; echo $?
[ '' '!' '-o' '-a' ] 2>/dev/null; echo $?
[ '-a' '=' '' '=' ] 2>/dev/null; echo $?
[ '-eq' '=' '(' '-o' '' '-o' '!' ] 2>/dev/null; echo $?
[ '!' '-a' '(' '-z' ] 2>/dev/null; echo $?
[ '!' '!' '-n' ] 2>/dev/null; echo $?
[ '-a' ')' '=' ')' ] 2>/dev/null; echo $?
[ ')' '-a' '!' '=' ] 2>/dev/null; echo $?
[ '!' '-z' '(' '-o' ] 2>/dev/null; echo $?
[ '-a' '!' '=' '' ] 2>/dev/null; echo $?
[ '=' ')' '' '' ] 2>/dev/null; echo $?
[ '' '-n' '=' '-a' ] 2>/dev/null; echo $?
[ '-n' '' '-e' '=' '!' ] 2>/dev/null; echo $?
[ '!' '!' '-eq' '=' 'x' '1' '!' ] 2>/dev/null; echo $?
[ '(' '!' '-a' ] 2>/dev/null; echo $?
[ '1' '' '1' '-z' '-e' 'x' ] 2>/dev/null; echo $?
[ '-e' '-e' '' '!' '-n' '(' ] 2>/dev/null; echo $?
[ '!' '-z' '-z' '=' ] 2>/dev/null; echo $?
[ ')' '' '(' '-n' ] 2>/dev/null; echo $?
[ '!' '-eq' '-n' '-o' 'x' ] 2>/dev/null; echo $?
[ '-n' '-eq' '!' '-e' '-e' '(' '-a' ] 2>/dev/null; echo $?
[ '-o' '-z' '-n' 'x' ] 2>/dev/null; echo $?
[ '(' 'x' '-a' '-n' '-z' '-z' ] 2>/dev/null; echo $?
[ 'x' '=' '!' '-o' ] 2>/dev/null; echo $?
[ '-o' '=' '!' '-n' ] 2>/dev/null; echo $?
[ '-o' '=' ')' '' '1' '=' ] 2>/dev/null; echo $?
[ ')' '' '' '-n' ] 2>/dev/null; echo $?
[ 'x' '' '=' '!' '-z' '(' ] 2>/dev/null; echo $?
[ 'x' '(' '-n' ')' ] 2>/dev/null; echo $?
[ '-n' '' '' '-n' ] 2>/dev/null; echo $?
[ '!' 'x' '-n' '=' ] 2>/dev/null; echo $?
[ '' '-z' 'x' '!' ] 2>/dev/null; echo $?
[ '-z' '(' '-n' '!' ] 2>/dev/null; echo $?
[ '-o' '-n' 'x' '(' '-n' ] 2>/dev/null; echo $?
[ '-z' '=' 'x' '-z' ] 2>/dev/null; echo $?
[ 'x' '-n' '-o' '-a' ] 2>/dev/null; echo $?
[ '!' '-z' '-a' '-a' '!' '-n' '-o' ] 2>/dev/null; echo $?
[ '!' 'x' ')' '' ] 2>/dev/null; echo $?
[ '1' '-o' 'x' '-o' '-e' '(' ] 2>/dev/null; echo $?
[ '=' '=' 'x' '(' ] 2>/dev/null; echo $?
[ '=' '-z' '-o' '-z' ] 2>/dev/null; echo $?
[ 'x' '(' ')' '=' ] 2>/dev/null; echo $?
[ '-a' 'x' '(' ')' ] 2>/dev/null; echo $?
[ ')' '=' '-z' ')' ] 2>/dev/null; echo $?
[ '-o' '(' 'x' ')' ] 2>/dev/null; echo $?
[ ')' '-z' ')' ')' ] 2>/dev/null; echo $?
[ ')' '-a' '-o' 'x' ] 2>/dev/null; echo $?
[ '-eq' '!' '-eq' '-e' '(' ] 2>/dev/null; echo $?
[ '-eq' '' '-n' '(' '-n' '-e' ')' ] 2>/dev/null; echo $?
[ '(' '-o' '' ')' ] 2>/dev/null; echo $?
[ '!' 'x' '-z' '' ] 2>/dev/null; echo $?
[ '!' '!' '=' '!' '-o' '-z' ] 2>/dev/null; echo $?
[ ')' '=' '-n' ] 2>/dev/null; echo $?
[ '' '=' '-o' '-o' ] 2>/dev/null; echo $?
[ '!' '-a' '!' '-n' ] 2>/dev/null; echo $?
[ '=' 'x' ')' '=' '1' '=' '-a' ] 2>/dev/null; echo $?
[ '=' '-o' '-n' '' ] 2>/dev/null; echo $?
[ '-a' '' '-a' '-o' ] 2>/dev/null; echo $?
[ '-n' '-z' '' ] 2>/dev/null; echo $?
[ '=' '-a' '!' ] 2>/dev/null; echo $?
[ '(' '-z' '-z' '-o' ] 2>/dev/null; echo $?
[ '-a' '!' '-n' '-z' ] 2>/dev/null; echo $?
[ '-z' 'x' '!' '-z' ] 2>/dev/null; echo $?
[ '=' '!' '!' '' ] 2>/dev/null; echo $?
[ ')' '' '' '-n' '' '!' '-a' ] 2>/dev/null; echo $?
[ '-o' '-o' '(' '' ] 2>/dev/null; echo $?
[ '-z' ')' '' '' ] 2>/dev/null; echo $?
[ '=' '!' '' '' ] 2>/dev/null; echo $?
[ '-e' 'x' '-a' 'x' '' ] 2>/dev/null; echo $?
[ ')' '-eq' '!' '=' '-o' '-o' ] 2>/dev/null; echo $?
[ '(' '!' ')' ')' ] 2>/dev/null; echo $?
[ '-n' ')' ')' '-a' ] 2>/dev/null; echo $?
[ '-e' 'x' ')' '-e' '-z' '-z' ] 2>/dev/null; echo $?
[ '-a' '-a' '-a' '=' ] 2>/dev/null; echo $?
[ '-z' '' '(' 'x' ] 2>/dev/null; echo $?
[ '-n' '-a' '' 'x' ] 2>/dev/null; echo $?
[ '-n' '(' 'x' 'x' ] 2>/dev/null; echo $?
[ '' '-n' ')' '(' ] 2>/dev/null; echo $?
[ '=' '' '' '-z' ] 2>/dev/null; echo $?
[ '(' 'x' '-z' '!' ] 2>/dev/null; echo $?
[ '(' '=' ')' '-z' ] 2>/dev/null; echo $?
[ ')' '=' 'x' '=' ] 2>/dev/null; echo $?
[ ')' '-n' 'x' ')' ] 2>/dev/null; echo $?
[ '-z' '-o' '-n' '=' ] 2>/dev/null; echo $?
[ '(' '!' '-z' '(' ] 2>/dev/null; echo $?
[ 'x' '(' 'x' '!' ] 2>/dev/null; echo $?
[ '-o' '=' '-o' '!' ] 2>/dev/null; echo $?
[ '!' '(' 'x' 'x' ] 2>/dev/null; echo $?
[ '=' '=' '' '!' ] 2>/dev/null; echo $?
[ '-a' '-o' '' '=' ] 2>/dev/null; echo $?
[ '!' '!' '!' 'x' ] 2>/dev/null; echo $?
[ ')' ')' '' '-n' ] 2>/dev/null; echo $?
[ '-e' '-a' '-e' '-z' '1' '-e' ] 2>/dev/null; echo $?
[ '-a' '(' '-z' '=' ] 2>/dev/null; echo $?
[ ')' '-o' '' '=' ] 2>/dev/null; echo $?
[ 'x' '(' '-eq' '-z' '-z' ] 2>/dev/null; echo $?
[ '-z' '-n' '=' '-o' ] 2>/dev/null; echo $?
[ '-o' '-o' '-n' 'x' ] 2>/dev/null; echo $?
[ '-e' '' '1' 'x' '!' ] 2>/dev/null; echo $?
[ '(' '-o' '-z' '=' ] 2>/dev/null; echo $?
[ ')' ')' '-eq' '-eq' 'x' ] 2>/dev/null; echo $?
[ ')' 'x' '-n' '!' ] 2>/dev/null; echo $?
[ '-n' '-n' '-n' '-z' ] 2>/dev/null; echo $?
[ '-n' '1' '-e' '' '-n' '-eq' ] 2>/dev/null; echo $?
[ '-z' '-n' '' ] 2>/dev/null; echo $?
[ '' '-a' '-o' 'x' ] 2>/dev/null; echo $?
[ '=' '-a' 'x' '' ] 2>/dev/null; echo $?
[ '=' '-n' '-a' '' ] 2>/dev/null; echo $?
[ '=' '-n' '1' '-z' '-o' ] 2>/dev/null; echo $?
[ '-z' '-n' '=' '' ] 2>/dev/null; echo $?
[ '-a' '-o' '-n' '-z' ] 2>/dev/null; echo $?
[ '-a' '' '-a' '' ] 2>/dev/null; echo $?
[ '-a' '(' '!' '(' ] 2>/dev/null; echo $?
[ '!' '-o' '!' ] 2>/dev/null; echo $?
[ '-e' '-eq' '!' '' '1' ] 2>/dev/null; echo $?
[ '=' '' '-a' '=' ] 2>/dev/null; echo $?
[ 'x' '-n' 'x' '' ] 2>/dev/null; echo $?
[ '-z' '=' '=' '-n' '=' '-a' '-eq' ] 2>/dev/null; echo $?
[ '(' '' ')' '-a' ] 2>/dev/null; echo $?
[ '-a' '-z' '-z' '(' ] 2>/dev/null; echo $?
[ '!' ')' '(' '!' ] 2>/dev/null; echo $?
[ '-a' '-a' '!' '!' ] 2>/dev/null; echo $?
[ '-n' '!' '(' '' ] 2>/dev/null; echo $?
[ '-n' '(' '-a' '-a' ] 2>/dev/null; echo $?
[ '-z' '-a' '-n' '-o' ] 2>/dev/null; echo $?
[ '=' '1' '-n' '1' '=' '' ] 2>/dev/null; echo $?
[ 'x' '-z' '=' ')' ] 2>/dev/null; echo $?
[ '=' '!' '-a' '-z' ] 2>/dev/null; echo $?
[ '!' '(' ')' 'x' ] 2>/dev/null; echo $?
[ '(' '!' 'x' '' ] 2>/dev/null; echo $?
[ '=' '-a' ')' ] 2>/dev/null; echo $?
[ ')' '(' ')' ] 2>/dev/null; echo $?
[ '-z' '!' '-z' '-n' ] 2>/dev/null; echo $?
[ '-n' '=' '=' '=' ] 2>/dev/null; echo $?
[ '-a' '(' 'x' '=' ] 2>/dev/null; echo $?
[ '-z' '-o' '-o' '-e' '-n' '=' '=' ] 2>/dev/null; echo $?
[ '!' '-a' '-n' '=' ] 2>/dev/null; echo $?
[ '' '-a' '-z' '-z' ] 2>/dev/null; echo $?
[ '-z' 'x' '-a' ')' ] 2>/dev/null; echo $?
[ 'x' '-z' ] 2>/dev/null; echo $?
[ '-z' '-a' '' '=' ] 2>/dev/null; echo $?
[ ')' 'x' '-z' ] 2>/dev/null; echo $?
[ '-a' '-eq' '-o' '-eq' '1' '-eq' ] 2>/dev/null; echo $?
[ '-a' '!' '-o' '-z' ] 2>/dev/null; echo $?
[ '=' '-a' '-z' ')' ] 2>/dev/null; echo $?
[ '(' '-n' '!' '-n' ] 2>/dev/null; echo $?
[ '!' '-a' '-n' '(' ] 2>/dev/null; echo $?
[ '-o' '=' '' '-o' ] 2>/dev/null; echo $?
[ ')' '-z' ')' '-o' ] 2>/dev/null; echo $?
[ '=' ')' '(' '=' ] 2>/dev/null; echo $?
[ '-n' '=' '(' '!' ] 2>/dev/null; echo $?
[ '-a' ')' '=' '-n' ] 2>/dev/null; echo $?
[ '-n' '-a' '-a' '-n' ] 2>/dev/null; echo $?
[ '!' ')' '=' '(' ] 2>/dev/null; echo $?
[ 'x' 'x' '' '=' ] 2>/dev/null; echo $?
[ '-n' '-o' ')' ')' ] 2>/dev/null; echo $?
[ ')' '-a' '=' '-n' ] 2>/dev/null; echo $?
[ '' '-a' '=' '-o' ] 2>/dev/null; echo $?
[ '=' '-n' '-eq' '=' '-a' '1' ] 2>/dev/null; echo $?
[ '-a' '-z' '!' ] 2>/dev/null; echo $?
[ ')' ')' '-eq' 'x' '-a' '1' '=' ] 2>/dev/null; echo $?
[ '(' '(' '=' '-a' ] 2>/dev/null; echo $?
[ '-e' '-a' '-z' '=' '-z' ] 2>/dev/null; echo $?
[ '-a' '-n' '!' ] 2>/dev/null; echo $?
[ '-z' '-a' '' ')' ] 2>/dev/null; echo $?
[ '-n' '(' '!' ')' '-e' ] 2>/dev/null; echo $?
[ '1' '-e' '-o' '' '-z' '-o' ] 2>/dev/null; echo $?
[ '-n' '=' '=' ')' ] 2>/dev/null; echo $?
[ '=' '!' '-z' '1' '-z' '=' '(' ] 2>/dev/null; echo $?
[ '' '!' 'x' ] 2>/dev/null; echo $?
[ 'x' ')' '-z' '' '-o' '' '=' ] 2>/dev/null; echo $?
[ 'x' '-n' '=' '(' ] 2>/dev/null; echo $?
[ 'x' ')' '-z' '-z' ] 2>/dev/null; echo $?
[ 'x' '-n' '-z' '-a' ] 2>/dev/null; echo $?
[ '-a' '-n' '-a' '!' ] 2>/dev/null; echo $?
[ ')' '=' '-eq' ')' '-z' ] 2>/dev/null; echo $?
[ ')' '=' '' ] 2>/dev/null; echo $?
[ '' '-n' '-a' '-z' ] 2>/dev/null; echo $?
[ '=' 'x' '-n' '-o' '1' '-a' ] 2>/dev/null; echo $?
[ '1' '=' '-a' '-e' '-n' ] 2>/dev/null; echo $?
[ '-a' '-n' '!' '-z' ] 2>/dev/null; echo $?
[ '-o' '-o' 'x' '-n' '=' '=' '=' ] 2>/dev/null; echo $?
[ '-n' '!' '-eq' '-a' '-n' ] 2>/dev/null; echo $?
[ '=' 'x' '=' 'x' ] 2>/dev/null; echo $?
[ '-z' '-o' '-a' '-n' ] 2>/dev/null; echo $?
[ '(' '-eq' '' '-o' '!' '-e' ] 2>/dev/null; echo $?
[ '!' '' '(' '!' ] 2>/dev/null; echo $?
[ '(' '(' 'x' '!' ] 2>/dev/null; echo $?
[ '(' '' '' '!' ] 2>/dev/null; echo $?
[ '!' '' '-n' '' ] 2>/dev/null; echo $?
[ '' '' '-n' 'x' ] 2>/dev/null; echo $?
[ 'x' '-o' ')' '-a' ] 2>/dev/null; echo $?
[ ')' '' '-z' '' 'x' '=' '-e' ] 2>/dev/null; echo $?
[ 'x' '!' ')' '!' ] 2>/dev/null; echo $?
[ '!' ')' '(' '=' ] 2>/dev/null; echo $?
[ '=' '-a' ')' '-z' ] 2>/dev/null; echo $?
[ '' ')' '-n' '-n' ] 2>/dev/null; echo $?
[ '-n' ')' 'x' '-o' ] 2>/dev/null; echo $?
[ '' '-n' '-o' 'x' ] 2>/dev/null; echo $?
[ '' '-z' '=' '-z' ] 2>/dev/null; echo $?
[ ')' '-a' 'x' '!' ] 2>/dev/null; echo $?
[ '-z' ')' '' '-eq' '!' '' ] 2>/dev/null; echo $?
[ '-o' ')' '-o' '=' ] 2>/dev/null; echo $?
[ '=' '' '-n' '-z' ] 2>/dev/null; echo $?
[ ')' '(' 'x' '=' ] 2>/dev/null; echo $?
[ '' '-z' ')' '(' ] 2>/dev/null; echo $?
[ '-n' '!' 'x' '' ] 2>/dev/null; echo $?
[ '' 'x' 'x' 'x' ] 2>/dev/null; echo $?
[ '-z' '!' '-n' ] 2>/dev/null; echo $?
[ ')' '-a' '-n' '-n' ] 2>/dev/null; echo $?
[ ')' ')' '-n' ] 2>/dev/null; echo $?
[ '-z' '-o' '' '-z' ] 2>/dev/null; echo $?
[ ')' '!' '-o' '=' ] 2>/dev/null; echo $?
[ '-z' '(' '!' 'x' ] 2>/dev/null; echo $?
[ '' '' '-n' ')' ] 2>/dev/null; echo $?
[ '-z' '(' '-a' ')' ] 2>/dev/null; echo $?
[ '-n' '-z' '-e' '1' '=' '-a' ] 2>/dev/null; echo $?
[ '(' '-n' '-z' '=' ] 2>/dev/null; echo $?
[ '-o' '!' '=' 'x' ] 2>/dev/null; echo $?
[ '!' '(' '(' '=' ] 2>/dev/null; echo $?
[ '-z' 'x' '-z' ] 2>/dev/null; echo $?
[ 'x' '-o' 'x' '!' ] 2>/dev/null; echo $?
[ 'x' '!' '(' ] 2>/dev/null; echo $?
[ '' '-n' '' 'x' ] 2>/dev/null; echo $?
[ '-z' '(' ')' '(' ] 2>/dev/null; echo $?
[ '=' '-z' 'x' '!' ] 2>/dev/null; echo $?
[ '-z' '' '-a' '(' ] 2>/dev/null; echo $?
[ '!' '-o' '(' '-z' ] 2>/dev/null; echo $?
[ '-a' '!' '' '-n' ] 2>/dev/null; echo $?
[ '-z' 'x' '-z' '!' ] 2>/dev/null; echo $?
[ 'x' 'x' ')' ')' ] 2>/dev/null; echo $?
[ '-n' 'x' '-z' '-eq' '-n' '!' '-e' ] 2>/dev/null; echo $?
[ '=' '!' '-o' '-o' ] 2>/dev/null; echo $?
[ '!' ')' '-a' '-o' ] 2>/dev/null; echo $?
[ '-z' '!' '!' ')' ] 2>/dev/null; echo $?
[ 'x' '-n' '' '1' '=' ] 2>/dev/null; echo $?
[ '' '-n' '-eq' '1' '1' ] 2>/dev/null; echo $?
[ '-o' '!' '-z' '' ] 2>/dev/null; echo $?
[ '=' '=' ')' '-z' ] 2>/dev/null; echo $?"#, b"2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n0\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n1\n0\n2\n2\n2\n2\n2\n2\n2\n2\n2\n0\n2\n1\n2\n2\n1\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n0\n2\n2\n0\n2\n1\n2\n2\n2\n2\n2\n1\n2\n2\n2\n1\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n0\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n1\n2\n0\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n1\n2\n2\n2\n0\n1\n2\n2\n0\n2\n2\n2\n2\n2\n0\n2\n1\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n1\n2\n0\n2\n2\n2\n2\n2\n2\n2\n1\n2\n2\n1\n2\n2\n1\n2\n2\n2\n0\n1\n2\n0\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n1\n2\n2\n1\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n1\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n1\n2\n2\n2\n2\n2\n1\n2\n0\n2\n2\n2\n2\n2\n2\n2\n2\n2\n0\n2\n2\n2\n2\n2\n2\n0\n2\n2\n2\n2\n2\n2\n1\n1\n2\n2\n2\n2\n0\n1\n2\n2\n2\n2\n2\n2\n2\n0\n0\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n1\n2\n2\n2\n2\n2\n2\n2\n1\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n0\n2\n2\n2\n2\n2\n2\n0\n2\n2\n2\n2\n2\n1\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n2\n1\n2\n2\n2\n2\n2\n", 0),
        ],
    );
}

#[test]
fn printf_floats_are_exact_long_double() {
    // bash 5.2 (glibc, x87 long double): `%f %e %g` round the exact binary value; a sample of 300 of 8694 forms.
    check(
        "pflt",
        &[
            (r#"printf '%e\n' '0.545'
printf '%.3g\n' '1e-10'
printf '%.0f\n' '0.570'
printf '%.10f\n' '0.030'
printf '%-10.2f|\n' '65536.00001'
printf '%#.3e\n' '1E5'
printf '%.2f\n' '0.305'
printf '%.0g\n' '20848'
printf '%.10g\n' '0.135'
printf '%#.0f\n' '1.0000000000000000001'
printf '%E\n' '357.64407'
printf '%e\n' '0.760'
printf '%-10.2f|\n' '-inf'
printf '%.10g\n' '0.01'
printf '%.10g\n' '69044.684576'
printf '%f\n' '0.880'
printf '%.17g\n' '0.735'
printf '%g\n' '0.585'
printf '%-10.2f|\n' '0.305'
printf '%.10g\n' '549'
printf '%5.1f\n' '0.430'
printf '%.0e\n' '0.845'
printf '%010.2f\n' 'inf'
printf '%f\n' '0.335'
printf '%.10g\n' '0'
printf '%#.0f\n' '0.615'
printf '%F\n' '0.255'
printf '%-10.2f|\n' '0.930'
printf '%.20f\n' '0.695'
printf '%+.2f\n' '74.8398'
printf '%.3g\n' '0.835'
printf '%10.3f\n' '99.5'
printf '%5.1f\n' '0.490'
printf '%.2f\n' '0.025'
printf '%.0g\n' '0.240'
printf '%f\n' '0.7'
printf '%.10f\n' '6179.3'
printf '%.10g\n' '0.160'
printf '%.0e\n' '0.335'
printf '%10.3f\n' '0.00'
printf '%#.3e\n' '0.0'
printf '%F\n' '0.295'
printf '%.15g\n' '0.250'
printf '%.3g\n' '27231.013'
printf '%.10g\n' '0.047'
printf '%g\n' '0.345'
printf '%.2f\n' '.5'
printf '%.10g\n' '0.200'
printf '%010.2f\n' '0.675'
printf '%#g\n' '0.290'
printf '%.0g\n' '0.5'
printf '%.3g\n' '100000'
printf '%#g\n' '0.560'
printf '%.3g\n' '549'
printf '%.3e\n' '0.00'
printf '%.0e\n' '999999'
printf '%E\n' '0.095'
printf '%.0f\n' '0.9999995'
printf '%.15g\n' '0.990'
printf '%#.3e\n' '0.225'
printf '%E\n' '0.865'
printf '%f\n' '0.99999949999'
printf '%g\n' '0.145'
printf '%-10.2f|\n' '1e5'
printf '%.0f\n' '0.355'
printf '%F\n' '0.360'
printf '%#.3e\n' '1.0000000000000000001'
printf '%.17g\n' '0.305'
printf '%F\n' '0.980'
printf '%.20f\n' '27231.013'
printf '%10.3f\n' '0.510'
printf '%.10g\n' '98670'
printf '%E\n' '0.7'
printf '%e\n' '0.745'
printf '%e\n' '0.535'
printf '%#.3e\n' '0.035'
printf '%G\n' '5.'
printf '%.0e\n' '0.480'
printf '%E\n' '0.320'
printf '%.10f\n' '279.482'
printf '%.0g\n' '0.355'
printf '%.3g\n' '0.035'
printf '%.3e\n' '0.445'
printf '%010.2f\n' '0.740'
printf '%.17g\n' '0.590'
printf '%.15g\n' '0.790'
printf '%.0e\n' '8262.96'
printf '%E\n' '16.648448'
printf '%f\n' '0.660'
printf '%.0g\n' '1e-300'
printf '%F\n' '0.740'
printf '%.3g\n' '0.485'
printf '%.10g\n' '0.495'
printf '%f\n' '0.620'
printf '%.0e\n' '100000'
printf '%f\n' '0.480'
printf '%.3e\n' '295.863807'
printf '%+.2f\n' '0.385'
printf '%.17g\n' '0.395'
printf '%F\n' '1.005'
printf '%.3g\n' '0.855'
printf '%#.0f\n' '31826.483'
printf '%#.3e\n' '0.030'
printf '%.0e\n' '0.360'
printf '%010.2f\n' '0.065'
printf '%#.3e\n' '0.310'
printf '%.10f\n' '1E5'
printf '%.3g\n' '327.21220'
printf '%010.2f\n' '0.195'
printf '%10.3f\n' '67.7934'
printf '%10.3f\n' '0.215'
printf '%#.0f\n' '0.735'
printf '%.17g\n' '0.125'
printf '%.3e\n' '0.100'
printf '%.10f\n' '0.465'
printf '%#.0f\n' '0.055'
printf '%#.0f\n' '0.9999995'
printf '%.3e\n' '0.960'
printf '%5.1f\n' '0.615'
printf '%.10f\n' '0.505'
printf '%F\n' '0.415'
printf '%010.2f\n' '.5'
printf '%G\n' '0.065'
printf '%.10f\n' '0.515'
printf '%G\n' '0.245'
printf '%F\n' '0.525'
printf '%E\n' '0.385'
printf '%F\n' '0.280'
printf '%.0g\n' '0.710'
printf '%.0e\n' '0.640'
printf '%.10g\n' '0.5'
printf '%.10g\n' '357.64407'
printf '%.0g\n' '0.870'
printf '%.2f\n' '51249.60'
printf '%#.0f\n' '0.155'
printf '%.3g\n' '0.980'
printf '%.17g\n' '82205.500'
printf '%f\n' ' 7.5'
printf '%.0f\n' '0.435'
printf '%e\n' '0.295'
printf '%.3g\n' '0.105'
printf '%.2f\n' '0.7'
printf '%.20f\n' '0.345'
printf '%.20f\n' '0.560'
printf '%#g\n' '0.300'
printf '%E\n' '0.525'
printf '%.15g\n' '0.380'
printf '%#g\n' '1e4000'
printf '%e\n' '0.315'
printf '%.0e\n' '295.863807'
printf '%.10f\n' '0.635'
printf '%10.3f\n' '0.345'
printf '%.10g\n' '4.35'
printf '%.10f\n' '0.770'
printf '%#.3e\n' '0.380'
printf '%#.0f\n' '0'
printf '% f\n' '0.910'
printf '%.17g\n' '0.375'
printf '%-10.2f|\n' '99.55'
printf '%G\n' '74.8398'
printf '%.3e\n' '0.220'
printf '%e\n' '0.150'
printf '%.10f\n' '0.055'
printf '%.17g\n' '0.500'
printf '%F\n' '0.0'
printf '%F\n' '0.640'
printf '%5.1f\n' '0.440'
printf '%.10f\n' '0.3'
printf '%.2f\n' '0.975'
printf '%G\n' '0.010'
printf '%.2f\n' '0.640'
printf '%#.0f\n' '0.800'
printf '% f\n' '0.150'
printf '%.17g\n' '.5'
printf '%.3g\n' '0.735'
printf '%.0f\n' '0.635'
printf '%.3e\n' '0.125'
printf '%.0f\n' '29.879'
printf '%.10g\n' '0.060'
printf '%.20f\n' '0.120'
printf '%f\n' '0.420'
printf '%.2f\n' '0.0363'
printf '%f\n' '0.885'
printf '%e\n' '0.805'
printf '%.1f\n' '0.685'
printf '%G\n' '279.482'
printf '%.0f\n' '0.1'
printf '%.17g\n' '16.648448'
printf '%#.3e\n' '1e-300'
printf '% f\n' '0.045'
printf '%.0g\n' '1.005'
printf '%#.3e\n' '0.675'
printf '%.3g\n' '0.020'
printf '%f\n' '31'
printf '%.3g\n' '0.755'
printf '%#.0f\n' '5e-324'
printf '%#.0f\n' '0.575'
printf '%e\n' '1.5'
printf '%.1f\n' '0.850'
printf '%.17g\n' '0.120'
printf '%.15g\n' '0.850'
printf '%.0f\n' '16.648448'
printf '% f\n' '0.115'
printf '%010.2f\n' '0.815'
printf '%G\n' '0.835'
printf '%.3g\n' '0.355'
printf '%G\n' '0.910'
printf '%.15g\n' '0.99999949999'
printf '%+.2f\n' '0.920'
printf '%.15g\n' '12345.6789'
printf '%.20f\n' '0.08'
printf '%+.2f\n' '18446744073709551615.5'
printf '%.20f\n' '0.540'
printf '%#.0f\n' '0.715'
printf '%+.2f\n' '2.5'
printf '%.17g\n' '0.065'
printf '%+.2f\n' '0.700'
printf '%+.2f\n' '0'
printf '%e\n' '1e15'
printf '% f\n' '0.840'
printf '%5.1f\n' '0.9552'
printf '%.17g\n' '5e-324'
printf '%5.1f\n' '0.680'
printf '%.10g\n' '0.710'
printf '%#.3e\n' '0.535'
printf '%010.2f\n' '0.745'
printf '%.3g\n' '0.765'
printf '%.20f\n' '1234567'
printf '%.10g\n' '27567.64'
printf '%#g\n' '0.880'
printf '%.0f\n' '0.845'
printf '%f\n' '20848'
printf '%E\n' '0.545'
printf '%10.3f\n' '0.100'
printf '%.15g\n' '17'
printf '%G\n' '0.070'
printf '%#g\n' '141202.41'
printf '%.0f\n' '0.65'
printf '%10.3f\n' '.5'
printf '%.3g\n' '0.830'
printf '%.0f\n' '0.250'
printf '%010.2f\n' '0.840'
printf '%.0g\n' '0.015'
printf '%f\n' '0.025'
printf '%#.0f\n' '0.515'
printf '%.3g\n' '0.280'
printf '%g\n' '0.065'
printf '%-10.2f|\n' '0.190'
printf '%.1f\n' '25.5445'
printf '%.2f\n' '5.'
printf '%.15g\n' '0.08'
printf '%.2f\n' '0.215'
printf '%.3e\n' '65536.00001'
printf '%.17g\n' '1e-10'
printf '%+.2f\n' '4.45'
printf '%E\n' '0.130'
printf '%.3g\n' '0.750'
printf '%#.3e\n' '0.00'
printf '%#.0f\n' 'INF'
printf '%#.0f\n' '0.685'
printf '%.1f\n' '-99.55'
printf '%.0g\n' '16928.990912'
printf '%010.2f\n' '0.00'
printf '%E\n' '0.400'
printf '%.20f\n' '1000000'
printf '%.1f\n' '31'
printf '%.0e\n' '0.510'
printf '%+.2f\n' '0.450'
printf '%.3e\n' '0.655'
printf '%.1f\n' '0.905'
printf '%#.3e\n' '0.915'
printf '%5.1f\n' '0.480'
printf '%.3e\n' '0.08'
printf '%F\n' '4.45'
printf '%.2f\n' '0.720'
printf '%5.1f\n' '0.830'
printf '%e\n' '0.765'
printf '%.20f\n' '0.785'
printf '%.15g\n' '-0.005'
printf '%.10g\n' '0.490'
printf '%.2f\n' '0.030'
printf '%.0g\n' '0.835'
printf '%F\n' '0.0363'
printf '%010.2f\n' '29.879'
printf '%.3g\n' '0.955'
printf '%.10f\n' '0.000'
printf '%E\n' '0.350'
printf '%f\n' '0.550'
printf '%-10.2f|\n' '0.175'
printf '%.0g\n' '0.915'
printf '%.10g\n' '0.225'
printf '%g\n' '0.110'
printf '%.0e\n' '0.575'
printf '%.15g\n' '0.015'
printf '%F\n' '0.930'
printf '%.10g\n' '0.009'
printf '%e\n' '0.005'
printf '%.1f\n' '1e-10'
printf '%G\n' '0.870'
printf '%010.2f\n' '0.925'"#, b"5.450000e-01\n1e-10\n1\n0.0300000000\n65536.00  |\n1.000e+05\n0.31\n2e+04\n0.135\n1.\n3.576441E+02\n7.600000e-01\n-inf      |\n0.01\n69044.68458\n0.880000\n0.735\n0.585\n0.31      |\n549\n  0.4\n8e-01\n       inf\n0.335000\n0\n1.\n0.255000\n0.93      |\n0.69499999999999999999\n+74.84\n0.835\n    99.500\n  0.5\n0.03\n0.2\n0.700000\n6179.3000000000\n0.16\n3e-01\n     0.000\n0.000e+00\n0.295000\n0.25\n2.72e+04\n0.047\n0.345\n0.50\n0.2\n0000000.68\n0.290000\n0.5\n1e+05\n0.560000\n549\n0.000e+00\n1e+06\n9.500000E-02\n1\n0.99\n2.250e-01\n8.650000E-01\n0.999999\n0.145\n100000.00 |\n0\n0.360000\n1.000e+00\n0.305\n0.980000\n27231.01299999999999990052\n     0.510\n98670\n7.000000E-01\n7.450000e-01\n5.350000e-01\n3.500e-02\n5\n5e-01\n3.200000E-01\n279.4820000000\n0.4\n0.035\n4.450e-01\n0000000.74\n0.59\n0.79\n8e+03\n1.664845E+01\n0.660000\n1e-300\n0.740000\n0.485\n0.495\n0.620000\n1e+05\n0.480000\n2.959e+02\n+0.38\n0.395\n1.005000\n0.855\n31826.\n3.000e-02\n4e-01\n0000000.06\n3.100e-01\n100000.0000000000\n327\n0000000.19\n    67.793\n     0.215\n1.\n0.125\n1.000e-01\n0.4650000000\n0.\n1.\n9.600e-01\n  0.6\n0.5050000000\n0.415000\n0000000.50\n0.065\n0.5150000000\n0.245\n0.525000\n3.850000E-01\n0.280000\n0.7\n6e-01\n0.5\n357.64407\n0.9\n51249.60\n0.\n0.98\n82205.5\n7.500000\n0\n2.950000e-01\n0.105\n0.70\n0.34500000000000000000\n0.56000000000000000000\n0.300000\n5.250000E-01\n0.38\n1.00000e+4000\n3.150000e-01\n3e+02\n0.6350000000\n     0.345\n4.35\n0.7700000000\n3.800e-01\n0.\n 0.910000\n0.375\n99.55     |\n74.8398\n2.200e-01\n1.500000e-01\n0.0550000000\n0.5\n0.000000\n0.640000\n  0.4\n0.3000000000\n0.98\n0.01\n0.64\n1.\n 0.150000\n0.5\n0.735\n1\n1.250e-01\n30\n0.06\n0.12000000000000000000\n0.420000\n0.04\n0.885000\n8.050000e-01\n0.7\n279.482\n0\n16.648448\n1.000e-300\n 0.045000\n1\n6.750e-01\n0.02\n31.000000\n0.755\n0.\n1.\n1.500000e+00\n0.9\n0.12\n0.85\n17\n 0.115000\n0000000.81\n0.835\n0.355\n0.91\n0.99999949999\n+0.92\n12345.6789\n0.08000000000000000000\n+18446744073709551616.00\n0.54000000000000000002\n1.\n+2.50\n0.065\n+0.70\n+0.00\n1.000000e+15\n 0.840000\n  1.0\n5e-324\n  0.7\n0.71\n5.350e-01\n0000000.75\n0.765\n1234567.00000000000000000000\n27567.64\n0.880000\n1\n20848.000000\n5.450000E-01\n     0.100\n17\n0.07\n141202.\n1\n     0.500\n0.83\n0\n0000000.84\n0.01\n0.025000\n1.\n0.28\n0.065\n0.19      |\n25.5\n5.00\n0.08\n0.22\n6.554e+04\n1e-10\n+4.45\n1.300000E-01\n0.75\n0.000e+00\ninf\n1.\n-99.6\n2e+04\n0000000.00\n4.000000E-01\n1000000.00000000000000000000\n31.0\n5e-01\n+0.45\n6.550e-01\n0.9\n9.150e-01\n  0.5\n8.000e-02\n4.450000\n0.72\n  0.8\n7.650000e-01\n0.78500000000000000002\n-0.005\n0.49\n0.03\n0.8\n0.036300\n0000029.88\n0.955\n0.0000000000\n3.500000E-01\n0.550000\n0.17      |\n0.9\n0.225\n0.11\n6e-01\n0.015\n0.930000\n0.009\n5.000000e-03\n0.0\n0.87\n0000000.93\n", 0),
        ],
    );
}

#[test]
fn printf_integers_clamp_and_wrap() {
    // bash 5.2: `%d %i` clamp to the signed range, `%u %x %X %o` read an unsigned value and wrap a negative; a range warning is not a failure.
    check(
        "pint",
        &[
            (r#"{
printf '%d\n' '0'; echo rc=$?
printf '%i\n' '0'; echo rc=$?
printf '%u\n' '0'; echo rc=$?
printf '%x\n' '0'; echo rc=$?
printf '%X\n' '0'; echo rc=$?
printf '%o\n' '0'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"0\nrc=0\n0\nrc=0\n0\nrc=0\n0\nrc=0\n0\nrc=0\n0\nrc=0\n", 0),
            (r#"{
printf '%d\n' '9223372036854775807'; echo rc=$?
printf '%i\n' '9223372036854775807'; echo rc=$?
printf '%u\n' '9223372036854775807'; echo rc=$?
printf '%x\n' '9223372036854775807'; echo rc=$?
printf '%X\n' '9223372036854775807'; echo rc=$?
printf '%o\n' '9223372036854775807'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"9223372036854775807\nrc=0\n9223372036854775807\nrc=0\n9223372036854775807\nrc=0\n7fffffffffffffff\nrc=0\n7FFFFFFFFFFFFFFF\nrc=0\n777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '9223372036854775808'; echo rc=$?
printf '%i\n' '9223372036854775808'; echo rc=$?
printf '%u\n' '9223372036854775808'; echo rc=$?
printf '%x\n' '9223372036854775808'; echo rc=$?
printf '%X\n' '9223372036854775808'; echo rc=$?
printf '%o\n' '9223372036854775808'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: 9223372036854775808: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 9223372036854775808: Numerical result out of range\n9223372036854775807\nrc=0\n9223372036854775808\nrc=0\n8000000000000000\nrc=0\n8000000000000000\nrc=0\n1000000000000000000000\nrc=0\n", 0),
            (r#"{
printf '%d\n' '18446744073709551615'; echo rc=$?
printf '%i\n' '18446744073709551615'; echo rc=$?
printf '%u\n' '18446744073709551615'; echo rc=$?
printf '%x\n' '18446744073709551615'; echo rc=$?
printf '%X\n' '18446744073709551615'; echo rc=$?
printf '%o\n' '18446744073709551615'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: 18446744073709551615: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 18446744073709551615: Numerical result out of range\n9223372036854775807\nrc=0\n18446744073709551615\nrc=0\nffffffffffffffff\nrc=0\nFFFFFFFFFFFFFFFF\nrc=0\n1777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '18446744073709551616'; echo rc=$?
printf '%i\n' '18446744073709551616'; echo rc=$?
printf '%u\n' '18446744073709551616'; echo rc=$?
printf '%x\n' '18446744073709551616'; echo rc=$?
printf '%X\n' '18446744073709551616'; echo rc=$?
printf '%o\n' '18446744073709551616'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: 18446744073709551616: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 18446744073709551616: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 18446744073709551616: Numerical result out of range\n18446744073709551615\nrc=0\nprintf: warning: 18446744073709551616: Numerical result out of range\nffffffffffffffff\nrc=0\nprintf: warning: 18446744073709551616: Numerical result out of range\nFFFFFFFFFFFFFFFF\nrc=0\nprintf: warning: 18446744073709551616: Numerical result out of range\n1777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '-9223372036854775808'; echo rc=$?
printf '%i\n' '-9223372036854775808'; echo rc=$?
printf '%u\n' '-9223372036854775808'; echo rc=$?
printf '%x\n' '-9223372036854775808'; echo rc=$?
printf '%X\n' '-9223372036854775808'; echo rc=$?
printf '%o\n' '-9223372036854775808'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"-9223372036854775808\nrc=0\n-9223372036854775808\nrc=0\n9223372036854775808\nrc=0\n8000000000000000\nrc=0\n8000000000000000\nrc=0\n1000000000000000000000\nrc=0\n", 0),
            (r#"{
printf '%d\n' '-9223372036854775809'; echo rc=$?
printf '%i\n' '-9223372036854775809'; echo rc=$?
printf '%u\n' '-9223372036854775809'; echo rc=$?
printf '%x\n' '-9223372036854775809'; echo rc=$?
printf '%X\n' '-9223372036854775809'; echo rc=$?
printf '%o\n' '-9223372036854775809'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: -9223372036854775809: Numerical result out of range\n-9223372036854775808\nrc=0\nprintf: warning: -9223372036854775809: Numerical result out of range\n-9223372036854775808\nrc=0\n9223372036854775807\nrc=0\n7fffffffffffffff\nrc=0\n7FFFFFFFFFFFFFFF\nrc=0\n777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '-1'; echo rc=$?
printf '%i\n' '-1'; echo rc=$?
printf '%u\n' '-1'; echo rc=$?
printf '%x\n' '-1'; echo rc=$?
printf '%X\n' '-1'; echo rc=$?
printf '%o\n' '-1'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"-1\nrc=0\n-1\nrc=0\n18446744073709551615\nrc=0\nffffffffffffffff\nrc=0\nFFFFFFFFFFFFFFFF\nrc=0\n1777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '-18446744073709551615'; echo rc=$?
printf '%i\n' '-18446744073709551615'; echo rc=$?
printf '%u\n' '-18446744073709551615'; echo rc=$?
printf '%x\n' '-18446744073709551615'; echo rc=$?
printf '%X\n' '-18446744073709551615'; echo rc=$?
printf '%o\n' '-18446744073709551615'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: -18446744073709551615: Numerical result out of range\n-9223372036854775808\nrc=0\nprintf: warning: -18446744073709551615: Numerical result out of range\n-9223372036854775808\nrc=0\n1\nrc=0\n1\nrc=0\n1\nrc=0\n1\nrc=0\n", 0),
            (r#"{
printf '%d\n' '-18446744073709551616'; echo rc=$?
printf '%i\n' '-18446744073709551616'; echo rc=$?
printf '%u\n' '-18446744073709551616'; echo rc=$?
printf '%x\n' '-18446744073709551616'; echo rc=$?
printf '%X\n' '-18446744073709551616'; echo rc=$?
printf '%o\n' '-18446744073709551616'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: -18446744073709551616: Numerical result out of range\n-9223372036854775808\nrc=0\nprintf: warning: -18446744073709551616: Numerical result out of range\n-9223372036854775808\nrc=0\nprintf: warning: -18446744073709551616: Numerical result out of range\n18446744073709551615\nrc=0\nprintf: warning: -18446744073709551616: Numerical result out of range\nffffffffffffffff\nrc=0\nprintf: warning: -18446744073709551616: Numerical result out of range\nFFFFFFFFFFFFFFFF\nrc=0\nprintf: warning: -18446744073709551616: Numerical result out of range\n1777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '0x7fffffffffffffff'; echo rc=$?
printf '%i\n' '0x7fffffffffffffff'; echo rc=$?
printf '%u\n' '0x7fffffffffffffff'; echo rc=$?
printf '%x\n' '0x7fffffffffffffff'; echo rc=$?
printf '%X\n' '0x7fffffffffffffff'; echo rc=$?
printf '%o\n' '0x7fffffffffffffff'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"9223372036854775807\nrc=0\n9223372036854775807\nrc=0\n9223372036854775807\nrc=0\n7fffffffffffffff\nrc=0\n7FFFFFFFFFFFFFFF\nrc=0\n777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '0x8000000000000000'; echo rc=$?
printf '%i\n' '0x8000000000000000'; echo rc=$?
printf '%u\n' '0x8000000000000000'; echo rc=$?
printf '%x\n' '0x8000000000000000'; echo rc=$?
printf '%X\n' '0x8000000000000000'; echo rc=$?
printf '%o\n' '0x8000000000000000'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: 0x8000000000000000: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 0x8000000000000000: Numerical result out of range\n9223372036854775807\nrc=0\n9223372036854775808\nrc=0\n8000000000000000\nrc=0\n8000000000000000\nrc=0\n1000000000000000000000\nrc=0\n", 0),
            (r#"{
printf '%d\n' '0xffffffffffffffff'; echo rc=$?
printf '%i\n' '0xffffffffffffffff'; echo rc=$?
printf '%u\n' '0xffffffffffffffff'; echo rc=$?
printf '%x\n' '0xffffffffffffffff'; echo rc=$?
printf '%X\n' '0xffffffffffffffff'; echo rc=$?
printf '%o\n' '0xffffffffffffffff'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: 0xffffffffffffffff: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 0xffffffffffffffff: Numerical result out of range\n9223372036854775807\nrc=0\n18446744073709551615\nrc=0\nffffffffffffffff\nrc=0\nFFFFFFFFFFFFFFFF\nrc=0\n1777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '0x10000000000000000'; echo rc=$?
printf '%i\n' '0x10000000000000000'; echo rc=$?
printf '%u\n' '0x10000000000000000'; echo rc=$?
printf '%x\n' '0x10000000000000000'; echo rc=$?
printf '%X\n' '0x10000000000000000'; echo rc=$?
printf '%o\n' '0x10000000000000000'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: 0x10000000000000000: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 0x10000000000000000: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 0x10000000000000000: Numerical result out of range\n18446744073709551615\nrc=0\nprintf: warning: 0x10000000000000000: Numerical result out of range\nffffffffffffffff\nrc=0\nprintf: warning: 0x10000000000000000: Numerical result out of range\nFFFFFFFFFFFFFFFF\nrc=0\nprintf: warning: 0x10000000000000000: Numerical result out of range\n1777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '01777777777777777777777'; echo rc=$?
printf '%i\n' '01777777777777777777777'; echo rc=$?
printf '%u\n' '01777777777777777777777'; echo rc=$?
printf '%x\n' '01777777777777777777777'; echo rc=$?
printf '%X\n' '01777777777777777777777'; echo rc=$?
printf '%o\n' '01777777777777777777777'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: 01777777777777777777777: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 01777777777777777777777: Numerical result out of range\n9223372036854775807\nrc=0\n18446744073709551615\nrc=0\nffffffffffffffff\nrc=0\nFFFFFFFFFFFFFFFF\nrc=0\n1777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '02000000000000000000000'; echo rc=$?
printf '%i\n' '02000000000000000000000'; echo rc=$?
printf '%u\n' '02000000000000000000000'; echo rc=$?
printf '%x\n' '02000000000000000000000'; echo rc=$?
printf '%X\n' '02000000000000000000000'; echo rc=$?
printf '%o\n' '02000000000000000000000'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: 02000000000000000000000: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 02000000000000000000000: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 02000000000000000000000: Numerical result out of range\n18446744073709551615\nrc=0\nprintf: warning: 02000000000000000000000: Numerical result out of range\nffffffffffffffff\nrc=0\nprintf: warning: 02000000000000000000000: Numerical result out of range\nFFFFFFFFFFFFFFFF\nrc=0\nprintf: warning: 02000000000000000000000: Numerical result out of range\n1777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '99999999999999999999999'; echo rc=$?
printf '%i\n' '99999999999999999999999'; echo rc=$?
printf '%u\n' '99999999999999999999999'; echo rc=$?
printf '%x\n' '99999999999999999999999'; echo rc=$?
printf '%X\n' '99999999999999999999999'; echo rc=$?
printf '%o\n' '99999999999999999999999'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: 99999999999999999999999: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 99999999999999999999999: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: 99999999999999999999999: Numerical result out of range\n18446744073709551615\nrc=0\nprintf: warning: 99999999999999999999999: Numerical result out of range\nffffffffffffffff\nrc=0\nprintf: warning: 99999999999999999999999: Numerical result out of range\nFFFFFFFFFFFFFFFF\nrc=0\nprintf: warning: 99999999999999999999999: Numerical result out of range\n1777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '-99999999999999999999999'; echo rc=$?
printf '%i\n' '-99999999999999999999999'; echo rc=$?
printf '%u\n' '-99999999999999999999999'; echo rc=$?
printf '%x\n' '-99999999999999999999999'; echo rc=$?
printf '%X\n' '-99999999999999999999999'; echo rc=$?
printf '%o\n' '-99999999999999999999999'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: -99999999999999999999999: Numerical result out of range\n-9223372036854775808\nrc=0\nprintf: warning: -99999999999999999999999: Numerical result out of range\n-9223372036854775808\nrc=0\nprintf: warning: -99999999999999999999999: Numerical result out of range\n18446744073709551615\nrc=0\nprintf: warning: -99999999999999999999999: Numerical result out of range\nffffffffffffffff\nrc=0\nprintf: warning: -99999999999999999999999: Numerical result out of range\nFFFFFFFFFFFFFFFF\nrc=0\nprintf: warning: -99999999999999999999999: Numerical result out of range\n1777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '+18446744073709551615'; echo rc=$?
printf '%i\n' '+18446744073709551615'; echo rc=$?
printf '%u\n' '+18446744073709551615'; echo rc=$?
printf '%x\n' '+18446744073709551615'; echo rc=$?
printf '%X\n' '+18446744073709551615'; echo rc=$?
printf '%o\n' '+18446744073709551615'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: +18446744073709551615: Numerical result out of range\n9223372036854775807\nrc=0\nprintf: warning: +18446744073709551615: Numerical result out of range\n9223372036854775807\nrc=0\n18446744073709551615\nrc=0\nffffffffffffffff\nrc=0\nFFFFFFFFFFFFFFFF\nrc=0\n1777777777777777777777\nrc=0\n", 0),
            (r#"{
printf '%d\n' '12abc'; echo rc=$?
printf '%i\n' '12abc'; echo rc=$?
printf '%u\n' '12abc'; echo rc=$?
printf '%x\n' '12abc'; echo rc=$?
printf '%X\n' '12abc'; echo rc=$?
printf '%o\n' '12abc'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: 12abc: invalid number\n12\nrc=1\nprintf: 12abc: invalid number\n12\nrc=1\nprintf: 12abc: invalid number\n12\nrc=1\nprintf: 12abc: invalid number\nc\nrc=1\nprintf: 12abc: invalid number\nC\nrc=1\nprintf: 12abc: invalid number\n14\nrc=1\n", 0),
            (r#"{
printf '%d\n' '0x'; echo rc=$?
printf '%i\n' '0x'; echo rc=$?
printf '%u\n' '0x'; echo rc=$?
printf '%x\n' '0x'; echo rc=$?
printf '%X\n' '0x'; echo rc=$?
printf '%o\n' '0x'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: 0x: invalid hex number\n0\nrc=1\nprintf: 0x: invalid hex number\n0\nrc=1\nprintf: 0x: invalid hex number\n0\nrc=1\nprintf: 0x: invalid hex number\n0\nrc=1\nprintf: 0x: invalid hex number\n0\nrc=1\nprintf: 0x: invalid hex number\n0\nrc=1\n", 0),
            (r#"{
printf '%d\n' 'abc'; echo rc=$?
printf '%i\n' 'abc'; echo rc=$?
printf '%u\n' 'abc'; echo rc=$?
printf '%x\n' 'abc'; echo rc=$?
printf '%X\n' 'abc'; echo rc=$?
printf '%o\n' 'abc'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: abc: invalid number\n0\nrc=1\nprintf: abc: invalid number\n0\nrc=1\nprintf: abc: invalid number\n0\nrc=1\nprintf: abc: invalid number\n0\nrc=1\nprintf: abc: invalid number\n0\nrc=1\nprintf: abc: invalid number\n0\nrc=1\n", 0),
            (r#"{
printf '%d\n' '1.5'; echo rc=$?
printf '%i\n' '1.5'; echo rc=$?
printf '%u\n' '1.5'; echo rc=$?
printf '%x\n' '1.5'; echo rc=$?
printf '%X\n' '1.5'; echo rc=$?
printf '%o\n' '1.5'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: 1.5: invalid number\n1\nrc=1\nprintf: 1.5: invalid number\n1\nrc=1\nprintf: 1.5: invalid number\n1\nrc=1\nprintf: 1.5: invalid number\n1\nrc=1\nprintf: 1.5: invalid number\n1\nrc=1\nprintf: 1.5: invalid number\n1\nrc=1\n", 0),
            (r#"{
printf '%d\n' "'a"; echo rc=$?
printf '%i\n' "'a"; echo rc=$?
printf '%u\n' "'a"; echo rc=$?
printf '%x\n' "'a"; echo rc=$?
printf '%X\n' "'a"; echo rc=$?
printf '%o\n' "'a"; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"97\nrc=0\n97\nrc=0\n97\nrc=0\n61\nrc=0\n61\nrc=0\n141\nrc=0\n", 0),
            (r#"{
printf '%d\n' " 12"; echo rc=$?
printf '%i\n' " 12"; echo rc=$?
printf '%u\n' " 12"; echo rc=$?
printf '%x\n' " 12"; echo rc=$?
printf '%X\n' " 12"; echo rc=$?
printf '%o\n' " 12"; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"12\nrc=0\n12\nrc=0\n12\nrc=0\nc\nrc=0\nC\nrc=0\n14\nrc=0\n", 0),
            (r#"{
printf '%d\n' '-0x8000000000000000'; echo rc=$?
printf '%i\n' '-0x8000000000000000'; echo rc=$?
printf '%u\n' '-0x8000000000000000'; echo rc=$?
printf '%x\n' '-0x8000000000000000'; echo rc=$?
printf '%X\n' '-0x8000000000000000'; echo rc=$?
printf '%o\n' '-0x8000000000000000'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"-9223372036854775808\nrc=0\n-9223372036854775808\nrc=0\n9223372036854775808\nrc=0\n8000000000000000\nrc=0\n8000000000000000\nrc=0\n1000000000000000000000\nrc=0\n", 0),
            (r#"{
printf '%d\n' '-0x8000000000000001'; echo rc=$?
printf '%i\n' '-0x8000000000000001'; echo rc=$?
printf '%u\n' '-0x8000000000000001'; echo rc=$?
printf '%x\n' '-0x8000000000000001'; echo rc=$?
printf '%X\n' '-0x8000000000000001'; echo rc=$?
printf '%o\n' '-0x8000000000000001'; echo rc=$?
} 2>&1 | sed -e 's/^bash: line [0-9]*: //' -e 's/^ish: //'"#, b"printf: warning: -0x8000000000000001: Numerical result out of range\n-9223372036854775808\nrc=0\nprintf: warning: -0x8000000000000001: Numerical result out of range\n-9223372036854775808\nrc=0\n9223372036854775807\nrc=0\n7fffffffffffffff\nrc=0\n7FFFFFFFFFFFFFFF\nrc=0\n777777777777777777777\nrc=0\n", 0),
        ],
    );
}

#[test]
fn printf_modifiers_nul_and_nonfinite() {
    // bash 5.2: length modifiers, `%c` of an empty argument, `inf` and `nan`, and a literal newline in the format.
    check(
        "pmisc",
        &[
            (
                r#"printf '%ld
' 5"#,
                b"5\n",
                0,
            ),
            (
                r#"printf '%F
' 1.5"#,
                b"1.500000\n",
                0,
            ),
            (
                r#"printf '%f
' inf"#,
                b"inf\n",
                0,
            ),
            (
                r#"printf '%f|%F|%e|%G\n' inf -inf nan NAN"#,
                b"inf|-INF|nan|NAN\n",
                0,
            ),
            (
                r#"printf '%8.2f|%-8f|%08f|\n' inf inf inf"#,
                b"     inf|inf     |     inf|\n",
                0,
            ),
            (
                r#"printf '%lld %hd %Lf %jd %zd %td\n' 1 2 3.5 4 5 6"#,
                b"1 2 3.500000 4 5 6\n",
                0,
            ),
            (
                r#"printf '%c' '' | wc -c; printf '%c' | wc -c; printf '%cX' '' | wc -c; printf '%c%c' a '' | wc -c"#,
                b"1\n1\n2\n2\n",
                0,
            ),
            (
                r#"printf '%s' '' | wc -c; printf '%5c|' '' | wc -c"#,
                b"0\n6\n",
                0,
            ),
            (
                r#"printf '%f\n' ' 1.5'; echo rc=$?"#,
                b"1.500000\nrc=0\n",
                0,
            ),
            (r#"printf '%f\n' ''; echo rc=$?"#, b"0.000000\nrc=0\n", 0),
            (r#"printf '%.3f %.3f\n' 1.0005 1.0015"#, b"1.001 1.002\n", 0),
            (r#"printf '%d %i\n' "'a" '"b'"#, b"97 98\n", 0),
            (r#"printf '%*d|%-*d|\n' 5 42 5 42"#, b"   42|42   |\n", 0),
            (r#"printf '%.*f|\n' 2 3.14159"#, b"3.14|\n", 0),
            (
                r#"printf '%5.1f|%5.1f|\n' 99.55 99.45"#,
                b" 99.6| 99.4|\n",
                0,
            ),
            (
                r#"printf '%.2f\n' 0.005 0.015 0.025 0.035 0.045"#,
                b"0.00\n0.01\n0.03\n0.04\n0.05\n",
                0,
            ),
            (
                r#"printf '%s\n' 'a' ; printf '%f %f\n' 1"#,
                b"a\n1.000000 0.000000\n",
                0,
            ),
        ],
    );
}

#[test]
fn wc_total_and_dash() {
    // GNU wc: the total counts operands, readable or not; `-` is named `-`.
    check(
        "wc",
        &[
            (r#"wc -l f1 nonexist; echo rc=$?"#, b"0 total\nrc=1\n", 0),
            (
                r#"echo x > f1; wc -l f1 nonexist; echo rc=$?"#,
                b"1 f1\n1 total\nrc=1\n",
                0,
            ),
            (
                r#"echo x > f1; wc -l nonexist f1 2>&1; echo rc=$?"#,
                b"wc: nonexist: No such file or directory\n1 f1\n1 total\nrc=1\n",
                0,
            ),
            (
                r#"wc -l nonexist nonexist2; echo rc=$?"#,
                b"0 total\nrc=1\n",
                0,
            ),
            (
                r#"echo x > f1; wc -l f1 nonexist nonexist2; echo rc=$?"#,
                b"1 f1\n1 total\nrc=1\n",
                0,
            ),
            (r#"echo x | wc -l -"#, b"1 -\n", 0),
            (
                r#"echo x | wc -l - -"#,
                b"      1 -\n      0 -\n      1 total\n",
                0,
            ),
            (
                r#"echo x > f1; echo y | wc -l f1 -"#,
                b"      1 f1\n      1 -\n      2 total\n",
                0,
            ),
            (r#"echo x | wc -c -"#, b"2 -\n", 0),
            (r#"echo x | wc -"#, b"      1       1       2 -\n", 0),
            (r#"echo x | wc -l"#, b"1\n", 0),
            (
                r#"echo x > f1; wc -lw f1 nonexist; echo rc=$?"#,
                b"1 1 f1\n1 1 total\nrc=1\n",
                0,
            ),
        ],
    );
}

#[test]
fn tr_class_rules() {
    // GNU tr: only [:upper:] and [:lower:] in SET2, and aligned with the opposite class in SET1.
    check(
        "tr",
        &[
            (
                r#"echo 12 34 | tr '[:digit:]' '[:alpha:]'; echo rc=$?"#,
                b"rc=1\n",
                0,
            ),
            (
                r#"echo 12 34 | tr '[:digit:]' 'a-z'; echo rc=$?"#,
                b"bc de\nrc=0\n",
                0,
            ),
            (
                r#"echo abc | tr '[:lower:]' '[:upper:]'; echo rc=$?"#,
                b"ABC\nrc=0\n",
                0,
            ),
            (
                r#"echo abc | tr '[:upper:]' '[:lower:]'; echo rc=$?"#,
                b"abc\nrc=0\n",
                0,
            ),
            (
                r#"echo abc | tr 'a-z' '[:upper:]'; echo rc=$?"#,
                b"rc=1\n",
                0,
            ),
            (
                r#"echo abc | tr '[:lower:]' 'A-Z'; echo rc=$?"#,
                b"ABC\nrc=0\n",
                0,
            ),
            (
                r#"echo abc | tr '[:alpha:]' 'x'; echo rc=$?"#,
                b"xxx\nrc=0\n",
                0,
            ),
            (
                r#"echo abc | tr '[:alpha:]' '[:alpha:]'; echo rc=$?"#,
                b"rc=1\n",
                0,
            ),
            (
                r#"echo abc | tr '[:lower:]' '[:alpha:]'; echo rc=$?"#,
                b"rc=1\n",
                0,
            ),
            (
                r#"echo abc | tr '[:lower:]' '[:digit:]'; echo rc=$?"#,
                b"rc=1\n",
                0,
            ),
            (
                r#"echo abc | tr 'abc' '[:upper:]'; echo rc=$?"#,
                b"rc=1\n",
                0,
            ),
            (
                r#"echo abc | tr 'abc' '[:alpha:]'; echo rc=$?"#,
                b"rc=1\n",
                0,
            ),
            (
                r#"echo abc | tr '[:alpha:]' 'XYZ'; echo rc=$?"#,
                b"ZZZ\nrc=0\n",
                0,
            ),
            (
                r#"echo abc | tr -d '[:alpha:]'; echo rc=$?"#,
                b"\nrc=0\n",
                0,
            ),
            (
                r#"echo abc 123 | tr -s '[:alpha:]' '[:upper:]'; echo rc=$?"#,
                b"rc=1\n",
                0,
            ),
            (
                r#"echo 'a b' | tr '[:space:]' '[:upper:]'; echo rc=$?"#,
                b"rc=1\n",
                0,
            ),
            (
                r#"echo abc | tr '[:blank:]' '[:lower:]'; echo rc=$?"#,
                b"rc=1\n",
                0,
            ),
        ],
    );
}

#[test]
fn basename_dirname_options() {
    // GNU basename and dirname: `-a`, `-s`, `-z`, `--`, missing and extra operands, leading slashes.
    check(
        "bd",
        &[
            (r#"basename -a; echo rc=$?"#, b"rc=1\n", 0),
            (r#"basename -- -a; echo rc=$?"#, b"-a\nrc=0\n", 0),
            (r#"basename -a a/b c/d; echo rc=$?"#, b"b\nd\nrc=0\n", 0),
            (
                r#"basename -s .c x/y.c z.c; echo rc=$?"#,
                b"y\nz\nrc=0\n",
                0,
            ),
            (
                r#"basename -a -s .c x/y.c z.c; echo rc=$?"#,
                b"y\nz\nrc=0\n",
                0,
            ),
            (
                r#"basename -as .c x/y.c z.c; echo rc=$?"#,
                b"y\nz\nrc=0\n",
                0,
            ),
            (
                r#"basename -zs .c x/y.c | wc -c; basename -z a/b | wc -c; basename -az a/b c/d | wc -c"#,
                b"2\n2\n4\n",
                0,
            ),
            (r#"basename -s .c; echo rc=$?"#, b"rc=1\n", 0),
            (r#"basename; echo rc=$?"#, b"rc=1\n", 0),
            (r#"basename a b c; echo rc=$?"#, b"rc=1\n", 0),
            (
                r#"basename a/b/ .b; basename .b .b; basename a.b b"#,
                b"b\n.b\na.\n",
                0,
            ),
            (r#"basename -s '' a.b; echo rc=$?"#, b"a.b\nrc=0\n", 0),
            (
                r#"basename --suffix=.b a.b; basename --multiple a/b c; echo rc=$?"#,
                b"a\nb\nc\nrc=0\n",
                0,
            ),
            (r#"basename -x a; echo rc=$?"#, b"rc=1\n", 0),
            (
                r#"basename -- a/b; basename -a -- -b -c; echo rc=$?"#,
                b"b\n-b\n-c\nrc=0\n",
                0,
            ),
            (
                r#"basename ''; echo rc=$?; basename /; basename //; basename ///a//"#,
                b"\nrc=0\n/\n/\na\n",
                0,
            ),
            (
                r#"dirname ///a; dirname //a; dirname //; dirname ///; dirname /; dirname a//b//; dirname //a//b"#,
                b"/\n/\n/\n/\n/\na\n//a\n",
                0,
            ),
            (
                r#"dirname /a; dirname a; dirname a/; dirname ''; dirname .; dirname ..; dirname a/b/c; dirname /a/b/"#,
                b"/\n.\n.\n.\n.\n.\na/b\n/a\n",
                0,
            ),
            (r#"dirname; echo rc=$?"#, b"rc=1\n", 0),
            (r#"dirname a b; echo rc=$?"#, b".\n.\nrc=0\n", 0),
            (
                r#"dirname -z a/b | wc -c; dirname -- -a/b; dirname -x a; echo rc=$?"#,
                b"2\n-a\nrc=1\n",
                0,
            ),
            (
                r#"dirname ///a//b; dirname ////; dirname /a//; dirname a///b"#,
                b"///a\n/\n/\na\n",
                0,
            ),
            (r#"echo a; exit -1"#, b"a\n", 255),
            (r#"cd '' ; echo rc=$?"#, b"rc=0\n", 0),
        ],
    );
}

#[test]
fn printf_q_is_not_a_length_modifier() {
    // bash 5.2: `%q` quotes for the shell; `%ld` and `%lld` take the number.
    check(
        "pq",
        &[
            (r#"printf '%q\n' 'a b'"#, b"a\\ b\n", 0),
            (r#"printf '%q\n' 'a"b$c`d'"#, b"a\\\"b\\$c\\`d\n", 0),
            (r#"printf '%q\n' 'a	b'"#, b"$'a\\tb'\n", 0),
            (r#"printf '%q\n'"#, b"''\n", 0),
            (r#"printf '%q|%s\n' 'x y' z"#, b"x\\ y|z\n", 0),
            (
                r#"printf '%ld|%lld|%hd|%5ld|%-5ld|\n' 1 2 3 4 5"#,
                b"1|2|3|    4|5    |\n",
                0,
            ),
        ],
    );
}

#[test]
fn sed_multiline_case_and_script_words() {
    // GNU sed 4.9: `s///M`, `\`` and `\'`, `\u\L`, and which words are files once -e is given.
    check(
        "sed",
        &[
            (
                r#"printf 'a\nb\n' | sed 'N;s/\`a/X/M;s/b\'"'"'/Y/M'"#,
                b"X\nY\n",
                0,
            ),
            (r#"printf 'a\nb\n' | sed 'N;s/^b/Y/M'"#, b"a\nY\n", 0),
            (r#"printf 'a\nb\n' | sed 'N;s/a$/Y/M'"#, b"Y\nb\n", 0),
            (r#"printf 'a\nb\n' | sed 'N;s/a$/Y/'"#, b"a\nb\n", 0),
            (r#"printf 'a\nb\n' | sed 'N;s/^/>/Mg'"#, b">a\n>b\n", 0),
            (r#"printf 'a\nb\n' | sed 'N;s/$/</Mg'"#, b"a<\nb<\n", 0),
            (r#"printf 'a\nb\n' | sed 'N;s/\`/>/Mg'"#, b">a\nb\n", 0),
            (r#"printf 'a\nb\n' | sed 'N;s/\'"'"'/</Mg'"#, b"a\nb<\n", 0),
            (
                r#"printf 'a\nb\n' | sed -E 'N;s/^(.)$/[\1]/Mg'"#,
                b"[a]\n[b]\n",
                0,
            ),
            (r#"printf 'one\n' | sed 's/one/\u\L&X/'"#, b"onex\n", 0),
            (r#"printf 'one\n' | sed 's/one/\L\u&X/'"#, b"Onex\n", 0),
            (r#"printf 'one\n' | sed 's/one/\u\U&x/'"#, b"ONEX\n", 0),
            (
                r#"printf 'one two\n' | sed 's/\(one\) \(two\)/\U\1\E \u\2/'"#,
                b"ONE Two\n",
                0,
            ),
            (r#"printf 'one\n' | sed 's/one/\u\E&X/'"#, b"oneX\n", 0),
            (r#"printf 'one\n' | sed 's/one/\l\U&X/'"#, b"ONEX\n", 0),
            (
                r#"printf 'a\nb\nc\nd\n' | sed -e '/b/a\' -e 'text'"#,
                b"a\nb\ntext\nc\nd\n",
                0,
            ),
            (
                r#"printf 'a\nb\n' | sed -e '1{a z' -e '}'"#,
                b"a\nz\nb\n",
                0,
            ),
            (r#"printf 'a\n' | sed -e 's/a/b/' -e 's/b/c/'"#, b"c\n", 0),
            (r#"printf 'a\n' | sed -e 'i\' -e 'hello'"#, b"hello\na\n", 0),
            (r#"printf 'a\n' | sed -e '1c\' -e 'hello'"#, b"hello\n", 0),
            (r#"printf 'a\nb\n' | sed -n -e '$p'"#, b"b\n", 0),
            (r#"printf 'a\nb\n' > f1; sed -e 'p' f1 -n"#, b"a\nb\n", 0),
            (r#"printf 'a\nb\n' > f1; sed p f1 -n"#, b"a\nb\n", 0),
            (r#"printf 'a\nb\n' > f1; sed -n f1 -e 2p"#, b"b\n", 0),
            (r#"printf 'a\nb\n' > f1; sed -n -e 2p f1 f1"#, b"b\n", 0),
        ],
    );
}

//! Round 60c: differences between the in-process POSIX engine (`ish -b`) and
//! bash and GNU sed/coreutils: sed commands inside `{ }` and after `!`, the hold
//! space, `n`/`N`/`D`; several redirections on one command; `export z+=x`;
//! `printf` `%b` with `\c` and `%q`; `cut -c` and `-b` counting bytes; the
//! order of `cat`'s errors and output when both go to one place; the status of
//! an expansion error in a `-c` string; and the refusal of `for ((...))`.
//!
//! Every expected stdout and exit status below was produced by GNU bash 5.2 and
//! GNU sed 4.9 / coreutils 9.7 on Debian with the same script, except where a
//! case says otherwise.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_r60c_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `ish -b -c <script>` in `dir`, feeding `stdin` when given. Returns
/// (raw stdout, stderr, exit code).
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

fn ish_bash(script: &str, dir: &Path) -> (String, String, i32) {
    let (out, err, code) = ish_raw(script, dir);
    (
        String::from_utf8_lossy(&out).replace("\r\n", "\n"),
        err,
        code,
    )
}

/// Each case runs in a fresh directory, so files one case makes do not reach
/// the next.
fn check(tag: &str, cases: &[(&str, &str, i32)]) {
    for (n, (script, want, want_code)) in cases.iter().enumerate() {
        let dir = scratch(&format!("{tag}{n}"));
        let (out, err, code) = ish_bash(script, &dir);
        assert_eq!(
            (out.as_str(), code),
            (*want, *want_code),
            "{script}: stderr {err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn sed_blocks_run_the_commands_gnu_runs() {
    // Each expected output is GNU sed 4.9 on Debian (bash 5.2 runs the pipeline).
    check(
        "sed_bl",
        &[
            (
                "printf 'a\\nb\\nc\\n' | sed -n '/b/{=;p}'",
                "2\nb\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed -n '2{l;p}'",
                "b$\nb\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed -n '2{n;p}'",
                "c\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed -n '2{h;p}'",
                "b\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '2{h;d}'",
                "a\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '2{h;d};${G}'",
                "a\nc\nb\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '2{x;p;x}'",
                "a\n\nb\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\ne\\n' | sed -n '2,4{/c/!p}'",
                "b\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\ne\\n' | sed '2,4{/c/!d}'",
                "a\nc\ne\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\ne\\n' | sed '/a/,/b/{/b/!s/$/!/}'",
                "a!\nb\nc\nd\ne\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\ne\\n' | sed -n '$!{n;p}'",
                "b\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\ne\\n' | sed -n 'N;P;D'",
                "a\nb\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\ne\\n' | sed '$!N;s/\\n/-/'",
                "a-b\nc-d\ne\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\ne\\n' | sed 'N;N;s/\\n/,/g'",
                "a,b,c\nd\ne\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '1!G;h;$p'",
                "d\nc\nb\na\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '1!G;h;$!d'",
                "d\nc\nb\na\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '$='",
                "4\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '='",
                "1\na\n2\nb\n3\nc\n4\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n 'l'",
                "a$\nb$\nc$\nd$\n",
                0,
            ),
            (
                "printf 'a\\tb\\\\c\\001\\303\\251\\n' | sed -n l",
                "a\\tb\\\\c\\001\\303\\251$\n",
                0,
            ),
            (
                "printf '%0100d\\n' 0 | sed -n l",
                "000000000000000000000000000000000000000000000000000000000000000000000\\\n0000000000000000000000000000000$\n",
                0,
            ),
            (
                "printf '%068d\\n' 0 | sed -n l",
                "00000000000000000000000000000000000000000000000000000000000000000000$\n",
                0,
            ),
            (
                "printf '%069d\\n' 0 | sed -n l",
                "000000000000000000000000000000000000000000000000000000000000000000000$\n",
                0,
            ),
            (
                "printf '%070d\\n' 0 | sed -n l",
                "000000000000000000000000000000000000000000000000000000000000000000000\\\n0$\n",
                0,
            ),
            (
                "printf 'abc\\nxyz\\n' | sed 'y/abcxyz/ABCXYZ/'",
                "ABC\nXYZ\n",
                0,
            ),
            (
                "printf 'a/b\\n' | sed 'y/\\//|/'",
                "a|b\n",
                0,
            ),
            (
                "printf 'a b\\n' | sed 'y,a ,A_,'",
                "A_b\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2,3!d'",
                "b\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2!d'",
                "b\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '2,3!p'",
                "a\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '/b/!s/$/ x/'",
                "a x\nb\nc x\nd x\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2{N;s/\\n/+/}'",
                "a\nb+c\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2q'",
                "a\nb\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2Q'",
                "a\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '2{p;q}'",
                "b\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2{a\\\nX\n}'",
                "a\nb\nX\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2{i\\\nX\n}'",
                "a\nX\nb\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2{c\\\nX\n}'",
                "a\nX\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2,3{c\\\nX\n}'",
                "a\nX\nX\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2,3c\\\nX'",
                "a\nX\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -e '2{' -e 'p' -e '}'",
                "a\nb\nb\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '1{2{p}}'",
                "a\nb\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '1,3{2,3{p}}'",
                "b\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '1,3{/b/,/c/{p}}'",
                "b\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '2{N;N;p}'",
                "b\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '3{N;N;p}'",
                "",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '3{N;N;s/\\n/_/g}'",
                "a\nb\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed 'n;d'",
                "a\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\ne\\n' | sed 'n;d'",
                "a\nc\ne\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\ne\\n' | sed '$!n;s/^/>/'",
                "a\n>b\nc\n>d\n>e\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed 'G'",
                "a\n\nb\n\nc\n\nd\n\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n 'H;${x;s/\\n/,/g;p}'",
                ",a,b,c,d\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n 'H;${g;p}'",
                "\na\nb\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed 'x'",
                "\na\nb\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n 'x;p'",
                "\na\nb\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '1h;2,$H;$!d;g'",
                "a\nb\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed 'h;s/./X/;G'",
                "X\na\nX\nb\nX\nc\nX\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '2{p;p}'",
                "b\nb\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc' | sed -n '$p'",
                "c",
                0,
            ),
            (
                "printf 'a\\nb\\nc' | sed '$!N;P;D'",
                "a\nb\nc",
                0,
            ),
            (
                "printf 'a\\nb\\nc' | sed 'G'",
                "a\n\nb\n\nc\n\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc' | sed -n '$='",
                "3\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc' | sed -n '$l'",
                "c$\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc' | sed '$a\\\nX'",
                "a\nb\nc\nX\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '/b/,/c/{=}'",
                "2\n3\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '2,3{p;p}'",
                "b\nb\nc\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2,3{s/./X/;n}'",
                "a\nX\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed 'N;P;D'",
                "a\nb\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\n\\n\\nb\\n\\n\\n\\nc\\n' | sed '/^$/N;/\\n$/D'",
                "a\n\nb\n\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '$!{h;d};x;G;p'",
                "c\nd\n",
                0,
            ),
            (
                "printf 'a b\\n' | sed 's/ /\\n/;P;D'",
                "a\nb\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '2{p}\n3{p}'",
                "b\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '2{p;};3{p;}'",
                "b\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n -e '2{p' -e '}'",
                "b\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2 ! d'",
                "b\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '2,3 ! d'",
                "b\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '/a/,/b/!p'",
                "c\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed '$!d'",
                "d\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '2!{p}'",
                "a\nc\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '1!{2!{p}}'",
                "c\nd\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed -n '2{=;l;p}'",
                "2\nb$\nb\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\nd\\n' | sed 'y/ab/xy/;2d'",
                "x\nc\nd\n",
                0,
            ),
            (
                "echo hello | sed 'y/el/ip/'",
                "hippo\n",
                0,
            ),
            (
                "echo hello | sed 'y/e/ab/'",
                "",
                1,
            ),
            (
                "echo hello | sed 'y/ee/ab/'",
                "hallo\n",
                0,
            ),
            (
                "echo hello | sed 'l;d'",
                "hello$\n",
                0,
            ),
            (
                "printf 'a\\nb\\n' | sed '1{h;d};G'",
                "b\na\n",
                0,
            ),
            (
                "printf 'a\\nb\\n' | sed -n '$!{N;P;D};p'",
                "a\nb\n",
                0,
            ),
            (
                "printf 'a\\nb\\n' | sed 'N;N;N'",
                "a\nb\n",
                0,
            ),
            (
                "printf 'a\\nb\\n' | sed -n 'N;N;p'",
                "",
                0,
            ),
            (
                "printf 'a\\nb\\n' | sed 'N;N;s/^/X/'",
                "a\nb\n",
                0,
            ),
            (
                "printf 'a\\nb\\n' | sed '$!N;$!D'",
                "a\nb\n",
                0,
            ),
            (
                "printf 'a\\n' | sed 'n;s/a/X/'",
                "a\n",
                0,
            ),
            (
                "printf 'a\\n' | sed -n 'n;p'",
                "",
                0,
            ),
            (
                "printf 'a\\n' | sed 'N;s/a/X/'",
                "a\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '2a\\\nX'",
                "a\nb\nX\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '2{a\\\nX\nn}'",
                "a\nb\nX\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '2{a\\\nX\nN}'",
                "a\nX\nb\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '1{a\\\nX\nd}'",
                "X\nb\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '1{a X\n}'",
                "a\nX\nb\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '1{i X\n}'",
                "X\na\nb\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '1{c X\n}'",
                "X\nb\nc\n",
                0,
            ),
            (
                "printf 'a\\nb\\nc\\n' | sed '1{h;G;G}'",
                "a\na\na\nb\nc\n",
                0,
            ),
        ],
    );
}

#[test]
fn several_redirections_open_every_file() {
    // bash opens each file left to right; the last one per descriptor receives the output.
    check(
        "severa",
        &[
            (
                "echo hi > f1 > f2; ls; cat f1 | wc -c; cat f2",
                "f1\nf2\n0\nhi\n",
                0,
            ),
            (
                "echo old > f1; echo hi > f1 > f2; cat f1 | wc -c; cat f2",
                "0\nhi\n",
                0,
            ),
            (
                "echo old > f1; : > f1 > f2; wc -c < f1; wc -c < f2",
                "0\n0\n",
                0,
            ),
            ("echo old > f1; true > f1 > f2; wc -c < f1", "0\n", 0),
            (
                "echo hi > f1 2> f2 > f3; ls; wc -c < f1; cat f3",
                "f1\nf2\nf3\n0\nhi\n",
                0,
            ),
            (
                "echo old > f1; echo hi > f1 2> f2 > f3; wc -c < f1; wc -c < f2",
                "0\n0\n",
                0,
            ),
            ("x=1 echo hi > f1 > f2; ls", "f1\nf2\n", 0),
            ("echo hi >> f1 >> f2; ls; wc -c < f1", "f1\nf2\n0\n", 0),
            (
                "echo old > f1; echo hi >> f1 > f2; wc -c < f1; cat f2",
                "4\nhi\n",
                0,
            ),
            (
                "echo hi > f1 2>&1 > f2; ls; wc -c < f1; cat f2",
                "f1\nf2\n0\nhi\n",
                0,
            ),
            ("echo hi 2> f1 > f2; ls", "f1\nf2\n", 0),
        ],
    );
}

#[test]
fn export_plus_equals_appends() {
    // `export z+=2` appends to z; on an unset z it sets it.
    check(
        "export",
        &[
            ("export z=1; export z+=2; echo $z", "12\n", 0),
            ("export z=a; export z+=b w=c; echo $z $w", "ab c\n", 0),
            ("export z=1; export z+=; echo $z", "1\n", 0),
            ("export z+=x y; echo $z", "x\n", 0),
        ],
    );
}

#[test]
fn printf_stops_at_backslash_c_and_quotes_with_q() {
    // `%b` ends all output at `\c`; `%q` quotes as bash does.
    check(
        "printf",
        &[
            (
                "printf '%b\\n' 'a\\cb'; echo after",
                "aafter\n",
                0,
            ),
            (
                "printf '%b|' 'x\\cy' 'z'; echo after",
                "xafter\n",
                0,
            ),
            (
                "printf '%b|%s|' 'x' 'y\\cz'; echo after",
                "x|y\\cz|after\n",
                0,
            ),
            (
                "printf 'a\\cb%s\\n' x; echo after",
                "a\\cbx\nafter\n",
                0,
            ),
            (
                "printf '%s\\cX|\\n' a",
                "a\\cX|\n",
                0,
            ),
            (
                "printf '%b' 'abc\\c' 'def'; echo after",
                "abcafter\n",
                0,
            ),
            (
                "printf '%q\\n' 'a b' \"it's\" 'a\"b' '' 'x\\y' '~' 'a~b' '~a' '#a' 'a#b' 'a=~' ':~' 'x*' '(a)' 'a;b' 'a|b' 'a&b' 'a<b>' 'a$b' 'a`b' '{a,b}' '[a]' '!a' '^a' 'a,b' 'a:b' 'a+b' 'a%b' 'a/b' 'a.b' 'a-b' '=a' 'a@b'",
                "a\\ b\nit\\'s\na\\\"b\n''\nx\\\\y\n\\~\na~b\n\\~a\n\\#a\na#b\na=\\~\n:\\~\nx\\*\n\\(a\\)\na\\;b\na\\|b\na\\&b\na\\<b\\>\na\\$b\na\\`b\n\\{a\\,b\\}\n\\[a\\]\n\\!a\n\\^a\na\\,b\na:b\na+b\na%b\na/b\na.b\na-b\n=a\na@b\n",
                0,
            ),
            (
                "printf '%q\\n' 'a\nb' 'a\tb' 'a\nb c' \"it's\nx\" 'a\\b\nc'",
                "$'a\\nb'\n$'a\\tb'\n$'a\\nb c'\n$'it\\'s\\nx'\n$'a\\\\b\\nc'\n",
                0,
            ),
            (
                "printf '%q|' a 'b c'; echo",
                "a|b\\ c|\n",
                0,
            ),
            (
                "printf '%q\\n'",
                "''\n",
                0,
            ),
            (
                "printf '%q %q\\n' one",
                "one ''\n",
                0,
            ),
            (
                "printf '%10q|%-10q|\\n' 'a b' 'c'",
                "      a\\ b|c         |\n",
                0,
            ),
        ],
    );
}

#[test]
fn cat_writes_errors_in_the_order_of_the_files() {
    // With standard error on standard output, cat interleaves its errors with the files.
    check(
        "cat_wr",
        &[
            (
                "printf 'x\\n' > f; cat f nofile f 2>&1",
                "x\ncat: nofile: No such file or directory\nx\n",
                1,
            ),
            (
                "printf 'x\\n' > f; cat f nofile f 2>&1 | cat -n",
                "     1\tx\n     2\tcat: nofile: No such file or directory\n     3\tx\n",
                0,
            ),
            (
                "printf 'x\\n' > f; cat -n f nofile f 2>&1",
                "     1\tx\ncat: nofile: No such file or directory\n     2\tx\n",
                1,
            ),
            (
                "printf 'x\\n' > f; printf 'y\\n' > g; cat f nofile g nofile2 f 2>&1; echo \"[$?]\"",
                "x\ncat: nofile: No such file or directory\ny\ncat: nofile2: No such file or directory\nx\n[1]\n",
                0,
            ),
            (
                "printf 'x\\n' > f; cat nofile f nofile2 2>&1",
                "cat: nofile: No such file or directory\nx\ncat: nofile2: No such file or directory\n",
                1,
            ),
            (
                "printf 'x\\n' > f; cat f nofile f 2>/dev/null",
                "x\nx\n",
                1,
            ),
            (
                "printf 'x\\n' > f; cat f nofile f 2>err; cat err | wc -l",
                "x\nx\n1\n",
                0,
            ),
            (
                "printf 'x\\n' > f; { cat f nofile f; } 2>&1 | wc -l",
                "3\n",
                0,
            ),
            (
                "printf 'x\\n' > f; cat f nofile f > out 2>&1; cat out",
                "x\ncat: nofile: No such file or directory\nx\n",
                0,
            ),
            (
                "printf 'x\\n' > f; cat nofile - f 2>&1 < f",
                "cat: nofile: No such file or directory\nx\nx\n",
                1,
            ),
        ],
    );
}

#[test]
fn an_expansion_error_ends_a_c_string_with_127() {
    // bash 5.2 -c: 127 for a failed ${x:?} or an unbound variable; arithmetic is 1.
    check(
        "an_exp",
        &[
            ("echo ${y:?msg}; echo after", "", 127),
            ("echo ${y?msg}; echo after", "", 127),
            ("echo ${u:?}; echo after", "", 127),
            ("set -u; echo $u; echo after", "", 127),
            (
                "echo hi; echo ${y:?msg}\nx=${y:?m}; echo after",
                "hi\n",
                127,
            ),
            ("echo $((1/0)); echo after", "", 1),
        ],
    );
}

#[test]
fn cut_and_wc_count_bytes_of_ascii_the_same() {
    // ASCII lists of cut and a byte count, as GNU gives them.
    check(
        "cut_an",
        &[
            ("unset z; export z+=2; echo $z", "2\n", 0),
            ("printf 'abc\\n' | cut -c2", "b\n", 0),
            ("printf 'abc\\n' | cut -c2-", "bc\n", 0),
            ("printf 'abc\\n' | cut -c-2", "ab\n", 0),
            ("printf 'abc\\n' | cut -b1,3", "ac\n", 0),
            ("printf '\\303\\251\\n' | wc -c", "3\n", 0),
        ],
    );
}

#[test]
fn cut_c_and_b_count_bytes_and_pass_them_through() {
    // GNU cut in a UTF-8 locale: -c is -b. `h`, 0xC3 0xA9 (e-acute), newline.
    // The bytes are asserted raw, as bash's output is the raw bytes too.
    let dir = scratch("cutb");
    let cases: &[(&str, &[u8])] = &[
        ("printf 'h\\303\\251\\n' | cut -c1-2", b"h\xc3\n"),
        ("printf 'h\\303\\251\\n' | cut -b1-2", b"h\xc3\n"),
        ("printf 'h\\303\\251\\n' | cut -c3", b"\xa9\n"),
        ("printf 'h\\303\\251\\n' | cut -c2-3", b"\xc3\xa9\n"),
        ("printf 'h\\303\\251x\\n' | cut -c1,3,4", b"h\xa9x\n"),
        ("printf 'h\\303\\251\\n' | cut -c1-", b"h\xc3\xa9\n"),
        ("printf 'abc\\n' | cut -b1,3", b"ac\n"),
        ("printf '\\351\\n' | cut -c1", b"\xe9\n"),
    ];
    for (script, want) in cases {
        let (out, err, code) = ish_raw(script, &dir);
        assert_eq!((out.as_slice(), code), (*want, 0), "{script}: {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn printf_writes_octal_escapes_as_raw_bytes() {
    let dir = scratch("pfraw");
    let cases: &[(&str, &[u8])] = &[
        ("printf 'h\\303\\251\\n'", b"h\xc3\xa9\n"),
        ("printf '\\351'", b"\xe9"),
        ("printf '%b' 'a\\0351b'", b"a\xe9b"),
        ("printf '%b|' '\\xe9'", b"\xe9|"),
        ("printf '%s|' 'h\u{e9}'", b"h\xc3\xa9|"),
    ];
    for (script, want) in cases {
        let (out, err, code) = ish_raw(script, &dir);
        assert_eq!((out.as_slice(), code), (*want, 0), "{script}: {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_arithmetic_for_loop_is_refused_by_name_before_anything_runs() {
    let dir = scratch("arith");
    let (out, err, code) = ish_bash("echo before\nfor ((i=0;i<2;i++)); do echo $i; done", &dir);
    // bash runs it; the subset refuses it (exit 77) and nothing runs, the
    // first line included.
    assert_eq!((out.as_str(), code), ("", 77), "{err}");
    assert!(err.contains("for ((...))"), "stderr was {err}");
    assert!(err.contains("line 2"), "stderr was {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bare_sed_block_with_an_unsupported_command_is_not_run_in_process() {
    // `b` (a branch) is outside the in-process sed, so it runs in a system sed
    // with the script intact, or is refused naming the command: either way the
    // script is never half run. Here only the exit status and the absence of
    // a truncated result are asserted, as the machine may have no GNU sed.
    let dir = scratch("sedb");
    let (out, _err, code) = ish_bash("printf 'a\\nb\\nc\\nd\\n' | sed -n '2{p;b};p'", &dir);
    assert!(
        (code == 0 && out == "a\nb\nc\nd\n") || (code != 0 && out.is_empty()),
        "out {out:?} code {code}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

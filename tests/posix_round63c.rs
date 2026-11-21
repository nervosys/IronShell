//! Round 63c: differences between the in-process POSIX engine (`ish -b`) and
//! bash 5.2 and GNU coreutils / sed found in round 63: `set -e` after a
//! compound command whose last command is an exempt `&&` list or `!`
//! pipeline; arguments with quotes and backslashes for programs on the
//! MSYS runtime (Windows); `\"`, `\'` and `\?` in a printf format; `^` and `$`
//! in the middle of a basic regular expression; `mktemp`; a program that feeds
//! `head` stopping when `head` is done; `wc` of a file on standard input;
//! `/dev/stdout`, `/dev/stderr` and `/dev/stdin`; `head` and `tail` headers when
//! an operand is missing; `cd` without HOME; `${#1}`; `command -v`; and the
//! refusal hints for constructs agents write often.
//!
//! Every expected stdout and exit status in the `check` tables was produced by
//! GNU bash 5.2 and GNU coreutils / sed on Debian with the same script. Cases
//! that cannot be run under bash (this shell's own decisions: `$OSTYPE`, the
//! temp directory under the workspace jail, refusals) are written by hand. The
//! bytes are asserted as they are, not through `od`, whose output differs on
//! macOS.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_r63c_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("w")).expect("scratch dir");
    dir
}

/// Runs `ish -b -c <script>` in `dir/w`, which is the workspace. The session
/// state is kept beside it, so `ls` there lists only what a script made.
fn ish_raw_env(script: &str, dir: &Path, env: &[(&str, &str)]) -> (Vec<u8>, String, i32) {
    let work = dir.join("w");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["-b", "-c", script])
        .current_dir(&work)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("state"))
        .env("IRONSHELL_WORKSPACE", &work)
        .env_remove("IRONSHELL_MODE")
        .env_remove("OLDPWD")
        .env_remove("IFS");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run ish");
    (
        out.stdout,
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn ish_raw(script: &str, dir: &Path) -> (Vec<u8>, String, i32) {
    ish_raw_env(script, dir, &[])
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

/// Whether a program of this name is on PATH (with `.exe` on Windows).
fn have(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|d| {
        d.join(name).is_file() || (cfg!(windows) && d.join(format!("{name}.exe")).is_file())
    })
}

/// A refused script prints nothing, exits 77 and names the construct and a
/// hint holding each of `needles` on standard error.
fn refused(tag: &str, script: &str, needles: &[&str]) {
    let dir = scratch(tag);
    let (out, err, code) = ish_raw(script, &dir);
    assert_eq!(code, 77, "{script}: {err}");
    assert!(out.is_empty(), "{script}: ran something: {out:?}");
    for n in needles {
        assert!(err.contains(n), "{script}: missing {n:?} in {err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn set_e_compound_commands() {
    check(
        "set_e_",
        &[
            (
                r#"set -e; for f in a b; do [ -f "$f" ] && echo "$f"; done; echo done"#,
                b"done\n",
                0,
            ),
            (
                r#"set -e; { [ 1 -eq 2 ] && echo no; }; echo done"#,
                b"done\n",
                0,
            ),
            (
                r#"set -e; if true; then false && true; fi; echo done"#,
                b"done\n",
                0,
            ),
            (
                r#"set -e; printf 'a\nb\n' > f; while read l; do [ "$l" = a ] && echo hit; done < f; echo done"#,
                b"hit\ndone\n",
                0,
            ),
            (
                r#"set -e; for i in 1 2; do ! true; done; echo done"#,
                b"done\n",
                0,
            ),
            (
                r#"set -e; printf 'a\nb\n' | while read l; do [ "$l" = a ] && echo hit; done; echo done"#,
                b"hit\n",
                1,
            ),
            (
                r#"set -e; for i in 1; do false; echo no; done; echo done"#,
                b"",
                1,
            ),
            (r#"set -e; { false; echo no; }; echo done"#, b"", 1),
            (r#"set -e; echo x | { false && true; }; echo done"#, b"", 1),
            (
                r#"set -e; { false && true; } | cat; echo done"#,
                b"done\n",
                0,
            ),
            (
                r#"set -e; for i in 1; do false && true; done; echo done; false; echo not"#,
                b"done\n",
                1,
            ),
            (
                r#"set -e; true && { false && true; }; echo done"#,
                b"done\n",
                0,
            ),
            (r#"set -e; ! { false && true; }; echo done"#, b"done\n", 0),
        ],
    );
}

#[test]
fn printf_escapes() {
    check(
        "printf",
        &[
            (r#"printf '{\"a\":1}\n'"#, b"{\"a\":1}\n", 0),
            (
                r#"printf '[\"] [\?] [\q] [\E]\n' | tr '\033' '@'"#,
                b"[\"] [?] [\\q] [@]\n",
                0,
            ),
            (r#"printf "[\\'] [\\\"]\n""#, b"['] [\"]\n", 0),
            (
                r#"printf '%b\n' '[\"] [\?] [\q]'"#,
                b"[\\\"] [\\?] [\\q]\n",
                0,
            ),
            (
                r#"echo -e '[\"] [\?] [\E]' | tr '\033' '@'"#,
                b"[\\\"] [\\?] [@]\n",
                0,
            ),
            (r#"printf '[\"]%s\n' x"#, b"[\"]x\n", 0),
        ],
    );
}

#[test]
fn bre_anchors() {
    check(
        "bre_an",
        &[
            (r#"echo 'a$b' | sed 's/a$b/X/'"#, b"X\n", 0),
            (
                r#"echo 'export P=$HOME/bin' | grep '$HOME'"#,
                b"export P=$HOME/bin\n",
                0,
            ),
            (r#"echo 'a^b' | grep 'a^b'"#, b"a^b\n", 0),
            (r#"echo ab | sed 's/\(^a\)/X/'"#, b"Xb\n", 0),
            (r#"echo ba | sed 's/a\|^b/X/g'"#, b"XX\n", 0),
            (r#"echo 'x[^a]y' | sed 's/[^a]/Z/g'"#, b"ZZZaZZ\n", 0),
            (r#"echo 'a$' | sed 's/a$/X/'"#, b"a$\n", 0),
            (r#"echo 'a$b' | sed -E 's/a$b/X/'"#, b"a$b\n", 0),
            (r#"echo 'a^b' | grep -E 'a^b'; echo $?"#, b"1\n", 0),
            (r#"echo '$5' | sed 's/$5/five/'"#, b"five\n", 0),
            (r#"echo foo | grep -c '^f\(o\)*$'"#, b"1\n", 0),
            (r#"echo 'ab$cd' | sed 's/b$c/X/; s/$/!/'"#, b"aXd!\n", 0),
            (r#"echo 'a^b' | sed 's/a^b/X/'"#, b"X\n", 0),
            (
                r#"echo 'a b' | sed 's/\(a\) \(^b\)/X/'; echo 'xa' | sed 's/\(x\|^y\)a/Z/'"#,
                b"a b\nZ\n",
                0,
            ),
            (r#"echo 'ab' | sed 's/[$]/X/;s/b$/B/'"#, b"aB\n", 0),
        ],
    );
}

#[test]
fn mktemp_in_process() {
    check(
        "mktemp",
        &[
            (
                r#"d=$(mktemp -d); [ -d "$d" ] && echo isdir; touch "$d/f"; ls "$d"; rm -r "$d""#,
                b"isdir\nf\n",
                0,
            ),
            (
                r#"f=$(mktemp); echo hi > "$f"; cat "$f"; rm "$f""#,
                b"hi\n",
                0,
            ),
            (
                r#"mktemp foo.XXXXXX | sed 's/[A-Za-z0-9]\{6\}$/R/'; ls | sed 's/[A-Za-z0-9]\{6\}$/R/'"#,
                b"foo.R\nfoo.R\n",
                0,
            ),
            (
                r#"f=$(mktemp -p . abcXXXXXX); echo "$f" | cut -c1-5; ls | wc -l"#,
                b"./abc\n1\n",
                0,
            ),
            (r#"mktemp XX 2>/dev/null; echo $?"#, b"1\n", 0),
            (
                r#"mktemp -u zzXXXXXX | cut -c1-2; ls | wc -l"#,
                b"zz\n0\n",
                0,
            ),
            (
                r#"mktemp --suffix=.txt fooXXXXXX | sed 's/[A-Za-z0-9]\{6\}\.txt$/R/'"#,
                b"fooR\n",
                0,
            ),
            (
                r#"mktemp -d dd.XXXXXX >/dev/null; ls -d dd.* | wc -l"#,
                b"1\n",
                0,
            ),
            (r#"mktemp a/b/cXXXXXX 2>/dev/null; echo $?"#, b"1\n", 0),
            (r#"mktemp -t 'a/bXXXXXX' 2>/dev/null; echo $?"#, b"1\n", 0),
            (r#"mktemp one two 2>/dev/null; echo $?"#, b"1\n", 0),
            (
                r#"mktemp fooXXXbar | sed 's/[A-Za-z0-9]\{3\}bar$/R/'"#,
                b"fooR\n",
                0,
            ),
            (r#"mktemp -z 2>/dev/null; echo $?"#, b"1\n", 0),
            (
                r#"mkdir sub; mktemp -p sub | sed 's/[A-Za-z0-9]\{10\}$/R/'; ls sub | wc -l"#,
                b"sub/tmp.R\n1\n",
                0,
            ),
            (
                r#"mkdir sub; mktemp -dp sub/ x.XXXX | sed 's/[A-Za-z0-9]\{4\}$/R/'"#,
                b"sub/x.R\n",
                0,
            ),
        ],
    );
}

#[test]
fn writer_stops_when_head_is_done() {
    if !have("yes") {
        return;
    }
    check(
        "writer",
        &[
            (r#"yes | head -3"#, b"y\ny\ny\n", 0),
            (r#"seq 1 100000 | head -1"#, b"1\n", 0),
            (r#"yes | head -3 | wc -l"#, b"3\n", 0),
            (
                r#"set -o pipefail; yes | head -1; echo rc=$?"#,
                b"y\nrc=141\n",
                0,
            ),
            (r#"yes | head -n 2; echo done"#, b"y\ny\ndone\n", 0),
            (r#"yes abc | head -c 8; echo"#, b"abc\nabc\n\n", 0),
            (r#"yes | head -2 | tr y z"#, b"z\nz\n", 0),
        ],
    );
}

#[test]
fn wc_width_for_stdin_file() {
    check(
        "wc_wid",
        &[
            (
                r#"printf 'a b\nc\n' > f; wc < f; wc -l < f; wc f; wc -lw f; cat f | wc; wc -c < f; wc -lc < f"#,
                b"2 3 6\n2\n2 3 6 f\n2 3 f\n      2       3       6\n6\n2 6\n",
                0,
            ),
            (
                r#"printf 'a b\nc\n' > f; wc -l f f; wc < f f"#,
                b" 2 f\n 2 f\n 4 total\n2 3 6 f\n",
                0,
            ),
        ],
    );
}

#[test]
fn dev_stdout_stderr_stdin() {
    check(
        "dev_st",
        &[
            (r#"{ echo a > /dev/stderr; } 2>&1 | cat"#, b"a\n", 0),
            (r#"x=$(echo a > /dev/stdout); echo "[$x]""#, b"[a]\n", 0),
            (
                r#"echo out > /dev/stdout; echo err > /dev/stderr 2>/dev/null; echo done"#,
                b"out\ndone\n",
                0,
            ),
            (
                r#"echo x | cat /dev/stdin; echo y | wc -l < /dev/stdin"#,
                b"x\n1\n",
                0,
            ),
            (r#"echo hi | cat /dev/fd/0"#, b"hi\n", 0),
            (r#"echo a | { cat; } > /dev/stdout"#, b"a\n", 0),
        ],
    );
}

#[test]
fn head_tail_headers_with_missing_operand() {
    check(
        "head_t",
        &[
            (
                r#"printf 'a\n' > f; head -n1 f missing 2>/dev/null; echo "rc=$?""#,
                b"==> f <==\na\nrc=1\n",
                0,
            ),
            (
                r#"printf 'a\n' > f; head -n1 missing f 2>/dev/null"#,
                b"==> f <==\na\n",
                1,
            ),
            (
                r#"printf 'a\nb\n' > f; printf 'c\n' > g; tail -n1 f missing g 2>/dev/null"#,
                b"==> f <==\nb\n\n==> g <==\nc\n",
                1,
            ),
            (
                r#"printf 'a\n' > f; head -n1 missing missing2 f 2>/dev/null"#,
                b"==> f <==\na\n",
                1,
            ),
        ],
    );
}

#[test]
fn cd_without_home_and_length_of_argument() {
    check(
        "cd_wit",
        &[
            (r#"unset HOME; cd 2>/dev/null; echo rc=$?"#, b"rc=1\n", 0),
            (r#"set -- hello world; echo ${#1} ${#2}"#, b"5 5\n", 0),
        ],
    );
}

#[test]
fn command_v() {
    check(
        "comman",
        &[
            (r#"command -v echo cd if; echo $?"#, b"echo\ncd\nif\n0\n", 0),
            (r#"command -v nosuch; echo $?"#, b"1\n", 0),
            (r#"command -v nosuch echo; echo $?"#, b"echo\n0\n", 0),
            (
                r#"if command -v nosuch >/dev/null 2>&1; then echo y; else echo n; fi"#,
                b"n\n",
                0,
            ),
            (r#"command -v echo >/dev/null && echo yes"#, b"yes\n", 0),
            (r#"command -v "" ; echo $?"#, b"1\n", 0),
            (r#"command -v -- echo"#, b"echo\n", 0),
            (r#"x=$(command -v printf); echo "[$x]""#, b"[printf]\n", 0),
            (r#"command -v nosuch1 nosuch2; echo $?"#, b"1\n", 0),
        ],
    );
}

/// Arguments reach a program as written, whatever quotes, backslashes, spaces
/// and empty words they hold. The program is this shell's own binary run on a
/// script that prints its arguments, so no tool of the runner is needed.
#[test]
fn arguments_reach_a_program_unchanged() {
    let dir = scratch("argv");
    let script = dir.join("argv.ish");
    std::fs::write(
        &script,
        "args() | map(fn(a) => \"[${a}]\") | str.join(\"\")\n",
    )
    .unwrap();
    let ish = env!("CARGO_BIN_EXE_ish").replace('\\', "/");
    let argv = script.to_string_lossy().replace('\\', "/");
    // No workspace jail here: it would not let a script run a program that
    // lies outside the workspace (this shell's own binary).
    let run = |script: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-b", "-c", script])
            .current_dir(dir.join("w"))
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", dir.join("state"))
            .env("R63_ISH", &ish)
            .env("R63_ARGV", &argv)
            .env_remove("IRONSHELL_WORKSPACE")
            .env_remove("IRONSHELL_MODE")
            .output()
            .expect("run ish");
        (
            out.stdout,
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    };
    let cases: [(&str, &str); 6] = [
        (
            r##""$R63_ISH" "$R63_ARGV" 'a"b' 'say "hi"' 'a b"c'"##,
            "[a\"b][say \"hi\"][a b\"c]",
        ),
        (
            r##""$R63_ISH" "$R63_ARGV" "it's" 'c d\' 'x\\y' '\"'"##,
            "[it's][c d\\][x\\\\y][\\\"]",
        ),
        (
            r##""$R63_ISH" "$R63_ARGV" '' a '' 'b  c'"##,
            "[][a][][b  c]",
        ),
        (
            r##""$R63_ISH" "$R63_ARGV" '\' '\\' 'a\' 'a\ b\'"##,
            "[\\][\\\\][a\\][a\\ b\\]",
        ),
        (
            r##""$R63_ISH" "$R63_ARGV" '"' "'" '""' '"a"'"##,
            "[\"]['][\"\"][\"a\"]",
        ),
        (
            r##""$R63_ISH" "$R63_ARGV" 'tab	x' '*' '2{p;b};p'"##,
            "[tab\tx][*][2{p;b};p]",
        ),
    ];
    for (script, want) in cases {
        let (out, err, code) = run(script);
        assert_eq!(
            (String::from_utf8_lossy(&out).into_owned(), code),
            (format!("{want}\n"), 0),
            "{script}: stderr {err}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// On Windows the GNU tools of Git for Windows run on the MSYS runtime, which
/// read a quote in an argument as the end of the word: `env echo 'a"b'` printed
/// `a\b`. Each argument below must reach the tool as written.
#[test]
fn msys_programs_get_their_arguments_unchanged() {
    if !cfg!(windows) || !have("env") || !have("sh") {
        return;
    }
    check(
        "msys",
        &[
            (
                r##"env printf '%s|' 'a"b' 'say "hi"' 'it'"'"'s' 'c d\' '' 'x\\y' '\"' '*'; echo"##,
                b"a\"b|say \"hi\"|it's|c d\\||x\\\\y|\\\"|*|\n",
                0,
            ),
            (r##"env echo 'a"b'"##, b"a\"b\n", 0),
            (r##"sh -c 'echo "hi there"'"##, b"hi there\n", 0),
            (r##"sh -c 'printf "%s|" "a b" c'; echo"##, b"a b|c|\n", 0),
        ],
    );
    if have("awk") {
        check("awk", &[(r##"echo 1 | awk '{print "x" $1}'"##, b"x1\n", 0)]);
    }
}

/// A Windows program's CRLF line ends are LF in `$( )`: `jq`, `tar`, `cmd`.
#[test]
fn crlf_of_a_program_is_dropped_in_command_substitution() {
    if !cfg!(windows) {
        return;
    }
    check(
        "crlf",
        &[
            (r##"x=$(cmd /c echo hi); echo "${#x}""##, b"2\n", 0),
            (
                r##"x=$(cmd /c echo hi); [ "$x" = hi ] && echo same"##,
                b"same\n",
                0,
            ),
        ],
    );
}

/// Variables a bash always has. `$OSTYPE` and `$UID` are this shell's own
/// decisions: `msys` and 1000 on Windows (as Git for Windows' bash reports the
/// first), `linux-gnu` or `darwin` and the user's id elsewhere.
#[test]
fn bash_identity_variables() {
    let ostype = if cfg!(windows) {
        "msys"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux-gnu"
    };
    check(
        "ident",
        &[
            (
                r##"echo "$OSTYPE""##,
                format!("{ostype}\n").leak().as_bytes(),
                0,
            ),
            (
                r##"[ "$UID" = "$EUID" ] && [ -n "$UID" ] && echo same"##,
                b"same\n",
                0,
            ),
            (r##"[ "$EUID" -ge 0 ] && echo number"##, b"number\n", 0),
            (r##"[ -n "$BASH_VERSION" ] && echo bash"##, b"bash\n", 0),
            (r##"echo "${BASH_VERSION%%.*}""##, b"5\n", 0),
            (r##"BASH_VERSION=x; echo "$BASH_VERSION""##, b"x\n", 0),
            (r##"[ -n "$HOSTNAME" ] && echo named"##, b"named\n", 0),
        ],
    );
}

/// Without a name, `mktemp` makes its file under the workspace's own temp
/// directory when the jail is on, and the path it prints is one the in-process
/// tools and the programs a script runs resolve alike.
#[test]
fn mktemp_stays_in_the_workspace_under_the_jail() {
    check(
        "mkjail",
        &[
            (
                r##"d=$(mktemp -d); echo "$d" | grep -c '/\.ironshell/tmp/tmp\.'; [ -d "$d" ] && echo isdir"##,
                b"1\nisdir\n",
                0,
            ),
            (
                r##"f=$(mktemp); echo "$f" | grep -c '^\(/\|[A-Za-z]:/\)'; [ -f "$f" ] && echo made"##,
                b"1\nmade\n",
                0,
            ),
        ],
    );
    // A directory outside the workspace is refused, as for `touch` and `mkdir`.
    let dir = scratch("mkout");
    let outside = scratch("mkout2");
    let target = outside.join("w").to_string_lossy().replace('\\', "/");
    let (out, err, code) = ish_raw(&format!("mktemp -p '{target}'; echo $?"), &dir);
    assert_eq!(String::from_utf8_lossy(&out), "1\n", "{err}");
    assert_eq!(code, 0);
    assert!(
        err.contains("mktemp: failed to create file via template"),
        "{err}"
    );
    assert_eq!(std::fs::read_dir(outside.join("w")).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&outside);
}

/// `head` and `tail` word an unreadable file as GNU does and call standard input
/// `standard input` in a header; `tee` takes the names of the standard streams.
#[test]
fn head_tail_messages_and_tee_streams() {
    check(
        "htmsg",
        &[
            (
                r##"printf 'a\nb\n' | head -n 1 nofile - 2>&1; echo rc=$?"##,
                b"head: cannot open 'nofile' for reading: No such file or directory\n==> standard input <==\na\nrc=1\n",
                0,
            ),
            (
                r##"printf 'a\n' > f; tail -n 1 f nofile 2>/dev/null; echo rc=$?"##,
                b"==> f <==\na\nrc=1\n",
                0,
            ),
            (
                r##"printf 'a\nb\n' | head -n 1 - - ; echo rc=$?"##,
                b"==> standard input <==\na\n\n==> standard input <==\nrc=0\n",
                0,
            ),
            (r##"echo hi | tee /dev/stdout; echo rc=$?"##, b"hi\nhi\nrc=0\n", 0),
            (
                r##"echo hi | tee /dev/stderr 2>&1 >/dev/null; echo rc=$?"##,
                b"hi\nrc=0\n",
                0,
            ),
            (
                r##"echo hi | tee /dev/stderr 2>/dev/null; echo rc=$?"##,
                b"hi\nrc=0\n",
                0,
            ),
        ],
    );
}

/// A process substitution as the target of a redirection is refused up front,
/// naming it (exit 77), not reported as a missing file name.
#[test]
fn process_substitution_target_is_refused() {
    refused(
        "psub1",
        "echo before; cat < <(echo hi)",
        &["process substitution", "line 1"],
    );
    refused("psub2", "echo a > >(cat)", &["process substitution"]);
}

/// The refusal hint of each construct names what to write instead.
#[test]
fn refusal_hints_help() {
    refused("h1", "set -euxo pipefail", &["set -x", "set -euo pipefail"]);
    refused("h2", "set -f", &["-e, -u and -o pipefail"]);
    refused("h3", "((i++))", &["arithmetic command", "i=$((i+1))"]);
    refused("h4", "echo ${BASH_SOURCE[0]}", &["$0", "arrays"]);
    refused("h5", "echo ${@:2}", &["shift"]);
    refused("h6", "local x=1", &["ordinary variables"]);
    refused("h7", "declare x=1", &["name=value", "export"]);
    refused("h8", "readonly X=1", &["name=value"]);
    refused("h9", "let x=1", &["i=$((i+1))"]);
    refused("h10", "getopts ab opt", &["shift"]);
    refused("h11", "eval \"echo hi\"", &["write the command out"]);
    refused("h12", "type ls", &["command -v"]);
    refused("h13", "command ls", &["command -v NAME"]);
    refused("h14", "shopt -s nullglob", &["nullglob"]);
}

/// `command -v` with a program that exists in the subset or on PATH.
#[test]
fn command_v_finds_in_process_tools_and_programs() {
    check(
        "cmdv",
        &[
            (r##"command -v ls >/dev/null; echo $?"##, b"0\n", 0),
            (r##"command -v ls grep sed | wc -l"##, b"3\n", 0),
            (r##"command -v ./nope; echo $?"##, b"1\n", 0),
            (r##"command -v /nonexistent/x; echo $?"##, b"1\n", 0),
        ],
    );
    // A script file: `command -v ./x` finds an executable file, `x` does not.
    let dir = scratch("cmdv2");
    let (out, _, _) = ish_raw(
        "printf 'echo hi\\n' > tool.sh; chmod +x tool.sh 2>/dev/null; command -v ./tool.sh",
        &dir,
    );
    let text = String::from_utf8_lossy(&out).into_owned();
    // Windows has no mode bits, so the file counts; elsewhere `chmod` is not
    // in this shell's subset and the file is not executable.
    assert!(text == "./tool.sh\n" || text.is_empty(), "{text:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An external program that feeds `head` is stopped, and under `pipefail` the
/// status is the SIGPIPE one.
#[test]
fn killed_writer_status_under_pipefail() {
    if !have("yes") {
        return;
    }
    check(
        "pfail",
        &[
            (
                r##"set -o pipefail; yes | head -1 > /dev/null; echo $?"##,
                b"141\n",
                0,
            ),
            (r##"yes | head -1 > /dev/null; echo $?"##, b"0\n", 0),
            (r##"yes | head -3 | head -1"##, b"y\n", 0),
        ],
    );
}

/// `set -e` still ends a script for a failure that is not exempt, also inside
/// a loop, and also in the last stage of a pipeline.
#[test]
fn set_e_still_ends_on_a_real_failure() {
    check(
        "setere",
        &[
            (
                r##"set -e; for i in 1 2; do echo $i; false; done; echo no"##,
                b"1\n",
                1,
            ),
            (r##"set -e; while true; do echo a; false; done"##, b"a\n", 1),
            (
                r##"set -e; if true; then false; echo no; fi; echo no"##,
                b"",
                1,
            ),
            (r##"set -e; echo x | false; echo no"##, b"", 1),
            (
                r##"set -e; false || { echo alt; }; echo done"##,
                b"alt\ndone\n",
                0,
            ),
            (r##"set -e; { true; false; }; echo no"##, b"", 1),
        ],
    );
}

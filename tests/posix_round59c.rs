//! Round 59c: shell-language differences between the in-process POSIX engine
//! (`ish -b`) and bash: redirection order, ambiguous redirects, quoted words in
//! `${x:-...}`, POSIX classes in trim patterns, `"$*"` and IFS, trailing-slash
//! globs, `read` line continuation, `time`, `x+=`, `cd` options and `OLDPWD`,
//! and pipeline stages as subshells.
//!
//! Every expected stdout and exit status below was produced by GNU bash 5.2 on
//! Debian with the same script, except where a case says otherwise (the text
//! of an error names `ish` where bash names itself, and `time` is refused).

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_r59c_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Runs `ish -b -c <script>` in `dir`, with `extra` environment variables.
/// Returns (stdout, stderr, exit code).
fn ish_bash_env(script: &str, dir: &Path, extra: &[(&str, &str)]) -> (String, String, i32) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["-b", "-c", script])
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join("state"))
        .env("IRONSHELL_WORKSPACE", dir)
        .env_remove("IRONSHELL_MODE")
        .env_remove("OLDPWD")
        .env_remove("IFS");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run ish");
    (
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn ish_bash(script: &str, dir: &Path) -> (String, String, i32) {
    ish_bash_env(script, dir, &[])
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
fn redirections_apply_left_to_right() {
    check(
        "redir",
        &[
            ("ls nofile 2>&1 >/dev/null | wc -l", "1\n", 0),
            ("ls nofile >/dev/null 2>&1 | wc -l", "0\n", 0),
            ("ls nofile 2>&1 >out | wc -l; cat out", "1\n", 0),
            ("echo hi >out 2>&1; cat out", "hi\n", 0),
            ("echo hi 2>&1 >&2 | wc -c", "3\n", 0),
            ("{ echo o; echo e >&2; } 2>&1 >/dev/null | cat", "e\n", 0),
            ("{ echo o; echo e >&2; } >/dev/null 2>&1 | wc -l", "0\n", 0),
            ("echo a 1>&2 2>/dev/null; echo b", "b\n", 0),
            (
                "{ echo o; ls nofile; } >out 2>err; cat out; echo --; cat err",
                "o\n--\nls: cannot access 'nofile': No such file or directory\n",
                0,
            ),
            (
                "{ ls nofile >o; } 2>e; echo --; cat e",
                "--\nls: cannot access 'nofile': No such file or directory\n",
                0,
            ),
        ],
    );
}

#[test]
fn an_ambiguous_redirect_fails_and_creates_nothing() {
    let dir = scratch("amb");
    // bash: `bash: line 1: $y: ambiguous redirect`, status 1, no file.
    let (out, err, code) = ish_bash("y='a b'; echo z > $y; echo $?; ls", &dir);
    assert_eq!((out.as_str(), code), ("1\n", 0), "{err}");
    assert!(
        err.contains("line 1: $y: ambiguous redirect"),
        "stderr was {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
    check(
        "amb",
        &[
            ("y=; echo z > $y; echo $?", "1\n", 0),
            ("echo z > $nonexist; echo $?", "1\n", 0),
            ("cat < $nonexist; echo $?", "1\n", 0),
            // Quoted, it is one name.
            ("echo z > \"a b\"; ls", "a b\n", 0),
            ("echo z > ${u:-\"x y\"}; ls", "x y\n", 0),
            ("touch a1; echo z > a*; echo $?; ls", "0\na1\n", 0),
        ],
    );
}

#[test]
fn quoted_words_in_a_default_operand_keep_their_text() {
    check(
        "quote",
        &[
            (
                "x=a; echo \"${x:+\"q  s\"}\"; echo \"${y:-\"d  d\"}\"",
                "q  s\nd  d\n",
                0,
            ),
            (
                "echo ${y:-\"a  b\"}; echo \"[${y:-\"a  b\"}]\"; echo ${y:-a  b}; echo ${y:-'a  b'}",
                "a  b\n[a  b]\na b\na  b\n",
                0,
            ),
            (
                "y=; echo [${y:-a b}] [${y:-\"a  b\"}] ${y:-x   y z}",
                "[a b] [a  b] x y z\n",
                0,
            ),
            ("y=; set -- ${y:-a b} z; echo $#", "3\n", 0),
            ("echo ${u:-\"a b\" c}; echo \"[${u:-x \"y  z\" w}]\"", "a b c\n[x y  z w]\n", 0),
            ("x=1; echo \"${x:+\"a b\"}${u:+\"c d\"}e\"", "a be\n", 0),
            ("echo \"${u:-$(echo \"p  q\")}\"", "p  q\n", 0),
            ("a=5; echo ${u:-${a:+\"m  n\"}}", "m  n\n", 0),
            ("echo \"[${u:-\"a $(echo b)\"}]\"", "[a b]\n", 0),
            // `:=` splits its result, as bash does.
            ("y=; echo ${y:=\"p  q\"}; echo \"$y\"", "p q\np  q\n", 0),
            // An empty default is no word, and `\"\"` is an empty one.
            // (Counted with printf and `$#`, not `od -c`, whose layout differs
            // between GNU and BSD.)
            ("printf \"[%s]\" \"${u:-}\" x ${u:-} y; echo", "[][x][y]\n", 0),
            ("set -- \"${u:-}\" x ${u:-} y; echo $#", "3\n", 0),
            ("set -- ${u:-\"\"}; echo $#", "1\n", 0),
        ],
    );
}

#[test]
fn posix_classes_in_trim_patterns() {
    check(
        "class",
        &[
            (
                "x=ab12; echo ${x#[[:alpha:]]} ${x%[[:digit:]]} ${x##[[:alpha:]]*} ${x%%[[:digit:]]*}; echo ${x#[[:alpha:]][[:alpha:]]}",
                "b12 ab1 ab\n12\n",
                0,
            ),
            (
                "x=ab; echo ${x#[[:upper:]]} ${x#[[:lower:]]} ${x%[![:alpha:]]} ${x##[[:alpha:]][[:alpha:]]}",
                "ab b ab\n",
                0,
            ),
            // Pathname globs take the same classes.
            (
                "touch a1 b2 c; echo [[:alpha:]][[:digit:]]; echo [[:alpha:]]",
                "a1 b2\nc\n",
                0,
            ),
        ],
    );
}

#[test]
fn star_joins_with_the_first_character_of_ifs() {
    check(
        "star",
        &[
            (
                "set -- \"a b\" c; IFS=,; echo \"$*\"; IFS=; echo \"$*\"; unset IFS; echo \"$*\"; IFS=:x; echo \"$*\"; echo $*",
                "a b,c\na bc\na b c\na b:c\na b c\n",
                0,
            ),
            ("set -- a b c; IFS=; echo \"$*\"; echo \"[$@]\"", "abc\n[a b c]\n", 0),
        ],
    );
}

#[test]
fn a_glob_with_a_trailing_slash_matches_directories_only() {
    check(
        "slash",
        &[
            (
                "mkdir d d/e; touch d/a d/b; echo d/*/; for x in d/*/; do echo \"[$x]\"; done; echo d/*; echo d/e/",
                "d/e/\n[d/e/]\nd/a d/b d/e\nd/e/\n",
                0,
            ),
            ("mkdir d d/e; touch d/a; echo d/z*/; echo d/*//", "d/z*/\nd/e/\n", 0),
            (
                "mkdir d; touch d/a d/b; for x in d/[ab]/; do echo $x; done; echo d/*; echo d/a/",
                "d/[ab]/\nd/a d/b\nd/a/\n",
                0,
            ),
        ],
    );
}

#[test]
fn read_without_r_joins_backslash_newline_lines() {
    check(
        "read",
        &[
            (
                "printf 'a\\\\\\nb\\nc\\n' | while read x; do echo \"[$x]\"; done",
                "[ab]\n[c]\n",
                0,
            ),
            (
                "printf 'a\\\\\\nb\\n' > f; read y < f; echo \"[$y]\"",
                "[ab]\n",
                0,
            ),
            (
                "printf 'a\\\\\\n' | { read x; echo \"[$x] $?\"; }",
                "[a] 1\n",
                0,
            ),
            (
                "printf 'a\\\\\\\\b\\n' | { read x; echo \"[$x]\"; }",
                "[a\\b]\n",
                0,
            ),
            (
                "printf 'a\\\\\\nb\\n' | { read -r x; echo \"[$x]\"; }",
                "[a\\]\n",
                0,
            ),
            (
                "printf 'a b\\\\\\n c d\\n' | { read x y; echo \"[$x][$y]\"; }",
                "[a][b c d]\n",
                0,
            ),
        ],
    );
}

#[test]
fn time_is_refused_before_anything_runs() {
    // bash runs the commands and prints the timing to stderr; the subset
    // refuses the keyword, like the other constructs it does not run.
    let dir = scratch("time");
    let (out, err, code) = ish_bash("echo before\ntime echo hi\necho after", &dir);
    assert_eq!((out.as_str(), code), ("", 77), "{err}");
    assert!(err.contains("`time` builtin"), "stderr was {err}");
    assert!(err.contains("line 2"), "stderr was {err}");
    assert!(err.contains("nothing in this script was run"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn append_assignment_appends_to_a_plain_variable() {
    check(
        "append",
        &[
            (
                "x=abc; x+=d; echo $x; x+=e; echo $x; y+=z; echo $y",
                "abcd\nabcde\nz\n",
                0,
            ),
            (
                "n=1; n+=2; echo $n; export e=a; e+=b; echo $e",
                "12\nab\n",
                0,
            ),
            ("x+=d y=1; echo $x $y", "d 1\n", 0),
            ("x=a; x+=\"b c\"; echo \"$x\"", "ab c\n", 0),
            ("x=a; x+=b; x+=$x; echo $x", "abab\n", 0),
            ("a=1 b+=2 echo x; echo \"[$b]\"", "x\n[]\n", 0),
        ],
    );
}

#[test]
fn cd_takes_its_options_and_checks_its_operands() {
    check(
        "cd",
        &[
            (
                "cd -- .; echo $?; cd -P .; echo $?; cd -L .; echo $?",
                "0\n0\n0\n",
                0,
            ),
            ("mkdir a; cd -P -- a; cd -L ..; cd -- a; echo $?", "0\n", 0),
            ("cd -LP .; echo $?", "0\n", 0),
            ("mkdir a b; cd a b; echo $?", "1\n", 0),
            ("cd -x; echo $?", "2\n", 0),
        ],
    );
    // bash: `cd: OLDPWD not set`, status 1, the directory unchanged. The
    // environment ish starts with holds an OLDPWD, which the script unset.
    let dir = scratch("oldpwd");
    let old = dir.to_string_lossy().into_owned();
    let (out, err, code) = ish_bash_env(
        "unset OLDPWD; mkdir d; cd -; echo $?",
        &dir,
        &[("OLDPWD", &old)],
    );
    assert_eq!((out.as_str(), code), ("1\n", 0), "{err}");
    assert!(err.contains("cd: OLDPWD not set"), "stderr was {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pipeline_stages_are_subshells() {
    check(
        "sub",
        &[
            ("echo x | read v; echo \"[$v]\"", "[]\n", 0),
            ("v=old; echo x | read v; echo \"[$v]\"", "[old]\n", 0),
            ("echo x | read v | cat; echo \"[$v]\"", "[]\n", 0),
            (
                "echo x | { read v; echo \"[$v]\"; }; echo \"[$v]\"",
                "[x]\n[]\n",
                0,
            ),
            ("x=1; { x=2; echo in; } | cat; echo $x", "in\n1\n", 0),
            ("x=1; echo y | { x=2; cat; }; echo $x", "y\n1\n", 0),
            ("echo a b | { read p q; echo \"$q $p\"; }", "b a\n", 0),
            (
                "set -- a b; echo x | { set -- c; echo \"$#\"; }; echo \"$#\"",
                "1\n2\n",
                0,
            ),
            ("read v < /dev/null; echo \"[$v] $?\"", "[] 1\n", 0),
        ],
    );
}

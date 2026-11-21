//! Round 65c: differences between the in-process POSIX engine (`ish -b`) and
//! bash 5.2 and GNU coreutils found in round 65: a command substitution is a
//! subshell; the CR of a native Windows program on the script's stdout and in
//! a `>` file; `bash -c CMD NAME ARGS`; the program a bare name runs is the
//! one `command -v` names; `"${@}"`, `"${*}"`, `${#@}` and `${#*}`; an endless
//! producer read by a program, or run by `timeout`; a refused `seq` or `yes`;
//! expansion errors under `2>&1`; `$((--3))`; `CDPATH`; `timeout -s KILL`; and a
//! shell script found on PATH that is outside the subset.
//!
//! Every expected stdout and exit status in the `check` tables was produced by
//! GNU bash 5.2 and GNU coreutils on Debian with the same script. Cases that
//! cannot be run under bash (this shell's own decisions: the refusals, the
//! removal of a half-written file) are written by hand. The bytes are asserted
//! as they are, not through `od`, whose output differs on macOS.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_r65c_{tag}_{}", std::process::id()));
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
        .env_remove("CDPATH")
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

#[test]
fn a_command_substitution_is_a_subshell() {
    check(
        "subst",
        &[
            (r#"x=5; y=$(x=6); echo $x"#, b"5\n", 0),
            (
                r#"mkdir d; y=$(cd d); test -d d && echo stayed"#,
                b"stayed\n",
                0,
            ),
            (
                r#"set -- a b; x=$(shift; echo $#); echo "$x $#""#,
                b"1 2\n",
                0,
            ),
            (
                r#"i=0; for f in a b c; do n=$(i=$((i+1)); echo $i); done; echo "$i $n""#,
                b"0 1\n",
                0,
            ),
            (
                r#"export V=1; y=$(export V=2; unset V); echo $V"#,
                b"1\n",
                0,
            ),
            (r#"IFS=,; y=$(IFS=:; echo x); echo "$IFS""#, b",\n", 0),
            // The substitution sees what its parent has.
            (
                r#"x=5; y=$(echo $x); echo $y; set -- a b; echo $(echo $# $1)"#,
                b"5\n2 a\n",
                0,
            ),
            (
                r#"z=`x=7; echo $x`; echo "$z ${x:-unset}""#,
                b"7 unset\n",
                0,
            ),
            (r#"set -e; y=$(set +e); false; echo no"#, b"", 1),
            (
                r#"mkdir d; cd d; x=$(cd ..; ls); ls | wc -l; echo $x"#,
                b"0\nd\n",
                0,
            ),
            (
                r#"mkdir d; cd d; cd ..; o=$OLDPWD; x=$(cd d); [ "$OLDPWD" = "$o" ] && echo same"#,
                b"same\n",
                0,
            ),
            (r#"x=$(exit 3); echo "after $?""#, b"after 3\n", 0),
            // A stage of a pipeline too.
            (r#"mkdir d; { cd d; echo in; } | cat; ls"#, b"in\nd\n", 0),
            (
                r#"mkdir d; echo a | { cd d; pwd > /dev/null; }; ls"#,
                b"d\n",
                0,
            ),
        ],
    );
}

#[test]
fn braced_at_and_star_and_their_counts() {
    check(
        "at",
        &[
            (
                r#"set -- a b; echo "${@}" "${*}" ${#@} ${#*} "${#}""#,
                b"a b a b 2 2 2\n",
                0,
            ),
            (
                r#"set -- a b; for i in "${@}"; do echo "<$i>"; done"#,
                b"<a>\n<b>\n",
                0,
            ),
            (
                r#"set -- a "b c"; for i in "${@}"; do echo "<$i>"; done; for i in "${*}"; do echo "<$i>"; done"#,
                b"<a>\n<b c>\n<a b c>\n",
                0,
            ),
            (r#"IFS=,; set -- a b; echo "${*}" "$*""#, b"a,b a,b\n", 0),
            (r#"set --; echo "${@}x" ${#@} "[${*}]""#, b"x 0 []\n", 0),
            (r#"set -- a b; echo ${@} ${*}"#, b"a b a b\n", 0),
            (r#"set -- a b; echo "x${@}y""#, b"xa by\n", 0),
        ],
    );
    // The forms with an operator stay outside the subset.
    let dir = scratch("atref");
    let (out, err, code) = ish_raw(r#"set -- a b c; echo "${@:2}""#, &dir);
    assert_eq!(code, 77, "{err}");
    assert!(out.is_empty(), "{out:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_signs_before_a_number() {
    check(
        "signs",
        &[
            (
                r#"echo $((--3)) $((++3)) $((- -3)) $((-(-3))) $((--  3))"#,
                b"3 3 3 3 3\n",
                0,
            ),
            (r#"y=1; echo $((--y)) $y $((++y)) $y"#, b"0 0 1 1\n", 0),
        ],
    );
}

/// An expansion error happens before the command's redirections apply, so it
/// goes to the standard error the command had, not into `2>&1`.
#[test]
fn an_expansion_error_is_not_redirected_with_the_command() {
    check(
        "experr",
        &[
            ("echo $((1/0)) 2>&1\necho after", b"after\n", 0),
            (r#"x=$(echo "${y:?bad}" 2>&1); echo "[$x]""#, b"[]\n", 0),
            (r#"echo ${y:?bad} 2>&1; echo after"#, b"", 127),
            (r#"set -u; echo $nope 2>&1; echo after"#, b"", 127),
            (r#"echo ${y:?bad} 2>o; echo after"#, b"", 127),
        ],
    );
    // The message is on the real standard error.
    let dir = scratch("experr2");
    let (out, err, code) = ish_raw(r#"echo ${y:?bad} 2>&1"#, &dir);
    assert!(out.is_empty(), "{out:?}");
    assert!(err.contains("y: bad"), "{err}");
    assert_eq!(code, 127);
    let _ = std::fs::remove_dir_all(&dir);
    // A group's own redirection does apply to the commands in it.
    let dir = scratch("experr3");
    let (out, _, code) = ish_raw(r#"{ echo ${y:?bad}; } 2>&1; echo after"#, &dir);
    assert!(String::from_utf8_lossy(&out).contains("y: bad"), "{out:?}");
    assert_eq!(code, 127);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `CDPATH`, as bash reads it: tried in order, an empty entry is the current
/// directory, the directory is printed when it came from a named entry, and
/// `./x` is never searched. In agent mode it is not read.
#[test]
fn cdpath_is_searched_by_cd() {
    // Each case has a directory of its own.
    let n = std::cell::Cell::new(0);
    let pwd_of = |script: &str| -> (String, i32) {
        n.set(n.get() + 1);
        let dir = scratch(&format!("cdpath{}", n.get()));
        let (out, err, code) = ish_raw(script, &dir);
        assert_eq!(code, 0, "{script}: {err}");
        let _ = std::fs::remove_dir_all(&dir);
        (String::from_utf8_lossy(&out).into_owned(), code)
    };
    // Found under a named entry: the directory is printed.
    let (out, _) = pwd_of(r#"mkdir -p a/b; CDPATH=$PWD/a; cd b; echo "rc=$?""#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    assert!(lines[0].ends_with("/a/b"), "{out}");
    assert_eq!(lines[1], "rc=0");
    // Under a substitution the printed directory is captured.
    let (out, _) = pwd_of(r#"mkdir -p a/b b; CDPATH=$PWD/a; x=$(cd b); echo "[$x]""#);
    assert!(
        out.starts_with('[') && out.trim_end().ends_with("/a/b]"),
        "{out}"
    );
    // An empty entry is the current directory, and prints nothing.
    let (out, _) = pwd_of(r#"mkdir -p a/b; CDPATH=:$PWD/a; mkdir b; cd b; echo "rc=$?""#);
    assert_eq!(out, "rc=0\n");
    // `.` is a named entry.
    let (out, _) = pwd_of(r#"mkdir -p a/b b; CDPATH=.:$PWD/a; cd b; echo "rc=$?""#);
    assert!(out.ends_with("/b\nrc=0\n"), "{out}");
    // Not found under CDPATH: the name itself is tried.
    let (out, _) = pwd_of(r#"mkdir -p a b; CDPATH=$PWD/a; cd b; echo "rc=$?""#);
    assert_eq!(out, "rc=0\n");
    let (out, _) = pwd_of(r#"mkdir b; CDPATH=/nonexistent; cd b; echo "rc=$?""#);
    assert_eq!(out, "rc=0\n");
    // `./b` and `../b` are never searched.
    let (out, _) = pwd_of(r#"mkdir -p a/b; CDPATH=$PWD/a; cd ./b 2>/dev/null; echo "rc=$?""#);
    assert_eq!(out, "rc=1\n");
    // Agent mode ignores CDPATH.
    let dir = scratch("cdpath_agent");
    let cdpath = dir.join("w").join("a");
    let (out, err, code) = ish_raw_env(
        r#"mkdir -p a/b; cd b; echo rc=$?"#,
        &dir,
        &[
            ("IRONSHELL_MODE", "agent"),
            ("CDPATH", &cdpath.to_string_lossy()),
        ],
    );
    assert!(err.contains("cd: b"), "{err}");
    assert_eq!(String::from_utf8_lossy(&out), "rc=1\n");
    assert_eq!(code, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn timeout_with_sigkill_exits_137() {
    if !have("sleep") {
        return;
    }
    check(
        "tmokill",
        &[
            (r#"timeout -s KILL 1 sleep 5; echo $?"#, b"137\n", 0),
            (r#"timeout -s 9 1 sleep 5; echo $?"#, b"137\n", 0),
            (r#"timeout -sKILL 1 sleep 5; echo $?"#, b"137\n", 0),
            (r#"timeout --signal=KILL 1 sleep 5; echo $?"#, b"137\n", 0),
            (
                r#"timeout --preserve-status -s KILL 1 sleep 5; echo $?"#,
                b"137\n",
                0,
            ),
            (r#"timeout -s TERM 1 sleep 5; echo $?"#, b"124\n", 0),
            (r#"timeout 1 sleep 5; echo $?"#, b"124\n", 0),
        ],
    );
}

/// A pipeline whose reader is a program, or whose source `timeout` runs.
#[test]
fn endless_producers_are_read_by_programs_and_timeouts() {
    check(
        "endless",
        &[
            (r#"timeout 1 yes | head -1; echo "rc=$?""#, b"y\nrc=0\n", 0),
            (r#"timeout 5 seq 1 3 | head -2"#, b"1\n2\n", 0),
        ],
    );
    if have("awk") {
        check(
            "endlessawk",
            &[
                (
                    r#"yes | awk 'NR==3{print; exit}'; echo "rc=$?""#,
                    b"y\nrc=0\n",
                    0,
                ),
                (
                    r#"seq 1 100000000 | awk 'NR==3{print; exit}'; echo "rc=$?""#,
                    b"3\nrc=0\n",
                    0,
                ),
                (
                    r#"seq 1 20 | awk 'NR==3{print; exit}'; echo "rc=$?""#,
                    b"3\nrc=0\n",
                    0,
                ),
            ],
        );
    }
}

/// A source the shell refuses fails the pipeline, shows no count from its
/// reader, and leaves no half-written file.
#[test]
fn a_refused_producer_fails_the_pipeline_and_leaves_no_file() {
    let dir = scratch("refused");
    let (out, err, code) = ish_raw("seq 1 100000000 | wc -l; echo rc=$?", &dir);
    assert_eq!(String::from_utf8_lossy(&out), "rc=1\n", "{err}");
    assert!(err.contains("seq: 100000000 numbers is more than"), "{err}");
    assert_eq!(code, 0);
    let _ = std::fs::remove_dir_all(&dir);
    check(
        "refusedfile",
        &[
            (
                r#"yes > f; echo rc=$?; test -e f || echo gone"#,
                b"rc=1\ngone\n",
                0,
            ),
            (r#"echo x > g; yes >> g; wc -c < g"#, b"2\n", 0),
            (r#"echo x > g; yes > g; wc -c < g"#, b"0\n", 0),
            (
                r#"cat /dev/zero > z; echo rc=$?; test -e z || echo gone"#,
                b"rc=1\ngone\n",
                0,
            ),
        ],
    );
}

#[test]
fn in_process_tools_are_found_whatever_path_says() {
    check(
        "pathtools",
        &[(
            r#"echo hi > f; PATH=/nonexistent; ls; echo rc=$?"#,
            b"f\nrc=0\n",
            0,
        )],
    );
}

/// The CR of a native Windows program is dropped on the script's stdout and
/// in a `>` file when the output is text, as on Linux.
#[test]
fn crlf_of_a_native_program_on_stdout_and_in_files() {
    if !cfg!(windows) {
        return;
    }
    check(
        "crlf",
        &[
            (r#"cmd /c echo x"#, b"x\n", 0),
            (r#"cmd /c echo x > out.txt; wc -c < out.txt"#, b"2\n", 0),
            (r#"cmd /c echo x >> out.txt; wc -c < out.txt"#, b"2\n", 0),
            (r#"cmd /c echo x | wc -c"#, b"2\n", 0),
            (r#"x=$(cmd /c echo hi); echo "${#x}""#, b"2\n", 0),
            // The in-process tools and this shell's own children keep a CR.
            (r#"printf 'a\r\n' > f; wc -c < f"#, b"3\n", 0),
            (r#"printf 'a\r\n'"#, b"a\r\n", 0),
        ],
    );
}

/// A bare name runs the program `command -v` names, not System32's. The shell
/// here is whichever `bash` and `sh` come first on PATH.
#[test]
fn bash_dash_c_takes_its_arguments() {
    if !have("bash") || !have("sh") {
        return;
    }
    // A single quote in the script text keeps the glob out: no file is named
    // `x` or `2` here, and brackets would be a pattern.
    check(
        "bashc",
        &[
            (
                r#"bash -c 'echo "$1|$#|$0"' _ x y; sh -c 'echo "$1|$#|$0"' a b c d"#,
                b"x|2|_\nb|3|a\n",
                0,
            ),
            (r#"bash -c 'echo "$0 $@"' a b c"#, b"a b c\n", 0),
            (r#"printf 'echo hi\n' | bash"#, b"hi\n", 0),
        ],
    );
}

#[test]
fn a_bare_name_runs_what_command_v_names() {
    if !cfg!(windows) || !have("expand") {
        return;
    }
    let dir = scratch("expand");
    let (out, _, _) = ish_raw("command -v expand", &dir);
    let named = String::from_utf8_lossy(&out).to_lowercase();
    // Where System32's own is the first on PATH, `command -v` names it too.
    if named.contains("system32") || named.contains("windows") {
        return;
    }
    let (out, err, code) = ish_raw(r#"printf 'a\tb\n' | expand"#, &dir);
    assert_eq!(String::from_utf8_lossy(&out), "a       b\n", "{err}");
    assert_eq!(code, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A shell script found on PATH that is outside the subset runs with the real
/// `sh`; with none on PATH it is refused, naming the script and the construct.
#[test]
fn a_script_on_path_outside_the_subset() {
    if !cfg!(windows) || !have("sh") {
        return;
    }
    let setup = r#"mkdir bin; printf '#!/bin/sh\ncase $1 in a) echo A;; *) echo other;; esac\n' > bin/mycase; "#;
    check(
        "pathcase",
        &[
            (
                &format!(r#"{setup}PATH="$PWD/bin:$PATH"; mycase a; mycase b"#),
                b"A\nother\n",
                0,
            ),
            // A script the subset runs keeps running in process.
            (
                r#"mkdir bin; printf '#!/bin/sh\necho "plain $1"\n' > bin/myplain; PATH="$PWD/bin:$PATH"; myplain z"#,
                b"plain z\n",
                0,
            ),
        ],
    );
    let dir = scratch("pathcase2");
    let (out, err, code) = ish_raw(
        &format!(r#"{setup}PATH="$PWD/bin"; mycase a; echo rc=$?"#),
        &dir,
    );
    assert_eq!(String::from_utf8_lossy(&out), "rc=77\n", "{err}");
    assert!(err.contains("mycase"), "{err}");
    assert!(err.contains("case"), "{err}");
    assert!(err.contains("found on PATH"), "{err}");
    assert_eq!(code, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

//! Round 64c: differences between the in-process POSIX engine (`ish -b`) and
//! bash 5.2 and GNU coreutils / sed / grep found in round 64: `$(< file)`;
//! `%`-patterns and defaults that start with a blank; the status of a loop
//! left by `break` or `continue`; `/dev/zero`, `/dev/urandom` and the other
//! devices; an endless producer (`yes`, a huge `seq`) read by `sed Nq`,
//! `grep -m` and the stages between; `$SECONDS`, `$MACHTYPE` and `$BASHPID`;
//! `grep -E '\d'`; a second `^` in a BRE; `wc` of `-` and of a directory;
//! `timeout`; command lookup on the shell's own PATH; the CR of a native
//! Windows program in a pipe; and the refusal hints for `select`, `umask`,
//! `read -t`, `read -n`, `export -n`, `exec`, a subshell and `PIPESTATUS`.
//!
//! Every expected stdout and exit status in the `check` tables was produced by
//! GNU bash 5.2 and GNU coreutils / sed / grep on Debian with the same script.
//! Cases that cannot be run under bash (this shell's own decisions: the cap on
//! an endless stream, the refusals) are written by hand. The bytes are asserted
//! as they are, not through `od`, whose output differs on macOS.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_r64c_{tag}_{}", std::process::id()));
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
fn command_substitution_of_a_file() {
    check(
        "csub",
        &[
            (r#"printf 'x\n\n\n' > f; x=$(<f); echo "[$x]""#, b"[x]\n", 0),
            (
                r#"printf 'a\nb\n' > f; echo "$(<f)"; echo "[$(< f)]""#,
                b"a\nb\n[a\nb]\n",
                0,
            ),
            (
                r#"printf 'hi\n' > f; g=f; echo "[$(< "$g")] [$(<$g)]""#,
                b"[hi] [hi]\n",
                0,
            ),
            (r#"x=$(<nofile); echo "[$x] rc=$?""#, b"[] rc=1\n", 0),
            (
                r#"printf 'v\n' > f; echo "pre-$(<f)-post""#,
                b"pre-v-post\n",
                0,
            ),
            (r#"printf 'v\n' > f; echo "a $(<f) b" | wc -c"#, b"6\n", 0),
        ],
    );
}

#[test]
fn patterns_and_defaults_that_start_with_a_blank() {
    check(
        "blank",
        &[
            (r#"v="a b c"; echo "${v%% *}""#, b"a\n", 0),
            (r#"v="a b c"; echo "${v% *}""#, b"a b\n", 0),
            (r#"v="a b c "; echo "[${v% }]""#, b"[a b c]\n", 0),
            (
                r#"v="a b c"; echo "${v#* }|${v## *}|${v##* }""#,
                b"b c|a b c|c\n",
                0,
            ),
            (r#"v="a  b"; echo "${v%% =*}|${v% b}""#, b"a  b|a \n", 0),
            (
                r#"unset u; echo "[${u:- default}]"; echo "[${u- d2}]"; echo "[${u:+ plus}]"; u=; echo "[${u:+ plus}]""#,
                b"[ default]\n[ d2]\n[]\n[]\n",
                0,
            ),
            (
                r#"u=; echo "[${u:= set}]"; echo "[$u]""#,
                b"[ set]\n[ set]\n",
                0,
            ),
        ],
    );
}

#[test]
fn a_loop_left_by_break_or_continue_succeeds() {
    check(
        "loopst",
        &[
            (
                r#"for i in 1 2; do [ $i = 2 ] && break; done; echo $?"#,
                b"0\n",
                0,
            ),
            (
                r#"i=0; while [ $i -lt 3 ]; do i=$((i+1)); [ $i = 2 ] && break; done; echo $?"#,
                b"0\n",
                0,
            ),
            (
                r#"i=0; until [ $i -ge 3 ]; do i=$((i+1)); [ $i = 3 ] && break; false; done; echo $?"#,
                b"0\n",
                0,
            ),
            (
                r#"for i in 1 2 3; do [ $i = 3 ] && continue; true; done; echo $?"#,
                b"0\n",
                0,
            ),
            (
                r#"for i in 1 2 3; do false; continue; done; echo $?"#,
                b"0\n",
                0,
            ),
            (
                r#"for i in 1 2; do for j in 1 2; do false; [ $j = 1 ] && break 2; done; done; echo $?"#,
                b"0\n",
                0,
            ),
            (
                r#"for i in 1 2; do for j in 1 2; do false; break; done; done; echo $?"#,
                b"0\n",
                0,
            ),
            (
                r#"for i in 1 2 3; do [ $i = 2 ] && break; false; done; echo $?"#,
                b"0\n",
                0,
            ),
            (r#"for i in 1 2; do false; done; echo $?"#, b"1\n", 0),
            (
                r#"printf '1\n2\n' | while read n; do [ $n = 2 ] && break; done; echo $?"#,
                b"0\n",
                0,
            ),
        ],
    );
}

#[test]
fn endless_devices_read_as_far_as_the_reader_wants() {
    check(
        "dev",
        &[
            (r#"head -c 16 /dev/urandom | wc -c"#, b"16\n", 0),
            (
                r#"tr -dc a-z0-9 </dev/urandom | head -c 12 | wc -c"#,
                b"12\n",
                0,
            ),
            (r#"cat /dev/zero | head -c 3 | wc -c"#, b"3\n", 0),
            (r#"head -n 1 /dev/urandom | wc -l"#, b"1\n", 0),
            (
                r#"tr -dc 'A-Za-z' < /dev/urandom | head -c 20 | grep -c '^[A-Za-z]*$'"#,
                b"1\n",
                0,
            ),
            (r#"head -c 5 /dev/zero | wc -c"#, b"5\n", 0),
            (r#"head -c 3 /dev/zero"#, b"\0\0\0", 0),
        ],
    );
    // Random bytes: the length is fixed, the content differs from run to run.
    let dir = scratch("devr");
    let (a, _, _) = ish_raw("head -c 32 /dev/urandom", &dir);
    let (b, _, _) = ish_raw("head -c 32 /dev/random", &dir);
    assert_eq!((a.len(), b.len()), (32, 32));
    assert_ne!(a, b);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn devices_exist_for_test() {
    check(
        "devt",
        &[
            (
                r#"[ -e /dev/null ] && echo e; [ -c /dev/null ] && echo c; [ -r /dev/null ] && echo r; [ -w /dev/null ] && echo w; [ -f /dev/null ] || echo nf; [ -d /dev/null ] || echo nd"#,
                b"e\nc\nr\nw\nnf\nnd\n",
                0,
            ),
            (
                r#"[ -e /dev/urandom ] && echo u; [ -c /dev/zero ] && echo z; [ -e /dev/random ] && echo r"#,
                b"u\nz\nr\n",
                0,
            ),
        ],
    );
}

#[test]
fn an_endless_producer_stops_when_a_later_stage_does() {
    check(
        "endless",
        &[
            (r#"yes | head -3"#, b"y\ny\ny\n", 0),
            (r#"yes abc def | head -2"#, b"abc def\nabc def\n", 0),
            (r#"yes | sed 5q | wc -l"#, b"5\n", 0),
            (r#"yes | grep -m2 y"#, b"y\ny\n", 0),
            (r#"yes | cat | head -1"#, b"y\n", 0),
            (r#"yes | tr y z | head -1"#, b"z\n", 0),
            (r#"yes | cut -c1 | head -2"#, b"y\ny\n", 0),
            (r#"yes | head -2 | tr y z"#, b"z\nz\n", 0),
            (r#"yes | head -1 | wc -l"#, b"1\n", 0),
            (r#"yes | head -c 5"#, b"y\ny\ny", 0),
            (r#"seq 1 100000000 | head -1"#, b"1\n", 0),
            (r#"seq 1 100000000 | head -3 | tail -1"#, b"3\n", 0),
            (r#"set -o pipefail; yes | head -1; echo $?"#, b"y\n141\n", 0),
            (
                r#"set -o pipefail; yes | sed 2q; echo $?"#,
                b"y\ny\n141\n",
                0,
            ),
            (
                r#"set -o pipefail; seq 1 100000000 | head -1; echo $?"#,
                b"1\n141\n",
                0,
            ),
            (r#"yes | head -1 > /dev/null; echo $?"#, b"0\n", 0),
        ],
    );
}

#[test]
fn an_endless_producer_nothing_stops_is_refused_not_hung() {
    for script in [
        "yes | wc -l; echo rc=$?",
        "yes | sort | head -1; echo rc=$?",
        "yes > /dev/null; echo rc=$?",
        "cat /dev/zero | wc -c; echo rc=$?",
    ] {
        let dir = scratch("noend");
        let (out, err, _) = ish_raw(script, &dir);
        // The pipeline gives no output and fails; the script goes on.
        assert_eq!(String::from_utf8_lossy(&out), "rc=1\n", "{script}: {err}");
        assert!(err.contains("endless input"), "{script}: {err}");
        assert!(err.contains("head, sed Nq or grep -m"), "{script}: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    // The old cap still names itself for a seq nothing reads in part.
    let dir = scratch("seqcap");
    let (out, err, _) = ish_raw("seq 1 100000000 | wc -l", &dir);
    assert_eq!(String::from_utf8_lossy(&out), "0\n");
    assert!(err.contains("seq: 100000000 numbers is more than"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shell_variables_seconds_machtype_bashpid() {
    check(
        "vars",
        &[
            (r#"echo $SECONDS | grep -c '^[0-9]*$'"#, b"1\n", 0),
            (r#"set -u; echo "$SECONDS" | grep -c '^[0-9]*$'"#, b"1\n", 0),
            (r#"echo "$BASHPID" | grep -c '^[0-9][0-9]*$'"#, b"1\n", 0),
        ],
    );
    let arch = std::env::consts::ARCH;
    let want = if cfg!(windows) {
        format!("{arch}-pc-msys\n")
    } else if cfg!(target_os = "macos") {
        format!("{arch}-apple-darwin\n")
    } else {
        format!("{arch}-pc-linux-gnu\n")
    };
    check("mach", &[(r#"echo "$MACHTYPE""#, want.as_bytes(), 0)]);
}

#[test]
fn regex_details() {
    check(
        "re",
        &[
            (
                r#"echo 'a1d' | grep -E '\d'; echo rc=$?"#,
                b"a1d\nrc=0\n",
                0,
            ),
            (r#"echo 'a1' | grep -E '\d'; echo rc=$?"#, b"rc=1\n", 0),
            (
                r#"echo 'a1d' | grep -E '\d+' ; echo rc=$?"#,
                b"a1d\nrc=0\n",
                0,
            ),
            (r#"echo '1' | grep -E '\w'; echo rc=$?"#, b"1\nrc=0\n", 0),
            (r#"echo 'x' | grep -E '\s'; echo rc=$?"#, b"rc=1\n", 0),
            (r#"echo '^x' | sed 's/^^x/y/'"#, b"y\n", 0),
            (r#"echo 'x' | sed 's/^^x/y/'"#, b"x\n", 0),
            (r#"echo '^^x' | sed 's/^^^x/y/'"#, b"y\n", 0),
            (r#"echo '^*x' | sed 's/^^*x/y/'"#, b"^*x\n", 0),
        ],
    );
}

#[test]
fn wc_of_dash_and_of_a_directory() {
    check(
        "wc",
        &[
            (r#"printf 'a\nb\n' > f; wc - < f"#, b"2 2 4 -\n", 0),
            (r#"printf 'a\nb\n' > f; wc -l - < f"#, b"2 -\n", 0),
            (r#"mkdir d; wc -l d; echo rc=$?"#, b"0 d\nrc=1\n", 0),
            (
                r#"mkdir d; wc d; echo rc=$?"#,
                b"      0       0       0 d\nrc=1\n",
                0,
            ),
            (
                r#"mkdir d; wc -c d f 2>&1; echo rc=$?"#,
                b"      0 d\nwc: d: Is a directory\nwc: f: No such file or directory\n      0 total\nrc=1\n",
                0,
            ),
            (
                r#"printf 'a\n' > f; cat f | wc -"#,
                b"      1       1       2 -\n",
                0,
            ),
        ],
    );
}

#[test]
fn tr_deletes_the_complement() {
    check(
        "trdc",
        &[
            (r#"printf 'a1b2-c\n' | tr -dc 'a-z'"#, b"abc", 0),
            (r#"printf 'a1b2-c\n' | tr -cd '0-9'"#, b"12", 0),
            (r#"printf 'a1b2-c\n' | tr -dc 'a-z\n'"#, b"abc\n", 0),
        ],
    );
}

#[test]
fn timeout_runs_the_command_and_ends_it() {
    check(
        "tmo",
        &[
            (r#"timeout 5 echo hi; echo $?"#, b"hi\n0\n", 0),
            (r#"timeout 1m echo hi; echo $?"#, b"hi\n0\n", 0),
            (r#"timeout abc echo hi; echo $?"#, b"125\n", 0),
            (r#"timeout 5; echo $?"#, b"125\n", 0),
            (r#"timeout; echo $?"#, b"125\n", 0),
            (r#"echo hi | timeout 5 cat; echo $?"#, b"hi\n0\n", 0),
            (r#"timeout 2 cat /dev/null; echo $?"#, b"0\n", 0),
            (r#"command -v timeout >/dev/null && echo has"#, b"has\n", 0),
        ],
    );
    if have("sleep") {
        check(
            "tmo2",
            &[
                (r#"timeout 1 sleep 5; echo $?"#, b"124\n", 0),
                (r#"timeout 0.5 sleep 3; echo $?"#, b"124\n", 0),
            ],
        );
    }
}

/// A program is looked up on the shell's own PATH variable, as `command -v`
/// looks it up: after an export, after a plain assignment, and for one command
/// with a prefix assignment. A script with a shebang runs too.
#[test]
fn commands_are_found_on_the_shells_path() {
    let setup = r#"mkdir bin; printf '#!/bin/sh\necho "mytool ran $*"\n' > bin/mytool; chmod +x bin/mytool 2>/dev/null; "#;
    // Where nothing can make the file run (no `chmod`, no `/bin/sh`), skip.
    if !cfg!(windows) && !(have("chmod") && Path::new("/bin/sh").exists()) {
        return;
    }
    check(
        "path",
        &[
            (
                &format!(r#"{setup}export PATH="$PWD/bin:$PATH"; mytool a b"#),
                b"mytool ran a b\n",
                0,
            ),
            (
                &format!(r#"{setup}PATH="$PWD/bin:$PATH"; mytool"#),
                b"mytool ran \n",
                0,
            ),
            (
                &format!(r#"{setup}PATH=$PWD/bin mytool x"#),
                b"mytool ran x\n",
                0,
            ),
            (
                &format!(r#"{setup}PATH="$PWD/bin:$PATH"; command -v mytool | sed 's/.*\///'"#),
                b"mytool\n",
                0,
            ),
            (
                &format!(r#"{setup}command -v mytool; echo rc=$?"#),
                b"rc=1\n",
                0,
            ),
        ],
    );
    // A name the changed PATH does not hold is not found, even when the
    // system's own lookup would find it.
    check(
        "nopath",
        &[(
            r#"PATH=/nonexistent/dir; nosuchprogram_r64; echo rc=$?"#,
            b"rc=127\n",
            0,
        )],
    );
}

/// In agent mode PATH is a steering variable: exporting it is refused, and a
/// plain assignment does not change which program runs.
#[test]
fn agent_mode_keeps_the_path_refusal() {
    let dir = scratch("agentpath");
    let (out, err, _) = ish_raw_env(
        r#"export PATH="$PWD/bin:$PATH"; echo rc=$?"#,
        &dir,
        &[("IRONSHELL_MODE", "agent")],
    );
    // The export fails with the policy status; the script goes on.
    assert_eq!(String::from_utf8_lossy(&out), "rc=77\n", "{err}");
    assert!(err.contains("E_POLICY_DENY"), "{err}");
    assert!(err.contains("PATH"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
    let dir = scratch("agentpath2");
    let (out, err, code) = ish_raw_env(
        r#"PATH=/nonexistent/dir; echo ran"#,
        &dir,
        &[("IRONSHELL_MODE", "agent")],
    );
    // The plain assignment stays in the shell; the in-process echo still runs.
    assert_eq!(
        (String::from_utf8_lossy(&out).into_owned(), code),
        ("ran\n".to_string(), 0),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The CR of a native Windows program's line ends is dropped when its output
/// feeds a pipeline stage, as it is in `$( )`; output to a file or a terminal
/// keeps it. The in-process tools are not touched.
#[test]
fn crlf_of_a_native_program_in_a_pipe() {
    // The in-process tools write what they are given, on every platform.
    check(
        "crlfi",
        &[
            (r#"printf 'a\r\nb\r\n' | wc -c"#, b"6\n", 0),
            (r#"printf 'a\r\n' > f; wc -c < f"#, b"3\n", 0),
        ],
    );
    if !cfg!(windows) {
        return;
    }
    check(
        "crlfw",
        &[
            (r#"cmd /c echo x | wc -c"#, b"2\n", 0),
            (
                r#"cmd /c echo x | while read n; do [ "$n" = x ] && echo same; done"#,
                b"same\n",
                0,
            ),
            (r#"cmd /c echo x | grep -x x | wc -l"#, b"1\n", 0),
            (r#"cmd /c echo x | cat | wc -c"#, b"2\n", 0),
            (r#"cmd /c echo x > out.txt; wc -c < out.txt"#, b"3\n", 0),
            (r#"x=$(cmd /c echo hi); echo "${#x}""#, b"2\n", 0),
        ],
    );
}

#[test]
fn refusal_hints_for_the_rarer_constructs() {
    refused(
        "s1",
        "select x in a b; do echo $x; break; done",
        &["select loop", "for x in a b"],
    );
    refused("s2", "umask 022", &["umask", "chmod"]);
    refused("s3", "read -t 1 x", &["read -t", "timeout N cmd"]);
    refused("s4", "read -n 1 x", &["read -r -n 1 x", "-r"]);
    refused("s5", "export -n A", &["export -n", "unset NAME"]);
    refused("s6", "exec 2>&1", &["exec", "{ cmd1; cmd2; } >f 2>&1"]);
    refused(
        "s7",
        "(cd d && ls)",
        &[
            "cd d && cmd; cd -",
            "{ cd d && cmd; }",
            "keeps the directory change",
        ],
    );
    refused(
        "s8",
        "false | true; echo ${PIPESTATUS[0]}",
        &["`PIPESTATUS` array", "set -o pipefail"],
    );
    refused("s9", "echo hi >&out.txt", &["> FILE 2>&1"]);
}

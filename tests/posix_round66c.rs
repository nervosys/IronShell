//! Round 66c: differences between the in-process POSIX engine (`ish -b`) and
//! bash 5.2 and GNU coreutils found in round 66: a `>` or `>>` target is
//! created or truncated before its command runs; commands inside a redirected
//! loop or group share the offset of the redirected input; `timeout -s` with a
//! signal GNU does not know; a script with no shebang run as a command; an
//! endless producer read by `sed` with `q` in a block or by `tee | head`.
//!
//! Every expected stdout and exit status in the `check` tables was produced by
//! GNU bash 5.2 and GNU coreutils on Debian with the same script. Cases that
//! cannot be run under bash (this shell's own decisions, the approval gate, the
//! journal) are written by hand. The bytes are asserted as they are, not
//! through `od`, whose output differs on macOS.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ae_r66c_{tag}_{}", std::process::id()));
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

#[test]
fn a_redirection_target_exists_before_its_command_runs() {
    check(
        "open",
        &[
            ("touch a b; ls > o; cat o", b"a\nb\no\n", 0),
            (
                "touch a b; for f in *; do echo $f; done > o; cat o",
                b"a\nb\no\n",
                0,
            ),
            // The words are expanded first: a new `o` is not in `*`.
            ("touch a b; echo * > o; cat o", b"a b\n", 0),
            ("touch a b; echo old > o; echo * > o; cat o", b"a b o\n", 0),
            ("touch a b; ls | sort > o; cat o", b"a\nb\no\n", 0),
            ("printf 'b\\na\\n' > f; sort f > f; wc -c < f", b"0\n", 0),
            ("echo a > f; cat f | sed 's/a/b/' > f; wc -c < f", b"0\n", 0),
            ("echo a > f; { cat f; echo b; } > f; cat f", b"b\n", 0),
            (
                "echo x > g; cat g > g; echo rc=$?; wc -c < g",
                b"rc=0\n0\n",
                0,
            ),
            ("echo x > g; cat g >> g; echo rc=$?; cat g", b"rc=1\nx\n", 0),
            (
                "echo x > o1 > o2; ls; wc -c < o1; cat o2",
                b"o1\no2\n0\nx\n",
                0,
            ),
            ("touch a; ls . 2> e; wc -c < e", b"a\ne\n0\n", 0),
            ("rm -f n; : >> n; ls n", b"n\n", 0),
        ],
    );
}

#[test]
fn a_redirection_that_cannot_open_stops_its_command() {
    check(
        "openfail",
        &[
            ("mkdir d; echo hi > d; echo rc=$?", b"rc=1\n", 0),
            ("mkdir d; { echo ran; } > d; echo rc=$?", b"rc=1\n", 0),
            ("echo hi > nodir/x || echo failed", b"failed\n", 0),
            ("echo hi > ../x; echo rc=$?", b"rc=1\n", 0),
        ],
    );
    // The jail judges a path outside the workspace before anything is made.
    let dir = scratch("openjail");
    let (_, err, _) = ish_raw("echo hi > ../outside; echo rc=$?", &dir);
    assert!(!dir.join("outside").exists(), "stderr {err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tee_opens_its_files_before_it_reads() {
    check(
        "tee",
        &[
            ("touch a; ls | tee t >/dev/null; wc -l < t", b"2\n", 0),
            ("echo old > t; printf '' | tee t; wc -c < t", b"0\n", 0),
            ("echo x | tee u v; cat u v", b"x\nx\nx\n", 0),
            // A file that exists is not truncated before an earlier stage reads it.
            (
                "printf 'b\\na\\n' > f; sort f | tee f > /dev/null; wc -c < f",
                b"4\n",
                0,
            ),
        ],
    );
}

#[test]
fn commands_in_a_redirected_loop_share_the_input_offset() {
    check(
        "cursor",
        &[
            (
                "printf '1\\n2\\n3\\n' > f; while read l; do x=$(cat); echo \"$l:[$x]\"; done < f",
                b"1:[2\n3]\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n' > f; while read l; do x=$(head -1); echo \"$l:[$x]\"; done < f",
                b"1:[2]\n3:[]\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; { head -1; cat; } < f",
                b"1\n2\n3\n4\n5\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; { head -n -1; echo ==; cat; } < f",
                b"1\n2\n3\n4\n==\n5\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; { head -c3; echo; echo ==; cat; } < f",
                b"1\n2\n==\n\n3\n4\n5\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; { head -n0; echo ==; cat; } < f",
                b"==\n1\n2\n3\n4\n5\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; { grep -m1 2; echo ==; cat; } < f",
                b"2\n==\n3\n4\n5\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; { grep -m1 -A1 2; echo ==; cat; } < f",
                b"2\n3\n==\n3\n4\n5\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; { sed 1q; echo ==; cat; } < f",
                b"1\n==\n2\n3\n4\n5\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; { sed -n 1p; echo ==; cat; } < f",
                b"1\n==\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; { sed -n '2{p;q}'; echo ==; cat; } < f",
                b"2\n==\n3\n4\n5\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; { tail -n1; echo ==; cat; } < f",
                b"5\n==\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; { read a; read b; echo \"$a $b\"; cat; } < f",
                b"1 2\n3\n4\n5\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; for i in 1 2; do head -n1; done < f",
                b"1\n2\n",
                0,
            ),
            (
                "printf '1\\n2\\n3\\n4\\n5\\n' > f; { head -n1 | cat; echo ==; cat; } < f",
                b"1\n==\n2\n3\n4\n5\n",
                0,
            ),
            (
                "printf 'a\\nb' > h; { head -n1; echo ==; cat; } < h",
                b"a\n==\nb",
                0,
            ),
            // A pipe is not seekable: `head` takes what it read ahead.
            (
                "printf '1\\n2\\n3\\n' | { head -n1; echo ==; cat; }",
                b"1\n==\n",
                0,
            ),
        ],
    );
}

#[test]
fn timeout_refuses_a_signal_gnu_does_not_know() {
    let dir = scratch("sig");
    let (out, err, code) = ish_raw("timeout -s FOO 1 echo ran", &dir);
    assert_eq!((out.as_slice(), code), (&b""[..], 125), "stderr {err}");
    assert!(
        err.contains("timeout: 'FOO': invalid signal"),
        "stderr {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
    check(
        "sigok",
        &[
            ("timeout -s TERM 5 echo a", b"a\n", 0),
            ("timeout -s SIGTERM 5 echo a", b"a\n", 0),
            ("timeout -s term 5 echo a", b"a\n", 0),
            ("timeout -s 9 5 echo a", b"a\n", 0),
            ("timeout -s 0 5 echo a", b"a\n", 0),
            ("timeout -s RTMIN+1 5 echo a", b"a\n", 0),
            ("timeout --signal=USR1 5 echo a", b"a\n", 0),
            ("timeout -s 65 1 echo ran; echo rc=$?", b"rc=125\n", 0),
            ("timeout -s '' 1 echo ran; echo rc=$?", b"rc=125\n", 0),
            (
                "timeout -s SIGSIGTERM 1 echo ran; echo rc=$?",
                b"rc=125\n",
                0,
            ),
            ("timeout -s 0x9 1 echo ran; echo rc=$?", b"rc=125\n", 0),
            ("timeout -k abc 1 echo ran; echo rc=$?", b"rc=125\n", 0),
            ("timeout -s; echo rc=$?", b"rc=125\n", 0),
            ("timeout 5; echo rc=$?", b"rc=125\n", 0),
        ],
    );
}

#[test]
fn an_endless_producer_is_stopped_by_sed_with_q_in_a_block_and_by_tee_head() {
    check(
        "endless",
        &[
            ("yes | sed -n '3{p;q}'", b"y\n", 0),
            ("yes | sed -n 'p;q'", b"y\n", 0),
            ("yes | tee f | head -1", b"y\n", 0),
        ],
    );
}

#[cfg(windows)]
#[test]
fn a_script_with_no_shebang_runs_as_a_shell_script() {
    check(
        "noshebang",
        &[
            (
                "printf 'echo hi $1\\n' > s1; chmod +x s1; ./s1 there; echo rc=$?",
                b"hi there\nrc=0\n",
                0,
            ),
            (
                "printf 'echo hi\\n' > s.sh; chmod +x s.sh; ./s.sh; echo rc=$?",
                b"hi\nrc=0\n",
                0,
            ),
            (
                "printf 'echo hi\\n' > s.sh; chmod +x s.sh; PATH=$PWD:$PATH; s.sh; echo rc=$?",
                b"hi\nrc=0\n",
                0,
            ),
            ("touch x; ./x 2>/dev/null; echo rc=$?", b"rc=126\n", 0),
        ],
    );
}

/// In agent mode an external command is judged by the approval gate before a
/// `>` target is created, so a refused command leaves the file as it was. An
/// in-process command keeps bash's order, and the journal records the
/// truncation of `sort f > f` so `undo` restores it.
#[test]
fn a_refused_program_leaves_its_redirection_target_alone() {
    let dir = scratch("gate");
    let work = dir.join("w");
    std::fs::write(work.join("a"), b"keep\n").unwrap();
    let (out, err, code) = ish_raw_env("env printf x > a", &dir, &[("IRONSHELL_MODE", "agent")]);
    assert_eq!(code, 75, "stdout {out:?} stderr {err}");
    assert_eq!(std::fs::read(work.join("a")).unwrap(), b"keep\n");
    // Also as a later stage of a pipeline, whose files are otherwise opened
    // before the first stage runs.
    for script in ["echo x | env printf y > a", "echo x | env printf y >> a"] {
        let (out, err, code) = ish_raw_env(script, &dir, &[("IRONSHELL_MODE", "agent")]);
        assert_eq!(code, 75, "{script}: stdout {out:?} stderr {err}");
        assert_eq!(
            std::fs::read(work.join("a")).unwrap(),
            b"keep\n",
            "{script}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_journalled_truncation_is_undone() {
    let dir = scratch("journal");
    let work = dir.join("w");
    std::fs::write(work.join("f"), b"b\na\n").unwrap();
    let (out, err, code) = ish_raw_env(
        "sort f > f; wc -c < f",
        &dir,
        &[("IRONSHELL_MODE", "agent")],
    );
    assert_eq!((out.as_slice(), code), (&b"0\n"[..], 0), "stderr {err}");
    assert_eq!(std::fs::read(work.join("f")).unwrap(), b"");
    // The same session undoes it with one step.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["--agent", "-c", "undo()"])
        .current_dir(&work)
        .env("IRONSHELL_SESSION_DIR", dir.join("state"))
        .env("IRONSHELL_WORKSPACE", &work);
    let undone = cmd.output().expect("run ish");
    assert!(undone.status.success(), "{:?}", undone);
    assert_eq!(std::fs::read(work.join("f")).unwrap(), b"b\na\n");
    let _ = std::fs::remove_dir_all(&dir);
}

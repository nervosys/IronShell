//! Round 36: the POSIX subset (`ish -b`) against bash 5.2 and GNU coreutils/grep
//! (WSL Debian, `LC_ALL=C.utf8`). Each expected string below is what bash printed.
//!
//! Each case runs the binary with `-b -c` in a fresh temp folder, with
//! IRONSHELL_SESSION_DIR set to a folder of its own and an empty PATH, which
//! makes these cases prove the in-process code ran.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("ae-round36-posix-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("session")).unwrap();
        std::fs::create_dir_all(root.join("empty-path")).unwrap();
        Sandbox(root)
    }

    fn run_args(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(args)
            .current_dir(&self.0)
            .env("NO_COLOR", "1")
            .env("LC_ALL", "C")
            .env("PATH", self.0.join("empty-path"))
            .env("IRONSHELL_SESSION_DIR", self.0.join("session"))
            .output()
            .expect("run ish")
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let out = self.run_args(&["-b", "-c", script]);
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    /// The script's stdout, which must have run cleanly.
    fn ok(&self, script: &str) -> String {
        let (out, err, code) = self.run(script);
        assert_eq!(code, 0, "{script}: stderr {err:?}");
        out
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const TREE: &str = "mkdir -p src/sub src/skip
printf 'TODO a\\nx\\nTODO b\\n' > src/a.txt
printf 'TODO c\\n' > src/b.md
printf 'TODO d\\n' > src/sub/c.txt
printf 'TODO e\\n' > src/skip/d.txt
";

// ── 1. grep --include / --exclude / --exclude-dir ────────────────────────

#[test]
fn grep_include_keeps_the_files_whose_base_name_matches() {
    let sb = Sandbox::new("include");
    let tree = TREE;
    // bash: grep -r --include='*.txt' -c TODO src | sort
    let want = "src/a.txt:2\nsrc/skip/d.txt:1\nsrc/sub/c.txt:1\n";
    assert_eq!(
        sb.ok(&format!(
            "{tree}grep -r --include='*.txt' -c TODO src | sort"
        )),
        want
    );
    // The two-word form takes the glob from the next argument.
    assert_eq!(sb.ok("grep -r --include '*.txt' -c TODO src | sort"), want);
    // Several --include globs add up.
    assert_eq!(
        sb.ok("grep -r --include='*.md' --include='*.txt' -l TODO src | sort"),
        "src/a.txt\nsrc/b.md\nsrc/skip/d.txt\nsrc/sub/c.txt\n"
    );
    // No file matches: status 1 and no output.
    assert_eq!(
        sb.ok("grep -r --include='src' -l TODO src; echo rc=$?"),
        "rc=1\n"
    );
}

#[test]
fn grep_exclude_and_exclude_dir_leave_files_and_directories_out() {
    let sb = Sandbox::new("exclude");
    let tree = TREE;
    assert_eq!(
        sb.ok(&format!("{tree}grep -r --exclude='*.txt' TODO src | sort")),
        "src/b.md:TODO c\n"
    );
    assert_eq!(
        sb.ok("grep -r --exclude-dir=skip TODO src | sort"),
        "src/a.txt:TODO a\nsrc/a.txt:TODO b\nsrc/b.md:TODO c\nsrc/sub/c.txt:TODO d\n"
    );
    assert_eq!(
        sb.ok("grep -r --exclude-dir skip -l TODO src | sort"),
        "src/a.txt\nsrc/b.md\nsrc/sub/c.txt\n"
    );
    assert_eq!(
        sb.ok("grep -r --exclude='a*' --exclude-dir='sub' -l TODO src | sort"),
        "src/b.md\nsrc/skip/d.txt\n"
    );
    // A glob with a slash is matched against the base name only, so it leaves
    // nothing out of a walk (bash lists every file).
    assert_eq!(
        sb.ok("grep -r --exclude='src/a.txt' -l TODO src | sort"),
        "src/a.txt\nsrc/b.md\nsrc/skip/d.txt\nsrc/sub/c.txt\n"
    );
}

#[test]
fn grep_include_and_exclude_judge_a_file_named_on_the_command_line() {
    let sb = Sandbox::new("cmdline");
    let tree = TREE;
    // bash: grep --include='*.txt' TODO src/a.txt src/b.md
    assert_eq!(
        sb.ok(&format!(
            "{tree}grep --include='*.txt' TODO src/a.txt src/b.md"
        )),
        "src/a.txt:TODO a\nsrc/a.txt:TODO b\n"
    );
    // bash: grep --exclude='*.txt' TODO src/a.txt src/b.md
    assert_eq!(
        sb.ok("grep --exclude='*.txt' TODO src/a.txt src/b.md"),
        "src/b.md:TODO c\n"
    );
    // --exclude-dir leaves out a directory named on the command line too.
    assert_eq!(
        sb.ok("grep -r --exclude-dir='src' -l TODO src; echo rc=$?"),
        "rc=1\n"
    );
}

#[cfg(windows)]
#[test]
fn an_unsupported_grep_flag_is_named_not_the_flag_before_it() {
    let sb = Sandbox::new("named");
    let tree = TREE;
    // `--color=never` is a real GNU option the subset lacks. The refusal names
    // it, not the `-r` that is supported.
    let (out, err, code) = sb.run(&format!("{tree}grep -r --color=never -c TODO src"));
    assert_eq!((out.as_str(), code), ("", 2), "{err}");
    assert!(
        err.contains("ish: grep: the in-process grep does not support --color=never; a GNU-compatible grep on PATH would run it"),
        "{err}"
    );
    assert!(!err.contains("does not support -r"), "{err}");
}

// ── 2. printf flags ──────────────────────────────────────────────────────

#[test]
fn printf_takes_the_c_flags() {
    let sb = Sandbox::new("printf");
    for (script, want) in [
        (r"printf '%05d|%+d|%3d|\n' 42 7 5", "00042|+7|  5|\n"),
        (r"printf '[%+d] [%+d] [%+d]\n' 7 -7 0", "[+7] [-7] [+0]\n"),
        (r"printf '[% d] [% d] [% d]\n' 7 -7 0", "[ 7] [-7] [ 0]\n"),
        (
            r"printf '[%+.2f] [%+.2f] [% .1f] [%+f]\n' 3.14159 -2 5 0",
            "[+3.14] [-2.00] [ 5.0] [+0.000000]\n",
        ),
        (
            r"printf '[%#x] [%#X] [%#o] [%#x] [%#o]\n' 255 255 8 0 0",
            "[0xff] [0XFF] [010] [0] [0]\n",
        ),
        (
            r"printf '[%+5d] [%-+5d] [%+05d] [% 05d] [%-05d]\n' 7 7 7 7 7",
            "[   +7] [+7   ] [+0007] [ 0007] [7    ]\n",
        ),
        (
            r"printf '[%+x] [% x] [%+o] [%+u] [% u]\n' 255 255 8 5 5",
            "[ff] [ff] [10] [5] [5]\n",
        ),
        (r"printf '[%u] [%u]\n' 5 -1", "[5] [18446744073709551615]\n"),
        (
            r"printf '[%#10x] [%#010x] [%-#10x]|\n' 255 255 255",
            "[      0xff] [0x000000ff] [0xff      ]|\n",
        ),
        (
            r"printf '[%+e] [% e] [%+g] [%#g] [%#.3g] [%g]\n' 12345.678 12345.678 0.5 1.5 1.5 100000",
            "[+1.234568e+04] [ 1.234568e+04] [+0.5] [1.50000] [1.50] [100000]\n",
        ),
        // The flags mean nothing to a string or a character.
        (r"printf '[%+s] [% s] [%#s]\n' ab ab ab", "[ab] [ab] [ab]\n"),
        (
            r"printf '[%0+5d] [%0 5d] [%+05.1f] [%08.3f] [%-8.3f]|\n' 3 3 3 3.14159 3.14159",
            "[+0003] [ 0003] [+03.0] [0003.142] [3.142   ]|\n",
        ),
        (r"printf '[%#.0f] [%#e]\n' 3 3", "[3.] [3.000000e+00]\n"),
        (
            r"printf '[%++d] [%--5d] [%00d] [%  d]\n' 1 2 3 4",
            "[+1] [2    ] [3] [ 4]\n",
        ),
        // A precision on an integer is its least number of digits.
        (r"printf '[%+.3d]\n' 7", "[+007]\n"),
        (r"printf '[%0-5d]|[%-05d]|\n' 7 7", "[7    ]|[7    ]|\n"),
        (r"printf '[%+X] [%.3x]\n' 255 5", "[FF] [005]\n"),
    ] {
        assert_eq!(sb.ok(script), want, "{script}");
    }
}

// ── 3. /dev/null as input ────────────────────────────────────────────────

#[test]
fn dev_null_is_an_empty_file_to_read() {
    let sb = Sandbox::new("devnull");
    for (script, want) in [
        ("cat < /dev/null; echo rc=$?", "rc=0\n"),
        ("cat /dev/null; echo rc=$?", "rc=0\n"),
        ("wc -l < /dev/null", "0\n"),
        ("grep -c x /dev/null; echo rc=$?", "0\nrc=1\n"),
        ("grep x /dev/null; echo rc=$?", "rc=1\n"),
        ("head /dev/null; echo rc=$?", "rc=0\n"),
        ("tail /dev/null; echo rc=$?", "rc=0\n"),
        ("sort /dev/null; echo rc=$?", "rc=0\n"),
        ("cat < /dev/null | wc -c", "0\n"),
        ("echo abc > f1; cat f1 /dev/null f1", "abc\nabc\n"),
        ("grep -c x /dev/null f1", "/dev/null:0\nf1:0\n"),
        (
            "head -n1 /dev/null f1",
            "==> /dev/null <==\n\n==> f1 <==\nabc\n",
        ),
        (
            "cat /dev/null /dev/null x 2>&1; echo rc=$?",
            "cat: x: No such file or directory\nrc=1\n",
        ),
    ] {
        assert_eq!(sb.ok(script), want, "{script}");
    }
}

// ── 4. ${x:offset:length} and here-strings ───────────────────────────────

#[test]
fn substring_expansion_matches_bash() {
    let sb = Sandbox::new("substring");
    let y = "y=abcdef; ";
    for (script, want) in [
        (
            r#"echo "[${y:0:2}] [${y:2}] [${y: -2}] [${y:(-2)}] [${y:1:-1}] [${y:-1}] [${y:0}]""#,
            "[ab] [cdef] [ef] [ef] [bcde] [abcdef] [abcdef]\n",
        ),
        (
            r#"echo "[${y:4:10}] [${y:10}] [${y:6}] [${y: -10}] [${y: -10:3}] [${y:2:0}] [${y: -3:2}] [${y:1:-4}]""#,
            "[ef] [] [] [] [] [] [de] [b]\n",
        ),
        (r#"echo "[${y:3:-3}] [${y: -2:-1}]""#, "[] [e]\n"),
        (
            r#"n=2; echo "[${y:$n}] [${y:$n:$n}] [${y:n:1}] [${y:1 : 3}]""#,
            "[cdef] [cd] [c] [bcd]\n",
        ),
        (
            r#"set -- hello world; echo "[${1:1:3}] [${2: -3}]""#,
            "[ell] [rld]\n",
        ),
        (r#"z=; unset q; echo "[${z:0:2}] [${q:1:2}]""#, "[] []\n"),
        // Unquoted, the result is split into fields.
        (
            r#"w="a b c"; for t in ${w:0:3}; do echo "<$t>"; done"#,
            "<a>\n<b>\n",
        ),
        // Characters, not bytes (bash in a UTF-8 locale; in the C locale it
        // counts bytes).
        (
            r#"u=héllo; echo "[${u:1:2}] [${u: -2}]""#,
            "[\u{e9}l] [lo]\n",
        ),
    ] {
        let script = script.replace("\\u00e9", "\u{e9}");
        assert_eq!(sb.ok(&format!("{y}{script}")), want, "{script}");
    }
}

#[test]
fn a_negative_length_that_ends_before_the_offset_is_an_error() {
    let sb = Sandbox::new("substring-error");
    // bash: line 1: -4: substring expression < 0, and the command does not run.
    let (out, err, code) = sb.run(r#"y=abcdef; echo "[${y:3:-4}]""#);
    assert_eq!(out, "", "{err}");
    assert_ne!(code, 0);
    assert!(err.contains("-4: substring expression < 0"), "{err}");
}

#[test]
fn a_here_string_is_the_commands_standard_input() {
    let sb = Sandbox::new("herestring");
    for (script, want) in [
        (r#"tr a-z A-Z <<< "abc""#, "ABC\n"),
        (r#"x=hello; tr a-z A-Z <<< "$x world""#, "HELLO WORLD\n"),
        (r#"wc -l <<< "a b""#, "1\n"),
        (r#"wc -c <<< "abc""#, "4\n"),
        (r#"grep -c b <<< "abc""#, "1\n"),
        (r#"cat <<< 'single $x'"#, "single $x\n"),
        (r#"x=hello; cat <<< $x"#, "hello\n"),
        ("cat <<< \"multi\nline\"", "multi\nline\n"),
        (r#"cat <<< """#, "\n"),
        (r#"read a b <<< "one two"; echo "$a|$b""#, "one|two\n"),
        // The last one wins, as in bash.
        (r#"cat <<< "a" <<< "b""#, "b\n"),
        (r#"sort <<< "b" | tr b B"#, "B\n"),
    ] {
        assert_eq!(sb.ok(script), want, "{script}");
    }
}

#[test]
fn a_here_document_is_still_refused() {
    let sb = Sandbox::new("heredoc");
    let (out, err, code) = sb.run("cat <<EOF\nabc\nEOF\n");
    assert_eq!((out.as_str(), code), ("", 77), "{err}");
    assert!(err.contains("here-document (<<)"), "{err}");
}

// ── 5. refusals that name what is refused ────────────────────────────────

#[test]
fn an_operand_that_is_not_an_option_is_an_invalid_option_as_in_gnu() {
    let sb = Sandbox::new("invalid");
    // GNU: tr: invalid option -- '_' (status 1)
    let (out, err, code) = sb.run("echo abc | tr '-_' '  '");
    assert_eq!((out.as_str(), code), ("", 1), "{err}");
    assert_eq!(
        err,
        "tr: invalid option -- '_'\nTry 'tr --help' for more information.\n"
    );
    // GNU: tr: unrecognized option '--foo' (status 1)
    let (_, err, code) = sb.run("echo abc | tr --foo a b");
    assert_eq!(code, 1, "{err}");
    assert_eq!(
        err,
        "tr: unrecognized option '--foo'\nTry 'tr --help' for more information.\n"
    );
    // GNU: sort: invalid option -- 'Z' (status 2)
    let (_, err, code) = sb.run("echo abc | sort -Z");
    assert_eq!(code, 2, "{err}");
    assert_eq!(
        err,
        "sort: invalid option -- 'Z'\nTry 'sort --help' for more information.\n"
    );
    // GNU: cut: invalid option -- 'Z' (status 1)
    let (_, err, code) = sb.run("echo abc | cut -Z");
    assert_eq!(code, 1, "{err}");
    assert_eq!(
        err,
        "cut: invalid option -- 'Z'\nTry 'cut --help' for more information.\n"
    );
    // GNU: grep: invalid option -- 'j' (status 2), with its usage line.
    let (_, err, code) = sb.run("echo abc | grep -j b");
    assert_eq!(code, 2, "{err}");
    assert_eq!(
        err,
        "grep: invalid option -- 'j'\nUsage: grep [OPTION]... PATTERNS [FILE]...\nTry 'grep --help' for more information.\n"
    );
    let (_, err, code) = sb.run("echo abc | grep --foo b");
    assert_eq!(code, 2, "{err}");
    assert_eq!(
        err,
        "grep: unrecognized option '--foo'\nUsage: grep [OPTION]... PATTERNS [FILE]...\nTry 'grep --help' for more information.\n"
    );
}

#[test]
fn after_double_dash_a_leading_minus_is_an_operand() {
    let sb = Sandbox::new("dashdash");
    // GNU: echo abc | tr -- '-_' '  '  prints abc
    assert_eq!(sb.ok("echo abc | tr -- '-_' '  '"), "abc\n");
    // A value-taking option's value may start with a minus.
    assert_eq!(sb.ok("echo a-b | cut -d - -f2"), "b\n");
    assert_eq!(sb.ok("echo a-b | grep -e -b"), "a-b\n");
}

#[cfg(windows)]
#[test]
fn a_flag_the_in_process_tool_lacks_is_named_first_and_gnu_second() {
    let sb = Sandbox::new("lacks");
    // `-c` is a real tr option; the in-process tr has no `-c`, and there is no
    // GNU tr on the empty PATH.
    let (out, err, code) = sb.run("echo abc | tr -c a b");
    assert_eq!((out.as_str(), code), ("", 2), "{err}");
    assert!(
        err.starts_with("ish: tr: the in-process tr does not support -c; a GNU-compatible tr on PATH would run it, and none was found\n"),
        "{err}"
    );
}

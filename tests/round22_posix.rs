//! Round 22: the POSIX subset (`ish -b`) and its refusals.
//!
//! Each case runs the binary with `-b -c` in a fresh temp folder, with
//! IRONSHELL_SESSION_DIR set to a folder of its own.

use std::path::PathBuf;
use std::process::{Command, Output};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("ae-round22-posix-{tag}-{}-{n}", std::process::id()));
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
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn sed_s_reads_an_escaped_slash_as_a_literal_slash() {
    let sb = Sandbox::new("sed-slash");
    let (out, err, code) = sb.run(r"printf '/index\n' | sed 's/\/index/X/'");
    assert_eq!((out.as_str(), code), ("X\n", 0), "stderr {err:?}");
    // The escaped delimiter in a group of the pattern, and `\/` in the
    // replacement, both as literal slashes.
    let (out, err, code) =
        sb.run(r"printf 'NAME /usr/x\n' | sed 's/\([A-Z]*\) \(\/[^ ]*\)/[\2 \1]/'");
    assert_eq!(
        (out.as_str(), code),
        ("[/usr/x NAME]\n", 0),
        "stderr {err:?}"
    );
    let (out, _, code) = sb.run(r"printf 'a/b\n' | sed 's/a/\/x\//'");
    assert_eq!((out.as_str(), code), ("/x//b\n", 0));
}

#[test]
fn sed_s_reads_an_escaped_delimiter_as_a_literal_character() {
    let sb = Sandbox::new("sed-pipe");
    // In BRE `\|` is alternation, but with `|` as the delimiter it is the
    // character itself: only the text `a|b` matches.
    let (out, err, code) = sb.run(r"printf 'a|b\nab\n' | sed 's|a\|b|c|'");
    assert_eq!((out.as_str(), code), ("c\nab\n", 0), "stderr {err:?}");
    // The same in ERE, where `|` is an operator unless escaped.
    let (out, err, code) = sb.run(r"printf 'a|b\nab\n' | sed -E 's|a\|b|c|'");
    assert_eq!((out.as_str(), code), ("c\nab\n", 0), "stderr {err:?}");
    // A `.` delimiter escaped is a literal dot, not any character.
    let (out, _, code) = sb.run(r"printf 'a.b\naxb\n' | sed 's.a\.b.Q.'");
    assert_eq!((out.as_str(), code), ("Q\naxb\n", 0));
}

#[cfg(windows)]
#[test]
fn a_refused_sed_form_names_the_part_it_does_not_support() {
    let sb = Sandbox::new("sed-refuse");
    // `w` writes a file, which the subset does not run. With no GNU sed on
    // PATH the refusal names the flag, and exits 2.
    let (out, err, code) = sb.run("printf 'x\\n' | sed 's/x/y/w out.txt'");
    assert_eq!(out, "");
    assert_eq!(code, 2, "{err}");
    assert!(
        err.starts_with(
            "ish: sed: the s command's flag `w` is not supported by the in-process subset, and no GNU-compatible sed is installed\n"
        ),
        "{err}"
    );
}

#[test]
fn an_arithmetic_refusal_is_at_the_dollar_sign() {
    // Nothing runs when the script is refused, and the refusal names the
    // `$((` itself: line 3, column 3, not the start of the statement.
    let sb = Sandbox::new("arith-col");
    let (out, err, _) = sb.run("echo a\necho b\nn=$((2+3))\n");
    assert_eq!(out, "");
    assert!(
        err.contains("arithmetic expansion $((...)) at line 3, column 3"),
        "{err}"
    );
}

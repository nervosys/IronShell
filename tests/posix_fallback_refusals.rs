//! Flags and formats the subset does not implement, and the fallback that runs
//! them.
//!
//! A subset command that declines runs a GNU-compatible program found on PATH,
//! and only that. On Windows a bare `sort` used to run `System32\sort.exe`, and
//! any name the subset did not know was looked up the same way, so an
//! impostor program could run in place of GNU. With no GNU program there is an
//! error that names the flag and exits 2 (not 127, "command not found").
//!
//! Also here: the forms the subset does implement now (`cat -n`, printf's
//! `%x`, `%o`, `%X`, `%.Nf`, `%.Ns` and zero padding, and sed's `s/a/b/N`).
//! The expected outputs are literals recorded from GNU coreutils 9 and sed 4.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        // Tests run in parallel threads: each sandbox gets its own folder.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ae-posix-fallback-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("session")).unwrap();
        std::fs::create_dir_all(root.join("empty-path")).unwrap();
        std::fs::create_dir_all(root.join("tree")).unwrap();
        std::fs::write(root.join("tree").join("top.rs"), "TODO\nplain\n").unwrap();
        std::fs::write(root.join("nums.txt"), "10\n9\n100\n-3\n").unwrap();
        std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(root.join("b.txt"), "no newline at end").unwrap();
        std::fs::write(root.join("c.txt"), "\n\nx\n").unwrap();
        std::fs::write(root.join("p.txt"), "pp\npa\nap\n").unwrap();
        std::fs::write(root.join("s.txt"), "pppp\nabcabc\n").unwrap();
        Sandbox(root)
    }

    /// `ish -b -c script`, with PATH set to `path` (so no GNU program is found
    /// unless the test puts one there).
    fn run_with_path(&self, script: &str, path: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-b", "-c", script])
            .current_dir(&self.0)
            .env("NO_COLOR", "1")
            .env("PATH", path)
            .env("IRONSHELL_SESSION_DIR", self.0.join("session"))
            .output()
            .expect("run ish")
    }

    fn run(&self, script: &str) -> (String, String, i32) {
        let empty = self.0.join("empty-path");
        let out = self.run_with_path(script, &empty);
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

fn ok(script: &str, want: &str) {
    let sb = Sandbox::new("ok");
    let (out, err, code) = sb.run(script);
    assert_eq!((out.as_str(), code), (want, 0), "{script}: stderr {err:?}");
    assert!(err.is_empty(), "{script}: stderr {err:?}");
}

#[test]
fn cat_n_numbers_lines_across_files_like_gnu() {
    // GNU: cat -n a.txt b.txt (a last line without a newline gets none)
    ok(
        "cat -n a.txt b.txt",
        "     1\tone\n     2\ttwo\n     3\tno newline at end",
    );
    // GNU: cat -n c.txt (empty lines are numbered too)
    ok("cat -n c.txt", "     1\t\n     2\t\n     3\tx\n");
}

#[test]
fn printf_forms_the_subset_now_runs_match_gnu() {
    // GNU: printf '%03d|%x\n' 7 255
    ok("printf '%03d|%x\\n' 7 255", "007|ff\n");
    // GNU: printf '%.2f\n' 3.14159
    ok("printf '%.2f\\n' 3.14159", "3.14\n");
    // GNU: printf '%X %o|%5.1f|\n' 255 8 2.25
    ok("printf '%X %o|%5.1f|\\n' 255 8 2.25", "FF 10|  2.2|\n");
    // GNU: printf '%.3s|%x\n' abcdef -1 (a negative number is 64 bits)
    ok("printf '%.3s|%x\\n' abcdef -1", "abc|ffffffffffffffff\n");
    // GNU: printf '%05.1f|%08x\n' -2.5 255
    ok("printf '%05.1f|%08x\\n' -2.5 255", "-02.5|000000ff\n");
    // GNU: printf '%.0f %f\n' 2.5 0
    ok("printf '%.0f %f\\n' 2.5 0", "2 0.000000\n");
    // GNU: printf '%03x|%.1f\n' 10 0.05
    ok("printf '%03x|%.1f\\n' 10 0.05", "00a|0.1\n");
}

#[test]
fn sed_numeric_flag_matches_gnu() {
    // GNU: sed 's/p/P/2' p.txt
    ok("sed 's/p/P/2' p.txt", "pP\npa\nap\n");
    // GNU: sed 's/p/P/2g' s.txt (the second match on, every one)
    ok("sed 's/p/P/2g' s.txt", "pPPP\nabcabc\n");
    // GNU: sed 's/p/P/3' s.txt
    ok("sed 's/p/P/3' s.txt", "ppPp\nabcabc\n");
    // GNU: sed 's/b/B/1' s.txt
    ok("sed 's/b/B/1' s.txt", "pppp\naBcabc\n");
    // GNU: sed -n 's/p/P/2p' s.txt
    ok("sed -n 's/p/P/2p' s.txt", "pPpp\n");
}

#[test]
fn a_flag_with_no_gnu_program_is_refused_with_status_2() {
    let sb = Sandbox::new("refuse");
    // GNU would print the listing. `ls -R` is outside the subset on every
    // platform (`-l` is inside it on Unix), so with no GNU ls on PATH the subset
    // says why it cannot and exits 2 rather than "command not found" (127).
    for (script, want) in [
        (
            "ls -R tree",
            "ish: ls: -R is not supported by the in-process subset, and no GNU-compatible ls is installed\n",
        ),
        (
            "sort -h nums.txt",
            "ish: sort: -h is not supported by the in-process subset, and no GNU-compatible sort is installed\n",
        ),
    ] {
        let (out, err, code) = sb.run(script);
        assert_eq!(out, "", "{script}");
        assert_eq!((err.as_str(), code), (want, 2), "{script}");
    }
}

#[cfg(windows)]
#[test]
fn system32_is_never_the_fallback_for_a_subset_name() {
    // With only System32 on PATH, the sort.exe there is not GNU and must not
    // run: the error is the refusal, not its "Input file specified" output.
    let sb = Sandbox::new("system32");
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let sys = PathBuf::from(root).join("System32");
    let out = sb.run_with_path("sort -h nums.txt", &sys);
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(
        err.contains("no GNU-compatible sort is installed"),
        "stderr was {err:?}"
    );
}

#[cfg(unix)]
mod with_a_program_on_path {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write_program(dir: &Path, name: &str, body: &str) {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn an_impostor_named_like_a_subset_command_never_runs() {
        let sb = Sandbox::new("impostor");
        let bin = sb.0.join("impostor-bin");
        std::fs::create_dir_all(&bin).unwrap();
        // It answers --version with something that is not GNU, and would print
        // "IMPOSTOR RAN" if it were run for the flag.
        write_program(
            &bin,
            "sort",
            "if [ \"$1\" = --version ]; then echo 'Impostor 1.0'; exit 0; fi\necho 'IMPOSTOR RAN'",
        );
        let out = sb.run_with_path("sort -h nums.txt", &bin);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!String::from_utf8_lossy(&out.stdout).contains("IMPOSTOR"));
        assert_eq!(out.status.code(), Some(2));
        assert!(
            err.contains("no GNU-compatible sort is installed"),
            "{err:?}"
        );
    }

    #[test]
    fn a_gnu_program_on_path_is_the_fallback() {
        let sb = Sandbox::new("gnu-fallback");
        let bin = sb.0.join("gnu-bin");
        std::fs::create_dir_all(&bin).unwrap();
        write_program(
            &bin,
            "sort",
            "if [ \"$1\" = --version ]; then echo 'sort (GNU coreutils) 9.9'; exit 0; fi\necho \"fake GNU sort: $*\"",
        );
        let out = sb.run_with_path("sort -h nums.txt", &bin);
        assert_eq!(out.status.code(), Some(0));
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "fake GNU sort: -h nums.txt\n"
        );
    }
}

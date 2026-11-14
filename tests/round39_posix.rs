//! `ish -b` / `--zsh`: an unmatched glob operand of `ls` reads as bash's; zsh's
//! `setopt nullglob` is refused beside a glob on the same line.

use std::process::Command;

fn run(args: &[&str]) -> (String, String, i32) {
    let dir = std::env::temp_dir().join(format!("ish-round39-posix-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join(".sessions"))
        .env_remove("IRONSHELL_ALLOW_SH")
        .output()
        .unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn ls_of_an_unmatched_glob_says_no_such_file() {
    let (out, err, code) = run(&["-b", "-c", "ls *.nomatch_round39"]);
    assert_eq!(out, "");
    assert!(
        err.contains("ls: cannot access '*.nomatch_round39': No such file or directory"),
        "stderr: {err}"
    );
    assert_eq!(code, 2);
}

#[test]
fn nullglob_beside_a_glob_is_refused_before_anything_runs() {
    let (out, err, code) = run(&["--zsh", "-c", "setopt nullglob; echo *.nomatch_round39"]);
    assert_eq!(out, "");
    assert!(err.contains("nullglob"), "stderr: {err}");
    assert_eq!(code, 77, "stderr: {err}");
}

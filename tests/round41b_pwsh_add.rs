//! `ish --pwsh`: PowerShell's `+` converts the right operand to the left
//! operand's type, so `'a' + 1` is `a1` and `1 + '2'` is 3 (translated to
//! `ps_add`).

use std::process::Command;

struct Out {
    stdout: String,
    stderr: String,
}

fn pwsh(script: &str) -> Out {
    let dir = std::env::temp_dir().join(format!("ish-round41b-pwsh-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--pwsh", "-c", script])
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join(".sessions"))
        .env_remove("IRONSHELL_ALLOW_SH")
        .output()
        .expect("run ish");
    Out {
        stdout: String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

#[test]
fn a_string_plus_a_number_concatenates() {
    let o = pwsh("$x = 'a'; $y = 1; $x + $y");
    assert_eq!(o.stdout, "a1\n", "stderr: {}", o.stderr);
    let o = pwsh("'a' + 1");
    assert_eq!(o.stdout, "a1\n", "stderr: {}", o.stderr);
}

#[test]
fn a_number_plus_a_numeric_string_adds() {
    let o = pwsh("1 + '2'");
    assert_eq!(o.stdout, "3\n", "stderr: {}", o.stderr);
}

#[test]
fn a_number_plus_a_non_numeric_string_is_an_error() {
    let o = pwsh("1 + 'x'");
    assert_eq!(o.stdout, "");
    assert!(o.stderr.contains("cannot convert"), "stderr: {}", o.stderr);
}

#[test]
fn two_strings_concatenate_and_two_numbers_add() {
    let o = pwsh("'ab' + 'cd'");
    assert_eq!(o.stdout, "abcd\n", "stderr: {}", o.stderr);
    let o = pwsh("1 + 2.5");
    assert_eq!(o.stdout, "3.5\n", "stderr: {}", o.stderr);
}

#[test]
fn multiplication_binds_tighter_than_the_converting_plus() {
    let o = pwsh("1 + 2 * 3");
    assert_eq!(o.stdout, "7\n", "stderr: {}", o.stderr);
    let o = pwsh("'a' + 1 + 2");
    assert_eq!(o.stdout, "a12\n", "stderr: {}", o.stderr);
}

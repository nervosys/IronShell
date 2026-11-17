//! Round 42b: an Int literal too large for an Int is E_PARSE (it was 0), and
//! `--deterministic` keeps a Float's point and prints `null` for Null.

use std::process::Command;

struct Out {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn ish(args: &[&str]) -> Out {
    let dir = std::env::temp_dir().join(format!("ish-round42b-lit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(&dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", dir.join(".sessions"))
        .output()
        .expect("run ish");
    Out {
        stdout: String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code(),
    }
}

#[test]
fn an_int_literal_too_large_is_refused() {
    let o = ish(&["-c", "123456789012345678901234567890"]);
    assert_ne!(o.code, Some(0), "stdout: {} stderr: {}", o.stdout, o.stderr);
    assert_eq!(o.stdout, "");
    assert!(
        o.stderr.contains("does not fit in an Int"),
        "stderr: {}",
        o.stderr
    );
    assert!(
        o.stderr.contains("123456789012345678901234567890"),
        "stderr: {}",
        o.stderr
    );
}

#[test]
fn the_largest_and_smallest_ints_still_work() {
    let o = ish(&["-c", "9223372036854775807"]);
    assert_eq!(o.stdout, "9223372036854775807\n", "stderr: {}", o.stderr);
    let o = ish(&["-c", "-9223372036854775808"]);
    assert_eq!(o.stdout, "-9223372036854775808\n", "stderr: {}", o.stderr);
    let o = ish(&["-c", "9223372036854775808"]);
    assert_ne!(o.code, Some(0));
    assert!(o.stderr.contains("does not fit"), "stderr: {}", o.stderr);
}

#[test]
fn an_si_suffix_that_overflows_is_refused() {
    let o = ish(&["-c", "9223372036854775807G"]);
    assert_ne!(o.code, Some(0), "stdout: {}", o.stdout);
    assert!(o.stderr.contains("does not fit"), "stderr: {}", o.stderr);
}

#[test]
fn deterministic_output_keeps_the_float_point() {
    let o = ish(&["--deterministic", "-c", "1.0"]);
    assert_eq!(o.stdout, "1.0\n", "stderr: {}", o.stderr);
    let o = ish(&["--deterministic", "-c", "[1.0, 2]"]);
    assert_eq!(o.stdout, "[1.0,2]\n", "stderr: {}", o.stderr);
    let o = ish(&["--deterministic", "-c", "-0.0"]);
    assert_eq!(o.stdout, "-0.0\n", "stderr: {}", o.stderr);
    let o = ish(&["--deterministic", "-c", "1e21"]);
    assert_eq!(o.stdout.trim(), "1.0e21", "stderr: {}", o.stderr);
}

#[test]
fn deterministic_output_prints_null() {
    let o = ish(&["--deterministic", "-c", "null"]);
    assert_eq!(o.stdout, "null\n", "stderr: {}", o.stderr);
}

//! Round 29: the in-process POSIX subset (`ish -b`) matches bash where it used
//! to refuse or differ.
//!
//! Expected outputs are bash's, written as literals so the test needs no bash.

use std::process::Command;

/// Run `script` under `ish -b -c` in a folder holding `files`, and return
/// stdout, stderr and the exit status. PATH is an empty folder, so a declined
/// command cannot be answered by a GNU program: it fails with stderr, which
/// makes these cases prove the in-process code ran.
fn run(tag: &str, files: &[(&str, &str)], script: &str) -> (String, String, i32) {
    let root = std::env::temp_dir().join(format!("ae-round29-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let work = root.join("work");
    let session = root.join("session");
    let empty_path = root.join("empty-path");
    for dir in [&work, &session, &empty_path] {
        std::fs::create_dir_all(dir).unwrap();
    }
    for (name, text) in files {
        std::fs::write(work.join(name), text).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", script])
        .current_dir(&work)
        .env("NO_COLOR", "1")
        .env("LC_ALL", "C")
        .env("PATH", &empty_path)
        .env("IRONSHELL_SESSION_DIR", &session)
        .output()
        .expect("run ish");
    let _ = std::fs::remove_dir_all(&root);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

// ---- printf: an integer past 64 bits is clamped and reported ----

#[test]
fn printf_clamps_a_decimal_past_i64_max_with_result_too_large() {
    // bash: printf "%d\n" 9999999999999999999
    let (out, err, code) = run("pf-big", &[], r#"printf "%d\n" 9999999999999999999"#);
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        (
            "9223372036854775807\n",
            "printf: 9999999999999999999: Result too large\n",
            1
        )
    );
}

#[test]
fn printf_clamps_a_negative_decimal_past_i64_min_with_result_too_large() {
    let (out, err, code) = run("pf-neg", &[], r#"printf "%d\n" -9999999999999999999"#);
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        (
            "-9223372036854775808\n",
            "printf: -9999999999999999999: Result too large\n",
            1
        )
    );
}

#[test]
fn printf_clamps_an_unsigned_looking_hex_that_overflows_i64() {
    // bash 5.2: printf "%d\n" 0xFFFFFFFFFFFFFFFF prints i64::MAX.
    let (out, err, code) = run("pf-hex", &[], r#"printf "%d\n" 0xFFFFFFFFFFFFFFFF"#);
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        (
            "9223372036854775807\n",
            "printf: 0xFFFFFFFFFFFFFFFF: Result too large\n",
            1
        )
    );
}

#[test]
fn printf_keeps_i64_max_exact_and_quiet() {
    let (out, err, code) = run("pf-max", &[], r#"printf "%d\n" 9223372036854775807"#);
    assert_eq!(
        (out.as_str(), err.as_str(), code),
        ("9223372036854775807\n", "", 0)
    );
}

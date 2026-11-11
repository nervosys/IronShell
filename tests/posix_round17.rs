//! Round-17 bash-compatibility fixes in the in-process POSIX subset: `read`
//! field splitting and its status at an unterminated last line, `echo -e` and
//! `printf` `\xHH`, `${10}`, `printf` `%e`/`%g`, the here-string refusal, and
//! running a shell script in a child `ish -b`. These drive the real binary, as
//! posix_shift does.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh directory for one test's script and session, under the test target
/// dir, never under the user's home or the working tree.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("posix_round17")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    dir
}

/// Run `ish` with `args` in `dir`. Returns (exit status, stdout, stderr).
fn ish(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_ALLOW_SH")
        .env_remove("IRONSHELL_MODE")
        .output()
        .expect("run ish");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn script(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write the script");
    path.to_str().unwrap().to_string()
}

#[test]
fn braced_positional_parameters_past_nine_are_one_parameter() {
    let dir = scratch("braced_positional");
    let s = script(&dir, "p.sh", "echo \"[${10}] [${11}] [${12}] [$10]\"\n");
    let (code, out, err) = ish(
        &dir,
        &[
            "-b", &s, "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k",
        ],
    );
    assert_eq!(code, 0, "{err}");
    // `$10` without braces is `$1` then `0`, as in bash.
    assert_eq!(out, "[j] [k] [] [a0]");
}

#[test]
fn braced_positional_past_nine_under_nounset_is_unbound() {
    let dir = scratch("braced_nounset");
    let s = script(&dir, "p.sh", "set -u\necho ${12}\n");
    let (code, out, err) = ish(&dir, &["-b", &s, "a"]);
    assert_ne!(code, 0, "{out}");
    assert!(out.is_empty(), "ran something: {out:?}");
    assert!(err.contains("12: unbound variable"), "{err}");
}

#[test]
fn echo_e_decodes_hex_escapes() {
    let dir = scratch("echo_hex");
    let s = script(
        &dir,
        "e.sh",
        "echo -e '\\x41\\x7e'\necho -e '\\x411'\necho -e '\\xg'\necho -e '\\xc3\\xa9'\n",
    );
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    // `\x` takes at most two hex digits, and a `\x` with none stays as written.
    assert_eq!(out, "A~\nA1\n\\xg\n\u{e9}");
}

#[test]
fn printf_decodes_hex_in_format_and_in_b() {
    let dir = scratch("printf_hex");
    let s = script(
        &dir,
        "p.sh",
        "printf '\\x41\\x42\\n'\nprintf '%b|' '\\x43\\x0a'\necho\n",
    );
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "AB\nC\n|");
}

#[test]
fn printf_e_uses_c_exponent_form() {
    let dir = scratch("printf_e");
    let s = script(
        &dir,
        "e.sh",
        "printf '%e\\n' 12345.678\nprintf '%.2e\\n' 0.000123\nprintf '%E\\n' 12345.678\nprintf '[%12.3e]\\n' 3.14159\nprintf '%010.2e\\n' 3.14159\nprintf '%e\\n' 0\n",
    );
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "1.234568e+04\n1.23e-04\n1.234568E+04\n[   3.142e+00]\n003.14e+00\n0.000000e+00"
    );
}

#[test]
fn printf_g_uses_the_shorter_of_f_and_e() {
    let dir = scratch("printf_g");
    let s = script(
        &dir,
        "g.sh",
        "printf '%g\\n' 0.0001\nprintf '%g\\n' 123456789\nprintf '%g\\n' 100\nprintf '%G\\n' 0.00001234\nprintf '%.3g\\n' 3.14159\nprintf '%g\\n' 1e20\nprintf '%g\\n' 0\n",
    );
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "0.0001\n1.23457e+08\n100\n1.234E-05\n3.14\n1e+20\n0");
}

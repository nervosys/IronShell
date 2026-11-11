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
fn read_splits_on_a_prefix_ifs_and_gives_the_rest_to_the_last_name() {
    let dir = scratch("read_ifs_prefix");
    std::fs::write(dir.join("row.csv"), "a,b c,\"d\"\n").expect("write row");
    let s = script(
        &dir,
        "r.sh",
        "IFS=, read -r a b c < row.csv\necho \"[$a][$b][$c]\"\n",
    );
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[a][b c][\"d\"]");
}

#[test]
fn an_ifs_prefix_is_for_one_read_and_a_plain_ifs_holds() {
    let dir = scratch("read_ifs_scope");
    std::fs::write(dir.join("row.csv"), "a,b c,\"d\"\n").expect("write row");
    std::fs::write(dir.join("words.txt"), "p q r\n").expect("write words");
    let s = script(
        &dir,
        "r.sh",
        "IFS=, read -r x < row.csv\nread -r a b c < words.txt\nIFS=,\nread -r d e < row.csv\necho \"[$a][$b][$c] [$d][$e] ifs=[$IFS]\"\n",
    );
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    // The prefix `IFS=,` ended with its read, so the second read splits on
    // blanks. The later `IFS=,` is an assignment, which stays for the third.
    assert_eq!(out, "[p][q][r] [a][b c,\"d\"] ifs=[,]");
}

#[test]
fn read_collapses_blank_ifs_and_keeps_empty_fields_for_a_comma() {
    let dir = scratch("read_ifs_fields");
    std::fs::write(dir.join("ws.txt"), "  x   y z  \n").expect("write ws");
    std::fs::write(dir.join("gap.txt"), "1,,3\n").expect("write gap");
    let s = script(
        &dir,
        "r.sh",
        "read a b < ws.txt\necho \"[$a][$b]\"\nIFS=, read a b c < gap.txt\necho \"[$a][$b][$c]\"\n",
    );
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[x][y z]\n[1][][3]");
}

#[test]
fn a_last_line_without_a_newline_is_read_but_fails() {
    let dir = scratch("read_no_newline");
    std::fs::write(dir.join("f.txt"), "a\nb").expect("write f");
    let s = script(
        &dir,
        "r.sh",
        "while read -r line; do echo \"[$line]\"; done < f.txt\necho \"last=[$line]\"\nread -r v < f.txt; echo \"status=$?\"\n",
    );
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    // The loop body does not run for `b`, but `read` still assigned it.
    assert_eq!(out, "[a]\nlast=[b]\nstatus=0");
}

#[test]
fn a_script_with_a_sh_shebang_runs_in_a_child_and_gives_its_status() {
    let dir = scratch("shebang_child");
    std::fs::write(
        dir.join("child.sh"),
        "#!/bin/sh\necho \"[$1] [$2]\"\nexit 3\n",
    )
    .expect("write child");
    let s = script(&dir, "parent.sh", "./child.sh a b\necho \"status=$?\"\n");
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[a] [b]\nstatus=3");
}

#[test]
fn a_script_in_a_folder_with_an_env_bash_shebang_runs_in_a_child() {
    let dir = scratch("shebang_folder");
    std::fs::create_dir_all(dir.join("scripts")).expect("scripts dir");
    std::fs::write(
        dir.join("scripts").join("x.sh"),
        "#!/usr/bin/env bash\necho \"ran $1\"\n",
    )
    .expect("write x");
    let s = script(&dir, "parent.sh", "scripts/x.sh one\necho \"status=$?\"\n");
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "ran one\nstatus=0");
}

#[test]
fn sh_and_bash_run_a_script_file_without_a_shebang_in_a_child() {
    let dir = scratch("sh_file");
    std::fs::write(dir.join("plain.sh"), "echo \"[$1]\"\n").expect("write plain");
    let s = script(&dir, "parent.sh", "sh plain.sh x\nbash plain.sh y\n");
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[x]\n[y]");
}

#[test]
fn a_child_refusal_is_the_command_status() {
    let dir = scratch("shebang_refused");
    std::fs::write(dir.join("f.sh"), "#!/bin/sh\nf() { echo x; }\n").expect("write f");
    let s = script(&dir, "parent.sh", "./f.sh\necho \"status=$?\"\n");
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "status=77");
    assert!(err.contains("function definition"), "{err}");
}

#[test]
fn a_child_script_in_agent_mode_stays_inside_the_workspace() {
    let dir = scratch("shebang_agent");
    let escape = dir.parent().unwrap().join("r17_escape.txt");
    let _ = std::fs::remove_file(&escape);
    std::fs::write(
        dir.join("child.sh"),
        "#!/bin/sh\necho hi > ../r17_escape.txt\necho \"write=$?\"\n",
    )
    .expect("write child");
    let s = script(&dir, "parent.sh", "./child.sh\n");
    let (_code, out, _err) = ish(&dir, &["--agent", "-b", &s]);
    assert!(!escape.exists(), "the child wrote outside the workspace");
    assert!(out.contains("write="), "{out:?}");
    assert!(!out.contains("write=0"), "{out:?}");
}

#[test]
fn a_here_string_is_named_and_refused_before_anything_runs() {
    let dir = scratch("here_string");
    let s = script(&dir, "h.sh", "echo before\ngrep -c b <<< \"abc\"\n");
    let (code, out, err) = ish(&dir, &["-b", &s]);
    assert_eq!(code, 77, "{err}");
    assert!(out.is_empty(), "ran something: {out:?}");
    assert!(
        err.contains("here-string (<<<) at line 2, column 11"),
        "{err}"
    );
    assert!(err.contains("pipe it instead: printf"), "{err}");
    assert!(!err.contains("here-document"), "{err}");
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

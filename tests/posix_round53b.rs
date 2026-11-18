//! Round 53b: `ish -b` syntax errors, the variables a shell always has, `test`
//! and `read` options, and the statuses of refusals.

use std::process::{Command, Output};

fn scratch(tag: &str) -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("posix_round53b")
        .join(tag);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("session")).expect("scratch dir");
    root
}

fn ish(root: &std::path::Path, args: &[&str], agent: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(args)
        .current_dir(root)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", root.join("session"))
        .env_remove("IRONSHELL_ALLOW_SH")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("OLDPWD")
        .env_remove("IRONSHELL_MODE");
    if agent {
        cmd.env("IRONSHELL_MODE", "agent");
    }
    cmd.output().expect("run ish")
}

fn sh(tag: &str, script: &str) -> Output {
    let root = scratch(tag);
    ish(&root, &["-b", "-c", script], false)
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// A syntax error: exit 2, E_PARSE, and nothing ran.
fn assert_syntax(out: &Output, token: &str) {
    let err = text(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(
        out.stdout.is_empty(),
        "ran something: {} bytes",
        out.stdout.len()
    );
    assert!(err.contains("E_PARSE"), "{err}");
    assert!(
        err.contains(&format!("syntax error near unexpected token `{token}'")),
        "{err}"
    );
    assert!(!err.contains("E_POLICY_DENY"), "{err}");
}

#[test]
fn b1_an_empty_condition_is_a_syntax_error() {
    assert_syntax(&sh("b1_while", "echo before; while; do echo x; done"), ";");
    assert_syntax(&sh("b1_if", "if; then echo T; fi"), ";");
    assert_syntax(&sh("b1_until", "until; do echo x; done"), ";");
    assert_syntax(&sh("b1_do", "while\ndo echo x; done"), "do");
    assert_syntax(&sh("b1_then", "if\nthen echo x; fi"), "then");
}

#[test]
fn b2_stray_separators_are_syntax_errors() {
    assert_syntax(&sh("b2_double", "echo a; ;echo b"), ";");
    assert_syntax(&sh("b2_lead", "; echo a"), ";");
    assert_syntax(&sh("b2_do", "for i in 1 2 do echo $i; done"), "done");
    // The ordinary forms still run.
    let ok = sh("b2_ok", "for i in 1 2; do echo $i; done; echo z;");
    assert_eq!(text(&ok.stdout), "1\n2\nz\n");
}

#[test]
fn b3_plain_syntax_errors_are_not_policy_denials() {
    assert_syntax(&sh("b3_and", "&& echo a"), "&&");
    assert_syntax(&sh("b3_pipe2", "echo a | | cat"), "|");
    assert_syntax(&sh("b3_dsemi", "echo a ;; echo b"), ";;");
    assert_syntax(&sh("b3_for", "for 1x in a; do echo; done"), "1x");
    let end = sh("b3_end", "echo a |");
    assert_eq!(end.status.code(), Some(2));
    assert!(
        text(&end.stderr).contains("E_PARSE"),
        "{}",
        text(&end.stderr)
    );
    let open = sh("b3_brace", "echo ${x\n");
    let err = text(&open.stderr);
    assert_eq!(open.status.code(), Some(2), "{err}");
    assert!(err.contains("E_PARSE"), "{err}");
    assert!(err.contains("unterminated ${"), "{err}");
}

#[test]
fn b4_an_if_without_then_says_so() {
    let out = sh("b4", "echo a\nif true; echo a; fi\n");
    assert_syntax(&out, "fi");
    assert!(
        text(&out.stderr).contains("needs `then`"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn b5_the_usual_variables_are_set() {
    let out = sh(
        "b5",
        "[ -n \"$PWD\" ] && [ -n \"$SHELL\" ] && [ -n \"$PPID\" ] && echo set; \
         [ \"$RANDOM\" -ge 0 ] && [ \"$RANDOM\" -le 32767 ] && echo rand; \
         echo \"[${OLDPWD-unset}]\"",
    );
    assert_eq!(
        text(&out.stdout),
        "set\nrand\n[unset]\n",
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn b6_pwd_and_basename_use_forward_slashes() {
    let root = scratch("b6");
    let out = ish(
        &root,
        &[
            "-b",
            "-c",
            "d=$(pwd); echo \"${d##*/}\"; basename \"$(pwd)\"; dirname \"$(pwd)\"",
        ],
        false,
    );
    let name = root.file_name().unwrap().to_string_lossy().into_owned();
    let got = text(&out.stdout);
    let lines: Vec<&str> = got.lines().collect();
    assert_eq!(lines.len(), 3, "{}", text(&out.stderr));
    assert_eq!(lines[0], name);
    assert_eq!(lines[1], name);
    if cfg!(windows) {
        assert!(!lines[2].contains('\\'), "{got}");
    }
}

#[test]
fn b7_test_compares_files_and_strings() {
    let root = scratch("b7");
    std::fs::write(root.join("old"), "a").unwrap();
    std::fs::write(root.join("new"), "b").unwrap();
    let past = std::time::SystemTime::now() - std::time::Duration::from_secs(1000);
    std::fs::File::options()
        .write(true)
        .open(root.join("old"))
        .unwrap()
        .set_modified(past)
        .unwrap();
    let script = "[ new -nt old ] && echo nt; [ old -ot new ] && echo ot; \
                  [ old -nt new ] || echo notnt; [ new -ef ./new ] && echo ef; \
                  [ new -ef old ] || echo notef; [ a \\< b ] && echo lt; \
                  [ b \\> a ] && echo gt; [ b \\< a ] || echo notlt";
    let out = ish(&root, &["-b", "-c", script], false);
    assert_eq!(
        text(&out.stdout),
        "nt\not\nnotnt\nef\nnotef\nlt\ngt\nnotlt\n",
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn b7_read_takes_a_prompt_only_for_a_terminal() {
    let out = sh(
        "b7_read",
        "printf 'hi\\n' | { read -p 'P> ' v; echo \"[$v]\"; }",
    );
    assert_eq!(text(&out.stdout), "[hi]\n", "{}", text(&out.stderr));
    assert!(!text(&out.stderr).contains("P> "), "{}", text(&out.stderr));
    let eof = sh("b7_eof", "read -p 'P> ' v < /dev/null; echo \"status $?\"");
    assert_eq!(text(&eof.stdout), "status 1\n");
}

#[test]
fn b8_nounset_names_a_positional_with_its_dollar() {
    let out = sh("b8", "set -u\necho \"$1\"\n");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("$1: unbound variable"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn b9_a_byte_order_mark_before_the_shebang_is_ignored() {
    let root = scratch("b9");
    std::fs::write(root.join("x.sh"), "\u{feff}#!/bin/sh\necho hi\n").unwrap();
    let out = ish(&root, &["x.sh"], false);
    assert_eq!(text(&out.stdout), "hi\n", "{}", text(&out.stderr));
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn b10_a_policy_refusal_exits_77() {
    let root = scratch("b10");
    let prefix = ish(&root, &["-b", "-c", "IRONSHELL_WORKSPACE=/ cat x"], true);
    assert_eq!(prefix.status.code(), Some(77), "{}", text(&prefix.stderr));
    let export = ish(&root, &["-b", "-c", "export IRONSHELL_WORKSPACE=/"], true);
    assert_eq!(export.status.code(), Some(77), "{}", text(&export.stderr));
}

#[test]
fn b11_a_missing_powershell_operand_is_a_parse_error() {
    let root = scratch("b11");
    let out = ish(&root, &["--pwsh", "-c", "$x = 1 +"], false);
    let err = text(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(err.contains("E_PARSE"), "{err}");
    assert!(out.stdout.is_empty());
}

#[test]
fn b11_powershell_refusals_share_one_wording() {
    let root = scratch("b11_words");
    let out = ish(&root, &["--pwsh", "-c", "Write-Host @\"x\"@"], false);
    let err = text(&out.stderr);
    assert_eq!(out.status.code(), Some(77), "{err}");
    assert!(
        err.contains("at line 1, so nothing in this script was run"),
        "{err}"
    );
    assert!(!err.contains("; nothing was run"), "{err}");
}

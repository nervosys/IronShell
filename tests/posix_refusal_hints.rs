//! Each construct the in-process subset refuses gets a hint that says what to
//! write instead, and the `${...}` refusal names its operator. The hint ends with
//! the IRONSHELL_ALLOW_SH sentence, as before. The text is ASCII throughout.

use std::process::{Command, Output};

fn run(tag: &str, script: &str) -> Output {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("posix_refusal_hints")
        .join(tag);
    let _ = std::fs::remove_dir_all(&root);
    let session = root.join("session");
    std::fs::create_dir_all(&session).expect("scratch dir");
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", script])
        .current_dir(&root)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", &session)
        .env_remove("IRONSHELL_ALLOW_SH")
        .env_remove("IRONSHELL_MODE")
        .output()
        .expect("run ish")
}

fn refused_with(tag: &str, script: &str, named: &str, hint: &str) {
    let out = run(tag, script);
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(77), "{err}");
    assert!(out.stdout.is_empty(), "ran something: {:?}", out.stdout);
    assert!(
        err.contains(&format!(
            "error[E_POLICY_DENY]: {named} is not in the in-process POSIX subset"
        )),
        "{err}"
    );
    assert!(err.contains(hint), "the hint must say `{hint}`: {err}");
    assert!(
        err.contains(
            "Otherwise set IRONSHELL_ALLOW_SH=true to run the unsupported parts through bash -lc"
        ),
        "{err}"
    );
    assert!(err.is_ascii(), "the refusal must be ASCII: {err:?}");
}

#[test]
fn a_function_is_told_to_inline_its_body() {
    refused_with(
        "function",
        "f() { echo hi; }\nf\n",
        "function definition at line 1, column 1",
        "inline the body, or write the logic in ish.",
    );
}

#[test]
fn a_case_is_told_to_become_if_elif() {
    refused_with(
        "case",
        "case x in x) echo y;; esac\n",
        "case ... esac at line 1, column 1",
        "rewrite it as if/elif with [ \"$x\" = value ] tests",
    );
}

#[test]
fn a_here_document_is_told_to_use_printf_or_a_file() {
    refused_with(
        "heredoc",
        "cat <<EOF\nhi\nEOF\n",
        "here-document (<<) at line 1, column 5",
        "printf '%s\\n' ... | cmd",
    );
}

#[test]
fn a_subshell_is_told_to_drop_the_parentheses() {
    refused_with(
        "subshell",
        "(echo hi)\n",
        "subshell at line 1, column 1",
        "drop the parentheses, or group the commands with { ...; }",
    );
}

#[test]
fn a_background_job_is_told_to_run_in_the_foreground() {
    refused_with(
        "background",
        "sleep 0 &\n",
        "background job at line 1, column 9",
        "run it in the foreground: drop the trailing &",
    );
}

#[test]
fn double_brackets_are_told_to_use_test() {
    refused_with(
        "double_brackets",
        "if [[ -n x ]]; then echo y; fi\n",
        "[[ ... ]] at line 1, column 4",
        "use [ ... ] (test) instead of [[ ... ]]",
    );
}

#[test]
fn a_suffix_strip_names_its_operator_and_suggests_sed() {
    refused_with(
        "suffix",
        "f=a.txt; echo ${f%.txt}\n",
        "${...} operator `%` at line 1, column 15",
        "strip a suffix with sed",
    );
}

#[test]
fn a_substring_names_the_offset_and_suggests_cut() {
    refused_with(
        "offset",
        "x=abc; echo ${x:1}\n",
        "${...} operator `:offset` at line 1, column 13",
        "cut -c",
    );
}

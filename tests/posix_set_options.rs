//! `set -e`, `set -u` and `set -o pipefail` (and their `+` forms, and the
//! combinations scripts start with) in the in-process POSIX subset, with bash's
//! semantics, and the refusal up front of the `set` options the subset does not
//! run. These drive the real binary, as posix_shift does. The expected output is
//! what bash prints for the same script.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A fresh directory for one test's session, under the test target dir, never
/// under the user's home or the working tree.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("posix_set_options")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("scratch dir");
    dir
}

/// Run `ish -b -c src` in `dir`. Returns (exit status, stdout, stderr).
fn run(name: &str, src: &str) -> (i32, String, String) {
    let dir = scratch(name);
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", src])
        .current_dir(&dir)
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

// ── errexit ──────────────────────────────────────────────────────────────

#[test]
fn errexit_ends_the_script_at_a_failing_command() {
    let (code, out, err) = run("errexit_stops", "set -e\necho before\nfalse\necho after\n");
    assert_eq!(code, 1, "{err}");
    assert_eq!(out, "before");
}

#[test]
fn errexit_does_not_stop_for_conditions_or_lists_or_negation() {
    let src = "set -e\nif false; then echo no; fi\nfalse || echo ok\nfalse && echo no\n! false\necho end\n";
    let (code, out, err) = run("errexit_exempt", src);
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "ok\nend");
}

#[test]
fn errexit_stops_at_the_last_operand_of_a_list() {
    let (code, out, _) = run("errexit_last", "set -e\ntrue && false\necho after\n");
    assert_eq!(code, 1);
    assert_eq!(out, "");
}

#[test]
fn errexit_applies_inside_an_if_body() {
    let (code, out, _) = run(
        "errexit_if_body",
        "set -e\nif true; then false; echo no; fi\necho after\n",
    );
    assert_eq!(code, 1);
    assert_eq!(out, "");
}

#[test]
fn errexit_does_not_stop_a_pipeline_without_pipefail() {
    let (code, out, _) = run("errexit_pipe", "set -e\nfalse | true\necho pipe-ok\n");
    assert_eq!(code, 0);
    assert_eq!(out, "pipe-ok");
}

#[test]
fn errexit_is_not_inherited_by_command_substitution() {
    // Bash clears -e in a command substitution unless inherit_errexit is set.
    let (code, out, _) = run(
        "errexit_subst",
        "set -e\nx=$(false; echo hi)\necho \"[$x]\"\n",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "[hi]");
}

#[test]
fn set_plus_e_turns_errexit_off() {
    let (code, out, _) = run("plus_e", "set -e\nset +e\nfalse\necho still\n");
    assert_eq!(code, 0);
    assert_eq!(out, "still");

    let (code, out, _) = run(
        "plus_o_errexit",
        "set -o errexit\nset +o errexit\nfalse\necho still\n",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "still");
}

#[test]
fn set_o_errexit_is_the_same_as_e() {
    let (code, out, _) = run("o_errexit", "set -o errexit\nfalse\necho no\n");
    assert_eq!(code, 1);
    assert_eq!(out, "");
}

// ── nounset ──────────────────────────────────────────────────────────────

#[test]
fn nounset_names_an_unset_variable_and_exits_1() {
    let (code, out, err) = run(
        "nounset_var",
        "set -u\necho \"[${NOPE_ISH_UNSET_X}]\"\necho after\n",
    );
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(
        err.contains("ish: NOPE_ISH_UNSET_X: unbound variable"),
        "{err}"
    );
}

#[test]
fn nounset_refuses_an_unset_positional_parameter() {
    let (code, out, err) = run("nounset_pos", "set -u\necho \"$1\"\n");
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(err.contains("1: unbound variable"), "{err}");
}

#[test]
fn nounset_allows_at_and_star_and_set_variables() {
    let (code, out, err) = run("nounset_ok", "set -u\nx=1\necho \"[$x]\"\necho \"[$@]\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[1]\n[]");
}

#[test]
fn nounset_allows_default_value_forms() {
    let (code, out, err) = run(
        "nounset_default",
        "set -u\necho \"${NOPE_ISH_UNSET_X:-default}\"\necho \"${NOPE_ISH_UNSET_X-plain}\"\n",
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "default\nplain");
}

#[test]
fn set_plus_u_turns_nounset_off() {
    let (code, out, err) = run("plus_u", "set -u\nset +u\necho \"[${NOPE_ISH_UNSET_X}]\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[]");
}

// ── pipefail ─────────────────────────────────────────────────────────────

#[test]
fn without_pipefail_a_pipeline_takes_the_last_status() {
    let (code, out, _) = run("no_pipefail", "false | true\necho $?\n");
    assert_eq!(code, 0);
    assert_eq!(out, "0");
}

#[test]
fn pipefail_takes_the_last_failing_status() {
    let (code, out, _) = run("pipefail", "set -o pipefail\nfalse | true\necho $?\n");
    assert_eq!(code, 0);
    assert_eq!(out, "1");

    let (code, out, _) = run(
        "pipefail_last_fail",
        "set -o pipefail\ntrue | false | true\necho $?\n",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "1");
}

#[test]
fn set_plus_o_pipefail_turns_it_off() {
    let (code, out, _) = run(
        "plus_pipefail",
        "set -o pipefail\nset +o pipefail\nfalse | true\necho $?\n",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "0");
}

#[test]
fn pipefail_with_errexit_ends_the_script() {
    let (code, out, _) = run(
        "pipefail_errexit",
        "set -eo pipefail\nfalse | true\necho after\n",
    );
    assert_eq!(code, 1);
    assert_eq!(out, "");
}

// ── combinations ─────────────────────────────────────────────────────────

#[test]
fn set_euo_pipefail_runs() {
    let (code, out, err) = run(
        "euo_pipefail",
        "set -euo pipefail\necho ok\ntrue | true\necho \"[$?]\"\n",
    );
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "ok\n[0]");
}

#[test]
fn separate_option_words_run() {
    let (code, out, err) = run("e_u_o", "set -e -u -o pipefail\nx=1\necho $x\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "1");

    let (code, out, _) = run(
        "o_errexit_o_nounset",
        "set -o errexit -o nounset\necho \"[${NOPE_ISH_UNSET_X}]\"\n",
    );
    assert_eq!(code, 1);
    assert_eq!(out, "");
}

#[test]
fn set_dash_dash_still_sets_the_parameters_after_options() {
    let (code, out, err) = run("e_dash_dash", "set -e -- a b\necho \"[$#] [$1]\"\n");
    assert_eq!(code, 0, "{err}");
    assert_eq!(out, "[2] [a]");
}

// ── refusals ─────────────────────────────────────────────────────────────

#[test]
fn set_x_is_refused_before_anything_runs() {
    let (code, out, err) = run("refuse_x", "echo before\nset -x\n");
    assert_eq!(code, 77, "{err}");
    assert!(out.is_empty(), "ran something: {out:?}");
    assert!(err.contains("`set -x` at line 2, column 1"), "{err}");
}

#[test]
fn an_unrun_option_in_a_cluster_is_named() {
    let (code, out, err) = run("refuse_cluster", "set -ex\necho no\n");
    assert_eq!(code, 77, "{err}");
    assert!(out.is_empty(), "ran something: {out:?}");
    assert!(err.contains("`set -x` at line 1, column 1"), "{err}");
}

#[test]
fn an_unrun_long_option_is_named() {
    let (code, out, err) = run("refuse_xtrace", "set -o xtrace\necho no\n");
    assert_eq!(code, 77, "{err}");
    assert!(out.is_empty(), "ran something: {out:?}");
    assert!(err.contains("`set -o xtrace` at line 1, column 1"), "{err}");
}

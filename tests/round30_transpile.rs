//! `ish --zsh` and `ish --pwsh`: a transpiled script either does what the other
//! shell would do, or is refused before anything runs, naming the construct
//! and the line. Round 30 found the opposite: only the last statement printed,
//! `;` was not a separator, `setopt` swallowed its line, flags were read as
//! data, and common commands were mapped to module members that do not exist.
//!
//! zsh source that stays inside the POSIX subset runs in the in-process engine
//! `ish -b` uses (after zsh's own words are rewritten); PowerShell has only the
//! transpiler. These tests drive the real binary.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Out {
    stdout: String,
    stderr: String,
    code: i32,
}

fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ish-round30-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn run_in(dir: &Path, args: &[&str]) -> Out {
    let sessions = dir.join(".sessions");
    let _ = std::fs::create_dir_all(&sessions);
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", &sessions)
        .env_remove("IRONSHELL_ALLOW_SH")
        .output()
        .expect("run ish");
    Out {
        stdout: String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code().unwrap_or(-1),
    }
}

fn zsh(dir: &Path, script: &str) -> Out {
    run_in(dir, &["--zsh", "-c", script])
}

fn pwsh(dir: &Path, script: &str) -> Out {
    run_in(dir, &["--pwsh", "-c", script])
}

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(name), text).expect("write fixture");
}

/* ---- zsh: statements, separators, options --------------------------------- */

#[test]
fn zsh_every_statement_prints_in_order() {
    let d = fresh_dir("zsh-order");
    let o = zsh(&d, "echo a\necho b\necho c");
    assert_eq!(o.stdout, "a\nb\nc\n", "stderr: {}", o.stderr);
    assert_eq!(o.code, 0);
}

#[test]
fn zsh_semicolon_separates_statements() {
    let d = fresh_dir("zsh-semi");
    let o = zsh(&d, "echo a; echo b");
    assert_eq!(o.stdout, "a\nb\n", "stderr: {}", o.stderr);
    let o = zsh(&d, "echo 'a;b'");
    assert_eq!(o.stdout, "a;b\n", "a quoted `;` is data");
}

#[test]
fn zsh_setopt_is_its_own_statement() {
    let d = fresh_dir("zsh-setopt");
    let o = zsh(&d, "setopt nullglob; echo hi");
    assert_eq!(o.stdout, "hi\n", "stderr: {}", o.stderr);
    let o = zsh(&d, "setopt extended_glob\necho after");
    assert_eq!(o.stdout, "after\n", "stderr: {}", o.stderr);
}

#[test]
fn zsh_an_option_that_changes_meaning_is_refused_by_name() {
    let d = fresh_dir("zsh-setopt-refused");
    let o = zsh(&d, "echo before\nsetopt shwordsplit\necho after");
    assert_eq!(o.stdout, "", "nothing may run");
    assert!(o.stderr.contains("shwordsplit"), "stderr: {}", o.stderr);
    assert!(o.stderr.contains("line 2"), "stderr: {}", o.stderr);
    assert_ne!(o.code, 0);
}

#[test]
fn zsh_nullglob_with_a_glob_is_refused() {
    let d = fresh_dir("zsh-nullglob");
    write(&d, "a.txt", "x\n");
    let o = zsh(&d, "setopt nullglob\necho *.txt");
    assert_eq!(o.stdout, "");
    assert!(o.stderr.contains("nullglob"), "stderr: {}", o.stderr);
}

/* ---- zsh: flags ----------------------------------------------------------- */

#[test]
fn zsh_echo_and_print_flags() {
    let d = fresh_dir("zsh-flags");
    assert_eq!(zsh(&d, "echo -n hi").stdout, "hi");
    assert_eq!(zsh(&d, "echo -e 'a\\nb'").stdout, "a\nb\n");
    // zsh's echo interprets escapes without -e.
    assert_eq!(zsh(&d, "echo 'a\\nb'").stdout, "a\nb\n");
    assert_eq!(zsh(&d, "print -l a b").stdout, "a\nb\n");
    assert_eq!(zsh(&d, "print -n hi").stdout, "hi");
    assert_eq!(zsh(&d, "print -r 'a\\nb'").stdout, "a\\nb\n");
    assert_eq!(zsh(&d, "print plain words").stdout, "plain words\n");
}

#[test]
fn zsh_a_print_flag_with_no_posix_spelling_is_refused() {
    let d = fresh_dir("zsh-print-refused");
    let o = zsh(&d, "print -P '%~'");
    assert_eq!(o.stdout, "");
    assert!(o.stderr.contains("print -P"), "stderr: {}", o.stderr);
}

/* ---- zsh: text tools, file commands ---------------------------------------- */

#[test]
fn zsh_text_tools_run_with_gnu_semantics() {
    let d = fresh_dir("zsh-tools");
    write(&d, "f.txt", "a\nb\nc\n");
    write(&d, "n.txt", "3\n1\n2\n");
    assert_eq!(zsh(&d, "cat f.txt | wc -l").stdout.trim(), "3");
    assert_eq!(zsh(&d, "echo hello | tr a-z A-Z").stdout, "HELLO\n");
    assert_eq!(zsh(&d, "grep b f.txt").stdout, "b\n");
    assert_eq!(zsh(&d, "head -n 1 f.txt").stdout, "a\n");
    assert_eq!(zsh(&d, "sort n.txt").stdout, "1\n2\n3\n");
    assert!(zsh(&d, "find . -name f.txt").stdout.contains("f.txt"));
}

#[test]
fn zsh_date_and_whoami_resolve() {
    let d = fresh_dir("zsh-date");
    let o = zsh(&d, "date +%Y");
    let year = o.stdout.trim();
    assert_eq!(year.len(), 4, "stdout: {:?} stderr: {}", o.stdout, o.stderr);
    assert!(year.parse::<u32>().is_ok());
    let o = zsh(&d, "whoami");
    assert!(!o.stdout.trim().is_empty(), "stderr: {}", o.stderr);
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
}

// `rm` is not in the POSIX engine: it runs the PATH program, which Windows lacks.
#[cfg(unix)]
#[test]
fn zsh_rm_removes_the_file() {
    let d = fresh_dir("zsh-rm");
    write(&d, "gone.txt", "x\n");
    let o = zsh(&d, "rm gone.txt");
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    assert!(!d.join("gone.txt").exists());
}

#[test]
fn zsh_ls_with_an_unknown_flag_never_reads_it_as_a_path() {
    let d = fresh_dir("zsh-ls");
    let o = zsh(&d, "ls -la");
    assert!(
        !o.stderr.contains("no directory named '-la'"),
        "stderr: {}",
        o.stderr
    );
}

/* ---- zsh: lists, refusals, globs ------------------------------------------- */

#[test]
fn zsh_and_or_lists_run() {
    let d = fresh_dir("zsh-lists");
    assert_eq!(zsh(&d, "true && echo ok").stdout, "ok\n");
    assert_eq!(zsh(&d, "echo hi && echo there").stdout, "hi\nthere\n");
    assert_eq!(zsh(&d, "false || echo fallback").stdout, "fallback\n");
}

#[test]
fn zsh_outside_the_subset_is_refused_naming_construct_and_line() {
    let d = fresh_dir("zsh-refused");
    let o = zsh(&d, "echo before\nf() { echo hi; }\nf");
    assert_eq!(o.stdout, "", "nothing may run before a refusal");
    assert!(
        o.stderr.contains("function definition at line 2"),
        "stderr: {}",
        o.stderr
    );
    assert_ne!(o.code, 0);
}

#[test]
fn zsh_arrays_are_refused_by_name() {
    let d = fresh_dir("zsh-array");
    let o = zsh(&d, "arr=(1 2 3)\necho done");
    assert_eq!(o.stdout, "");
    assert!(
        o.stderr.contains("array assignment"),
        "stderr: {}",
        o.stderr
    );
    assert!(o.stderr.contains("line 1"), "stderr: {}", o.stderr);
}

#[test]
fn zsh_default_expansion_and_globs_and_cd() {
    let d = fresh_dir("zsh-misc");
    write(&d, "a.txt", "x\n");
    write(&d, "b.txt", "y\n");
    std::fs::create_dir_all(d.join("sub")).unwrap();
    assert_eq!(
        zsh(&d, "echo ${UNSET_ROUND30:-default}").stdout,
        "default\n"
    );
    assert_eq!(zsh(&d, "echo *.txt").stdout, "a.txt b.txt\n");
    let o = zsh(&d, "cd sub; pwd");
    assert!(
        o.stdout.trim_end().ends_with("sub"),
        "stdout: {:?}",
        o.stdout
    );
}

#[test]
fn zsh_script_file_runs_and_names_the_failing_line() {
    let d = fresh_dir("zsh-file");
    write(&d, "ok.zsh", "setopt extendedglob\necho one\necho two\n");
    let o = run_in(&d, &["ok.zsh"]);
    assert_eq!(o.stdout, "one\ntwo\n", "stderr: {}", o.stderr);
    write(
        &d,
        "bad.zsh",
        "echo one\n\ncase x in\n  x) echo y;;\nesac\n",
    );
    let o = run_in(&d, &["bad.zsh"]);
    assert_eq!(o.stdout, "");
    assert!(o.stderr.contains("line 3"), "stderr: {}", o.stderr);
}

#[test]
fn zsh_recursive_glob_runs_alone_and_is_refused_in_company() {
    let d = fresh_dir("zsh-globstar");
    std::fs::create_dir_all(d.join("deep")).unwrap();
    write(&d, "deep/n.txt", "x\n");
    let o = zsh(&d, "ls **/*.txt");
    assert!(
        o.stdout.contains("n.txt"),
        "stdout: {} stderr: {}",
        o.stdout,
        o.stderr
    );
    let o = zsh(&d, "ls **/*.txt; echo x");
    assert_eq!(o.stdout, "");
    assert!(o.stderr.contains("**"), "stderr: {}", o.stderr);
}

/* ---- PowerShell ------------------------------------------------------------ */

#[test]
fn pwsh_every_statement_prints_in_order() {
    let d = fresh_dir("ps-order");
    let o = pwsh(&d, "Write-Output one\nWrite-Output two");
    assert_eq!(o.stdout, "one\ntwo\n", "stderr: {}", o.stderr);
    let o = pwsh(&d, "Write-Host a b");
    assert_eq!(o.stdout, "a b\n", "stderr: {}", o.stderr);
}

#[test]
fn pwsh_semicolon_separates_statements() {
    let d = fresh_dir("ps-semi");
    let o = pwsh(&d, "$x = 5; $x + 2");
    assert_eq!(o.stdout, "7\n", "stderr: {}", o.stderr);
    let o = pwsh(&d, "Write-Output 'a;b'");
    assert_eq!(o.stdout, "a;b\n", "stderr: {}", o.stderr);
}

#[test]
fn pwsh_cmdlets_resolve_and_run() {
    let d = fresh_dir("ps-cmdlets");
    write(&d, "f.txt", "x\n");
    let o = pwsh(&d, "Get-Location");
    assert!(!o.stdout.contains("<builtin"), "stdout: {}", o.stdout);
    assert!(!o.stdout.trim().is_empty(), "stderr: {}", o.stderr);
    let o = pwsh(&d, "Get-Date");
    assert!(
        o.stdout.trim_start().starts_with("20"),
        "stdout: {} stderr: {}",
        o.stdout,
        o.stderr
    );
    let o = pwsh(&d, "Get-Item f.txt");
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    assert!(!o.stdout.trim().is_empty());
    let o = pwsh(&d, "Remove-Item f.txt");
    assert_eq!(o.code, 0, "stderr: {}", o.stderr);
    assert!(!d.join("f.txt").exists());
}

#[test]
fn pwsh_a_parameter_is_refused_by_name() {
    let d = fresh_dir("ps-param");
    let o = pwsh(&d, "Write-Output before\nGet-ChildItem -Recurse");
    assert_eq!(o.stdout, "", "nothing may run");
    assert!(o.stderr.contains("-Recurse"), "stderr: {}", o.stderr);
    assert!(o.stderr.contains("line 2"), "stderr: {}", o.stderr);
    assert_ne!(o.code, 0);
}

#[test]
fn pwsh_an_untranslated_command_is_refused_with_its_line() {
    let d = fresh_dir("ps-untranslated");
    write(&d, "f.txt", "x\n");
    let o = pwsh(&d, "Write-Output before\nGet-Content f.txt");
    assert_eq!(o.stdout, "");
    assert!(o.stderr.contains("Get-Content"), "stderr: {}", o.stderr);
    assert!(o.stderr.contains("line 2"), "stderr: {}", o.stderr);
    assert_ne!(o.code, 0);
}

#[test]
fn pwsh_an_unread_variable_expression_is_refused() {
    let d = fresh_dir("ps-env");
    let o = pwsh(&d, "Write-Output $env:PATH");
    assert_eq!(o.stdout, "");
    assert!(o.stderr.contains("$env:PATH"), "stderr: {}", o.stderr);
}

#[test]
fn pwsh_a_variable_named_like_a_module_still_works() {
    let d = fresh_dir("ps-module-name");
    let o = pwsh(&d, "$file = 'a.txt'\nWrite-Output $file");
    assert_eq!(o.stdout, "a.txt\n", "stderr: {}", o.stderr);
}

#[test]
fn zsh_a_variable_named_like_a_module_is_renamed_in_the_translation() {
    let code = iron_shell::transpile::zsh::transpile_zsh_to_ae("file=a.txt\necho $file").unwrap();
    assert!(code.contains("let sh_file = "), "got: {}", code);
    assert!(code.contains("print(sh_file)"), "got: {}", code);
}

/* ---- the mapping tables ---------------------------------------------------- */

/// A mapping target is a builtin, or `module.member` that the module has.
fn resolves(target: &str) -> bool {
    match target.split_once('.') {
        Some((module, member)) => iron_shell::modules::is_module_member(module, member),
        None => iron_shell::builtins::is_dispatched(target),
    }
}

#[test]
fn every_zsh_mapping_target_exists() {
    for (name, target, _) in iron_shell::transpile::zsh::ZSH_MAP.iter() {
        assert!(
            resolves(target),
            "zsh `{}` maps to `{}`, which is not a builtin or module member",
            name,
            target
        );
    }
}

#[test]
fn every_powershell_mapping_target_exists() {
    for (name, target, _) in iron_shell::transpile::powershell::PS_MAP.iter() {
        assert!(
            resolves(target),
            "PowerShell `{}` maps to `{}`, which is not a builtin or module member",
            name,
            target
        );
    }
}

#[test]
fn the_tables_have_no_duplicate_names() {
    let mut seen = std::collections::HashSet::new();
    for (name, _, _) in iron_shell::transpile::zsh::ZSH_MAP.iter() {
        assert!(seen.insert(*name), "zsh `{}` is listed twice", name);
    }
    let mut seen = std::collections::HashSet::new();
    for (name, _, _) in iron_shell::transpile::powershell::PS_MAP.iter() {
        assert!(seen.insert(*name), "PowerShell `{}` is listed twice", name);
    }
}

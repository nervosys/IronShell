//! `ish --pwsh`: parenthesised expressions, mutable variables, negative
//! literals, expression assignments, named script-block cmdlets, and one shape
//! for every transpile-time refusal (E_POLICY_DENY, exit 77, with a hint).

use std::process::Command;

struct Out {
    stdout: String,
    stderr: String,
    code: i32,
}

fn pwsh(script: &str) -> Out {
    let dir = std::env::temp_dir().join(format!("ish-round39-pwsh-{}", std::process::id()));
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
        code: out.status.code().unwrap_or(-1),
    }
}

#[test]
fn a_parenthesised_argument_is_an_expression() {
    let o = pwsh("Write-Host (1+2)");
    assert_eq!(o.stdout, "3\n", "stderr: {}", o.stderr);
    let o = pwsh("$x = 4; Write-Output ($x * 2)");
    assert_eq!(o.stdout, "8\n", "stderr: {}", o.stderr);
}

#[test]
fn a_parenthesised_command_is_refused_by_name() {
    let o = pwsh("Write-Host (Get-Date)");
    assert_eq!(o.stdout, "");
    assert!(o.stderr.contains("(Get-Date)"), "stderr: {}", o.stderr);
    assert_eq!(o.code, 77, "stderr: {}", o.stderr);
}

#[test]
fn a_variable_can_be_assigned_twice() {
    let o = pwsh("$x = 5; $x = 6; $x");
    assert_eq!(o.stdout, "6\n", "stderr: {}", o.stderr);
}

#[test]
fn a_runtime_error_names_no_wrong_line() {
    let o = pwsh("$x = 5\nWrite-Output $x\nRemove-Item nonexistent_file_round39");
    assert!(!o.stderr.contains("at line"), "stderr: {}", o.stderr);
}

#[test]
fn every_refusal_is_policy_deny_with_exit_77() {
    for script in [
        "$x = 5; $x / 2",
        "Get-ChildItem -Recurse",
        "Write-Output $env:X",
    ] {
        let o = pwsh(script);
        assert_eq!(o.stdout, "", "{script}");
        assert_eq!(o.code, 77, "{script}: {}", o.stderr);
        assert!(o.stderr.contains("E_POLICY_DENY"), "{script}: {}", o.stderr);
        assert!(o.stderr.contains("hint"), "{script}: {}", o.stderr);
        assert!(
            o.stderr.contains("nothing was run"),
            "{script}: {}",
            o.stderr
        );
    }
}

#[test]
fn where_object_and_foreach_object_are_named() {
    for (script, name) in [
        (
            "Get-Process | Where-Object { $_.Name -eq 'x' }",
            "Where-Object",
        ),
        ("Get-Process | Where-Object { $_.Name }", "Where-Object"),
        ("1,2 | ForEach-Object { $_ }", "ForEach-Object"),
    ] {
        let o = pwsh(script);
        assert!(o.stderr.contains(name), "{script}: {}", o.stderr);
        assert_eq!(o.code, 77, "{script}: {}", o.stderr);
    }
}

#[test]
fn a_negative_literal_and_an_expression_assignment() {
    let o = pwsh("7 % -3");
    assert_eq!(o.stdout, "1\n", "stderr: {}", o.stderr);
    let o = pwsh("$a = 1; $b = $a + 1; $b");
    assert_eq!(o.stdout, "2\n", "stderr: {}", o.stderr);
}

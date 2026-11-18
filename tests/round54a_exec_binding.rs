//! Round 54a: what an exec approval binds, the env denylist, `--timeout` over
//! child processes, and the hints of `sql` and a missing script.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round54a_exec_binding")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("session");
    std::fs::create_dir_all(dir.join("proj")).expect("proj");
    dir
}

fn base(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.current_dir(dir.join("proj"))
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("IRONSHELL_ALLOW_SH", "1")
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_APPROVE_ALL")
        .env_remove("IRONSHELL_APPROVE");
    cmd
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The first `apv_` token in the output of a run, panicking without it.
fn token_of(out: &Output) -> String {
    let t = text(out);
    let i = t.find("apv_").unwrap_or_else(|| {
        panic!(
            "no approval token in: {}",
            t.chars().take(600).collect::<String>()
        )
    });
    t[i..i + 20].to_string()
}

fn posix_token(dir: &Path, script: &str) -> String {
    let out = base(dir)
        .args(["-b", "--agent", "-c", script])
        .output()
        .expect("run ish");
    token_of(&out)
}

#[test]
fn a_posix_child_env_is_part_of_its_token() {
    let dir = scratch("posix_env");
    let plain = posix_token(&dir, "git --version");
    let prefixed = posix_token(&dir, "FOO=1 git --version");
    let exported = posix_token(&dir, "export FOO=1; git --version");
    // DOTNET_STARTUP_HOOKS is refused outright now, so a harmless second name
    // stands in for "a different variable".
    let hooked = posix_token(&dir, "BAR=1 git --version");
    assert_ne!(plain, prefixed, "a prefix assignment is not bound");
    assert_ne!(plain, exported, "an exported variable is not bound");
    assert_ne!(plain, hooked);
    assert_ne!(prefixed, hooked, "a different name gives the same token");
    // The same environment, however it is written, is the same approval.
    assert_eq!(prefixed, exported);
    assert_ne!(prefixed, posix_token(&dir, "FOO=2 git --version"));
    assert_eq!(prefixed, posix_token(&dir, "FOO=1 git --version"));
    assert_eq!(plain, posix_token(&dir, "git --version"));
}

#[test]
fn dotnet_and_windows_names_are_refused_everywhere() {
    let dir = scratch("names");
    let names = [
        "DOTNET_STARTUP_HOOKS",
        "dotnet_root",
        "DOTNET_ADDITIONAL_DEPS",
        "COR_PROFILER",
        "COR_ENABLE_PROFILING",
        "CORECLR_PROFILER",
        "COMPlus_ReadyToRun",
        "NUGET_PACKAGES",
        "windir",
        "__COMPAT_LAYER",
        "PSDefaultParameterValues",
    ];
    for name in names {
        for call in ["set_env", "env_set"] {
            let out = base(&dir)
                .args(["--agent", "-c", &format!(r#"{call}("{name}", "x")"#)])
                .output()
                .expect("run ish");
            assert!(
                text(&out).contains("E_POLICY_DENY"),
                "{call}({name}) was not refused"
            );
        }
        let out = base(&dir)
            .args(["-b", "--agent", "-c", &format!("export {name}=x")])
            .output()
            .expect("run ish");
        assert_eq!(out.status.code(), Some(77), "export {name}");
        let out = base(&dir)
            .args(["-b", "--agent", "-c", &format!("{name}=x someprog")])
            .output()
            .expect("run ish");
        assert_ne!(out.status.code(), Some(0), "prefix {name}");
    }
    for name in ["LANG", "TZ", "RUST_LOG", "RUST_BACKTRACE", "PROMPT"] {
        let out = base(&dir)
            .args(["--agent", "-c", &format!(r#"set_env("{name}", "x")"#)])
            .output()
            .expect("run ish");
        assert!(out.status.success(), "{name} was refused: {}", text(&out));
    }
}

fn sh_token(dir: &Path, call: &str) -> String {
    let out = base(dir)
        .args(["--agent", "-c", call])
        .output()
        .expect("run ish");
    token_of(&out)
}

#[test]
fn a_file_named_inside_an_argument_is_bound() {
    let dir = scratch("operands");
    let proj = dir.join("proj");
    let calls = [
        r#"sh(["pwsh", "-NoProfile", "-File:x.ps1"])"#,
        r#"sh(["pwsh", "-NoProfile", "-Command", ". ./x.ps1"])"#,
        r#"sh(["cmd", "/c", "call x.cmd"])"#,
        r#"sh(["tool", "--config=x.cfg"])"#,
    ];
    for file in ["x.ps1", "x.cmd", "x.cfg"] {
        std::fs::write(proj.join(file), "one").expect("write");
    }
    let before: Vec<String> = calls.iter().map(|c| sh_token(&dir, c)).collect();
    for file in ["x.ps1", "x.cmd", "x.cfg"] {
        std::fs::write(proj.join(file), "two").expect("rewrite");
    }
    let after: Vec<String> = calls.iter().map(|c| sh_token(&dir, c)).collect();
    for (i, call) in calls.iter().enumerate() {
        assert_ne!(before[i], after[i], "editing the file left {call} approved");
    }
    // Unchanged, the token is stable.
    assert_eq!(after[1], sh_token(&dir, calls[1]));
}

#[test]
fn timeout_stops_an_sh_child() {
    let dir = scratch("timeout_sh");
    let exe = env!("CARGO_BIN_EXE_ish").replace('\\', "/");
    let call = format!(r#"sh(["{exe}", "-c", "sleep(20)"])"#);
    let started = Instant::now();
    let out = base(&dir)
        .args(["--agent", "--timeout", "2", "-c", &call])
        .env("IRONSHELL_APPROVE_ALL", "1")
        .output()
        .expect("run ish");
    let took = started.elapsed();
    let t = text(&out);
    assert!(took < Duration::from_secs(15), "ran {took:?}");
    assert!(!out.status.success(), "a killed child reported success");
    assert!(t.contains("E_BUDGET_EXCEEDED"), "{t}");
    assert!(t.contains("--timeout"), "{t}");
}

#[test]
fn sql_stopped_by_timeout_names_it() {
    let dir = scratch("sql_timeout");
    let code = r#"[{a: 1}] | sql("with recursive c(x) as (select 1 union all select x + 1 from c) select count(*) from c")"#;
    let out = base(&dir)
        .args(["--agent", "--timeout", "2", "-c", code])
        .output()
        .expect("run ish");
    let t = text(&out);
    assert!(t.contains("E_BUDGET_EXCEEDED"), "{t}");
    assert!(t.contains("--timeout"), "{t}");
    assert!(!t.contains("30s budget"), "{t}");
}

#[test]
fn a_missing_script_hint_does_not_name_the_workspace_as_its_base() {
    let dir = scratch("script_hint");
    let out = base(&dir)
        .current_dir(&dir)
        .args(["--agent", "--workspace", "proj", "nope.ish"])
        .output()
        .expect("run ish");
    let t = text(&out);
    assert!(t.contains("E_NOT_FOUND"), "{t}");
    assert!(t.contains("script argument"), "{t}");
    assert!(t.contains("not against --workspace"), "{t}");
}

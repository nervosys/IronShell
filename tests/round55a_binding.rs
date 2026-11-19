//! Round 55a: what an exec approval binds (quoted file names, the file cap,
//! environment names that differ in case, links) and the env denylist.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round55a_binding")
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

/// The first 600 characters of a run's output, for a failure message.
fn head(out: &Output) -> String {
    text(out).chars().take(600).collect()
}

/// The first `apv_` token in the output of a run, panicking without it.
fn token_of(out: &Output) -> String {
    let t = text(out);
    let i = t
        .find("apv_")
        .unwrap_or_else(|| panic!("no approval token in: {}", head(out)));
    t[i..i + 20].to_string()
}

fn posix_token(dir: &Path, script: &str) -> String {
    let out = base(dir)
        .args(["-b", "--agent", "-c", script])
        .output()
        .expect("run ish");
    token_of(&out)
}

fn sh_token(dir: &Path, call: &str) -> String {
    let out = base(dir)
        .args(["--agent", "-c", call])
        .output()
        .expect("run ish");
    token_of(&out)
}

// ---- A1: a name inside quotes is bound whole ------------------------------

#[test]
fn a_quoted_file_name_with_a_space_is_bound() {
    let dir = scratch("quoted");
    let proj = dir.join("proj");
    std::fs::write(proj.join("a b.ps1"), "one").expect("write");
    let posix = [
        "pwsh -NoProfile -File 'a b.ps1'",
        r#"pwsh -NoProfile -Command "& './a b.ps1'""#,
    ];
    let sh_call = r#"sh(["pwsh", "-NoProfile", "-Command", "& 'a b.ps1'"])"#;
    let before: Vec<String> = posix.iter().map(|s| posix_token(&dir, s)).collect();
    let before_sh = sh_token(&dir, sh_call);
    std::fs::write(proj.join("a b.ps1"), "two").expect("rewrite");
    for (i, script) in posix.iter().enumerate() {
        assert_ne!(
            before[i],
            posix_token(&dir, script),
            "editing 'a b.ps1' left `{script}` approved"
        );
    }
    assert_ne!(before_sh, sh_token(&dir, sh_call), "sh() kept its token");
    // Unchanged, the token is stable.
    assert_eq!(sh_token(&dir, sh_call), sh_token(&dir, sh_call));
}

// ---- A2: the file cap is 256 and past it the command is refused ------------

fn names_call(prefix: &str, count: usize) -> String {
    let names: Vec<String> = (1..=count)
        .map(|i| format!("\"m/{prefix}{i:03}.txt\""))
        .collect();
    format!(r#"sh(["cmd", "/c", "echo", {}])"#, names.join(", "))
}

#[test]
fn a_file_past_the_old_cap_of_32_is_bound() {
    let dir = scratch("cap");
    let m = dir.join("proj").join("m");
    std::fs::create_dir_all(&m).expect("m");
    for i in 1..=40 {
        std::fs::write(m.join(format!("f{i:03}.txt")), "one").expect("write");
    }
    let call = names_call("f", 40);
    let before = sh_token(&dir, &call);
    std::fs::write(m.join("f040.txt"), "two").expect("rewrite");
    assert_ne!(before, sh_token(&dir, &call), "f040 is not bound");
    std::fs::write(m.join("f033.txt"), "two").expect("rewrite");
    let mid = sh_token(&dir, &call);
    std::fs::write(m.join("f033.txt"), "three").expect("rewrite");
    assert_ne!(mid, sh_token(&dir, &call), "f033 is not bound");
}

#[test]
fn more_than_256_workspace_files_are_refused() {
    let dir = scratch("over_cap");
    let m = dir.join("proj").join("m");
    std::fs::create_dir_all(&m).expect("m");
    for i in 1..=300 {
        std::fs::write(m.join(format!("g{i:03}.txt")), "x").expect("write");
    }
    let out = base(&dir)
        .args(["--agent", "-c", &names_call("g", 300)])
        .output()
        .expect("run ish");
    let t = text(&out);
    assert!(t.contains("E_POLICY_DENY"), "{}", head(&out));
    assert!(
        t.contains("more than 256 workspace files"),
        "{}",
        head(&out)
    );
    assert!(!t.contains("apv_"), "a token was offered: {}", head(&out));
}

// ---- A6: a file reached through a link out of the workspace is bound -------

/// Make `link` a link (a junction on Windows) to the directory `target`.
fn link_dir(link: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|o| o.status.success())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (link, target);
        false
    }
}

#[test]
fn a_file_outside_the_workspace_named_through_a_link_is_bound() {
    let dir = scratch("link");
    let outside = dir.join("outside");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::write(outside.join("s.ps1"), "one").expect("write");
    if !link_dir(&dir.join("proj").join("jout"), &outside) {
        eprintln!("cannot make a link here; skipped");
        return;
    }
    let call = r#"sh(["pwsh", "-NoProfile", "-File", "jout/s.ps1"])"#;
    let before = sh_token(&dir, call);
    std::fs::write(outside.join("s.ps1"), "two").expect("rewrite");
    assert_ne!(before, sh_token(&dir, call), "the linked file is not bound");
}

// ---- A3: names that differ only in case ------------------------------------

#[test]
fn exports_differing_in_case_follow_the_platform() {
    let dir = scratch("case_posix");
    let one_two = posix_token(&dir, "export FOO=1; export foo=2; git --version");
    let two_one = posix_token(&dir, "export foo=2; export FOO=1; git --version");
    if cfg!(windows) {
        // One variable there: the later export is what the child sees.
        assert_ne!(one_two, two_one, "the two orders share a token");
        let only_two = posix_token(&dir, "export FOO=2; git --version");
        assert_eq!(one_two, only_two, "FOO then foo=2 is FOO=2");
    } else {
        // Two variables here, and the child sees both either way.
        assert_eq!(one_two, two_one, "two variables, one set");
    }
}

#[test]
fn set_env_names_differing_in_case_follow_the_platform() {
    let dir = scratch("case_set_env");
    let call = r#"sh(["git", "--version"])"#;
    let a = sh_token(
        &dir,
        &format!(r#"set_env("foo", "2"); set_env("FOO", "1"); {call}"#),
    );
    let b = sh_token(
        &dir,
        &format!(r#"set_env("FOO", "1"); set_env("foo", "2"); {call}"#),
    );
    if cfg!(windows) {
        assert_ne!(a, b, "the two orders share a token");
    } else {
        assert_eq!(a, b, "two variables, one set");
    }
}

// ---- A7: unset of an inherited variable ------------------------------------

#[test]
fn unset_of_an_inherited_variable_changes_the_token_and_the_child() {
    let dir = scratch("unset");
    let plain = {
        let out = base(&dir)
            .env("A7_VAR", "SENTINEL_7a1")
            .args(["-b", "--agent", "-c", "git --version"])
            .output()
            .expect("run ish");
        token_of(&out)
    };
    let unset = {
        let out = base(&dir)
            .env("A7_VAR", "SENTINEL_7a1")
            .args(["-b", "--agent", "-c", "unset A7_VAR; git --version"])
            .output()
            .expect("run ish");
        token_of(&out)
    };
    assert_ne!(plain, unset, "an unset inherited variable is not bound");

    let exe = env!("CARGO_BIN_EXE_ish").replace('\\', "/");
    let show = |pre: &str| {
        let script = format!(r#"{pre}"{exe}" -c 'env("A7_VAR")'"#);
        let out = base(&dir)
            .env("A7_VAR", "SENTINEL_7a1")
            .args(["-b", "-c", &script])
            .output()
            .expect("run ish");
        text(&out)
    };
    assert!(show("").contains("SENTINEL_7a1"), "control: {}", show(""));
    let after = show("unset A7_VAR; ");
    assert!(
        !after.contains("SENTINEL_7a1"),
        "the child still sees it: {}",
        after.chars().take(300).collect::<String>()
    );
}

#[test]
fn export_with_an_option_and_set_dash_a_are_refused_by_name() {
    let dir = scratch("refused");
    for script in ["export -n FOO", "set -a"] {
        let out = base(&dir)
            .args(["-b", "-c", script])
            .output()
            .expect("run ish");
        assert_eq!(out.status.code(), Some(77), "{script}: {}", head(&out));
        assert!(
            text(&out).contains(script.split(' ').next().unwrap()),
            "{script}"
        );
    }
}

// ---- A4: more names that steer a child, and names that are no names --------

#[test]
fn shell_and_toolchain_names_are_refused_everywhere() {
    let dir = scratch("denylist");
    let names = [
        "SHELLOPTS",
        "BASHOPTS",
        "PS4",
        "ps1",
        "PS2",
        "PS0",
        "GLOBIGNORE",
        "CDPATH",
        "NODE_TLS_REJECT_UNAUTHORIZED",
        "NODE_REPL_EXTERNAL_MODULE",
        "NODE_EXTRA_CA_CERTS",
        "JAVA_HOME",
        "MSBUILDENABLEALLPROPERTYFUNCTIONS",
        "MSBuildSDKsPath",
        "NUGET_PLUGIN_PATHS",
        "PSMODULEANALYSISCACHEPATH",
        "PSModuleAutoLoadingPreference",
        "PSHOME",
        "PSLockdownPolicy",
        "__PSLockdownPolicy",
        "PWSH_FOO",
    ];
    for name in names {
        for call in ["set_env", "env_set"] {
            let out = base(&dir)
                .args(["--agent", "-c", &format!(r#"{call}("{name}", "x")"#)])
                .output()
                .expect("run ish");
            assert!(
                text(&out).contains("E_POLICY_DENY"),
                "{call}({name}) was not refused: {}",
                head(&out)
            );
        }
        let out = base(&dir)
            .args(["-b", "--agent", "-c", &format!("export {name}=x")])
            .output()
            .expect("run ish");
        assert_eq!(out.status.code(), Some(77), "export {name}");
    }
}

#[test]
fn a_name_with_whitespace_or_equals_is_not_a_valid_name() {
    let dir = scratch("bad_names");
    for name in ["PATH ", " PATH", "A=B", "", "\tTMP"] {
        let call = format!(r#"set_env("{name}", "x")"#);
        let out = base(&dir)
            .args(["--agent", "-c", &call])
            .output()
            .expect("run ish");
        let t = text(&out);
        assert!(t.contains("E_BAD_ARG"), "{name:?}: {}", head(&out));
        assert!(
            t.contains("not a valid environment variable name"),
            "{name:?}: {}",
            head(&out)
        );
    }
    // An ordinary name still works.
    let out = base(&dir)
        .args(["--agent", "-c", r#"set_env("A4_PLAIN", "x")"#])
        .output()
        .expect("run ish");
    assert!(out.status.success(), "{}", head(&out));
}

// ---- A5: a --timeout kill ends an `ish -b` run with E_BUDGET_EXCEEDED -------

#[test]
fn a_timeout_kill_in_a_pipeline_exits_69_whichever_stage_was_killed() {
    let dir = scratch("timeout_pipe");
    let exe = env!("CARGO_BIN_EXE_ish").replace('\\', "/");
    let slow = format!(r#""{exe}" -c 'sleep(20)'"#);
    let scripts = [
        format!("echo hi | {slow} | sort"),
        format!("echo hi | {slow}"),
        format!("echo hi | sort | {slow} | sort; echo SENTINEL_AFTER"),
    ];
    for script in scripts {
        let started = std::time::Instant::now();
        let out = base(&dir)
            .args(["-b", "--timeout", "2", "-c", &script])
            .output()
            .expect("run ish");
        let t = text(&out);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(15),
            "ran {:?}",
            started.elapsed()
        );
        assert_eq!(out.status.code(), Some(69), "{script}: {}", head(&out));
        assert!(t.contains("E_BUDGET_EXCEEDED"), "{script}: {}", head(&out));
        assert!(
            !t.contains("SENTINEL_AFTER"),
            "the script went on: {}",
            head(&out)
        );
    }
}

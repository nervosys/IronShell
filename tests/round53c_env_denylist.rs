//! Round 53c: `set_env` in agent mode refuses more of the variables that steer
//! a program an approved command runs (`PSModulePath` made an approved
//! `pwsh -Command Get-Evil` load code the agent had written), and the approval
//! of an `sh` call is bound to the environment the agent changed, so a token
//! given before a `set_env` does not approve the call after it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round53c_env_denylist")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("session");
    std::fs::create_dir_all(dir.join("proj")).expect("proj");
    dir
}

fn agent(dir: &Path, code: &str, approve: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["--agent", "-c", code])
        .current_dir(dir.join("proj"))
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("IRONSHELL_ALLOW_SH", "1")
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_APPROVE_ALL")
        .env_remove("IRONSHELL_APPROVE");
    if let Some(token) = approve {
        cmd.env("IRONSHELL_APPROVE", token);
    }
    cmd.output().expect("run ish")
}

fn first_error(out: &Output) -> serde_json::Value {
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().next().unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {err:?}"))
}

fn exe() -> String {
    env!("CARGO_BIN_EXE_ish").replace('\\', "/")
}

#[test]
fn the_added_names_are_refused_through_every_setter() {
    let dir = scratch("names");
    let names = [
        "PSModulePath",
        "psmodulepath",
        "PSExecutionPolicyPreference",
        "POWERSHELL_TELEMETRY_OPTOUT",
        "PERL5LIB",
        "PERL5OPT",
        "PERLLIB",
        "RUBYLIB",
        "RUBYOPT",
        "GEM_HOME",
        "GEM_PATH",
        "BUNDLE_GEMFILE",
        "JAVA_TOOL_OPTIONS",
        "_JAVA_OPTIONS",
        "JDK_JAVA_OPTIONS",
        "CLASSPATH",
        "NODE_PATH",
        "npm_config_registry",
        "BASH_ENV",
        "ENV",
        "ZDOTDIR",
        "PROMPT_COMMAND",
        "DYLD_INSERT_LIBRARIES",
        "LD_PRELOAD",
        "PYTHONWARNINGS",
        "PIP_INDEX_URL",
        "UV_INDEX_URL",
        "CARGO_HOME",
        "RUSTC_WRAPPER",
        "RUSTFLAGS",
        "CC",
        "CXX",
        "MAKEFLAGS",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "CURL_CA_BUNDLE",
        "REQUESTS_CA_BUNDLE",
        "GIT_SSH_COMMAND",
        "TEMP",
        "TMP",
        "TMPDIR",
        "XDG_CONFIG_HOME",
        "AETHER_SESSION_DIR",
    ];
    for name in names {
        for call in ["set_env", "env_set"] {
            let out = agent(&dir, &format!(r#"{call}("{name}", "x")"#), None);
            let json = first_error(&out);
            assert_eq!(
                json["error"]["code"], "E_POLICY_DENY",
                "{call}({name}) was not refused: {json}"
            );
        }
        let out = agent(&dir, &format!(r#"env_unset("{name}")"#), None);
        assert_eq!(
            first_error(&out)["error"]["code"],
            "E_POLICY_DENY",
            "{name}"
        );
    }
}

#[test]
fn an_ordinary_name_is_still_settable() {
    let dir = scratch("ordinary");
    let out = agent(&dir, r#"set_env("MY_APP_FLAG", "1")"#, None);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn a_token_given_before_a_set_env_does_not_approve_the_call_after_it() {
    let dir = scratch("bound");
    let call = format!(r#"sh(["{}", "--version"])"#, exe());
    let first = agent(&dir, &call, None);
    let json = first_error(&first);
    assert_eq!(json["error"]["code"], "E_NEEDS_APPROVAL", "{json}");
    let token = json["error"]["approval"]["token"]
        .as_str()
        .expect("a token")
        .to_string();
    // The token approves the same call.
    let again = agent(&dir, &call, Some(&token));
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );

    // After a set_env the same call needs another approval.
    let with_env = format!(r#"set_env("MY_APP_FLAG", "1"); {call}"#);
    let out = agent(&dir, &with_env, Some(&token));
    let json2 = first_error(&out);
    assert_eq!(json2["error"]["code"], "E_NEEDS_APPROVAL", "{json2}");
    let token2 = json2["error"]["approval"]["token"]
        .as_str()
        .expect("token2");
    assert_ne!(token2, token, "the token ignores the environment change");
    let hint = json2["error"]["hint"].as_str().unwrap_or("");
    assert!(hint.contains("environment"), "{hint}");
    // And its own token approves it.
    let ok = agent(&dir, &with_env, Some(token2));
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
}

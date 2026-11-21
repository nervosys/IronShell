//! Round 61a: configuration-path steering names, the sql error that echoed a
//! secret cell, Digest and AWS SigV4 headers, short and camel-case secret
//! names, the pre-gate address check for four more web builtins, link-local and
//! cloud-metadata hosts, `session_redo` through a hard link, and `rmdir` of the
//! workspace root. Every secret here is fake.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("round61a_security")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ws")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Dirs { root }
    }

    fn ws(&self) -> PathBuf {
        self.root.join("ws")
    }

    fn cmd(&self, args: &[&str], agent: bool) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
        cmd.args(args)
            .current_dir(self.ws())
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .env_remove("IRONSHELL_APPROVE")
            .env_remove("IRONSHELL_APPROVE_ALL")
            .env_remove("IRONSHELL_REDACT")
            .env_remove("IRONSHELL_NET_ALLOW")
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_WORKSPACE");
        if agent {
            cmd.env("IRONSHELL_MODE", "agent")
                .env("IRONSHELL_WORKSPACE", self.ws());
        }
        cmd
    }

    fn run(&self, code: &str) -> Output {
        self.cmd(&["-c", code], true).output().expect("run ish")
    }

    fn run_human(&self, code: &str) -> Output {
        self.cmd(&["-c", code], false).output().expect("run ish")
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn error_json(o: &Output) -> serde_json::Value {
    let e = err(o);
    let line = e
        .lines()
        .find(|l| l.starts_with('{'))
        .unwrap_or_else(|| panic!("no JSON error in: {e}"));
    serde_json::from_str(line).unwrap_or_else(|x| panic!("not JSON ({x}): {line}"))
}

fn code_of(o: &Output) -> String {
    error_json(o)["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn config_path_names_are_refused_so_rg_cannot_run_an_agent_program() {
    let d = Dirs::new("steer");
    for name in [
        "RIPGREP_CONFIG_PATH",
        "GNUMAKEFLAGS",
        "MFLAGS",
        "PYTEST_ADDOPTS",
        "PYTEST_PLUGINS",
        "FZF_DEFAULT_OPTS",
        "BAT_CONFIG_PATH",
        "DELTA_PAGER",
        "PERL_MM_OPT",
        "PERL_MB_OPT",
        "CONTAINERS_CONF",
        "CONTAINER_HOST",
        "KUBE_CONFIG_PATH",
        "TERRAFORM_CONFIG",
        "SOPS_AGE_KEY_FILE",
        "MYVIMRC",
        "VIMRUNTIME",
        "GEMRC",
        "NPMRC",
        "IRBRC",
        "PODMAN_CONNECTION",
        "BUILDAH_RUNTIME",
        "GLAB_CONFIG_DIR",
        "STARSHIP_CONFIG",
        "HOMEBREW_NO_INSTALL_FROM_API",
        "JUST_CHOOSER",
        "ZELLIJ_CONFIG_FILE",
        "TMUX_TMPDIR",
        // The generic rules: any tool's config path, config file, config
        // directory, run-control file or option string.
        "SOMETOOL_CONFIG_PATH",
        "sometool_config_file",
        "SOMETOOL_CONFIG_DIR",
        "SOMETOOL_RC",
        "SOMETOOL_OPTS",
        "SOMETOOL_ADDOPTS",
    ] {
        let o = d.run(&format!(r#"set_env("{name}", "x")"#));
        assert_eq!(code_of(&o), "E_POLICY_DENY", "{name}: {}", err(&o));
    }
}

#[test]
fn the_ripgrep_config_attack_does_not_run_the_pre_program() {
    let d = Dirs::new("rgattack");
    let ws = d.ws();
    std::fs::write(ws.join("hay.txt"), "needle\n").unwrap();
    let marker = d.root.join("marker_rg.txt");
    let pre = if cfg!(windows) {
        let p = ws.join("pre.bat");
        std::fs::write(
            &p,
            format!("@echo off\r\necho x > \"{}\"\r\n", marker.display()),
        )
        .unwrap();
        p
    } else {
        let p = ws.join("pre.sh");
        std::fs::write(&p, format!("#!/bin/sh\necho x > '{}'\n", marker.display())).unwrap();
        p
    };
    let config = format!("--pre={}\n", pre.display()).replace('\\', "/");
    std::fs::write(ws.join("rgrc"), config).unwrap();
    let o = d.run(r#"set_env("RIPGREP_CONFIG_PATH", "rgrc"); rg_search("needle", "hay.txt")"#);
    assert_eq!(code_of(&o), "E_POLICY_DENY", "{}", err(&o));
    assert!(!marker.exists(), "the --pre program ran");
}

#[test]
fn harmless_names_can_still_be_set() {
    let d = Dirs::new("harmless");
    for name in ["MY_APP_FLAG", "LANG", "TZ", "SRC", "CPU_COUNT", "FORCE"] {
        let o = d.run(&format!(r#"set_env("{name}", "x")"#));
        assert!(!err(&o).contains("E_POLICY_DENY"), "{name}: {}", err(&o));
    }
}

const SQL_CSV: &str = "user,password,token,amount\na,hunter2secret,tok_abcdef123456,5\n";

#[test]
fn sql_strict_aggregate_error_does_not_echo_the_cell_in_agent_mode() {
    let d = Dirs::new("sqlerr");
    std::fs::write(d.ws().join("t.csv"), SQL_CSV).unwrap();
    for q in [
        r#"open("t.csv") | sql("select sum(password) from t")"#,
        r#"open("t.csv") | sql("select avg(token) from t")"#,
        r#"open("t.csv") | sql("select total(password) from t")"#,
        r#"open("t.csv") | sql("select sum(user || password) from t")"#,
        r#"sql_value("t.csv", "select sum(password) from t")"#,
    ] {
        let o = d.run(q);
        assert_eq!(code_of(&o), "E_BAD_ARG", "{q}: {}", err(&o));
        let all = format!("{}{}", out(&o), err(&o));
        for secret in ["hunter2secret", "tok_abcdef123456"] {
            assert!(!all.contains(secret), "{q} leaked {secret}: {all}");
        }
        assert!(all.contains("not a clean number"), "{q}: {all}");
    }
}

#[test]
fn sql_strict_aggregate_error_still_shows_the_text_to_a_person() {
    let d = Dirs::new("sqlerrhuman");
    std::fs::write(d.ws().join("t.csv"), SQL_CSV).unwrap();
    let o = d.run_human(r#"open("t.csv") | sql("select sum(user) from t")"#);
    let all = format!("{}{}", out(&o), err(&o));
    assert!(all.contains("the text 'a'"), "{all}");
}

#[test]
fn digest_sigv4_and_bare_bearer_are_redacted() {
    let d = Dirs::new("headers");
    for (code, secrets, kept) in [
        (
            r#""Authorization: Digest username=\"u\", response=\"abcdef0123456789\", cnonce=\"c0ffee123456\"""#,
            &["abcdef0123456789", "c0ffee123456"][..],
            "Authorization: Digest username=",
        ),
        (
            r#""Authorization: AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE0/20240101/us-east-1/s3/aws4_request, SignedHeaders=host, Signature=abcdef0123456789abcdef""#,
            &["AKIDEXAMPLE0", "abcdef0123456789abcdef"][..],
            "Authorization: AWS4-HMAC-SHA256 Credential=[REDACTED]",
        ),
        (
            r#""Bearer abcdefghijklmnop1234""#,
            &["abcdefghijklmnop1234"][..],
            "Bearer [REDACTED]",
        ),
    ] {
        let o = d.run(code);
        let text = out(&o);
        for secret in secrets {
            assert!(!text.contains(secret), "{code}: {text}");
        }
        assert!(text.contains(kept), "{code}: {text}");
    }
    for prose in ["a bearer of bad news", "Bearer tokens expire"] {
        let o = d.run(&format!(r#""{prose}""#));
        assert!(out(&o).contains(prose), "{prose}: {}", out(&o));
    }
}

#[test]
fn short_and_camel_case_secret_names_are_redacted() {
    let d = Dirs::new("names");
    for key in [
        "pw",
        "session",
        "sid",
        "sessionId",
        "session_id",
        "csrf",
        "xsrf",
        "jwt",
        "auth",
        "otp",
        "pin",
        "accessKey",
        "AccessKey",
        "access_key",
        "apiKey",
        "secretKey",
    ] {
        let o = d.run(&format!(r#"{{"{key}": "Zq81xv04mm"}}"#));
        let text = out(&o);
        assert!(!text.contains("Zq81xv04mm"), "{key}: {text}");
        assert!(text.contains("[REDACTED]"), "{key}: {text}");
    }
}

#[test]
fn names_about_a_secret_are_not_redacted() {
    let d = Dirs::new("namesfalse");
    for key in [
        "cookie_policy",
        "session_name",
        "auth_method",
        "auth_enabled",
        "pin_required",
        "jwt_name",
        "pinned",
        "spin",
        "authority",
        "sessions_dir",
    ] {
        let o = d.run(&format!(r#"{{"{key}": "plain-value"}}"#));
        assert!(out(&o).contains("plain-value"), "{key}: {}", out(&o));
    }
    for key in ["session_count", "sid_len", "auth_count"] {
        let o = d.run(&format!(r#"{{"{key}": 12345}}"#));
        assert!(out(&o).contains("12345"), "{key}: {}", out(&o));
    }
}

#[test]
fn four_more_web_builtins_refuse_this_machine_before_the_gate() {
    let d = Dirs::new("web4");
    // A listener nobody should reach: refused calls must not connect to it.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    for call in [
        format!(r#"web_get("http://127.0.0.1:{port}/")"#),
        format!(r#"web_sitemap("http://127.0.0.1:{port}/")"#),
        format!(r#"web_robots_txt("http://127.0.0.1:{port}/")"#),
        format!(r#"web_websocket("ws://127.0.0.1:{port}/")"#),
        format!(r#"web_get("http://localhost:{port}/")"#),
        r#"web_get("http://169.254.169.254/latest/meta-data/")"#.to_string(),
        r#"web_websocket("ws://10.0.0.5/")"#.to_string(),
    ] {
        let o = d.run(&call);
        let j = error_json(&o);
        assert_eq!(j["error"]["code"], "E_POLICY_DENY", "{call}: {j}");
        assert!(j["error"].get("approval").is_none(), "{call}: {j}");
        assert!(!j.to_string().contains("apv_"), "{call}: {j}");
    }
    assert!(
        listener.accept().is_err(),
        "a refused call connected to the listener"
    );
}

#[test]
fn four_more_web_builtins_still_ask_for_a_public_url() {
    let d = Dirs::new("web4public");
    for call in [
        r#"web_get("https://example.com/")"#,
        r#"web_sitemap("https://example.com/")"#,
        r#"web_robots_txt("https://example.com/")"#,
        r#"web_websocket("wss://example.com/")"#,
    ] {
        let o = d.run(call);
        assert_eq!(code_of(&o), "E_NEEDS_APPROVAL", "{call}: {}", err(&o));
    }
}

#[test]
fn raw_tcp_and_scp_refuse_link_local_and_metadata_hosts() {
    let d = Dirs::new("metadata");
    for call in [
        r#"net_connect("169.254.169.254", 80)"#,
        r#"net_connect("169.254.1.1", 80)"#,
        r#"net_connect("fd00:ec2::254", 80)"#,
        r#"net_connect("fe80::1", 80)"#,
        r#"net_connect("::ffff:169.254.169.254", 80)"#,
        r#"scp_download("u@169.254.169.254:/x", "o.txt")"#,
        r#"scp_upload("hay.txt", "u@169.254.169.254:/x")"#,
        r#"scp_download("u@metadata.google.internal:/x", "o.txt")"#,
    ] {
        let o = d.run(call);
        let j = error_json(&o);
        assert_eq!(j["error"]["code"], "E_POLICY_DENY", "{call}: {j}");
        assert!(j["error"].get("approval").is_none(), "{call}: {j}");
    }
}

#[test]
fn raw_tcp_and_scp_to_loopback_still_need_an_approval() {
    let d = Dirs::new("loopback");
    std::fs::write(d.ws().join("hay.txt"), "x\n").unwrap();
    for call in [
        r#"net_connect("127.0.0.1", 9)"#,
        r#"scp_download("u@127.0.0.1:/x", "o.txt")"#,
    ] {
        let o = d.run(call);
        assert_eq!(code_of(&o), "E_NEEDS_APPROVAL", "{call}: {}", err(&o));
    }
}

#[test]
fn rmdir_of_the_workspace_root_is_refused_in_agent_mode() {
    let d = Dirs::new("rmroot");
    std::fs::create_dir(d.ws().join("e")).unwrap();
    std::fs::write(d.ws().join("keep.txt"), "x\n").unwrap();
    for call in [
        r#"rmdir(".", true)"#,
        r#"rmdir("e/..", true)"#,
        r#"rmdir(".")"#,
    ] {
        let o = d.run(call);
        let j = error_json(&o);
        assert_eq!(j["error"]["code"], "E_POLICY_DENY", "{call}: {j}");
        assert!(j["error"].get("approval").is_none(), "{call}: {j}");
        assert!(j.to_string().contains("workspace root"), "{call}: {j}");
    }
    assert!(d.ws().join("keep.txt").exists());
    // A directory inside it is still an ordinary approved delete.
    let o = d.run(r#"rmdir("e", true)"#);
    assert_eq!(code_of(&o), "E_NEEDS_APPROVAL");
}

#[test]
fn session_redo_does_not_write_through_a_hard_link_to_the_audit_log() {
    let d = Dirs::new("redolink");
    let ws = d.ws();
    let journal = [("IRONSHELL_JOURNAL", "on")];
    let run = |code: &str| {
        let mut cmd = d.cmd(&["-c", code], true);
        for (k, v) in journal {
            cmd.env(k, v);
        }
        cmd.output().expect("run ish")
    };
    let o = run(r#"file.write("h.txt", "ORIGINAL")"#);
    assert!(o.status.success(), "{}", err(&o));
    let o = run("session_undo()");
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    assert!(!ws.join("h.txt").exists(), "the undo left h.txt");
    let log = ws.join(".ironshell").join("audit.log");
    let before = std::fs::read(&log).expect("the audit log exists after the writes");
    if std::fs::hard_link(&log, ws.join("h.txt")).is_err() {
        // A file system with no hard links: nothing to test here.
        return;
    }
    let o = run("session_redo({force: true})");
    let after = std::fs::read(&log).unwrap();
    assert!(
        !String::from_utf8_lossy(&after).starts_with("ORIGINAL"),
        "the redo wrote into the audit log: {}{}",
        out(&o),
        err(&o)
    );
    assert!(
        after.len() >= before.len(),
        "the audit log shrank from {} to {} bytes",
        before.len(),
        after.len()
    );
    assert!(
        format!("{}{}", out(&o), err(&o)).contains("hard link")
            || format!("{}{}", out(&o), err(&o)).contains("audit"),
        "{}{}",
        out(&o),
        err(&o)
    );
}

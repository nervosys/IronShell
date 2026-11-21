//! Round 60a: header-style secrets redacted by name and by text, URL validation
//! and the SSRF literal check before the approval gate for every web builtin,
//! `rmdir` of a non-empty directory refused before the gate, and more steering
//! names. Every secret here is fake.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("round60a_security")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ws")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Dirs { root }
    }

    fn ws(&self) -> PathBuf {
        self.root.join("ws")
    }

    fn run_with(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
        cmd.args(args)
            .current_dir(self.ws())
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", self.ws())
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .env_remove("IRONSHELL_APPROVE")
            .env_remove("IRONSHELL_APPROVE_ALL")
            .env_remove("IRONSHELL_REDACT")
            .env_remove("IRONSHELL_NET_ALLOW");
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().expect("run ish")
    }

    fn run(&self, code: &str) -> Output {
        self.run_with(&["-c", code], &[])
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

#[test]
fn header_style_record_keys_are_redacted() {
    let d = Dirs::new("keys");
    for key in [
        "authorization",
        "Authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "Set-Cookie",
        "x-api-key",
        "X-Api-Key",
        "bearer",
        "pass",
        "pwd",
        "db_pass",
        "DB_PWD",
        "dbPass",
    ] {
        let o = d.run(&format!(r#"{{"{key}": "abcdef123456zz"}}"#));
        let text = out(&o);
        assert!(!text.contains("abcdef123456zz"), "{key}: {text}");
        assert!(text.contains("[REDACTED]"), "{key}: {text}");
    }
}

#[test]
fn unrelated_names_that_contain_pass_or_cookie_are_not_redacted() {
    let d = Dirs::new("falsepos");
    for key in [
        "passage",
        "compass",
        "bypass",
        "bypass_cache",
        "passenger",
        "cookie_count",
        "cookies_total",
        "pass_len",
    ] {
        let o = d.run(&format!(r#"{{"{key}": 12345}}"#));
        let text = out(&o);
        assert!(text.contains("12345"), "{key} was redacted: {text}");
    }
    for key in ["passage", "compass", "bypass_cache"] {
        let o = d.run(&format!(r#"{{"{key}": "plain words here"}}"#));
        assert!(out(&o).contains("plain words here"), "{key}: {}", out(&o));
    }
    // A string under a cookie name is a cookie, and a number under a bare
    // secret name is a secret. (Round 61: a trailing `count` or `policy` says
    // the field describes the cookie, so `cookie_count` is no longer one.)
    let o = d.run(r#"{"cookie": "sid=abc123def"}"#);
    assert!(!out(&o).contains("abc123def"), "{}", out(&o));
    let o = d.run(r#"{"pwd": 987654321}"#);
    assert!(!out(&o).contains("987654321"), "{}", out(&o));
}

#[test]
fn header_style_text_is_redacted() {
    let d = Dirs::new("text");
    for (code, secret) in [
        (r#""Authorization: Bearer abcdef123456""#, "abcdef123456"),
        (r#""authorization: bearer abcdef123456""#, "abcdef123456"),
        (
            r#""Authorization: Basic dXNlcjpwYXNzd29yZA==""#,
            "dXNlcjpwYXNz",
        ),
        (r#""Cookie: sid=abc123def456; theme=dark""#, "abc123def456"),
        (
            r#""Set-Cookie: sid=abc123def456; Path=/; HttpOnly""#,
            "abc123def456",
        ),
        (r#""Cookie: k=v""#, "k=v"),
        (r#""x-api-key: abcdef123456""#, "abcdef123456"),
        (
            r#"to_json({"headers": "Authorization: Bearer abcdef123456"})"#,
            "abcdef123456",
        ),
    ] {
        let o = d.run(code);
        let text = out(&o);
        assert!(!text.contains(secret), "{code}: {text}");
        assert!(text.contains("[REDACTED]"), "{code}: {text}");
    }
    // The scheme stays, so the line still reads as a header.
    let o = d.run(r#""Authorization: Bearer abcdef123456""#);
    assert!(
        out(&o).contains("Authorization: Bearer [REDACTED]"),
        "{}",
        out(&o)
    );
}

#[test]
fn prose_about_headers_is_left_alone() {
    let d = Dirs::new("prose");
    for text in [
        "authorization: required",
        "the cookie: yes please",
        "a bearer of bad news",
        "Authorization header is optional",
    ] {
        let o = d.run(&format!(r#""{text}""#));
        assert!(out(&o).contains(text), "{text}: {}", out(&o));
    }
}

const WEB_CALLS: &[&str] = &[
    r#"http_get("{u}")"#,
    r#"http.get("{u}")"#,
    r#"web.fetch("{u}")"#,
    r#"http.post("{u}", "x")"#,
    r#"http.download("{u}")"#,
    r#"web_headers("{u}")"#,
    r#"web_json_get("{u}")"#,
    // An output path, because a URL that names no file is a bad argument first.
    r#"wget_download("{u}", "out.bin")"#,
];

#[test]
fn web_builtins_refuse_this_machine_before_the_gate() {
    let d = Dirs::new("webssrf");
    for u in [
        "http://127.0.0.1:8765/",
        "http://localhost:8765/",
        "http://localhost.:8765/",
        "http://foo.localhost:8765/",
        "http://10.0.0.5/x",
    ] {
        for call in WEB_CALLS {
            let o = d.run(&call.replace("{u}", u));
            let j = error_json(&o);
            assert_eq!(j["error"]["code"], "E_POLICY_DENY", "{call} {u}: {j}");
            assert!(j["error"].get("approval").is_none(), "{call} {u}: {j}");
            assert!(!j.to_string().contains("apv_"), "{call} {u}: {j}");
        }
    }
}

#[test]
fn web_builtins_refuse_a_bad_url_before_the_gate() {
    let d = Dirs::new("webbad");
    for u in ["not a url", "ftp://x/y", "example.com"] {
        for call in WEB_CALLS {
            let o = d.run(&call.replace("{u}", u));
            let j = error_json(&o);
            assert_eq!(j["error"]["code"], "E_BAD_ARG", "{call} {u}: {j}");
            assert!(j["error"].get("approval").is_none(), "{call} {u}: {j}");
        }
    }
}

#[test]
fn a_public_url_still_asks_for_approval() {
    let d = Dirs::new("webpublic");
    for call in [
        r#"http_get("https://example.com/")"#,
        r#"web.fetch("https://example.com/")"#,
    ] {
        let o = d.run(call);
        assert_eq!(
            error_json(&o)["error"]["code"],
            "E_NEEDS_APPROVAL",
            "{call}"
        );
    }
}

#[test]
fn rmdir_of_a_non_empty_directory_without_recursive_issues_no_token() {
    let d = Dirs::new("rmdirfull");
    std::fs::create_dir_all(d.ws().join("t/a")).unwrap();
    std::fs::write(d.ws().join("t/a/f.txt"), "x\n").unwrap();
    let o = d.run(r#"rmdir("t/a")"#);
    let j = error_json(&o);
    assert_eq!(j["error"]["code"], "E_BAD_STATE", "{j}");
    assert!(j["error"].get("approval").is_none(), "{j}");
    assert!(j.to_string().contains("not empty"), "{j}");
    assert!(d.ws().join("t/a/f.txt").exists());
    // Recursive, and an empty directory, still ask.
    let o = d.run(r#"rmdir("t/a", true)"#);
    assert_eq!(error_json(&o)["error"]["code"], "E_NEEDS_APPROVAL");
    std::fs::create_dir(d.ws().join("empty")).unwrap();
    let o = d.run(r#"rmdir("empty")"#);
    assert_eq!(error_json(&o)["error"]["code"], "E_NEEDS_APPROVAL");
}

#[test]
fn more_steering_names_are_refused() {
    let d = Dirs::new("steer");
    for name in [
        "KUBECTL_EXTERNAL_DIFF",
        "KUBE_EDITOR",
        "KUBECTL_PLUGINS_PATH",
        "EXINIT",
        "VIMINIT",
        "HGRCPATH",
        "GCC_EXEC_PREFIX",
        "CPP",
        "LD",
        "LIBRARY_PATH",
        "CPATH",
        "LESSKEY",
        "NETRC",
        "MANOPT",
        "SYSTEMD_PAGER",
        "OLLAMA_HOST",
    ] {
        let o = d.run(&format!(r#"set_env("{name}", "x")"#));
        assert_eq!(error_json(&o)["error"]["code"], "E_POLICY_DENY", "{name}");
    }
    let o = d.run(r#"set_env("MY_APP_FLAG", "x")"#);
    assert!(!err(&o).contains("E_POLICY_DENY"), "{}", err(&o));
}

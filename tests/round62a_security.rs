//! Round 62a: CSV and TSV text with secret columns, the text form of the short
//! secret names, x-amz-signature and bare Digest credentials, look-alike and
//! leet spellings of secret names, scp and net_connect to metadata hosts in
//! numeric spellings, more steering environment names, the false positives of
//! the round 61 generic rules, `whoami` read from the OS, the audit entry of
//! `set_env`, and `rmdir` of an own temp directory. Every secret here is fake.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("round62a_security")
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

fn code_of(o: &Output) -> String {
    let e = err(o);
    let line = e
        .lines()
        .find(|l| l.starts_with('{'))
        .unwrap_or_else(|| panic!("no JSON error in: {e}"));
    let v: serde_json::Value =
        serde_json::from_str(line).unwrap_or_else(|x| panic!("not JSON ({x}): {line}"));
    v["error"]["code"].as_str().unwrap_or_default().to_string()
}

// ---------------------------------------------------------------- M1: CSV

#[test]
fn to_csv_output_hides_a_password_column() {
    let d = Dirs::new("tocsv");
    let o = d.run(r#"to_csv([{password:"hunter2xx",user:"u"}])"#);
    let text = out(&o);
    assert!(!text.contains("hunter2xx"), "{text}{}", err(&o));
    assert!(text.contains("password,user"), "{text}");
    assert!(text.contains("[REDACTED],u"), "{text}");
}

#[test]
fn cat_of_a_csv_file_hides_secret_columns_and_keeps_the_shape() {
    let d = Dirs::new("catcsv");
    std::fs::write(
        d.ws().join("c.csv"),
        "id,api_token,note,pin\n1,abcdefghijkl,\"hello, world\",4821\n2,\"zzzzzzzz9999\",plain,9999\n3,,empty,\n",
    )
    .unwrap();
    let o = d.run(r#"cat("c.csv")"#);
    let text = out(&o);
    for secret in ["abcdefghijkl", "zzzzzzzz9999", "4821", "9999"] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    assert!(
        text.contains("1,[REDACTED],\"hello, world\",[REDACTED]"),
        "{text}"
    );
    assert!(text.contains("2,\"[REDACTED]\",plain,[REDACTED]"), "{text}");
    assert!(text.contains("3,,empty,"), "{text}");
    // open() already redacted the same file.
    let o = d.run(r#"open("c.csv")"#);
    assert!(!out(&o).contains("abcdefghijkl"), "{}", out(&o));
}

#[test]
fn a_tsv_with_a_secret_column_is_redacted() {
    let d = Dirs::new("tsv");
    std::fs::write(d.ws().join("t.tsv"), "user\tpassword\nu\thunter2xx\n").unwrap();
    let o = d.run(r#"cat("t.tsv")"#);
    assert!(!out(&o).contains("hunter2xx"), "{}", out(&o));
    assert!(out(&o).contains("u\t[REDACTED]"), "{}", out(&o));
}

#[test]
fn csv_rule_is_conservative() {
    let d = Dirs::new("csvcons");
    // A field-count mismatch, a lone header, a header with no secret name, and
    // a count column are all left alone.
    for (name, text) in [
        ("a.csv", "password,user\nhunter2,u,extra\n"),
        ("b.csv", "password,user\n"),
        ("c.csv", "name,city\nann,paris\n"),
        ("d.csv", "session_count,user\n1234567,u\n"),
    ] {
        std::fs::write(d.ws().join(name), text).unwrap();
        let o = d.run(&format!(r#"cat("{name}")"#));
        let got = out(&o);
        assert_eq!(got.trim_end(), text.trim_end(), "{name}: {got}");
    }
}

// ---------------------------------------------------------- M2: text names

#[test]
fn short_secret_names_are_redacted_in_text() {
    let d = Dirs::new("textnames");
    let o = d.run(
        r#""session: abc123def456\njwt=eyJhbGciOiJIUzI1NiJ9\npw=hunter2\notp: 123456\npin: 4821\nsession_id=sid123456""#,
    );
    let text = out(&o);
    for secret in [
        "abc123def456",
        "eyJhbGciOiJIUzI1NiJ9",
        "hunter2",
        "123456",
        "4821",
        "sid123456",
    ] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    assert_eq!(text.matches("[REDACTED]").count(), 6, "{text}");
}

#[test]
fn a_yaml_file_read_with_cat_hides_session() {
    let d = Dirs::new("yaml");
    std::fs::write(
        d.ws().join("y.yaml"),
        "session: zzz999\npassword: abcdefgh\nport: 8080\n",
    )
    .unwrap();
    let o = d.run(r#"cat("y.yaml")"#);
    let text = out(&o);
    assert!(
        !text.contains("zzz999") && !text.contains("abcdefgh"),
        "{text}"
    );
    assert!(text.contains("port: 8080"), "{text}");
}

#[test]
fn text_rule_keeps_prose_counts_and_expiry_fields() {
    let d = Dirs::new("textfp");
    let o = d.run(
        r#""jwt_expiry_seconds: 3600\nsession: active\nauth: required\nmax_tokens: 100000\nsession_count: 12345678\npinned: 1234567""#,
    );
    let text = out(&o);
    for keep in [
        "jwt_expiry_seconds: 3600",
        "session: active",
        "auth: required",
        "max_tokens: 100000",
        "session_count: 12345678",
        "pinned: 1234567",
    ] {
        assert!(text.contains(keep), "{keep} was redacted: {text}");
    }
}

// ------------------------------------------------------------- M3: amz, digest

#[test]
fn x_amz_signature_is_redacted_as_key_header_and_pair() {
    let d = Dirs::new("amz");
    let sig = "0123456789abcdef0123456789abcdef0123456789abcdef";
    let o = d.run(&format!(
        r#"[{{"x-amz-signature":"{sig}"}}, "x-amz-signature: {sig}", "X-Amz-Signature={sig}", {{"x-amz-security-token":"{sig}", "x-amz-credential":"{sig}"}}]"#
    ));
    let text = out(&o);
    assert!(!text.contains(sig), "{text}");
}

#[test]
fn a_bare_digest_credential_list_is_redacted() {
    let d = Dirs::new("digest");
    let o = d.run(
        r#""Digest username=\"u\", realm=\"r\", response=\"0123456789abcdef0123456789abcdef\", cnonce=\"abcdef1234567890\"\nresponse=\"fedcba9876543210fedcba9876543210\"""#,
    );
    let text = out(&o);
    for secret in [
        "0123456789abcdef0123456789abcdef",
        "abcdef1234567890",
        "fedcba9876543210fedcba9876543210",
    ] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    assert!(
        text.contains("username=\"u\"") && text.contains("realm=\"r\""),
        "{text}"
    );
}

// ------------------------------------------------------------- M4: evasion

#[test]
fn look_alike_and_leet_key_spellings_are_redacted() {
    let d = Dirs::new("evasion");
    let o = d.run(
        "{\"p\u{0430}ssword\":\"v1\u{0430}lue111\",\"\u{ff50}\u{ff41}\u{ff53}\u{ff53}\u{ff57}\u{ff4f}\u{ff52}\u{ff44}\":\"v2value222\",\"passw0rd\":\"v3value333\",\"s3ssion\":\"v4value444\",\"p@ssword\":\"v5value555\",\"\u{03bf}tp\":\"v6value666\"}",
    );
    let text = out(&o);
    for n in 1..=6 {
        assert!(!text.contains(&format!("value{n}{n}{n}")), "{text}");
    }
    assert!(!text.contains("v1\u{0430}lue111"), "{text}");
}

#[test]
fn ordinary_digit_names_are_not_redacted() {
    let d = Dirs::new("digits");
    let o = d.run(r#"{"user1":"aaaaaaaa","s3_bucket":"bbbbbbbb","pinned":"cccccccc","h264_codec":"dddddddd"}"#);
    let text = out(&o);
    for v in ["aaaaaaaa", "bbbbbbbb", "cccccccc", "dddddddd"] {
        assert!(text.contains(v), "{v} was redacted: {text}");
    }
}

// ------------------------------------------------------------- M5: scp

const METADATA_FORMS: &[&str] = &[
    "0xa9fea9fe",
    "0xA9.254.169.254",
    "0251.0376.0251.0376",
    "2852039166",
    "169.254.169.254.",
    "169.254.169.254",
    "100.100.100.200",
    "100.100.100.200.",
    "[fd00:ec2::254]",
];

#[test]
fn scp_upload_and_download_deny_every_metadata_spelling() {
    let d = Dirs::new("scp");
    std::fs::write(d.ws().join("c.csv"), "a,b\n1,2\n").unwrap();
    for host in METADATA_FORMS {
        let o = d.run(&format!(r#"scp_upload("c.csv", "u@{host}:/x")"#));
        assert_eq!(code_of(&o), "E_POLICY_DENY", "upload {host}: {}", err(&o));
        let o = d.run(&format!(r#"scp_download("u@{host}:/x", "o.csv")"#));
        assert_eq!(code_of(&o), "E_POLICY_DENY", "download {host}: {}", err(&o));
    }
}

#[test]
fn net_connect_refuses_every_metadata_spelling() {
    let d = Dirs::new("netconn");
    for host in METADATA_FORMS {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let o = d.run(&format!(r#"net_connect("{host}", 80)"#));
        assert_eq!(code_of(&o), "E_POLICY_DENY", "{host}: {}", err(&o));
    }
}

#[test]
fn scp_to_an_ordinary_host_still_needs_approval() {
    let d = Dirs::new("scpok");
    std::fs::write(d.ws().join("c.csv"), "a,b\n1,2\n").unwrap();
    let o = d.run(r#"scp_upload("c.csv", "u@example.org:/x")"#);
    assert_eq!(code_of(&o), "E_NEEDS_APPROVAL", "{}", err(&o));
}

#[test]
fn human_mode_does_not_refuse_the_metadata_policy() {
    // The refusal is an agent-mode policy; a person at a prompt is not stopped
    // by it (the call then fails for another reason, or not at all).
    let d = Dirs::new("scphuman");
    std::fs::write(d.ws().join("c.csv"), "a,b\n1,2\n").unwrap();
    let o =
        d.run_human(r#"try { scp_upload("c.csv", "u@0xa9fea9fe:/x") } catch e { e.error.code }"#);
    assert!(!out(&o).contains("E_POLICY_DENY"), "{}{}", out(&o), err(&o));
}

// ------------------------------------------------------------- M6: env names

#[test]
fn more_steering_names_are_refused() {
    let d = Dirs::new("steer");
    for name in [
        "COREHOST_TRACEFILE",
        "COREHOST_TRACE",
        "ZSH_ENV",
        "BASH_XTRACEFD",
        "GZIP",
        "XZ_OPT",
        "XZ_DEFAULTS",
        "ZIPOPT",
        "UNZIP",
        "BZIP2",
        "TEXMFCNF",
        "MAGICK_CONFIGURE_PATH",
        "MAGICK_HOME",
        "FONTCONFIG_FILE",
        "FONTCONFIG_PATH",
        "DOCKER_CONTEXT",
        "TZDIR",
        "MSYS",
        "CYGWIN",
        "SystemDrive",
        "ProgramFiles",
        "ALLUSERSPROFILE",
        "LUA_INIT_5_4",
        "MANPATH",
        "GOBIN",
        "docker_context",
    ] {
        let o = d.run(&format!(r#"set_env("{name}", "x")"#));
        assert_eq!(code_of(&o), "E_POLICY_DENY", "{name}: {}", err(&o));
    }
}

#[test]
fn tz_and_term_values_that_name_a_file_are_refused() {
    let d = Dirs::new("tzterm");
    for code in [
        r#"set_env("TZ", ":/tmp/zone")"#,
        r#"set_env("TZ", "/tmp/zone")"#,
        r#"set_env("TZ", "../zone")"#,
        r#"set_env("TERM", "../x")"#,
        r#"set_env("TERM", "a/b")"#,
        r#"env_set("TZ", ":/tmp/zone")"#,
    ] {
        let o = d.run(code);
        assert_eq!(code_of(&o), "E_POLICY_DENY", "{code}: {}", err(&o));
    }
    for code in [
        r#"set_env("TZ", "America/New_York")"#,
        r#"set_env("TZ", "UTC")"#,
        r#"set_env("TERM", "xterm-256color")"#,
        r#"set_env("LC_ALL", "C")"#,
    ] {
        let o = d.run(code);
        assert!(o.status.success(), "{code}: {}", err(&o));
    }
}

// ------------------------------------------------------------- Low

#[test]
fn ordinary_names_the_generic_rules_blocked_are_allowed() {
    let d = Dirs::new("fp");
    for name in [
        "NODE_ENV", "LAST_RC", "EXIT_RC", "EXITRC", "APP_ENV", "RET_RC",
    ] {
        let o = d.run(&format!(r#"set_env("{name}", "x")"#));
        assert!(o.status.success(), "{name}: {}", err(&o));
    }
    // Steering run-control and option names stay refused.
    for name in [
        "SOMETOOL_RC",
        "MY_RC",
        "NPMRC",
        "APP_ADDOPTS",
        "NPM_CONFIG_REGISTRY",
    ] {
        let o = d.run(&format!(r#"set_env("{name}", "x")"#));
        assert_eq!(code_of(&o), "E_POLICY_DENY", "{name}: {}", err(&o));
    }
}

#[test]
fn the_refusal_for_npm_config_says_why_and_who_may_set_it() {
    let d = Dirs::new("npmmsg");
    let o = d.run(r#"set_env("NPM_CONFIG_REGISTRY", "http://x")"#);
    let e = err(&o);
    assert!(e.contains("NPM_CONFIG_*"), "{e}");
    assert!(e.contains("outside the agent"), "{e}");
}

#[cfg(windows)]
#[test]
fn whoami_ignores_an_agent_set_username() {
    let d = Dirs::new("whoami");
    let real = out(&d.run("whoami()"));
    let o = d.run(r#"set_env("USERNAME", "root"); whoami()"#);
    assert_eq!(out(&o).trim(), real.trim(), "{}", err(&o));
    assert!(!out(&o).contains("root") || real.contains("root"));
}

#[test]
fn the_audit_entry_for_set_env_names_the_variable_not_the_value() {
    let d = Dirs::new("audit");
    let o = d.run(r#"set_env("R62_AUDIT_NAME", "r62-audit-value-xyz")"#);
    assert!(o.status.success(), "{}", err(&o));
    let log = d.ws().join(".ironshell").join("audit.log");
    let text = std::fs::read_to_string(&log).expect("the audit log exists");
    assert!(text.contains("R62_AUDIT_NAME"), "{text}");
    assert!(!text.contains("r62-audit-value-xyz"), "{text}");
    assert!(!text.contains("<2 arguments>"), "{text}");
}

#[test]
fn rmdir_of_an_own_tempdir_needs_no_approval_with_or_without_recursion() {
    let d = Dirs::new("rmdir");
    let o = d.run("d = fs_tempdir(); rmdir(d)");
    assert!(o.status.success(), "{}", err(&o));
    let o = d.run("d = fs_tempdir(); rmdir(d, true)");
    assert!(o.status.success(), "{}", err(&o));
    // A directory that is not an own tempdir still needs approval.
    std::fs::create_dir(d.ws().join("sub")).unwrap();
    let o = d.run(r#"rmdir("sub")"#);
    assert_eq!(code_of(&o), "E_NEEDS_APPROVAL", "{}", err(&o));
    assert!(d.ws().join("sub").exists());
}

//! Round 63a: a private key is not read in agent mode whatever its name, a key
//! split into lines, colon-prefixed `key=value` text, name/value pair records,
//! CSV lines, `LANG`/`LC_*` path values, `session:` text, more steering names,
//! `pwd=` paths, `rmdir` of an own temp directory with a trailing separator.
//! Every key and secret here is fake, and the key text is built at run time.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("round63a_security")
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
            .env_remove("IRONSHELL_SECRETS")
            .env_remove("IRONSHELL_POLICY")
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

    fn run_b(&self, script: &str) -> Output {
        self.cmd(&["-b", "-c", script], true)
            .output()
            .expect("run ish")
    }

    fn run_human(&self, code: &str) -> Output {
        self.cmd(&["-c", code], false).output().expect("run ish")
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.ws().join(name), text).unwrap();
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

/// The marker lines of a key block, built from parts so no source line of this
/// file looks like a key.
fn begin(kind: &str) -> String {
    ["-----BEGIN", kind, "KEY-----"].join(" ")
}

fn end(kind: &str) -> String {
    ["-----END", kind, "KEY-----"].join(" ")
}

fn key_text(kind: &str) -> String {
    format!(
        "{}\nR63BODYLINEONE\nR63BODYLINETWO\n{}\n",
        begin(kind),
        end(kind)
    )
}

const BODY: &str = "R63BODYLINE";

#[test]
fn a_key_under_any_name_is_refused_by_every_read_builtin() {
    let d = Dirs::new("readers");
    d.write("key.txt", &key_text("PRIVATE"));
    d.write("deploy", &key_text("OPENSSH PRIVATE"));
    d.write("id_rsa.bak", &key_text("RSA PRIVATE"));
    for file in ["key.txt", "deploy", "id_rsa.bak"] {
        for code in [
            format!(r#"cat("{file}")"#),
            format!(r#"cat("{file}") | lines"#),
            format!(r#"file.read("{file}")"#),
            format!(r#"open("{file}")"#),
            format!(r#"head("{file}")"#),
            format!(r#"tail("{file}")"#),
            format!(r#"grep("{BODY}", "{file}")"#),
        ] {
            let o = d.run(&code);
            assert!(!out(&o).contains(BODY), "{code}: {}", out(&o));
            assert_eq!(code_of(&o), "E_POLICY_DENY", "{code}: {}", err(&o));
        }
    }
    // A glob that reaches the key is refused as well.
    let o = d.run(r#"cat("*")"#);
    assert!(!out(&o).contains(BODY), "{}", out(&o));
}

#[test]
fn other_key_formats_are_judged_by_content() {
    let d = Dirs::new("formats");
    d.write("a.txt", &key_text("EC PRIVATE"));
    d.write("b.txt", &key_text("ENCRYPTED PRIVATE"));
    d.write(
        "c.txt",
        &format!("{} PGP PRIVATE KEY BLOCK-----\n{BODY}X\n", "-----BEGIN"),
    );
    d.write(
        "d.txt",
        &format!("{}{}: ssh-ed25519\n{BODY}Y\n", "PuTTY-User-", "Key-File-3"),
    );
    for f in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        let o = d.run(&format!(r#"cat("{f}")"#));
        assert_eq!(code_of(&o), "E_POLICY_DENY", "{f}: {}", err(&o));
    }
}

#[test]
fn a_certificate_and_a_public_key_are_not_private_keys() {
    let d = Dirs::new("certs");
    d.write(
        "k.crt",
        &format!(
            "{}\nR63CERTBODY\n{}\n",
            begin("CERTIFICATE").replace(" KEY", ""),
            "-----END CERTIFICATE-----"
        ),
    );
    d.write("id_rsa.pub", "ssh-rsa R63PUBLICBODY me@host\n");
    let o = d.run(r#"cat("k.crt")"#);
    assert!(out(&o).contains("R63CERTBODY"), "{}{}", out(&o), err(&o));
    let o = d.run(r#"cat("id_rsa.pub")"#);
    assert!(out(&o).contains("R63PUBLICBODY"), "{}{}", out(&o), err(&o));
}

#[test]
fn source_that_mentions_the_marker_is_not_a_key() {
    let d = Dirs::new("source");
    d.write(
        "a.rs",
        &format!(
            "let re = r\"{}[A-Z ]*PRIVATE KEY\";\nlet r63ok = 1;\n",
            "-----BEGIN "
        ),
    );
    let o = d.run(r#"cat("a.rs")"#);
    assert!(out(&o).contains("r63ok"), "{}{}", out(&o), err(&o));
}

#[test]
fn metadata_digest_and_count_of_a_key_are_allowed() {
    let d = Dirs::new("meta");
    d.write("key.txt", &key_text("PRIVATE"));
    for code in [
        r#"fs_stat("key.txt").size"#,
        r#"crypto_hash_file("key.txt")"#,
        r#"wc("key.txt")"#,
        r#"ls(".") | map(.name)"#,
    ] {
        let o = d.run(code);
        assert!(o.status.success(), "{code}: {}", err(&o));
        assert!(!out(&o).contains(BODY), "{code}");
    }
}

#[test]
fn search_code_skips_a_key_file_and_still_searches_the_rest() {
    let d = Dirs::new("search");
    d.write("key.txt", &key_text("PRIVATE"));
    d.write("notes.txt", "R63BODYLINE in notes\n");
    let o = d.run(r#"search_code("R63BODYLINE")"#);
    let text = out(&o);
    assert!(o.status.success(), "{}", err(&o));
    assert!(text.contains("notes.txt"), "{text}");
    assert!(!text.contains("key.txt"), "{text}");
    assert!(!text.contains("R63BODYLINEONE"), "{text}");
}

#[test]
fn human_mode_and_the_secrets_switch_still_read_a_key() {
    let d = Dirs::new("human");
    d.write("key.txt", &key_text("PRIVATE"));
    let o = d.run_human(r#"cat("key.txt")"#);
    assert!(out(&o).contains(BODY), "{}{}", out(&o), err(&o));
    let o = d
        .cmd(&["-c", r#"cat("key.txt")"#], true)
        .env("IRONSHELL_SECRETS", "allow")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", err(&o));
}

#[test]
fn ish_b_tools_refuse_a_key_but_wc_counts_it() {
    let d = Dirs::new("posix");
    d.write("key.txt", &key_text("PRIVATE"));
    for script in [
        "cat key.txt",
        "sed -n 2,3p key.txt",
        "grep R63BODY key.txt",
        "head -n 3 key.txt",
        "tail -n 3 key.txt",
        "cut -c1-5 key.txt",
        "sort key.txt",
        "uniq key.txt",
        "cat < key.txt",
        "grep -r R63BODY .",
    ] {
        let o = d.run_b(script);
        assert!(!out(&o).contains(BODY), "{script}: {}{}", out(&o), err(&o));
    }
    let o = d.run_b("cat key.txt");
    assert!(err(&o).contains("E_POLICY_DENY"), "{}", err(&o));
    let o = d.run_b("wc -l key.txt");
    assert!(out(&o).contains('4'), "{}{}", out(&o), err(&o));
}

#[test]
fn key_file_names_are_blocked_by_name_too() {
    let d = Dirs::new("names");
    for name in [
        "id_rsa",
        "id_dsa",
        "id_ecdsa",
        "id_ed25519",
        "k.p8",
        "k.ppk",
        "k.p12",
        "k.pfx",
    ] {
        d.write(name, "not a key at all\n");
        let o = d.run(&format!(r#"cat("{name}")"#));
        assert_eq!(code_of(&o), "E_POLICY_DENY", "{name}: {}", err(&o));
    }
}

#[test]
fn key_lines_in_an_array_are_redacted_across_elements() {
    let d = Dirs::new("lines");
    let code = format!(
        r#"["before", "{}", "{BODY}A", "{BODY}B", "{}", "after"]"#,
        begin("PRIVATE"),
        end("PRIVATE")
    );
    let o = d.run(&code);
    let text = out(&o);
    assert!(!text.contains(BODY), "{text}");
    assert!(text.contains("before") && text.contains("after"), "{text}");
}

#[test]
fn colon_prefixed_assignments_are_redacted() {
    let d = Dirs::new("colon");
    d.write(".env", "A=1\nDB_PASSWORD=hunter2hunter2\n");
    d.write("notes.txt", "see hunter2hunter2 here\n");
    let o = d.run(r#"search_code("hunter2hunter2")"#);
    let text = out(&o);
    assert!(text.contains("DB_PASSWORD=[REDACTED]"), "{text}");
    assert!(!text.contains("hunter2hunter2\""), "{text}");
    for s in [
        "x:DB_PASSWORD=hunter2hunter2",
        "./.env:2:DB_PASSWORD=hunter2hunter2",
        "f.py:3:token=abcdef123456",
        "p:password=hunter2hunter2",
        "a,DB_PASSWORD=hunter2hunter2",
        "(api_key=abcdef123456)",
    ] {
        let o = d.run(&format!(r#""{s}""#));
        let text = out(&o);
        assert!(text.contains("[REDACTED]"), "{s}: {text}");
        assert!(
            !text.contains("hunter2hunter2") && !text.contains("abcdef123456"),
            "{s}: {text}"
        );
    }
    let o = d.run_b("grep -n PASSWORD .env");
    assert!(out(&o).contains("2:DB_PASSWORD=[REDACTED]"), "{}", out(&o));
}

#[test]
fn colon_prefixed_text_that_is_no_secret_is_kept() {
    let d = Dirs::new("colonkeep");
    for s in [
        "src/a.rs:12:let key = 1;",
        "a.txt:3:status=running now",
        "x:count=1234567",
    ] {
        let o = d.run(&format!(r#""{s}""#));
        assert!(!out(&o).contains("[REDACTED]"), "{s}: {}", out(&o));
    }
}

#[test]
fn name_value_pair_records_hide_the_value() {
    let d = Dirs::new("pairs");
    for code in [
        r#"[{name:"DB_PASSWORD", value:"Zk3p9Qw2LmXY"}]"#,
        r#"[{Name:"MYSQL_ROOT_PASSWORD", Value:"Zk3p9Qw2LmXY"}]"#,
        r#"[{key:"api_key", value:"Zk3p9Qw2LmXY"}]"#,
        r#"{name:"DB_PASSWORD", val:"Zk3p9Qw2LmXY"}"#,
        r#""{\"name\":\"DB_PASSWORD\",\"value\":\"Zk3p9Qw2LmXY\"}""#,
        r#""{\"value\":\"Zk3p9Qw2LmXY\",\"name\":\"DB_PASSWORD\"}""#,
        r#"to_json([{name:"DB_PASSWORD", value:"Zk3p9Qw2LmXY"}])"#,
    ] {
        let o = d.run(code);
        assert!(!out(&o).contains("Zk3p9Qw2LmXY"), "{code}: {}", out(&o));
        assert!(out(&o).contains("REDACTED"), "{code}: {}", out(&o));
    }
    // A pair whose name is no secret keeps its value.
    let o = d.run(r#"[{name:"HOME_DIR", value:"keepme"}]"#);
    assert!(out(&o).contains("keepme"), "{}", out(&o));
    let o = d.run(r#"[{name:"max_tokens", value:"4096"}]"#);
    assert!(out(&o).contains("4096"), "{}", out(&o));
}

#[test]
fn csv_lines_are_redacted_when_the_header_is_among_them() {
    let d = Dirs::new("csvlines");
    d.write("c.csv", "user,password\nbob,hunter2xyz\nann,swordfish99\n");
    let o = d.run(r#"cat("c.csv") | lines"#);
    let text = out(&o);
    assert!(
        !text.contains("hunter2xyz") && !text.contains("swordfish99"),
        "{text}"
    );
    assert!(
        text.contains("bob") && text.contains("user,password"),
        "{text}"
    );
}

#[test]
fn locale_variables_refuse_path_values() {
    let d = Dirs::new("locale");
    for code in [
        r#"set_env("LC_ALL", "/tmp/evil")"#,
        r#"set_env("LANG", "../x")"#,
        r#"set_env("LANGUAGE", "en:/tmp/x")"#,
        r#"set_env("LC_CTYPE", "C:/x")"#,
        r#"set_env("LC_MESSAGES", "a/b")"#,
        r#"env_set("LC_ALL", "/tmp/evil")"#,
    ] {
        let o = d.run(code);
        assert_eq!(code_of(&o), "E_POLICY_DENY", "{code}: {}", err(&o));
    }
    for code in [
        r#"set_env("LANG", "en_US.UTF-8")"#,
        r#"set_env("LC_ALL", "C")"#,
        r#"set_env("LANGUAGE", "en:fr")"#,
    ] {
        let o = d.run(code);
        assert!(o.status.success(), "{code}: {}", err(&o));
    }
}

#[test]
fn session_text_is_redacted_unless_it_is_a_state_word() {
    let d = Dirs::new("session");
    for s in [
        "session: SESSLEAKxyz",
        "session: abcdefgh",
        "session = qwertyuiop",
    ] {
        let o = d.run(&format!(r#""{s}""#));
        assert!(out(&o).contains("[REDACTED]"), "{s}: {}", out(&o));
    }
    for s in [
        "session: active",
        "session: expired",
        "session: closed",
        "session: none",
    ] {
        let o = d.run(&format!(r#""{s}""#));
        assert!(!out(&o).contains("[REDACTED]"), "{s}: {}", out(&o));
    }
}

#[test]
fn a_path_after_session_is_not_a_secret() {
    let d = Dirs::new("sessionpath");
    let o = d.run(r#"session_export("../zz")"#);
    assert_eq!(code_of(&o), "E_OUTSIDE_WORKSPACE", "{}", err(&o));
    assert!(
        !err(&o).contains("session_export: [REDACTED]"),
        "{}",
        err(&o)
    );
}

#[test]
fn more_steering_names_are_refused() {
    let d = Dirs::new("steer");
    for name in [
        "GCM_TRACE",
        "COMPOSE_FILE",
        "COMPOSE_PROJECT_NAME",
        "GITHUB_API_URL",
        "PRE_COMMIT_HOME",
        "TRIVY_CACHE_DIR",
        "PACKER_PLUGIN_PATH",
        "FCEDIT",
        "MOZ_PLUGIN_PATH",
        "MY_TOOL_PLUGIN_PATH",
        "CUSTOM_API_URL",
    ] {
        let o = d.run(&format!(r#"set_env("{name}", "x")"#));
        assert_eq!(code_of(&o), "E_POLICY_DENY", "{name}: {}", err(&o));
    }
    let o = d.run(r#"set_env("MY_FLAG", "x")"#);
    assert!(o.status.success(), "{}", err(&o));
}

#[test]
fn pwd_with_a_path_is_kept_and_pwd_with_a_password_is_not() {
    let d = Dirs::new("pwd");
    let o = d.run_b("echo 'pwd=/x'; echo pwd=/home/me/work; echo pwd=C:/a/b; echo OLDPWD=../up");
    let text = out(&o);
    assert!(!text.contains("REDACTED"), "{text}");
    assert!(
        text.contains("pwd=/home/me/work") && text.contains("pwd=C:/a/b"),
        "{text}"
    );
    let o = d.run_b("echo pwd=hunter2hunter");
    assert!(out(&o).contains("pwd=[REDACTED]"), "{}", out(&o));
    assert!(!out(&o).contains("hunter2hunter"), "{}", out(&o));
}

#[test]
fn rmdir_of_an_own_tempdir_with_a_trailing_separator_needs_no_approval() {
    let d = Dirs::new("rmdirslash");
    let o = d.run(r#"d = fs_tempdir(); rmdir(d + "/", true)"#);
    assert!(o.status.success(), "{}{}", out(&o), err(&o));
    // Another directory still needs approval, with or without the separator.
    std::fs::create_dir(d.ws().join("sub")).unwrap();
    let o = d.run(r#"rmdir("sub/", true)"#);
    assert!(
        matches!(code_of(&o).as_str(), "E_NEEDS_APPROVAL" | "E_BAD_ARG"),
        "{}",
        err(&o)
    );
    assert!(d.ws().join("sub").exists());
}

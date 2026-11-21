//! Round 66a: a private key written with escapes or split into joined string
//! literals is refused; secret literals in every part of a code value are
//! redacted; `--timeout` ends `ish -b` loops and YAML parses; `ish -b` strings
//! stop at the 8 MB cap; the journal and the audit log have per-run budgets; an
//! approval token binds the content of a `<` source; the audit entry of an
//! exec lists its argv and redirections.
//! Every key and secret here is fake, and the key text is built at run time.
//! Each run that could loop has `--timeout` and an outer wait that kills it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use iron_shell::safety::redact_str;

struct Dirs {
    root: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("round66a_security")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ws")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Dirs { root }
    }

    fn ws(&self) -> PathBuf {
        self.root.join("ws")
    }

    fn cmd(&self, args: &[&str], approve: Option<&str>) -> Command {
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
            .env_remove("IRONSHELL_JOURNAL_MAX_ENTRIES")
            .env_remove("IRONSHELL_AUDIT_MAX_ENTRIES")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", self.ws());
        if let Some(t) = approve {
            cmd.env("IRONSHELL_APPROVE", t);
        }
        cmd
    }

    fn run(&self, code: &str) -> Output {
        self.cmd(&["-c", code], None).output().expect("run ish")
    }

    fn run_b(&self, script: &str, approve: Option<&str>) -> Output {
        self.cmd(&["-b", "-c", script], approve)
            .output()
            .expect("run ish")
    }

    /// Run with `--timeout`, and kill the child after 60 s if it ignores it.
    /// Returns the output, the wall time and whether the outer wait had to kill.
    fn run_bounded(&self, mut cmd: Command) -> (Output, Duration, bool) {
        let start = Instant::now();
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn ish");
        let mut killed = false;
        loop {
            match child.try_wait().expect("wait") {
                Some(_) => break,
                None if start.elapsed() > Duration::from_secs(60) => {
                    let _ = child.kill();
                    killed = true;
                    break;
                }
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        let o = child.wait_with_output().expect("output");
        (o, start.elapsed(), killed)
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

fn begin() -> String {
    ["-----BEGIN", "PRIVATE", "KEY-----"].join(" ")
}

fn end() -> String {
    ["-----END", "PRIVATE", "KEY-----"].join(" ")
}

const BODY: &str = "R66BODYLINEA1";

fn denied(o: &Output) -> bool {
    err(o).contains("E_POLICY_DENY") && err(o).contains("private key")
}

// ---- 1: escaped and joined keys ----

/// `s` with every `-` written as `\u002d`.
fn dashes_escaped(s: &str) -> String {
    s.replace('-', "\\u002d")
}

fn escaped_files() -> Vec<(&'static str, String)> {
    let (b, e) = (begin(), end());
    let b_u = b.replace("PRIVATE", "\\u0050RIVATE");
    vec![
        (
            "u000a.json",
            format!("{{\"k\":\"{b}\\u000a{BODY}\\u000a{e}\"}}"),
        ),
        (
            "u000d.json",
            format!("{{\"k\":\"{b}\\u000d\\u000a{BODY}\\u000d\\u000a{e}\"}}"),
        ),
        (
            "u000a_array.json",
            format!("{{\"k\":[\"{b}\\u000a\",\"{BODY}\\u000a\",\"{e}\\u000a\"]}}"),
        ),
        (
            "slash.json",
            format!("{{\"k\":[\"{b}\\/\",\"{BODY}\\/\",\"{e}\"]}}"),
        ),
        (
            "dash_after.json",
            format!("{{\"k\":[\"{b}\\u002d\",\"{BODY}\"]}}"),
        ),
        (
            "dashes.json",
            format!("{{\"k\":[\"{}\",\"{BODY}\"]}}", dashes_escaped(&b)),
        ),
        ("letter.json", format!("{{\"k\":[\"{b_u}\",\"{BODY}\"]}}")),
        (
            "double.json",
            format!("{{\"k\":\"{b}\\\\n{BODY}\\\\n{e}\"}}"),
        ),
        (
            "yaml_x20.yaml",
            format!("k: \"{}\\n{BODY}\"\n", b.replace(' ', "\\x20")),
        ),
        (
            "yaml_x0a.yaml",
            format!("k: \"{b}\\x0a{BODY}\\x0a{e}\"\n"),
        ),
        (
            "split.json",
            format!("{{\"k\":[\"-----BEGIN \", \"PRIVATE KEY-----\", \"{BODY}\"]}}"),
        ),
        (
            "split_lines.json",
            format!("{{\"k\":[\n  \"-----BEGIN\",\n  \" PRIVATE KEY-----\",\n  \"{BODY}\"\n]}}"),
        ),
        (
            "concat.py",
            format!("k = \"-----BEGIN PRIVATE \" + \"KEY-----\\n{BODY}\"\n"),
        ),
        (
            "adjacent.py",
            format!("k = \"{b}\" \"\\n\" \"{BODY}\"\n"),
        ),
        (
            "concat_lines.js",
            format!("const k = \"-----BEGIN PRIVATE \" +\n  \"KEY-----\\n\" +\n  \"{BODY}\";\n"),
        ),
        (
            "tab_openssh.json",
            format!(
                "{{\"k\":[\"-----BEGIN OPENSSH PRIVATE KEY-----\\t\",\"{BODY}\\t\",\"-----END OPENSSH PRIVATE KEY-----\"]}}"
            ),
        ),
    ]
}

#[test]
fn a_key_written_with_escapes_or_joined_literals_is_refused() {
    let d = Dirs::new("escaped");
    for (name, text) in escaped_files() {
        d.write(name, &text);
        for code in [
            format!(r#"cat("{name}")"#),
            format!(r#"cat("{name}") | regex_find("R66BODY[A-Z0-9]+")"#),
            format!(r#"file.read("{name}") | replace("BEGIN", "B")"#),
        ] {
            let o = d.run(&code);
            assert!(!out(&o).contains(BODY), "{name}: {code}: {}", out(&o));
            assert!(denied(&o), "{name}: {code}: {}{}", out(&o), err(&o));
        }
    }
}

#[test]
fn source_that_quotes_the_marker_is_still_not_a_key() {
    let d = Dirs::new("source");
    let (b, e) = (begin(), end());
    let files = [
        (
            "consts.py",
            format!("KEY_HEADER = \"{b}\"\nKEY_FOOTER = \"{e}\"\nNAME = 'x'\n"),
        ),
        (
            "check.py",
            format!("if \"{b}\" in text:\n    raise ValueError(\"no keys here\")\n"),
        ),
        (
            "check.rs",
            format!("assert!(s.contains(\"{b}\"), \"expected a key\");\n"),
        ),
        (
            "names.rs",
            format!("let v = vec![\"{b}\", \"Something here\"];\n"),
        ),
        ("two.rs", format!("let a = [\"{b}\", \"{e}\"];\n")),
        // The marker with an escaped quote, then prose.
        (
            "msg.py",
            format!("msg = \"refusing a key that starts with \\\"{b}\\\" here\"\n"),
        ),
        // The marker in a string joined to a name, then code.
        (
            "joined.py",
            format!("pem = \"{b}\\n\" + header_name + \"x\"\nprint(pem)\n"),
        ),
        // Escaped text that has a marker-like run but is not one.
        (
            "dashes.md",
            "a line of dashes -----\\u002d\\u002d-- is a rule, not a key\n".to_string(),
        ),
    ];
    for (name, text) in files {
        d.write(name, &text);
        let o = d.run(&format!(r#"cat("{name}")"#));
        assert!(!denied(&o), "{name}: {}{}", out(&o), err(&o));
    }
}

#[test]
fn an_escaped_key_across_a_chunk_boundary_is_found() {
    let d = Dirs::new("chunks");
    let (b, e) = (begin(), end());
    let key = format!("{{\"k\":\"{b}\\u000a{BODY}{BODY}{BODY}\\u000a{e}\"}}");
    // The scan reads 64 KiB at a time: put the marker on, before and after the
    // first boundary, and across the second.
    for pad in [0usize, 1, 7, 50, 200, 400, 1000, 1500] {
        for edge in [65536usize, 131072] {
            let lead = edge - pad;
            let name = format!("big-{edge}-{pad}.json");
            d.write(&name, &format!("{}{key}", " ".repeat(lead)));
            let o = d.run(&format!(
                r#"cat("{name}") | regex_find("R66BODY[A-Z0-9]+")"#
            ));
            assert!(
                denied(&o),
                "{name}: {}{}",
                out(&o),
                err(&o).chars().take(300).collect::<String>()
            );
        }
    }
}

// ---- 2: secrets in every part of a code value ----

fn redacts(text: &str, leak: &str) {
    let (r, changed) = redact_str(text);
    assert!(!r.contains(leak), "{text:?} -> {r:?}");
    assert!(changed, "{text:?} was not changed");
}

fn keeps(text: &str) {
    let (r, changed) = redact_str(text);
    assert_eq!(r, text, "{text:?}");
    assert!(!changed, "{text:?}");
}

#[test]
fn every_secret_literal_of_a_code_value_is_redacted() {
    for (text, leak) in [
        ("PASSWORD = '''Zk3p9Qw2LmXLEAK07'''", "LEAK07"),
        ("password = \"\"\"Zk3p9Qw2LmXLEAK08\"\"\"", "LEAK08"),
        (r#"password = ["Zk3p9Qw2LmXLEAK19"]"#, "LEAK19"),
        (
            r#"passwords = ["Zk3p9Qw2LmXLEAK20", "Yd8q7Rt1NvBLEAK21"]"#,
            "LEAK21",
        ),
        ("password = `${a}Zk3p9Qw2LmXLEAK15`", "LEAK15"),
        (
            r#"let password = format!("{}-Zk3p9Qw2LmXLEAK14", x);"#,
            "LEAK14",
        ),
        (r#"password = f"{user}-Zk3p9Qw2LmXLEAK22""#, "LEAK22"),
        (r#"api_key = "key-%s" % "Zk3p9Qw2LmXLEAK03""#, "LEAK03"),
        (r#"secret = "sk-" + "abcdefghijklmnopLEAK04""#, "LEAK04"),
        (r#"password = "Zk3p9Q" + "w2LmXLEAK05""#, "LEAK05"),
        (r#"password = "Zk3p9Q" + "w2LmX" + "LEAK06""#, "LEAK06"),
        (r#"token = prefix + "Zk3p9Qw2LmXLEAK09""#, "LEAK09"),
        (r#"{"password": "Zk3p9Qw2LmXLEAK10", "id": 1}"#, "LEAK10"),
    ] {
        redacts(text, leak);
    }
    // The code around the literals stays, and a concatenation is one literal.
    let (r, _) = redact_str(r#"password = "Zk3p9Q" + "w2LmXLEAK05" + suffix"#);
    assert!(r.starts_with("password = \"[REDACTED]\" + suffix"), "{r}");
    let (r, _) = redact_str(r#"password = ["Zk3p9Qw2LmXLEAK19"] # keep"#);
    assert_eq!(r, r#"password = ["[REDACTED]"] # keep"#);
}

#[test]
fn ordinary_code_is_still_readable() {
    for text in [
        "api_key = os.environ['API_KEY']",
        "password = input('Password: ')",
        "password = os.environ.get(\"DB_PASSWORD\", \"changeit_not_here\")",
        "secret = os.getenv(\"my_secret_name\", \"/etc/secret/path\")",
        "token = headers.get(\"Content-Type\")",
        "name = \"Zk3p9Qw2LmXLEAK\" + \"id\"",
        "title = \"the user's password\" + \" is hidden\"",
        "let id = format!(\"{}-{}\", a, b);",
        "tokens = [\"alpha\", \"beta\", \"gamma\"]",
        "secret = \"\" + name",
    ] {
        keeps(text);
    }
}

// ---- 3: --timeout and budgets ----

#[test]
fn a_posix_loop_stops_at_the_timeout() {
    let d = Dirs::new("loops");
    for script in [
        "while true; do echo aaaa >> g.txt; done",
        "i=0; while true; do i=$((i+1)); done",
        "while true; do :; done",
        "for i in 1 2 3 4 5 6 7 8 9; do while true; do :; done; done",
    ] {
        let mut cmd = d.cmd(&["--timeout", "2", "-b", "-c", script], None);
        cmd.env("IRONSHELL_JOURNAL_MAX_ENTRIES", "0");
        let (o, took, killed) = d.run_bounded(cmd);
        assert!(!killed, "{script}: ran past 60 s");
        assert!(took < Duration::from_secs(30), "{script}: {took:?}");
        assert_eq!(o.status.code(), Some(69), "{script}: {}", err(&o));
        assert!(
            err(&o).contains("E_BUDGET_EXCEEDED"),
            "{script}: {}",
            err(&o)
        );
    }
}

#[test]
fn a_posix_string_stops_at_the_string_cap() {
    let d = Dirs::new("strings");
    let double = |n: usize| {
        let seq: Vec<String> = (1..=n).map(|i| i.to_string()).collect();
        format!(
            "x=aaaaaaaaaaaaaaaa; for i in {}; do x=$x$x; done; echo ${{#x}}",
            seq.join(" ")
        )
    };
    // 16 bytes doubled 18 times is 4 MiB: under the cap.
    let mut cmd = d.cmd(&["--timeout", "30", "-b", "-c", &double(18)], None);
    cmd.env("IRONSHELL_JOURNAL_MAX_ENTRIES", "0");
    let (o, _, _) = d.run_bounded(cmd);
    assert_eq!(out(&o).trim(), "4194304", "{}", err(&o));
    // 25 times would be 512 MB: refused at 16 MiB, long before.
    let mut cmd = d.cmd(&["--timeout", "30", "-b", "-c", &double(25)], None);
    cmd.env("IRONSHELL_JOURNAL_MAX_ENTRIES", "0");
    let (o, took, killed) = d.run_bounded(cmd);
    assert!(!killed && took < Duration::from_secs(30), "{took:?}");
    assert_eq!(o.status.code(), Some(69), "{}{}", out(&o), err(&o));
    assert!(err(&o).contains("E_BUDGET_EXCEEDED"), "{}", err(&o));
    assert!(out(&o).trim().is_empty(), "{}", out(&o));
    // One word that expands past the cap is refused too.
    let mut cmd = d.cmd(
        &[
            "--timeout",
            "30",
            "-b",
            "-c",
            "x=aaaaaaaaaaaaaaaa; for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19; do x=$x$x; done; echo $x$x$x",
        ],
        None,
    );
    cmd.env("IRONSHELL_JOURNAL_MAX_ENTRIES", "0");
    let (o, _, _) = d.run_bounded(cmd);
    assert_eq!(o.status.code(), Some(69), "{}{}", out(&o), err(&o));
}

fn yaml_stream(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("---\na: {i}\n"));
    }
    s
}

#[test]
fn a_yaml_stream_parses_in_linear_time_and_is_capped() {
    let d = Dirs::new("yaml");
    d.write("many.yaml", &yaml_stream(20_000));
    let cmd = d.cmd(
        &[
            "--timeout",
            "30",
            "-c",
            r#"len(from_yaml_all(cat("many.yaml")))"#,
        ],
        None,
    );
    let (o, took, killed) = d.run_bounded(cmd);
    assert!(!killed, "ran past 60 s");
    assert_eq!(out(&o).trim(), "20000", "{}", err(&o));
    // 20,000 documents took 51 s before the fix; the bound is generous.
    assert!(took < Duration::from_secs(20), "{took:?}");
    let cmd = d.cmd(
        &["--timeout", "30", "-c", r#"len(open("many.yaml"))"#],
        None,
    );
    let (o, took, _) = d.run_bounded(cmd);
    assert_eq!(out(&o).trim(), "20000", "{}", err(&o));
    assert!(took < Duration::from_secs(20), "{took:?}");
    // Past 100,000 documents is E_BUDGET_EXCEEDED, and quickly.
    d.write("lots.yaml", &yaml_stream(110_000));
    let cmd = d.cmd(
        &[
            "--timeout",
            "30",
            "-c",
            r#"len(from_yaml_all(cat("lots.yaml")))"#,
        ],
        None,
    );
    let (o, took, killed) = d.run_bounded(cmd);
    assert!(!killed && took < Duration::from_secs(30), "{took:?}");
    assert!(err(&o).contains("E_BUDGET_EXCEEDED"), "{}", err(&o));
}

#[test]
fn error_line_numbers_of_a_yaml_stream_still_count_the_whole_stream() {
    let d = Dirs::new("yamllines");
    d.write("two.yaml", "a: 1\n---\nb: 2\nc: [1, 2\n");
    let o = d.run(r#"from_yaml_all(cat("two.yaml"))"#);
    assert!(err(&o).contains("line 4"), "{}", err(&o));
}

#[test]
fn the_journal_has_a_per_run_budget() {
    let d = Dirs::new("journal");
    let mut cmd = d.cmd(
        &[
            "--timeout",
            "20",
            "-b",
            "-c",
            "while true; do echo a >> g.txt; done",
        ],
        None,
    );
    cmd.env("IRONSHELL_JOURNAL_MAX_ENTRIES", "25");
    let (o, took, killed) = d.run_bounded(cmd);
    assert!(!killed && took < Duration::from_secs(30), "{took:?}");
    assert_eq!(o.status.code(), Some(69), "{}", err(&o));
    assert!(err(&o).contains("E_BUDGET_EXCEEDED"), "{}", err(&o));
    assert!(err(&o).contains("journal"), "{}", err(&o));
    let files = std::fs::read_dir(d.root.join("session"))
        .map(|rd| {
            rd.flatten()
                .map(|e| {
                    std::fs::read_dir(e.path())
                        .map(|r| r.flatten().count())
                        .unwrap_or(1)
                })
                .sum::<usize>()
        })
        .unwrap_or(0);
    assert!(files <= 40, "{files} files in the session directory");
}

#[test]
fn the_audit_log_has_a_per_run_budget_and_drops_nothing() {
    let d = Dirs::new("audit");
    let mut cmd = d.cmd(
        &[
            "--timeout",
            "20",
            "-b",
            "-c",
            "while true; do echo a >> g.txt; done",
        ],
        None,
    );
    cmd.env("IRONSHELL_JOURNAL_MAX_ENTRIES", "0")
        .env("IRONSHELL_AUDIT_MAX_ENTRIES", "30");
    let (o, took, killed) = d.run_bounded(cmd);
    assert!(!killed && took < Duration::from_secs(30), "{took:?}");
    assert_eq!(o.status.code(), Some(69), "{}", err(&o));
    assert!(err(&o).contains("audit"), "{}", err(&o));
    // The log stays a valid chain: nothing was dropped to stay under the limit.
    let log = d.ws().join(".ironshell").join("audit.log");
    let lines = std::fs::read_to_string(&log).unwrap().lines().count();
    assert!((30..200).contains(&lines), "{lines} audit lines");
    let o = d.run(r#"audit_verify()"#);
    assert!(!err(&o).contains("E_"), "{}{}", out(&o), err(&o));
}

// ---- 4: tokens and the audit entry of an exec ----

fn token(o: &Output) -> String {
    let text = format!("{}{}", out(o), err(o));
    let at = text
        .find("apv_")
        .unwrap_or_else(|| panic!("no token in {text}"));
    text[at..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

#[test]
fn an_approval_token_binds_the_content_of_a_redirect_source() {
    let d = Dirs::new("source_hash");
    d.write("f1.txt", "one\n");
    let first = token(&d.run_b("env cat < f1.txt", None));
    assert_eq!(first, token(&d.run_b("env cat < f1.txt", None)));
    d.write("f1.txt", "two\n");
    let second = token(&d.run_b("env cat < f1.txt", None));
    assert_ne!(first, second);
    // The first token no longer approves the changed input.
    let o = d.run_b("env cat < f1.txt", Some(&first));
    assert!(err(&o).contains("E_NEEDS_APPROVAL"), "{}", err(&o));
    // Putting the content back gives the first token again.
    d.write("f1.txt", "one\n");
    assert_eq!(first, token(&d.run_b("env cat < f1.txt", None)));
    // The token of a command with no `<` source is the one round 65 gave.
    assert_eq!(
        token(&d.run_b("env printf x", None)),
        "apv_e5f322f280e32aed"
    );
}

#[test]
fn the_audit_entry_of_an_exec_lists_its_argv_and_redirections_redacted() {
    let d = Dirs::new("audit_argv");
    d.write("in.txt", "x\n");
    let _ = d.run_b("env echo hello world < in.txt > out.txt", None);
    let _ = d.run_b("env echo password=hunter2LEAKXX", None);
    let log = std::fs::read_to_string(d.ws().join(".ironshell").join("audit.log")).unwrap();
    assert!(
        log.contains(r#""argv":["env","echo","hello","world"]"#),
        "{log}"
    );
    assert!(log.contains("in.txt"), "{log}");
    assert!(log.contains("redirections"), "{log}");
    assert!(log.contains("out.txt"), "{log}");
    assert!(!log.contains("hunter2LEAKXX"), "{log}");
    assert!(log.contains("password=[REDACTED]"), "{log}");
}

//! Round 64a: a private key is found anywhere in a file (past 8 KiB, split across
//! a read chunk, in the middle of a line, JSON-escaped); `ish -b` approval tokens
//! keep argv boundaries; secret text in YAML env lists, `$env:`, `ENV` and
//! `--password` flags is redacted; code assignments and ordinary data are not
//! garbled; `LOCALE_ARCHIVE` is a steering name.
//! Every key and secret here is fake, and the key text is built at run time.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use iron_shell::safety::redact_str;

struct Dirs {
    root: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("round64a_security")
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

const BODY: &str = "R64BODYLINE";

fn key_block() -> String {
    format!("{}\n{BODY}ONE\n{BODY}TWO\n{}\n", begin(), end())
}

fn denied(o: &Output) -> bool {
    err(o).contains("E_POLICY_DENY") && err(o).contains("private key")
}

// ---- H1: a key past the first 8 KiB, or across a read chunk ----

#[test]
fn a_key_after_a_long_prefix_is_refused() {
    let d = Dirs::new("long");
    // A certificate chain before the key is the real case: 70000 bytes of text.
    let pad = "a".repeat(69_999);
    d.write("bundle.txt", &format!("{pad}\n{}", key_block()));
    for code in [
        r#"cat("bundle.txt") | lines | slice(2, 3)"#,
        r#"file.read("bundle.txt")"#,
        r#"tail("bundle.txt")"#,
        r#"grep("R64BODY", "bundle.txt")"#,
    ] {
        let o = d.run(code);
        assert!(!out(&o).contains(BODY), "{code}: {}", out(&o));
        assert!(denied(&o), "{code}: {}", err(&o));
    }
    let o = d.run_b("tail -n 2 bundle.txt", None);
    assert!(!out(&o).contains(BODY), "{}", out(&o));
    assert!(err(&o).contains("private key"), "{}", err(&o));
}

#[test]
fn a_marker_split_across_the_read_chunk_is_found() {
    let d = Dirs::new("split");
    // The scan reads 64 KiB at a time; put the marker across that boundary.
    for offset in [65_536 - 5, 65_536 - 1, 65_536 - 11, 65_536 + 3] {
        let pad = "b".repeat(offset - 1);
        d.write("split.txt", &format!("{pad}\n{}", key_block()));
        let o = d.run(r#"cat("split.txt") | lines | slice(2, 3)"#);
        assert!(denied(&o), "offset {offset}: {}{}", out(&o), err(&o));
        // The same file with the marker's first dash cut is no key.
        d.write(
            "nokey.txt",
            &format!(
                "{pad}\n{}",
                key_block().replacen("-----BEGIN", "----BEGIN", 1)
            ),
        );
        let o = d.run(r#"cat("nokey.txt") | lines | len"#);
        assert!(!denied(&o), "offset {offset}: {}", err(&o));
    }
}

#[test]
fn search_code_and_grep_r_skip_a_key_past_8_kib() {
    let d = Dirs::new("search");
    let pad = "a".repeat(9_000);
    d.write("bundle.pem.txt", &format!("{pad}\n{}", key_block()));
    d.write("plain.txt", "R64BODYLINE in an ordinary file\n");
    let o = d.run(r#"search_code("R64BODY")"#);
    let text = out(&o);
    assert!(text.contains("plain.txt"), "{text}{}", err(&o));
    assert!(!text.contains("bundle.pem.txt"), "{text}");
    assert!(!text.contains("R64BODYLINEONE"), "{text}");
    let o = d.run_b("grep -r R64BODY .", None);
    assert!(out(&o).contains("plain.txt"), "{}{}", out(&o), err(&o));
    assert!(!out(&o).contains("bundle.pem.txt"), "{}", out(&o));
    assert!(!out(&o).contains("R64BODYLINEONE"), "{}", out(&o));
}

// ---- H2: a key that begins in the middle of a line ----

#[test]
fn a_service_account_json_key_is_refused() {
    let d = Dirs::new("sa");
    // JSON text writes the newlines as a backslash and n.
    let escaped = format!(
        "{{\"type\":\"service_account\",\"private_key\":\"{}\\n{BODY}AAAA1111\\n{BODY}BBBB2222\\n{}\\n\"}}",
        begin(),
        end()
    );
    d.write("sa.json", &escaped);
    // Real newlines inside a string, as a hand-made file may hold.
    d.write(
        "js.json",
        &format!("{{\"k\":\"{}\n{BODY}CCCC3333\n{}\"}}", begin(), end()),
    );
    for file in ["sa.json", "js.json"] {
        for code in [
            format!(r#"open("{file}") | to_json | lines"#),
            format!(r#"cat("{file}") | lines"#),
            format!(r#"file.read("{file}")"#),
        ] {
            let o = d.run(&code);
            assert!(!out(&o).contains(BODY), "{code}: {}", out(&o));
            assert!(denied(&o), "{code}: {}", err(&o));
        }
        let o = d.run_b(&format!("grep -o 'R64BODY[A-Z0-9]*' {file}"), None);
        assert!(!out(&o).contains(BODY), "{file}: {}", out(&o));
        assert!(err(&o).contains("private key"), "{}", err(&o));
    }
    let o = d.run(r#"search_code("R64BODY")"#);
    assert!(!out(&o).contains(BODY), "{}", out(&o));
}

#[test]
fn source_that_quotes_the_marker_is_still_not_a_key() {
    let d = Dirs::new("quoted");
    d.write(
        "a.py",
        // The text redactor hides an unclosed block to the end of the text, so
        // the line that must show comes first.
        &format!(
            "r64ok = 1\nx = \"{}\"\ny = '{}\\n' + body\n",
            begin(),
            begin()
        ),
    );
    let o = d.run(r#"cat("a.py")"#);
    assert!(out(&o).contains("r64ok"), "{}{}", out(&o), err(&o));
    assert!(!denied(&o));
}

#[test]
fn text_with_an_escaped_pem_block_is_redacted_whole() {
    let json = format!(
        "{{\"private_key\":\"{}\\n{BODY}AAAA1111\\n{BODY}BBBB2222\\n{}\\n\",\"n\":1}}",
        begin(),
        end()
    );
    let (r, changed) = redact_str(&json);
    assert!(changed && !r.contains(BODY), "{r}");
    assert!(r.contains("\"n\":1"), "{r}");
    // A string that holds a PEM block anywhere in it.
    let text = format!("note: {}\n{BODY}AAAA1111\n{}\nafter", begin(), end());
    let (r, _) = redact_str(&text);
    assert!(!r.contains(BODY) && r.contains("after"), "{r}");
    // As a record value, and as one element of an array of lines.
    use iron_shell::value::Value;
    use std::collections::BTreeMap;
    let mut m = BTreeMap::new();
    m.insert("blob".to_string(), Value::Str(text.clone()));
    let v = iron_shell::builtins::redact_value(&Value::Record(m));
    assert!(!format!("{v:?}").contains(BODY), "{v:?}");
    let lines = Value::Array(
        text.lines()
            .map(|l| Value::Str(l.to_string()))
            .collect::<Vec<_>>(),
    );
    let v = iron_shell::builtins::redact_value(&lines);
    assert!(!format!("{v:?}").contains(BODY), "{v:?}");
}

// ---- M1: approval tokens keep argv boundaries ----

fn token_of(o: &Output) -> String {
    let e = err(o);
    let at = e
        .find("apv_")
        .unwrap_or_else(|| panic!("no approval token in: {e}"));
    e[at..at + 20].to_string()
}

#[test]
fn ish_b_tokens_differ_when_argv_boundaries_differ() {
    let d = Dirs::new("token");
    let one = "r64nosuchprog '[%s]' 'a b'";
    let two = "r64nosuchprog '[%s]' a b";
    let t_one = token_of(&d.run_b(one, None));
    let t_two = token_of(&d.run_b(two, None));
    assert_ne!(t_one, t_two);
    let t_git_one = token_of(&d.run_b("r64nosuchprog -C 'x y'", None));
    let t_git_two = token_of(&d.run_b("r64nosuchprog -C x y", None));
    assert_ne!(t_git_one, t_git_two);
    // Whitespace runs do not collapse into a single space.
    let t_a = token_of(&d.run_b("r64nosuchprog 'a  b'", None));
    let t_b = token_of(&d.run_b("r64nosuchprog 'a b'", None));
    assert_ne!(t_a, t_b);
    // An empty word is a word.
    let t_c = token_of(&d.run_b("r64nosuchprog a ''", None));
    let t_d = token_of(&d.run_b("r64nosuchprog a", None));
    assert_ne!(t_c, t_d);
    // The same command has the same token every time.
    assert_eq!(t_one, token_of(&d.run_b(one, None)));
    // A token approved for one form does not approve the other.
    let o = d.run_b(two, Some(&t_one));
    assert!(err(&o).contains("E_NEEDS_APPROVAL"), "{}", err(&o));
    // It approves its own form: the run goes on to look for the program.
    let o = d.run_b(one, Some(&t_one));
    assert!(!err(&o).contains("E_NEEDS_APPROVAL"), "{}", err(&o));
}

#[test]
fn a_command_without_spaces_in_its_words_keeps_its_token() {
    let d = Dirs::new("token_plain");
    // Words with no white space join unambiguously: the token is the one an
    // operator approved before round 64.
    let a = token_of(&d.run_b("r64nosuchprog -C x y", None));
    let b = token_of(&d.run_b("r64nosuchprog -C x y", None));
    assert_eq!(a, b);
}

// ---- M2: secret text that was not recognised ----

fn hides(text: &str, secret: &str) {
    let (r, changed) = redact_str(text);
    assert!(changed && !r.contains(secret), "{text:?} -> {r:?}");
}

fn keeps(text: &str) {
    let (r, changed) = redact_str(text);
    assert!(!changed && r == text, "{text:?} -> {r:?}");
}

#[test]
fn a_yaml_env_list_is_redacted() {
    let block = "env:\n  - name: DB_PASSWORD\n    value: Zk3p9Qw2LmXYa\n  - name: LOG_LEVEL\n    value: debug-verbose\n";
    let (r, _) = redact_str(block);
    assert!(!r.contains("Zk3p9Qw2LmXYa"), "{r}");
    assert!(
        r.contains("DB_PASSWORD") && r.contains("debug-verbose"),
        "{r}"
    );
    hides(
        "- name: SECRET_KEY\n  value: \"Zk3p9Qw2LmXYa\"\n",
        "Zk3p9Qw2LmXYa",
    );
    hides("name: API_TOKEN\nvalue: 'Zk3p9Qw2LmXYa'\n", "Zk3p9Qw2LmXYa");
    hides(
        "env: [{name: SECRET_KEY, value: Zk3p9Qw2LmXYa}]",
        "Zk3p9Qw2LmXYa",
    );
    // Windows line ends.
    hides(
        "- name: DB_PASSWORD\r\n  value: Zk3p9Qw2LmXYa\r\n",
        "Zk3p9Qw2LmXYa",
    );
    // A name that is no secret, or a value that is a reference, stays.
    keeps("- name: LOG_LEVEL\n  value: debug-verbose\n");
    keeps("- name: DB_PASSWORD\n  valueFrom:\n    secretKeyRef: x\n");
    // A value less indented than the name is not that name's value.
    keeps("- name: DB_PASSWORD\nvalue: Zk3p9Qw2LmXYa-not-this\n");
}

#[test]
fn env_assignments_and_secret_flags_are_redacted() {
    hides("$env:DB_PASSWORD = \"Zk3p9Qw2LmXYa\"", "Zk3p9Qw2LmXYa");
    hides("$env:API_TOKEN='Zk3p9Qw2LmXYa'", "Zk3p9Qw2LmXYa");
    hides("ENV DB_PASSWORD Zk3p9Qw2LmXYa\nRUN true", "Zk3p9Qw2LmXYa");
    hides("ENV DB_PASSWORD=Zk3p9Qw2LmXYa", "Zk3p9Qw2LmXYa");
    hides("mysql --password Zk3p9Qw2LmXYa -h db", "Zk3p9Qw2LmXYa");
    hides("mysql --password=Zk3p9Qw2LmXYa -h db", "Zk3p9Qw2LmXYa");
    hides("tool --token ghx-Zk3p9Qw2LmXYa run", "Zk3p9Qw2LmXYa");
    hides("tool --api-key Zk3p9Qw2LmXYa run", "Zk3p9Qw2LmXYa");
    hides("login --password x", "password x");
    // The flag stays visible, and the rest of the line too.
    let (r, _) = redact_str("mysql --password Zk3p9Qw2LmXYa -h db");
    assert_eq!(r, "mysql --password [REDACTED] -h db");
    // Other settings, help text and prose are left alone.
    keeps("ENV PATH /usr/bin");
    keeps("ENV DB_HOST db.internal");
    keeps("$env:DB_HOST = \"db.internal\"");
    keeps("$env:DB_PASSWORD = $secret");
    keeps("ENV DB_PASSWORD $DB_PASSWORD");
    keeps("  --password PASSWORD   the password to use");
    keeps("use --token to pass a token");
    keeps("--password-file /run/secrets/pw");
    keeps("--token-ttl 300");
    // `-p x` is ambiguous (a port, a path) and is not redacted.
    keeps("mysql -p Zk3p9Qw2LmXYa");
}

// ---- M3: false positives ----

#[test]
fn code_that_reads_a_secret_is_not_garbled() {
    for line in [
        "api_key = os.environ['API_KEY']",
        "password = input('Password: ')",
        "token = request.headers['Authorization']",
        "api_key=os.getenv(\"API_KEY\")",
        "let apiKey = process.env.API_KEY;",
        "api_key = os.environ",
        "secret = getenv(\"S\")",
        "api_key = ENV['API_KEY']",
        "password = getpass.getpass()",
        "password: ${DB_PASSWORD}",
        "password=$DB_PASSWORD",
    ] {
        keeps(line);
    }
    // Literals, quoted or not, are still secrets.
    hides("api_key = \"sk-abcdefghijklmnop\"", "abcdefghijklmnop");
    hides("API_KEY=abc123def456", "abc123def456");
    hides("password = hunter2hunter2", "hunter2hunter2");
    hides(
        "password = 'os.environ-looking-password'",
        "looking-password",
    );
    hides("DB_PASSWORD: Zk3p9Qw2LmXYa", "Zk3p9Qw2LmXYa");
    // A code line next to a literal: only the literal goes.
    let (r, _) = redact_str("a = os.environ['A']\nTOKEN=Zk3p9Qw2LmXYa\n");
    assert_eq!(r, "a = os.environ['A']\nTOKEN=[REDACTED]\n");
}

#[test]
fn ordinary_pair_records_and_policy_text_are_kept() {
    use iron_shell::value::Value;
    use std::collections::BTreeMap;
    let pair = |name: &str, value: Value| {
        let mut m = BTreeMap::new();
        m.insert("name".to_string(), Value::Str(name.to_string()));
        m.insert("value".to_string(), value);
        iron_shell::builtins::redact_value(&Value::Record(m))
    };
    let shown = |v: &Value| format!("{v:?}");
    assert!(shown(&pair("token_count", Value::Int(5))).contains("Int(5)"));
    assert!(shown(&pair("token_count", Value::Int(123456))).contains("Int(123456)"));
    assert!(shown(&pair("secret_santa", Value::Str("Alice".into()))).contains("Alice"));
    assert!(shown(&pair("api_key_header", Value::Str("X-Api-Key".into()))).contains("X-Api-Key"));
    // Real secrets still go, a number among them.
    assert!(!shown(&pair("DB_PASSWORD", Value::Str("Zk3p9Qw2LmXYa".into()))).contains("Zk3p9"));
    assert!(!shown(&pair("DB_PASSWORD", Value::Int(123456789))).contains("123456789"));
    // The same pair as JSON text.
    keeps(r#"{"name":"token_count","value":5}"#);
    keeps(r#"{"name":"api_key_header","value":"X-Api-Key"}"#);
    hides(
        r#"{"name":"DB_PASSWORD","value":"Zk3p9Qw2LmXYa"}"#,
        "Zk3p9Qw2LmXYa",
    );
    // Policy and state text.
    keeps("password_policy: strong");
    keeps("password: required");
    keeps("password: optional");
    keeps("password: <your-password>");
    keeps("password: ********");
    keeps("secret_name: db-creds");
    keeps("api_key_header: X-Api-Key");
    // `changeme` is a real password.
    hides("password: changeme", "changeme");
}

// ---- LOW ----

#[test]
fn locale_archive_is_a_steering_name() {
    let d = Dirs::new("locale");
    let o = d.run(r#"set_env("LOCALE_ARCHIVE", "/x/y")"#);
    assert!(err(&o).contains("E_POLICY_DENY"), "{}{}", out(&o), err(&o));
}

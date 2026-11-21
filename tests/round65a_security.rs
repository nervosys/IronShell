//! Round 65a: a private key stored as a list of lines is refused; secret string
//! literals inside code expressions, `curl -u`, `docker login -p`, `mysql -pX`,
//! `Set-Item env:X` and a YAML `!!str` value are redacted; an `ish -b` approval
//! token binds the redirections of an external command.
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
            .join("round65a_security")
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

const BODY: &str = "R65BODYLINEA1";

fn denied(o: &Output) -> bool {
    err(o).contains("E_POLICY_DENY") && err(o).contains("private key")
}

// ---- 1: a key stored as a list of lines ----

fn list_files() -> Vec<(&'static str, String)> {
    let (b, e) = (begin(), end());
    vec![
        (
            "n.ipynb",
            format!(
                "{{\"cells\": [{{\"cell_type\": \"code\", \"source\": [\"{b}\\n\", \"{BODY}\\n\", \"{e}\\n\"]}}]}}"
            ),
        ),
        (
            "arr.json",
            format!("{{\"key\":[\"{b}\",\"{BODY}\",\"{e}\"]}}"),
        ),
        (
            "pretty.json",
            format!("{{\n  \"key\": [\n    \"{b}\",\n    \"{BODY}\",\n    \"{e}\"\n  ]\n}}"),
        ),
        (
            "arr.yaml",
            format!("key:\n  - \"{b}\"\n  - \"{BODY}\"\n  - \"{e}\"\n"),
        ),
        (
            "bare.yaml",
            format!("key:\n  - '{b}'\n  - {BODY}\n  - '{e}'\n"),
        ),
        (
            "arr.toml",
            format!("key = [\"{b}\",\n \"{BODY}\",\n \"{e}\"]\n"),
        ),
        (
            "multi.env",
            format!("KEY=\"{b}\"\nB1=\"{BODY}\"\nE=\"{e}\"\n"),
        ),
        (
            "joined.txt",
            format!("pem = \"{b}\\n\" + \"{BODY}\\n\" + \"{e}\\n\"\n"),
        ),
    ]
}

#[test]
fn a_key_stored_as_a_list_of_lines_is_refused() {
    let d = Dirs::new("lists");
    for (name, text) in list_files() {
        d.write(name, &text);
        for code in [
            format!(r#"cat("{name}")"#),
            format!(r#"cat("{name}") | regex_find("R65BODY[A-Z0-9]+")"#),
            format!(r#"cat("{name}") | replace("BEGIN", "B")"#),
            format!(r#"file.read("{name}")"#),
        ] {
            let o = d.run(&code);
            assert!(!out(&o).contains(BODY), "{code}: {}", out(&o));
            assert!(denied(&o), "{code}: {}{}", out(&o), err(&o));
        }
    }
    // The notebook's cell array through a parse and a slice.
    let o = d.run(r#"from_json(cat("n.ipynb")).cells[0].source | slice(1, 3)"#);
    assert!(!out(&o).contains(BODY), "{}", out(&o));
    assert!(denied(&o), "{}", err(&o));
    let o = d.run_b("grep R65BODY n.ipynb", None);
    assert!(!out(&o).contains(BODY), "{}", out(&o));
}

#[test]
fn source_that_quotes_the_marker_is_not_a_key() {
    let d = Dirs::new("source");
    let (b, e) = (begin(), end());
    let files = [
        // A constant, then more constants.
        (
            "consts.py",
            format!("KEY_HEADER = \"{b}\"\nKEY_FOOTER = \"{e}\"\nNAME = 'x'\n"),
        ),
        // A check for the marker, then a message.
        (
            "check.py",
            format!("if \"{b}\" in text:\n    raise ValueError(\"no keys here\")\n"),
        ),
        (
            "check.rs",
            format!("assert!(s.contains(\"{b}\"), \"expected a key\");\n"),
        ),
        (
            "call.js",
            format!("const re = new RegExp(\"{b}\");\nfoo(\"{b}\");\n"),
        ),
        // A test file that builds a fake key from parts.
        (
            "fake.rs",
            format!(
                "let header = \"{b}\";\nlet pem = format!(\"{{}}\\n{{}}\\n\", header, body);\nlet footer = \"{e}\";\n"
            ),
        ),
        // A list of marker names, then prose.
        (
            "names.rs",
            format!("let v = vec![\"{b}\", \"Something here\"];\n"),
        ),
        ("two.rs", format!("let a = [\"{b}\", \"{e}\"];\n")),
    ];
    for (name, text) in files {
        d.write(name, &text);
        let o = d.run(&format!(r#"cat("{name}")"#));
        assert!(!denied(&o), "{name}: {}{}", out(&o), err(&o));
        assert!(out(&o).contains("PRIVATE KEY") || out(&o).contains("[REDACTED]"));
    }
}

#[test]
fn a_key_with_a_break_still_counts_and_a_certificate_does_not() {
    let d = Dirs::new("classic");
    let (b, e) = (begin(), end());
    d.write("k.pem", &format!("{b}\n{BODY}ONE\n{e}\n"));
    d.write(
        "sa.json",
        &format!("{{\"private_key\": \"{b}\\n{BODY}ONE\\n{e}\\n\"}}"),
    );
    d.write(
        "cert.pem",
        "-----BEGIN CERTIFICATE-----\nMIIBcertbodyline1\n-----END CERTIFICATE-----\n",
    );
    for name in ["k.pem", "sa.json"] {
        let o = d.run(&format!(r#"cat("{name}")"#));
        assert!(denied(&o), "{name}: {}{}", out(&o), err(&o));
    }
    let o = d.run(r#"cat("cert.pem")"#);
    assert!(!denied(&o), "{}{}", out(&o), err(&o));
}

// ---- 2: secrets inside code expressions ----

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
fn a_secret_literal_in_a_code_value_is_redacted() {
    for (text, leak) in [
        (r#"secret = os.getenv("S", "Zk3p9Qw2LmXYaLEAK4")"#, "LEAK4"),
        (r#"password = get_pw() or "Zk3p9Qw2LmXYaLEAK5""#, "LEAK5"),
        (
            r#"api_key = config.get("api_key") || "Zk3p9Qw2LmXYaLEAK6""#,
            "LEAK6",
        ),
        (r#"token = foo(bar) + "Zk3p9Qw2LmXYaLEAK7""#, "LEAK7"),
        (r#"password = f"Zk3p9Qw2LmXYaLEAK8""#, "LEAK8"),
        (
            r#"db_password = lookup("x", default="Zk3p9Qw2LmXYaLEAK14")"#,
            "LEAK14",
        ),
        (
            r#"token = os.environ["T"] if x else "Zk3p9Qw2LmXYaLEAK15""#,
            "LEAK15",
        ),
        (r#"password => "Zk3p9Qw2LmXYaLEAKE""#, "LEAKE"),
        (
            "password = process.env.PW || 'Zk3p9Qw2LmXYaLEAK16';",
            "LEAK16",
        ),
        (
            "api_key = os.environ.get('K', 'AKIAIOSFODNN7LEAK3X')",
            "LEAK3X",
        ),
    ] {
        redacts(text, leak);
    }
    // The code around the literal stays.
    let (r, _) = redact_str(r#"secret = os.getenv("S", "Zk3p9Qw2LmXYaLEAK4")"#);
    assert_eq!(r, r#"secret = os.getenv("S", "[REDACTED]")"#);
}

#[test]
fn code_without_a_secret_literal_stays_readable() {
    for text in [
        "api_key = os.environ['API_KEY']",
        "api_key = os.getenv(\"API_KEY\")",
        "api_key=os.getenv(\"API_KEY\")",
        "password = input('Password: ')",
        "password = getpass.getpass()",
        "let apiKey = process.env.API_KEY;",
        "token = request.headers['Authorization']",
        "token = headers.get(\"Content-Type\")",
        "password = os.environ.get(\"DB_PASSWORD\", \"changeit_not_here\")",
        "secret = os.getenv(\"my_secret_name\", \"/etc/secret/path\")",
        "password = args.password",
        "token = self.token",
        "token = settings.TOKEN",
        "secret = base64.b64decode(x)",
        "const token = localStorage.getItem(\"token\");",
        "tokens = tokenizer.encode(text)",
    ] {
        keeps(text);
    }
}

#[test]
fn an_unquoted_literal_that_looks_like_code_is_redacted() {
    for (text, leak) in [
        ("password=hunter2(LEAK12)", "LEAK12"),
        ("password: pa(ss)wordLEAK13", "LEAK13"),
        ("SECRET_KEY=abc.def.ghiLEAK11xyz", "LEAK11"),
        ("api_key: abcd.efgh.ijkl.mnop", "efgh"),
        ("password: !!str Zk3p9Qw2LmXYaLEAKG", "LEAKG"),
        ("password: !!str\tZk3p9Qw2LmXYaLEAKG", "LEAKG"),
    ] {
        redacts(text, leak);
    }
    // A path whose first name is a known object stays code.
    for text in [
        "token = process.env.API_KEY;",
        "password = config.db.password",
        "token = os.environ.get",
        "key = self.cfg.key",
    ] {
        keeps(text);
    }
    // A call with an identifier and a bracket pair stays code.
    keeps("password = get_pw()");
    keeps("password = vault.read(path)");
}

#[test]
fn command_line_passwords_are_redacted() {
    for (text, leak) in [
        ("curl -u admin:Zk3p9Qw2LmXYaLEAKN https://x", "LEAKN"),
        ("curl --user admin:Zk3p9Qw2LmXYaLEAKN https://x", "LEAKN"),
        ("docker login -p Zk3p9Qw2LmXYaLEAKO", "LEAKO"),
        (
            "docker login -u me -p Zk3p9Qw2LmXYaLEAKO reg.example",
            "LEAKO",
        ),
        ("docker login --password Zk3p9Qw2LmXYaLEAKO", "LEAKO"),
        ("mysql -h x -pZk3p9Qw2LmXYaLEAK88 db", "LEAK88"),
        ("mysql -u root -pZk3p9Qw2LmXYaLEAK88", "LEAK88"),
        ("Set-Item env:DB_PASSWORD Zk3p9Qw2LmXYaLEAK99", "LEAK99"),
        (
            "Set-Item -Path env:DB_PASSWORD -Value Zk3p9Qw2LmXYaLEAK99",
            "LEAK99",
        ),
    ] {
        redacts(text, leak);
    }
    for text in [
        "curl -u admin https://x",
        "curl -u admin:$PASS https://x",
        "curl -s -o out.txt https://x",
        "docker login -u me",
        "docker run -p 8080:80 nginx",
        "mysql -p",
        "mysql -h db -P 3306",
        "Set-Item env:PATH C:/x",
    ] {
        keeps(text);
    }
}

#[test]
fn the_cli_prints_the_redactions() {
    let d = Dirs::new("cli");
    d.write(
        "red.txt",
        "secret = os.getenv(\"S\", \"Zk3p9Qw2LmXYaLEAK4\")\ncurl -u admin:Zk3p9Qw2LmXYaLEAKN x\n",
    );
    let o = d.run(r#"cat("red.txt")"#);
    assert!(!out(&o).contains("LEAK"), "{}", out(&o));
    assert!(
        out(&o).contains("os.getenv(\"S\", \"[REDACTED]\")"),
        "{}",
        out(&o)
    );
}

// ---- 3: an approval token binds the redirections ----

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
fn an_approval_token_binds_the_redirections() {
    let d = Dirs::new("redir");
    d.write("a.txt", "a\n");
    d.write("b.txt", "b\n");
    let plain = token(&d.run_b("env printf x", None));
    let to_a = token(&d.run_b("env printf x > a.txt", None));
    let to_b = token(&d.run_b("env printf x > b.txt", None));
    let append_a = token(&d.run_b("env printf x >> a.txt", None));
    let from_a = token(&d.run_b("env printf x < a.txt", None));
    let all = [&plain, &to_a, &to_b, &append_a, &from_a];
    for (i, x) in all.iter().enumerate() {
        for y in &all[i + 1..] {
            assert_ne!(x, y, "tokens {all:?}");
        }
    }
    // The same command gives the same token, and a path spelled another way
    // that names the same file does too.
    assert_eq!(to_a, token(&d.run_b("env printf x > a.txt", None)));
    assert_eq!(to_a, token(&d.run_b("env printf x > ./a.txt", None)));
    // The plain command's token is the one earlier rounds gave.
    assert_eq!(plain, "apv_e5f322f280e32aed");
}

#[test]
fn approving_the_plain_command_does_not_approve_the_redirected_one() {
    let d = Dirs::new("redir2");
    d.write("victim.cfg", "keep\n");
    let plain = token(&d.run_b("env printf x", None));
    let o = d.run_b("env printf x > victim.cfg", Some(&plain));
    assert!(
        err(&o).contains("E_NEEDS_APPROVAL"),
        "{}{}",
        out(&o),
        err(&o)
    );
    let text = std::fs::read_to_string(d.ws().join("victim.cfg")).unwrap();
    // The refused command did not run, so `x` was not written (bash also opens
    // the file for the redirection before the command runs, which empties it).
    assert!(!text.contains('x'), "{text:?}");
    // Its own token approves it.
    let own = token(&o);
    let o = d.run_b("env printf x > victim.cfg", Some(&own));
    assert!(
        !err(&o).contains("E_NEEDS_APPROVAL"),
        "{}{}",
        out(&o),
        err(&o)
    );
}

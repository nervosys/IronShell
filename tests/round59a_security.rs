//! Round 59a: redaction of `print`, `dbg` and error extras, redaction by field
//! name for arrays and numbers, approval tokens that bind `set_env` for network
//! builtins, steering names, and refusals made before the approval gate.
//! Every secret here is fake.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("round59a_security")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("ws")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Dirs { root }
    }

    fn ws(&self) -> PathBuf {
        self.root.join("ws")
    }

    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.ws().join(name), text).unwrap();
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

const ENV_TEXT: &str = "API_KEY=supersecretvalue123\npassword=hunter2hunter2\n";

#[test]
fn print_redacts_a_secret_and_counts_it() {
    let d = Dirs::new("print");
    d.write(".env", ENV_TEXT);
    let o = d.run(r#"print(file.read(".env"))"#);
    assert!(!out(&o).contains("supersecretvalue123"), "{}", out(&o));
    assert!(!out(&o).contains("hunter2hunter2"), "{}", out(&o));
    assert!(out(&o).contains("[REDACTED]"), "{}", out(&o));
    assert!(err(&o).contains("redacted"), "{}", err(&o));
    let o = d.run(r#"print("password=hunter2hunter2")"#);
    assert!(!out(&o).contains("hunter2hunter2"), "{}", out(&o));
}

#[test]
fn print_redacts_under_deterministic_and_a_record_field() {
    let d = Dirs::new("printdet");
    d.write(".env", ENV_TEXT);
    let o = d.run_with(
        &["--deterministic", "-c", r#"print(file.read(".env"))"#],
        &[],
    );
    assert!(!out(&o).contains("supersecretvalue123"), "{}", out(&o));
    let o = d.run(r#"print({token: "abc123secret", n: 1})"#);
    assert!(!out(&o).contains("abc123secret"), "{}", out(&o));
}

#[test]
fn print_is_raw_when_redaction_is_off() {
    let d = Dirs::new("printoff");
    d.write(".env", ENV_TEXT);
    let o = d.run_with(
        &["-c", r#"print(file.read(".env"))"#],
        &[("IRONSHELL_REDACT", "off")],
    );
    assert!(out(&o).contains("supersecretvalue123"), "{}", out(&o));
}

#[test]
fn dbg_redacts_its_stderr_line() {
    let d = Dirs::new("dbg");
    d.write(".env", ENV_TEXT);
    let o = d.run(r#"dbg(file.read(".env"))"#);
    assert!(!err(&o).contains("supersecretvalue123"), "{}", err(&o));
    assert!(!err(&o).contains("hunter2hunter2"), "{}", err(&o));
    assert!(err(&o).contains("[DEBUG]"), "{}", err(&o));
}

#[test]
fn error_extras_are_redacted_by_field_name() {
    let d = Dirs::new("errextra");
    for code in [
        r#"error("m", {token: "abc123secret"})"#,
        r#"error("m", {password: "xyz123"})"#,
        r#"error("m", {password: ["xyz123"]})"#,
    ] {
        for args in [vec!["-c", code], vec!["--deterministic", "-c", code]] {
            let o = d.run_with(&args, &[]);
            let e = err(&o);
            assert!(!e.contains("abc123secret"), "{e}");
            assert!(!e.contains("xyz123"), "{e}");
            assert!(e.contains("[REDACTED]"), "{e}");
            assert_eq!(error_json(&o)["error"]["message"], "m");
        }
    }
    // A field with an ordinary name stays as written.
    let o = d.run(r#"error("m", {field: "age"})"#);
    assert_eq!(error_json(&o)["error"]["field"], "age");
}

#[test]
fn a_caught_error_keeps_its_fields_for_the_script() {
    let d = Dirs::new("errcaught");
    let o = d.run(r#"try { error("m", {field: "age"}) } catch e { e.field }"#);
    assert_eq!(out(&o).trim(), "age");
}

#[test]
fn secret_named_arrays_and_numbers_are_redacted() {
    let d = Dirs::new("byname");
    let o = d.run(r#"{password: ["hunter2", "b"], token: 99999, secrets: ["x"]}"#);
    let s = out(&o);
    assert!(!s.contains("hunter2"), "{s}");
    assert!(!s.contains("99999"), "{s}");
    assert!(!s.contains("\"x\""), "{s}");
    assert!(s.contains("[REDACTED]"), "{s}");
    // A record nested under a secret name has its strings redacted too.
    let o = d.run(r#"{credentials: {user: "alice", flag: true, none: null}}"#);
    let s = out(&o);
    assert!(!s.contains("alice"), "{s}");
    assert!(s.contains("true") && s.contains("null"), "{s}");
}

#[test]
fn bools_null_empty_strings_and_token_counts_stay() {
    let d = Dirs::new("byname-keep");
    let o = d.run(r#"{password: "", token: true, secret: null, full_tokens: 120, page_tokens: 7}"#);
    let s = out(&o);
    assert!(s.contains("full_tokens=120"), "{s}");
    assert!(s.contains("page_tokens=7"), "{s}");
    assert!(s.contains("token=true"), "{s}");
    assert!(!s.contains("[REDACTED]"), "{s}");
}

#[test]
fn a_network_approval_token_binds_set_env() {
    let d = Dirs::new("netenv");
    let token = |code: &str| -> String {
        let o = d.run(code);
        let j = error_json(&o);
        assert_eq!(j["error"]["code"], "E_NEEDS_APPROVAL", "{j}");
        j["error"]["approval"]["token"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let plain = token("gh.pr()");
    let changed = token(r#"set_env("MY_APP_FLAG", "1"); gh.pr()"#);
    assert_ne!(plain, changed, "set_env did not change the token");
    let other = token(r#"set_env("MY_APP_FLAG", "2"); gh.pr()"#);
    assert_ne!(changed, other);
    // The same call with no change is unchanged.
    assert_eq!(plain, token("gh.pr()"));
}

#[test]
fn steering_names_are_refused_for_set_env() {
    let d = Dirs::new("steer");
    for name in [
        "GH_HOST",
        "GH_REPO",
        "GH_CONFIG_DIR",
        "TAR_OPTIONS",
        "RSYNC_RSH",
        "MISE_CONFIG_FILE",
        "ASDF_DATA_DIR",
        "DIRENV_CONFIG",
        "VIRTUALENV_CONFIG_FILE",
        "ELECTRON_RUN_AS_NODE",
        "R_HOME",
        "PHP_INI_SCAN_DIR",
        "MAVEN_ARGS",
    ] {
        let o = d.run(&format!(r#"set_env("{name}", "x")"#));
        assert_eq!(error_json(&o)["error"]["code"], "E_POLICY_DENY", "{name}");
    }
    let o = d.run(r#"set_env("MY_APP_FLAG", "x")"#);
    assert!(!err(&o).contains("E_POLICY_DENY"), "{}", err(&o));
}

#[test]
fn rmdir_that_cannot_succeed_issues_no_token() {
    let d = Dirs::new("rmdir");
    d.write("x.txt", "one\n");
    let o = d.run(r#"rmdir("nonexist", true)"#);
    let j = error_json(&o);
    assert_eq!(j["error"]["code"], "E_NOT_FOUND", "{j}");
    assert!(j["error"].get("approval").is_none(), "{j}");
    let o = d.run(r#"rmdir("x.txt", true)"#);
    let j = error_json(&o);
    assert_eq!(j["error"]["code"], "E_BAD_ARG", "{j}");
    assert!(j["error"].get("approval").is_none(), "{j}");
    // A real directory still asks for approval.
    std::fs::create_dir(d.ws().join("real")).unwrap();
    let o = d.run(r#"rmdir("real", true)"#);
    assert_eq!(error_json(&o)["error"]["code"], "E_NEEDS_APPROVAL");
}

#[test]
fn file_move_onto_itself_issues_no_token() {
    let d = Dirs::new("movesame");
    d.write("x.txt", "one\n");
    let o = d.run(r#"file_move("x.txt", "x.txt")"#);
    let j = error_json(&o);
    assert_eq!(j["error"]["code"], "E_BAD_ARG", "{j}");
    assert!(j["error"].get("approval").is_none(), "{j}");
}

#[test]
fn file_move_into_itself_says_so() {
    let d = Dirs::new("moveinto");
    std::fs::create_dir(d.ws().join("d1")).unwrap();
    let o = d.run(r#"file_move("d1", "d1/sub/d1")"#);
    let j = error_json(&o);
    let text = j.to_string();
    assert!(text.contains("into itself"), "{j}");
    assert!(!text.contains("takes String"), "{j}");
    assert!(j["error"].get("approval").is_none(), "{j}");
}

#[test]
fn http_get_to_this_machine_is_denied_before_the_gate() {
    let d = Dirs::new("ssrf");
    for url in [
        "http://localhost:3002/health",
        "http://127.0.0.1:3002/health",
        "http://[::1]:3002/health",
        "http://10.0.0.5/x",
    ] {
        let o = d.run(&format!(r#"http_get("{url}")"#));
        let j = error_json(&o);
        assert_eq!(j["error"]["code"], "E_POLICY_DENY", "{url}: {j}");
        assert!(j["error"].get("approval").is_none(), "{url}: {j}");
        assert!(!j.to_string().contains("apv_"), "{url}: {j}");
    }
    // A wildcard, another host or another port in the allowlist does not
    // lift it.
    for allow in ["*", "example.com", "localhost:9999"] {
        let o = d.run_with(
            &["-c", r#"http_get("http://localhost:3002/health")"#],
            &[("IRONSHELL_NET_ALLOW", allow)],
        );
        assert_eq!(error_json(&o)["error"]["code"], "E_POLICY_DENY", "{allow}");
    }
    // Since round 66 the operator may list the exact host: the request is
    // then made (and fails, or not, on its own merits), never a policy refusal.
    let o = d.run_with(
        &["-c", r#"http_get("http://localhost:3002/health")"#],
        &[("IRONSHELL_NET_ALLOW", "localhost")],
    );
    assert!(!err(&o).contains("E_POLICY_DENY"), "{}", err(&o));
}

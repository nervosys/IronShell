//! Round 36: edges a red-team run of agent mode found. Temp paths stay in the
//! workspace, `try` does not swallow a policy refusal, Windows device names are
//! refused, a missing path is named as a path, an unmapped PowerShell command
//! is named as one, and the MCP `ironshell` tool takes `builtin` for `name`.
//!
//! The binary runs with IRONSHELL_SESSION_DIR in a temp dir of its own.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let root = std::env::temp_dir().join(format!("ae-round36-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("session")).unwrap();
        std::fs::create_dir_all(root.join("ws")).unwrap();
        Sandbox(root)
    }

    fn ws(&self) -> PathBuf {
        self.0.join("ws")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ish"));
        c.args(args)
            .current_dir(self.ws())
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", self.0.join("session"))
            .env_remove("IRONSHELL_ALLOW_SH")
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_AGENT")
            .env_remove("IRONSHELL_WORKSPACE");
        c
    }

    fn ish(&self, args: &[&str]) -> Output {
        self.command(args).output().expect("run ish")
    }

    /// `ish --agent --workspace <ws> -c <src>`.
    fn agent(&self, src: &str) -> Output {
        let ws = self.ws();
        self.ish(&["--agent", "--workspace", ws.to_str().unwrap(), "-c", src])
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn text(out: &[u8]) -> String {
    String::from_utf8_lossy(out).into_owned()
}

// 1
#[test]
fn item1_agent_temp_paths_are_created_inside_the_workspace() {
    let sb = Sandbox::new("temp");
    for call in ["fs_tempdir(\"x\")", "fs_tempfile(\"x\")"] {
        let out = sb.agent(call);
        let stdout = text(&out.stdout);
        assert!(out.status.success(), "{call}: {}", text(&out.stderr));
        assert!(stdout.contains(".ironshell"), "{call}: {stdout}");
    }
    let tmp = sb.ws().join(".ironshell").join("tmp");
    let names: Vec<String> = std::fs::read_dir(&tmp)
        .expect("the workspace temp dir exists")
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.iter().all(|n| n.starts_with("x_")), "{names:?}");
}

// 2
#[test]
fn item2_try_does_not_catch_a_policy_refusal_in_agent_mode() {
    let sb = Sandbox::new("try");
    std::fs::write(sb.ws().join("ap1.txt"), "keep").unwrap();
    let out = sb.agent("try { rm(\"ap1.txt\") } catch e { \"caught\" }");
    let stdout = text(&out.stdout);
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(75), "{stdout} {stderr}");
    assert!(!stdout.contains("caught"), "{stdout}");
    assert!(stderr.contains("E_NEEDS_APPROVAL"), "{stderr}");
    assert!(sb.ws().join("ap1.txt").exists());
}

// 3
#[cfg(windows)]
#[test]
fn item3_a_reserved_device_name_is_refused_on_windows() {
    let sb = Sandbox::new("device");
    for call in [
        "file_write(\"COM1\", \"x\")",
        "file_write(\"nul.txt\", \"x\")",
        "file_write(\"sub/Aux\", \"x\")",
        "file_mkdir(\"lpt9\")",
    ] {
        let out = sb.agent(call);
        let stderr = text(&out.stderr);
        assert!(!out.status.success(), "{call}: {}", text(&out.stdout));
        assert!(stderr.contains("E_BAD_ARG"), "{call}: {stderr}");
        assert!(
            stderr.contains("is a reserved device name on Windows"),
            "{call}: {stderr}"
        );
    }
    assert!(!sb.ws().join("sub").exists());
    // A name that only starts like a device is fine.
    let out = sb.agent("file_write(\"console.txt\", \"x\")");
    assert!(out.status.success(), "{}", text(&out.stderr));
}

// 4
#[test]
fn item4_a_missing_move_source_is_named_as_a_path() {
    let sb = Sandbox::new("move");
    let out = sb.agent("file_move(\"tree\", \"tree2\")");
    let stderr = text(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(stderr.contains("E_NOT_FOUND"), "{stderr}");
    assert!(
        stderr.contains("no file or directory at `tree`"),
        "{stderr}"
    );
    assert!(!stderr.contains("no source named"), "{stderr}");
    assert!(!stderr.contains("available sources"), "{stderr}");
}

// 5
#[test]
fn item5_an_unmapped_powershell_command_is_named_as_a_command() {
    let sb = Sandbox::new("pwsh");
    // Remove-Item, Set-Content and Copy-Item are mapped (PS_MAP); these are not.
    for cmd in ["Get-Content", "New-Item", "Test-Path", "Select-String"] {
        let out = sb.ish(&["--pwsh", "-c", &format!("{cmd} ../x")]);
        let stderr = text(&out.stderr);
        assert!(!out.status.success(), "{cmd}: {stderr}");
        assert!(
            stderr.contains(&format!(
                "command `{cmd}` at line 1 has no IronShell mapping"
            )),
            "{cmd}: {stderr}"
        );
        assert!(
            !stderr.contains("expression or operator"),
            "{cmd}: {stderr}"
        );
    }
    // A mapped cmdlet with a `..` path is translated, not refused as a range
    // operator. In agent mode the path outside the workspace is then refused
    // by the jail, so nothing outside the sandbox is touched.
    let out = sb.ish(&[
        "--agent",
        "--workspace",
        ".",
        "--pwsh",
        "-c",
        "Remove-Item ../x",
    ]);
    let stderr = text(&out.stderr);
    assert!(!stderr.contains("expression or operator"), "{stderr}");
    assert!(!stderr.contains("has no IronShell mapping"), "{stderr}");
}

// 6
#[test]
fn item6_the_mcp_ironshell_tool_accepts_builtin_for_name() {
    let sb = Sandbox::new("mcp");
    let mut child = sb
        .command(&["mcp", "stdio"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn ish mcp stdio");
    let requests = [
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ironshell","arguments":{"name":"len","args":["abc"]}}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ironshell","arguments":{"builtin":"len","args":["abc"]}}}"#,
    ];
    {
        let stdin = child.stdin.as_mut().unwrap();
        for r in requests {
            writeln!(stdin, "{r}").unwrap();
        }
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().expect("wait");
    let stdout = text(&out.stdout);
    assert!(!stdout.contains("missing 'name'"), "{stdout}");
    let replies: Vec<serde_json::Value> = stdout
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    assert_eq!(replies.len(), 2, "{stdout}");
    assert!(replies.iter().all(|r| r.get("error").is_none()), "{stdout}");
    assert_eq!(
        replies[0]["result"], replies[1]["result"],
        "`builtin` must behave as `name`: {stdout}"
    );
}

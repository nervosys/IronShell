//! Round 50b: archive, `ish -b`, journal and glob tool fixes.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round50b_tools")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj").join("src")).expect("project dir");
    std::fs::create_dir_all(dir.join("session")).expect("session dir");
    std::fs::write(
        dir.join("proj").join("src").join("main.rs"),
        b"fn main() {}\n",
    )
    .expect("file");
    dir
}

fn ish_with(dir: &Path, args: &[&str], agent: bool) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ish"));
    c.args(args)
        .current_dir(dir.join("proj"))
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_AGENT")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_APPROVE");
    if agent {
        c.env("IRONSHELL_MODE", "agent");
    }
    c.output().expect("run ish")
}

fn ish_in(dir: &Path, code: &str) -> Output {
    ish_with(dir, &["-c", code], false)
}

fn out(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn b1_archive_into_a_missing_folder_names_the_folder() {
    let dir = scratch("b1");
    for call in [
        r#"tar_create("out/p.tar.gz", "src")"#,
        r#"zip_create("out/p.zip", "src")"#,
    ] {
        let o = ish_in(&dir, call);
        let text = out(&o);
        assert!(!o.status.success(), "{text}");
        assert!(text.contains("E_NOT_FOUND"), "{text}");
        assert!(text.contains("folder"), "{text}");
        assert!(text.contains("out"), "{text}");
        assert!(!text.contains("no file at"), "{text}");
    }
}

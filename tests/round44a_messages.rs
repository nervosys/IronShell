//! Round 44a: an archive member outside the workspace is E_OUTSIDE_WORKSPACE,
//! shown redacted as every other builtin shows an outside path.
//!
//! `tar_create('o.tar', 'C:..\outside\secret.txt')` answered E_POLICY_DENY and
//! printed the member's full absolute path, which names the folders above the
//! workspace. `zip_create` and `zip_add` said the same.

use std::process::{Command, Output};

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn an_outside_member_is_the_jails_code_and_does_not_show_its_folder() {
    let root = std::env::temp_dir().join(format!("ae-r44a-msg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let ws = root.join("ws");
    let outside = root.join("zz-marker-outside");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(root.join("session")).unwrap();
    let member = outside.join("secret.txt");
    std::fs::write(&member, "s").unwrap();
    let shown = member.to_string_lossy().replace('\\', "/");
    for call in ["tar_create", "zip_create"] {
        let out = Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["--agent", "--workspace"])
            .arg(&ws)
            .args(["-c", &format!("{call}('o.{}', '{shown}')", &call[..3])])
            .current_dir(&root)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", root.join("session"))
            .output()
            .expect("run ish");
        let said = text(&out);
        assert!(!out.status.success(), "{call}: {said}");
        assert!(said.contains("E_OUTSIDE_WORKSPACE"), "{call}: {said}");
        assert!(
            !said.contains("zz-marker-outside"),
            "{call} named the folder above the workspace: {said}"
        );
        assert!(said.contains("[...]/secret.txt"), "{call}: {said}");
    }
    assert!(!ws.join("o.tar").exists() && !ws.join("o.zip").exists());
    let _ = std::fs::remove_dir_all(&root);
}

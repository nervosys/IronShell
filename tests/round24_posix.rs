//! Round 24: a sed command the in-process POSIX subset declines, with no sed on
//! PATH, is refused by name before the exec gate, not sent for approval.

#[cfg(windows)]
#[test]
fn sed_command_outside_the_subset_with_no_sed_on_path_is_refused_before_approval() {
    let root = std::env::temp_dir().join(format!("ae-round24-sed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let work = root.join("work");
    let session = root.join("session");
    let empty_path = root.join("empty-path");
    for dir in [&work, &session, &empty_path] {
        std::fs::create_dir_all(dir).unwrap();
    }

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--agent", "--workspace"])
        .arg(&work)
        .args(["-b", "-c", "echo abc | sed 'y/abc/xyz/'"])
        .current_dir(&work)
        .env("NO_COLOR", "1")
        .env("PATH", &empty_path)
        .env("IRONSHELL_SESSION_DIR", &session)
        .env_remove("IRONSHELL_APPROVE")
        .output()
        .expect("run ish");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(out.status.code(), Some(2), "{err}");
    assert!(!err.contains("E_NEEDS_APPROVAL"), "{err}");
    assert!(err.contains("sed command `y/abc/xyz/`"), "{err}");
    assert!(
        err.contains("is not in the in-process subset, which runs p, d, q and s///"),
        "{err}"
    );
}

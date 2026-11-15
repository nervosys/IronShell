//! Round 43a: a Windows drive-relative path (`C:..\outside\f`, a drive prefix with
//! no root) is judged and written at one place.
//!
//! `C:..` is the parent of the *drive's* current directory, not of the workspace.
//! Joined to the workspace it replaced the whole path, and the jail's walk then
//! dropped the `..` against the bare `C:` prefix and judged a file outside the
//! workspace as inside it: writes escaped while reads (judged elsewhere) were
//! refused. Every path now goes through `safety::resolve_path_str` and the one
//! jail walk, so the jail, the journal and the write name the same place.
//!
//! On non-Windows `C:..` is an ordinary relative name, so only the checks that
//! hold everywhere run there.

use iron_shell::safety::{is_drive_relative, windows_name_refusal, windows_read_name_refusal};

#[test]
fn only_a_drive_prefix_with_no_root_is_drive_relative_and_only_on_windows() {
    assert_eq!(is_drive_relative("C:.."), cfg!(windows));
    assert_eq!(is_drive_relative("C:foo/bar"), cfg!(windows));
    assert_eq!(is_drive_relative("c:"), cfg!(windows));
    assert!(!is_drive_relative(r"C:\ws\a.txt"));
    assert!(!is_drive_relative("C:/ws/a.txt"));
    assert!(!is_drive_relative("a/b.txt"));
    assert!(!is_drive_relative("../a.txt"));
    assert!(!is_drive_relative("/etc/hosts"));
    assert!(!is_drive_relative(""));
}

#[test]
fn a_forbidden_character_is_bad_arg_naming_it() {
    let e = windows_name_refusal("file_write", "bad<name.txt").expect("`<` must be refused");
    let said = e.to_string();
    assert!(said.contains("E_BAD_ARG"), "{said}");
    assert!(said.contains('<'), "{said}");
    assert!(!said.contains("E_OUTSIDE_WORKSPACE"), "{said}");
    for name in ["a>b", "a|b", "a\"b", "a\u{1}b"] {
        assert!(
            windows_name_refusal("file_write", name).is_some(),
            "{name:?} must be refused"
        );
    }
}

#[test]
fn a_read_is_refused_for_the_names_a_write_is_except_a_stream() {
    for (path, word) in [
        ("a.txt.", "strip"),
        ("dir /a.txt", "strip"),
        ("NUL", "device"),
        ("CON", "device"),
        ("sub/aux.txt", "device"),
        ("bad<name", "<"),
    ] {
        let e = windows_read_name_refusal("file_read", path)
            .unwrap_or_else(|| panic!("{path} must be refused"));
        let said = e.to_string();
        assert!(said.contains("E_BAD_ARG"), "{path}: {said}");
        assert!(said.contains(word), "{path}: {said}");
    }
    for path in [
        "a.txt",
        "dir/",
        "..",
        "./a.txt",
        r"C:\ws\a.txt",
        "f.txt:stream",
    ] {
        assert!(
            windows_read_name_refusal("file_read", path).is_none(),
            "{path} must pass"
        );
    }
    // A write still refuses the stream.
    assert!(windows_name_refusal("file_write", "f.txt:stream").is_some());
}

#[test]
fn a_refusal_shows_dot_dot_in_the_same_form_as_any_other_path() {
    use iron_shell::security::sanitize_path_in_error;
    assert_eq!(sanitize_path_in_error("a/secret.txt"), "[...]/secret.txt");
    assert_eq!(sanitize_path_in_error(".."), "[...]/..");
    assert_eq!(sanitize_path_in_error("sub/.."), "[...]/..");
    assert!(!sanitize_path_in_error("..").contains("REDACTED"));
}

#[cfg(windows)]
mod windows {
    use std::path::PathBuf;
    use std::process::{Command, Output};

    /// A workspace `ws`, a sibling `outside` holding `secret.txt`, and a
    /// session folder, all under a fresh temp dir.
    fn setup(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("ae-round43a-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ws = root.join("ws");
        let outside = root.join("outside");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        std::fs::write(outside.join("secret.txt"), "TOPSECRET").unwrap();
        std::fs::write(ws.join("a.txt"), "FILECONTENT").unwrap();
        (root, ws, outside)
    }

    /// The drive of `ws` as `X:`: the drive-relative form must name the drive the
    /// test folders are on, whichever it is.
    fn drive(ws: &std::path::Path) -> String {
        ws.to_string_lossy().chars().take(2).collect()
    }

    fn ish(ws: &std::path::Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .current_dir(ws)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", ws)
            .env("IRONSHELL_SESSION_DIR", ws.join("..").join("session"))
            .args(args)
            .output()
            .expect("run ish")
    }

    fn said(out: &Output) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    /// Nothing but the secret is in `outside`, and no stray `ws\outside` was made.
    fn untouched(ws: &std::path::Path, outside: &std::path::Path, what: &str) {
        let names: Vec<_> = std::fs::read_dir(outside)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["secret.txt".to_string()], "{what}: {names:?}");
        assert!(
            !ws.join("outside").exists(),
            "{what}: a stray ws\\outside was made"
        );
    }

    #[test]
    fn a_drive_relative_write_outside_the_workspace_is_refused_by_every_builtin() {
        let (root, ws, outside) = setup("builtins");
        let d = drive(&ws);
        for expr in [
            format!(r#"file.write("{d}../outside/w.txt", "x")"#),
            format!(r#"file.append("{d}../outside/w.txt", "x")"#),
            format!(r#"file.replace("{d}../outside/w.txt", "a", "b")"#),
            format!(r#"file.copy("a.txt", "{d}../outside/c.txt")"#),
            format!(r#"touch("{d}../outside/t.txt")"#),
            format!(r#"mkdir("{d}../outside/newdir")"#),
        ] {
            let out = ish(&ws, &["-c", &expr]);
            let text = said(&out);
            assert!(!out.status.success(), "{expr}: {text}");
            assert!(text.contains("E_OUTSIDE_WORKSPACE"), "{expr}: {text}");
            untouched(&ws, &outside, &expr);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_drive_relative_path_outside_the_workspace_is_refused_by_ish_b() {
        let (root, ws, outside) = setup("posix");
        let d = drive(&ws);
        for script in [
            format!("echo x > {d}../outside/b.txt"),
            format!("echo x | tee {d}../outside/b.txt"),
            format!("touch {d}../outside/b.txt"),
            format!("mkdir {d}../outside/newdir"),
            format!("mkdir -p {d}../outside/newdir/deeper"),
        ] {
            let out = ish(&ws, &["-b", "-c", &script]);
            assert!(!out.status.success(), "{script}: {}", said(&out));
            untouched(&ws, &outside, &script);
        }
        // A read is refused too: the secret is not printed and the folder is not listed.
        for script in [
            format!("cat {d}../outside/secret.txt"),
            format!("ls {d}../outside"),
        ] {
            let out = ish(&ws, &["-b", "-c", &script]);
            let text = said(&out);
            assert!(!out.status.success(), "{script}: {text}");
            assert!(!text.contains("TOPSECRET"), "{script}: {text}");
            // The refusal echoes the operand it refused, so only `ls` (whose
            // operand is the folder) must not mention the file inside it.
            if script.starts_with("ls ") {
                assert!(!text.contains("secret.txt"), "{script}: {text}");
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_drive_relative_path_inside_the_workspace_means_the_drives_current_directory() {
        let (root, ws, _outside) = setup("inside");
        let d = drive(&ws);
        // The child's working directory is the workspace, so `X:in.txt` is `ws\in.txt`.
        let out = ish(&ws, &["-c", &format!(r#"file.write("{d}in.txt", "x")"#)]);
        assert!(out.status.success(), "{}", said(&out));
        assert_eq!(std::fs::read_to_string(ws.join("in.txt")).unwrap(), "x");
        let out = ish(&ws, &["-b", "-c", &format!("echo y > {d}in2.txt")]);
        assert!(out.status.success(), "{}", said(&out));
        assert!(ws.join("in2.txt").exists());
        // And one that climbs back in is judged where it lands, not as spelled.
        let out = ish(
            &ws,
            &["-c", &format!(r#"file.write("{d}../ws/in3.txt", "z")"#)],
        );
        assert!(out.status.success(), "{}", said(&out));
        assert!(ws.join("in3.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn explain_predicts_the_deny_the_run_gives() {
        let (root, ws, _outside) = setup("explain");
        let d = drive(&ws);
        let out = ish(
            &ws,
            &[
                "--explain",
                "-c",
                &format!(r#"file.write("{d}../outside/w.txt", "x")"#),
            ],
        );
        let text = said(&out);
        assert!(text.contains("deny"), "{text}");
        let out = ish(
            &ws,
            &[
                "--explain",
                "-b",
                "-c",
                &format!("echo x > {d}../outside/b.txt"),
            ],
        );
        let text = said(&out);
        assert!(text.contains("deny"), "{text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_journal_records_the_resolved_path_and_undo_removes_that_file() {
        let (root, ws, _outside) = setup("journal");
        let d = drive(&ws);
        let out = ish(&ws, &["-c", &format!(r#"file.write("{d}j.txt", "x")"#)]);
        assert!(out.status.success(), "{}", said(&out));
        assert!(ws.join("j.txt").exists());
        let out = ish(&ws, &["-c", "undo()"]);
        let text = said(&out);
        assert!(!ws.join("j.txt").exists(), "undo left the file: {text}");
        assert!(!text.contains("absent now"), "{text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_resolved_drive_relative_path_is_absolute_and_has_no_bare_drive_prefix() {
        let cwd = std::env::current_dir().unwrap();
        let Some(parent) = cwd.parent() else { return };
        let d = drive(&cwd);
        let given = format!("{d}../outside/w.txt");
        let resolved = iron_shell::safety::resolve_path_str(&given);
        assert!(std::path::Path::new(&resolved).is_absolute(), "{resolved}");
        let recorded = iron_shell::journal::normalized_path(&given);
        assert_eq!(
            recorded,
            parent.join("outside").join("w.txt").display().to_string()
        );
    }

    #[test]
    fn a_read_of_a_name_windows_would_change_is_bad_arg_not_a_read_of_another_file() {
        let (root, ws, _outside) = setup("names");
        for (expr, word) in [
            (r#"file.read("a.txt.")"#, "strip"),
            (r#"file.read("a.txt ")"#, "strip"),
            (r#"file.read("NUL")"#, "reserved device name"),
            (r#"file.read("CON")"#, "reserved device name"),
            (r#"file.write("bad<name.txt", "x")"#, "<"),
        ] {
            let out = ish(&ws, &["-c", expr]);
            let text = said(&out);
            assert!(!out.status.success(), "{expr}: {text}");
            assert!(text.contains("E_BAD_ARG"), "{expr}: {text}");
            assert!(text.contains(word), "{expr}: {text}");
            assert!(!text.contains("E_OUTSIDE_WORKSPACE"), "{expr}: {text}");
            assert!(!text.contains("FILECONTENT"), "{expr}: {text}");
        }
        assert!(!ws.join("bad<name.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cd_dot_dot_shows_the_path_in_the_usual_redacted_form() {
        let (root, ws, _outside) = setup("cd");
        let out = ish(&ws, &["-c", r#"cd("..")"#]);
        let text = said(&out);
        assert!(!out.status.success(), "{text}");
        assert!(!text.contains("REDACTED_PATH"), "{text}");
        let _ = std::fs::remove_dir_all(&root);
    }
}

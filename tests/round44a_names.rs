//! Round 44a: the Windows name refusal, as the dispatcher words it and as
//! `--explain` and the archive extractors apply it.

use iron_shell::safety::windows_param_name_refusal;

#[test]
fn a_write_is_told_its_write_would_land_elsewhere_not_its_read() {
    let w = windows_param_name_refusal("file_write", "w.txt.", true).expect("refused");
    assert!(w.message.contains("the write would land"), "{}", w.message);
    let r = windows_param_name_refusal("file_read", "w.txt.", false).expect("refused");
    assert!(r.message.contains("the read would land"), "{}", r.message);
}

#[cfg(windows)]
mod archive_names {
    //! An entry Windows would not keep as written is refused by extraction
    //! (E_BAD_ARG, as `file.write` refuses it) and `archive_test` says false
    //! for it, by the same per-entry check.

    use std::io::Write as _;
    use std::process::{Command, Output};

    fn said(out: &Output) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    fn ish(dir: &std::path::Path, expr: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-c", expr])
            .current_dir(dir)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", dir.join("..").join("session"))
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_WORKSPACE")
            .output()
            .expect("run ish")
    }

    fn zip_of(dir: &std::path::Path, name: &str, entry: &str) {
        let file = std::fs::File::create(dir.join(name)).unwrap();
        let mut w = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file(entry, opts).unwrap();
        w.write_all(b"x").unwrap();
        w.finish().unwrap();
    }

    #[test]
    fn a_name_windows_would_rename_is_refused_and_does_not_test_clean() {
        let root = std::env::temp_dir().join(format!("ae-r44a-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ws = root.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        for (i, entry) in ["a.txt:ads", "nul.txt", "x.txt.", "trail./x", "sp ace /x"]
            .iter()
            .enumerate()
        {
            let zip = format!("z{i}.zip");
            zip_of(&ws, &zip, entry);
            let tested = ish(&ws, &format!("archive_test('{zip}')"));
            assert_eq!(said(&tested).trim(), "false", "{entry}: {}", said(&tested));
            let dest = format!("out{i}");
            let out = ish(&ws, &format!("zip_extract('{zip}', '{dest}')"));
            let text = said(&out);
            assert!(!out.status.success(), "{entry}: {text}");
            assert!(text.contains("E_BAD_ARG"), "{entry}: {text}");
            assert!(
                !ws.join(&dest).exists()
                    || std::fs::read_dir(ws.join(&dest)).unwrap().next().is_none(),
                "{entry}: something was extracted"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn a_separator_after_a_file_name_is_refused_only_for_a_file() {
    let dir = std::env::temp_dir().join(format!("ae-r44a-sep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("d")).unwrap();
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    let file = dir.join("a.txt").to_string_lossy().into_owned();
    let folder = dir.join("d").to_string_lossy().into_owned();
    for tail in ["\\", "/", "\\.", "/."] {
        let e =
            iron_shell::safety::windows_trailing_separator_refusal("cat", &format!("{file}{tail}"))
                .unwrap_or_else(|| panic!("a.txt{tail} must be refused"));
        assert!(e.message.contains("separator"), "{}", e.message);
        assert!(
            iron_shell::safety::windows_trailing_separator_refusal(
                "cat",
                &format!("{folder}{tail}")
            )
            .is_none(),
            "a folder may end in {tail}"
        );
    }
    assert!(iron_shell::safety::windows_trailing_separator_refusal("cat", &file).is_none());
    let missing = dir.join("none").to_string_lossy().into_owned();
    assert!(
        iron_shell::safety::windows_trailing_separator_refusal("cat", &format!("{missing}\\"))
            .is_none(),
        "a missing name is left to the open"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(windows)]
mod posix_readers {
    //! `ish -b` readers make the name check the builtins make, under the jail.

    use std::process::{Command, Output};

    fn said(out: &Output) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    #[test]
    fn cat_refuses_a_renamed_file_as_file_read_does() {
        let root = std::env::temp_dir().join(format!("ae-r44a-cat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ws = root.join("ws");
        std::fs::create_dir_all(ws.join("sub dir")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        std::fs::write(ws.join("a.txt"), "secret-a").unwrap();
        std::fs::write(ws.join("sub dir").join("x"), "secret-x").unwrap();
        let run = |args: &[&str]| -> Output {
            Command::new(env!("CARGO_BIN_EXE_ish"))
                .args(["--agent", "--workspace"])
                .arg(&ws)
                .args(args)
                .current_dir(&root)
                .env("NO_COLOR", "1")
                .env("IRONSHELL_SESSION_DIR", root.join("session"))
                .output()
                .expect("run ish")
        };
        for script in [
            "cat 'a.txt.'",
            "cat 'sub dir./x'",
            "cat 'a.txt\\'",
            "cat 'a.txt\\.'",
        ] {
            let out = run(&["-b", "-c", script]);
            let text = said(&out);
            assert!(!text.contains("secret-"), "{script} read the file: {text}");
            assert!(text.contains("E_BAD_ARG"), "{script}: {text}");
        }
        let out = run(&["-c", r#"file.read("a.txt\\")"#]);
        let text = said(&out);
        assert!(!text.contains("secret-a"), "{text}");
        assert!(text.contains("E_BAD_ARG"), "{text}");
        let out = run(&["-b", "-c", "cat a.txt"]);
        assert!(said(&out).contains("secret-a"), "{}", said(&out));
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(windows)]
mod explain_names {
    //! `--explain` predicts the Windows name refusals the run gives (E_BAD_ARG)
    //! and reports the run's own message, not an outside-the-workspace hint.

    use std::process::Command;

    fn explain(code: &str) -> serde_json::Value {
        let dir =
            std::env::temp_dir().join(format!("ae-r44a-explain-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["--explain", "-c", code])
            .current_dir(&dir)
            .env("IRONSHELL_SESSION_DIR", dir.join("session"))
            .env_remove("IRONSHELL_ALLOW_SH")
            .env_remove("IRONSHELL_WORKSPACE")
            .output()
            .unwrap();
        serde_json::from_slice(&out.stdout).expect("explain prints JSON")
    }

    #[test]
    fn a_name_the_run_refuses_is_deny_with_the_runs_message() {
        for (code, word) in [
            (r#"file.write("a.txt.", "x")"#, "strip"),
            (r#"file.write("b.txt ", "x")"#, "strip"),
            (r#"file.write("f.txt:s", "x")"#, "alternate data stream"),
            (r#"file.write("CON", "x")"#, "reserved device name"),
            (r#"file.write("a<b", "x")"#, "`<`"),
            (r#"file.read("a.txt.")"#, "strip"),
        ] {
            let e = explain(code);
            assert_eq!(e["decision"], "deny", "{code}: {e}");
            let hint = e["calls"][0]["hint"].as_str().unwrap_or_default();
            assert!(hint.contains(word), "{code}: {e}");
            assert!(!hint.contains("outside the workspace"), "{code}: {e}");
        }
    }

    #[test]
    fn content_that_reads_like_a_bad_name_is_not_judged_as_a_path() {
        let e = explain(r#"file.write("ok.txt", "a<b.")"#);
        assert_eq!(e["decision"], "allow", "{e}");
    }
}

#[test]
fn a_stream_name_is_refused_for_a_read_and_a_write_alike() {
    // Round 49c: a read of `name:stream` is refused as a write of it is.
    assert!(windows_param_name_refusal("file_write", "f.txt:s", true).is_some());
    assert!(windows_param_name_refusal("file_read", "f.txt:s", false).is_some());
    // A scheme or a drive is not a stream.
    assert!(windows_param_name_refusal("file_read", "https://x/y", false).is_none());
    assert!(windows_param_name_refusal("file_read", "C:\\x\\y.txt", false).is_none());
}

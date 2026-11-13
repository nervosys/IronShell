//! Round 38c: a Windows write is refused for a name Windows would not keep as
//! written: a device name (`NUL` as `CON`), a component ending in a dot or a
//! space, and an alternate data stream. One check, in `safety::guard`, so the
//! builtins and `ish -b` agree.

use iron_shell::safety::windows_name_refusal;

#[test]
fn the_name_check_names_each_kind_of_name() {
    for (path, word) in [
        (r"C:\ws\NUL", "device"),
        (r"C:\ws\nul.txt", "device"),
        (r"C:\ws\a.txt.", "strip"),
        (r"C:\ws\b.txt ", "strip"),
        (r"C:\ws\dir.\x", "strip"),
        (r"C:\ws\f.txt:stream", "alternate data stream"),
        (r"C:\ws\f.txt:stream:$DATA", "alternate data stream"),
        ("sub/f.txt:s", "alternate data stream"),
    ] {
        let e = windows_name_refusal("file_write", path)
            .unwrap_or_else(|| panic!("{path} must be refused"));
        let said = e.to_string();
        assert!(said.contains(word), "{path}: {said}");
    }
}

#[test]
fn ordinary_names_and_the_drive_prefix_pass() {
    for path in [
        r"C:\ws\a.txt",
        r"C:\ws\sub\..\b.txt",
        r"\\?\C:\ws\a.txt",
        "./a.txt",
        "dir/",
        "a.b.c",
        ".hidden",
        "..",
    ] {
        assert!(
            windows_name_refusal("file_write", path).is_none(),
            "{path} must pass"
        );
    }
}

#[cfg(windows)]
mod cli {
    use std::process::{Command, Output};

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

    fn workspace(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("ae-round38c-win-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ws = root.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        ws
    }

    #[test]
    fn nul_is_a_device_name_like_con_not_outside_the_workspace() {
        let ws = workspace("nul");
        for expr in [
            r#"file.write("NUL", "x")"#,
            r#"touch("NUL")"#,
            r#"file.write("CON", "x")"#,
        ] {
            let out = ish(&ws, &["-c", expr]);
            let text = said(&out);
            assert!(!out.status.success(), "{expr}: {text}");
            assert!(text.contains("E_BAD_ARG"), "{expr}: {text}");
            assert!(text.contains("reserved device name"), "{expr}: {text}");
            assert!(!text.contains("E_OUTSIDE_WORKSPACE"), "{expr}: {text}");
        }
        let _ = std::fs::remove_dir_all(ws.parent().unwrap());
    }

    #[test]
    fn a_trailing_dot_or_space_is_refused_not_stripped_onto_another_file() {
        let ws = workspace("dot");
        std::fs::write(ws.join("a.txt"), "keep").unwrap();
        let out = ish(&ws, &["-c", r#"file.write("a.txt.", "clobber")"#]);
        let text = said(&out);
        assert!(!out.status.success(), "{text}");
        assert!(text.contains("E_BAD_ARG"), "{text}");
        assert!(text.contains("strip"), "{text}");
        assert_eq!(std::fs::read_to_string(ws.join("a.txt")).unwrap(), "keep");
        let _ = std::fs::remove_dir_all(ws.parent().unwrap());
    }

    #[test]
    fn an_alternate_data_stream_is_refused_by_builtins_and_by_ish_b_alike() {
        let ws = workspace("ads");
        for args in [
            vec!["-c", r#"file.write("f.txt:s", "x")"#],
            vec!["-c", r#"touch("f.txt:s")"#],
            vec!["-b", "-c", "echo x > f.txt:s"],
        ] {
            let out = ish(&ws, &args);
            let text = said(&out);
            assert!(!out.status.success(), "{args:?}: {text}");
            assert!(text.contains("alternate data stream"), "{args:?}: {text}");
        }
        assert!(!ws.join("f.txt").exists(), "no file and no stream was made");
        let _ = std::fs::remove_dir_all(ws.parent().unwrap());
    }
}

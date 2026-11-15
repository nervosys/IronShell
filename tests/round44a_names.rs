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
fn a_stream_name_is_left_to_the_write_guard() {
    // The dispatcher lets `name:stream` through for a read and a write alike;
    // `safety::guard` refuses it for a write.
    assert!(windows_param_name_refusal("file_write", "f.txt:s", true).is_none());
    assert!(windows_param_name_refusal("file_read", "f.txt:s", false).is_none());
}

//! Round 53c: `zip_extract` and `tar_extract` are capped. A 305 KB archive of a
//! 300 MB file used to extract in full. The limits are 1 GiB of uncompressed
//! bytes (counted as written), 100000 entries, and for a zip entry a 1000:1
//! ratio over 100 MB; the operator lowers the first two with
//! `IRONSHELL_EXTRACT_MAX_BYTES` and `IRONSHELL_EXTRACT_MAX_ENTRIES`.
//!
//! These tests lower the limits instead of building a large archive: every
//! archive here is a few kilobytes.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round53c_extract_limits")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("session")).expect("session");
    dir
}

fn lit(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn zip_of(path: &Path, files: usize, each: usize) {
    let mut w = zip::ZipWriter::new(std::fs::File::create(path).expect("zip"));
    let opts =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for i in 0..files {
        w.start_file(format!("f{i}.txt"), opts).expect("start");
        w.write_all(&vec![b'a'; each]).expect("write");
    }
    w.finish().expect("finish");
}

fn tar_gz_of(path: &Path, files: usize, each: usize) {
    let gz = flate2::write::GzEncoder::new(
        std::fs::File::create(path).expect("tgz"),
        flate2::Compression::default(),
    );
    let mut b = tar::Builder::new(gz);
    for i in 0..files {
        let mut h = tar::Header::new_gnu();
        h.set_size(each as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, format!("f{i}.txt"), &vec![b'a'; each][..])
            .expect("append");
    }
    b.into_inner().expect("tar").finish().expect("gz");
}

fn ish(dir: &Path, code: &str, env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ish"));
    c.args(["--deterministic", "-c", code])
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_AGENT")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_EXTRACT_MAX_BYTES")
        .env_remove("IRONSHELL_EXTRACT_MAX_ENTRIES");
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("run ish")
}

fn all_text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn files_under(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).count())
        .unwrap_or(0)
}

#[test]
fn a_zip_over_the_byte_limit_is_refused_and_leaves_nothing() {
    let dir = scratch("zip_bytes");
    let archive = dir.join("a.zip");
    zip_of(&archive, 3, 600);
    let out = dir.join("out");
    let code = format!(r#"zip_extract("{}", "{}")"#, lit(&archive), lit(&out));
    let o = ish(&dir, &code, &[("IRONSHELL_EXTRACT_MAX_BYTES", "1000")]);
    let text = all_text(&o);
    assert!(text.contains("E_BUDGET_EXCEEDED"), "{text}");
    assert!(text.contains("1000"), "the limit is named: {text}");
    assert_eq!(files_under(&out), 0, "a partial extraction was left");
    // At the limit it extracts.
    let o = ish(&dir, &code, &[("IRONSHELL_EXTRACT_MAX_BYTES", "1800")]);
    assert!(o.status.success(), "{}", all_text(&o));
    assert_eq!(files_under(&out), 3);
}

#[test]
fn a_tar_over_the_byte_limit_is_refused_and_leaves_nothing() {
    let dir = scratch("tar_bytes");
    let archive = dir.join("a.tar.gz");
    tar_gz_of(&archive, 3, 600);
    let out = dir.join("out");
    let code = format!(r#"tar_extract("{}", "{}")"#, lit(&archive), lit(&out));
    let o = ish(&dir, &code, &[("IRONSHELL_EXTRACT_MAX_BYTES", "1000")]);
    let text = all_text(&o);
    assert!(text.contains("E_BUDGET_EXCEEDED"), "{text}");
    assert_eq!(files_under(&out), 0, "a partial extraction was left");
    let o = ish(&dir, &code, &[("IRONSHELL_EXTRACT_MAX_BYTES", "1800")]);
    assert!(o.status.success(), "{}", all_text(&o));
    assert_eq!(files_under(&out), 3);
}

#[test]
fn an_archive_over_the_entry_limit_is_refused() {
    let dir = scratch("entries");
    let z = dir.join("a.zip");
    let t = dir.join("a.tar.gz");
    zip_of(&z, 10, 1);
    tar_gz_of(&t, 10, 1);
    let env = [("IRONSHELL_EXTRACT_MAX_ENTRIES", "5")];
    for (call, archive, out) in [
        ("zip_extract", &z, dir.join("zout")),
        ("tar_extract", &t, dir.join("tout")),
    ] {
        let code = format!(r#"{call}("{}", "{}")"#, lit(archive), lit(&out));
        let o = ish(&dir, &code, &env);
        let text = all_text(&o);
        assert!(text.contains("E_BUDGET_EXCEEDED"), "{call}: {text}");
        assert!(text.contains("5 entries"), "{call} names the limit: {text}");
        assert_eq!(files_under(&out), 0, "{call} left files");
    }
}

#[test]
fn the_defaults_let_an_ordinary_archive_through() {
    let dir = scratch("defaults");
    let archive = dir.join("a.zip");
    zip_of(&archive, 10, 100);
    let out = dir.join("out");
    let code = format!(r#"zip_extract("{}", "{}")"#, lit(&archive), lit(&out));
    let o = ish(&dir, &code, &[]);
    assert!(o.status.success(), "{}", all_text(&o));
    assert_eq!(files_under(&out), 10);
}

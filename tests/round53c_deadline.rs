//! Round 53c: `--timeout` reaches the per-entry loops of archive extraction,
//! `glob` and the directory walks. The evaluator samples the clock every 1024
//! steps, and one builtin call is one step, so a 4000-entry extraction ran for
//! 45 s against `--timeout 20`. Each loop now reads the clock itself and fails
//! with `E_BUDGET_EXCEEDED`, the error `sleep` and the evaluator raise.
//!
//! The deadline is entered already expired, so the first check in each loop
//! fires; the archives and trees are a few entries.

use iron_shell::{env::Env, eval, parser, safety};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn run(code: &str) -> anyhow::Result<iron_shell::value::Value> {
    let stmts = parser::parse_program(code)?;
    let mut env = Env::default();
    eval::eval_program(&stmts, &mut env)
}

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round53c_deadline")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

fn lit(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Runs `code` under an expired deadline and returns the error text; panics
/// (with the type, never the value) when it succeeds.
fn expired(code: &str) -> String {
    let _guard = safety::enter_deadline(Duration::ZERO);
    match run(code) {
        Ok(v) => panic!("ran to completion past the deadline: {}", v.type_name()),
        Err(e) => format!("{e}"),
    }
}

fn small_zip(path: &Path) {
    let file = std::fs::File::create(path).expect("zip");
    let mut w = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    for i in 0..20 {
        w.start_file(format!("f{i}.txt"), opts).expect("start");
        w.write_all(b"x").expect("write");
    }
    w.finish().expect("finish");
}

#[test]
fn zip_extract_stops_at_the_deadline_and_writes_nothing() {
    let dir = scratch("zip");
    let archive = dir.join("a.zip");
    small_zip(&archive);
    let out = dir.join("out");
    let msg = expired(&format!(
        r#"zip_extract("{}", "{}")"#,
        lit(&archive),
        lit(&out)
    ));
    assert!(msg.contains("E_BUDGET_EXCEEDED"), "{msg}");
    assert!(!out.join("f0.txt").exists(), "an entry was written");
}

#[test]
fn tar_extract_stops_at_the_deadline_and_writes_nothing() {
    let dir = scratch("tar");
    let archive = dir.join("a.tar");
    {
        let mut b = tar::Builder::new(std::fs::File::create(&archive).expect("tar"));
        for i in 0..20 {
            let mut h = tar::Header::new_gnu();
            h.set_size(1);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, format!("f{i}.txt"), &b"x"[..])
                .expect("append");
        }
        b.finish().expect("finish");
    }
    let out = dir.join("out");
    let msg = expired(&format!(
        r#"tar_extract("{}", "{}")"#,
        lit(&archive),
        lit(&out)
    ));
    assert!(msg.contains("E_BUDGET_EXCEEDED"), "{msg}");
    assert!(!out.join("f0.txt").exists(), "an entry was written");
}

#[test]
fn glob_and_fs_walk_stop_at_the_deadline() {
    let dir = scratch("tree");
    for d in 0..5 {
        let sub = dir.join(format!("d{d}"));
        std::fs::create_dir_all(&sub).expect("sub");
        std::fs::write(sub.join("f.txt"), "x").expect("file");
    }
    let root = lit(&dir);
    let msg = expired(&format!(r#"glob("{root}/**/*")"#));
    assert!(msg.contains("E_BUDGET_EXCEEDED"), "glob: {msg}");
    let msg = expired(&format!(r#"fs_walk("{root}")"#));
    assert!(msg.contains("E_BUDGET_EXCEEDED"), "fs_walk: {msg}");
}

#[test]
fn without_a_deadline_the_same_calls_succeed() {
    let dir = scratch("ok");
    let archive = dir.join("a.zip");
    small_zip(&archive);
    let out = dir.join("out");
    run(&format!(
        r#"zip_extract("{}", "{}")"#,
        lit(&archive),
        lit(&out)
    ))
    .expect("extracts");
    assert!(out.join("f19.txt").exists());
}

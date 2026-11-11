//! Correctness defects found by an agent tester (round 15).
//!
//! `if` desugars to a two-arm match on `true`. A condition that is not a Bool
//! (a Record from `exists()`, an Int, a String) used to take the else branch
//! silently; it is now an E_BAD_ARG. Null stays false, and `where` keeps its
//! truthiness.

use iron_shell::value::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh scratch directory for the CLI tests: `proj` is the workspace and
/// `session` holds the journal, so no test shares state with another.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round15_correctness")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("proj")).expect("workspace dir");
    std::fs::create_dir_all(dir.join("session")).expect("session dir");
    dir
}

/// Run `code` in agent mode with `dir/proj` as the workspace, optionally with
/// `approve` as the operator's `IRONSHELL_APPROVE` list.
fn ish_agent(dir: &Path, code: &str, approve: Option<&str>) -> Output {
    let ws = dir.join("proj");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(["--agent", "--workspace"])
        .arg(&ws)
        .args(["-c", code])
        .current_dir(&ws)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_APPROVE_ALL")
        .env_remove("IRONSHELL_APPROVE")
        .env_remove("IRONSHELL_WORKSPACE");
    if let Some(token) = approve {
        cmd.env("IRONSHELL_APPROVE", token);
    }
    cmd.output().expect("run ish")
}

/// The structured refusal on the first stderr line.
fn first_error(out: &Output) -> serde_json::Value {
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().next().unwrap_or("");
    serde_json::from_str(line).unwrap_or_else(|e| panic!("not JSON ({e}): {err:?}"))
}

/// The approval token a refused call carries.
fn token_from_refusal(out: &Output) -> String {
    let json = first_error(out);
    assert_eq!(json["error"]["code"], "E_NEEDS_APPROVAL", "{json}");
    json["error"]["approval"]["token"]
        .as_str()
        .expect("the refusal carries a token")
        .to_string()
}

/// Run `code` by `ish -c` with `dir` as the working directory.
fn ish_in(dir: &Path, code: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", code])
        .current_dir(dir)
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_AGENT")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_APPROVE")
        .output()
        .expect("run ish")
}

fn run(src: &str) -> Result<Value, String> {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).map_err(|e| format!("{e:#}"))
}

// ── Item 1: a non-Bool `if` condition is an error, not a silent else ──

#[test]
fn an_if_on_a_record_is_an_error_not_a_silent_else() {
    let err = run(r#"r = {exists: true}; if r { "yes" } else { "no" }"#)
        .expect_err("a Record condition must be refused");
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(err.contains("Bool"), "{err}");
    assert!(
        err.contains("exists(p).exists") || err.contains("field"),
        "the hint says to test a field: {err}"
    );
}

#[test]
fn an_if_on_an_int_is_an_error() {
    let err = run(r#"n = 0; if n { "yes" } else { "no" }"#).expect_err("an Int is refused");
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(err.contains("n != 0"), "{err}");
}

#[test]
fn an_if_on_a_string_is_an_error() {
    let err = run(r#"s = ""; if s { "yes" } else { "no" }"#).expect_err("a String is refused");
    assert!(err.contains("E_BAD_ARG"), "{err}");
}

#[test]
fn an_if_on_an_array_is_an_error() {
    let err = run(r#"a = []; if a { "yes" } else { "no" }"#).expect_err("an Array is refused");
    assert!(err.contains("E_BAD_ARG"), "{err}");
    assert!(err.contains("len(a) > 0"), "{err}");
}

#[test]
fn an_if_on_null_stays_false() {
    assert_eq!(
        run(r#"if null { "yes" } else { "no" }"#),
        Ok(Value::Str("no".into()))
    );
    assert_eq!(run("if null { 1 }"), Ok(Value::Null));
}

#[test]
fn an_if_on_a_bool_and_a_comparison_still_work() {
    assert_eq!(
        run(r#"if 1 < 2 { "yes" } else { "no" }"#),
        Ok(Value::Str("yes".into()))
    );
    assert_eq!(
        run(r#"if false { "yes" } else if true { "second" } else { "no" }"#),
        Ok(Value::Str("second".into()))
    );
}

#[test]
fn an_if_comparing_a_record_field_works() {
    assert_eq!(
        run(r#"r = {exists: true}; if r.exists { "yes" } else { "no" }"#),
        Ok(Value::Str("yes".into()))
    );
}

// ── Item 2: a .tar.gz or .tgz archive is gzipped, and reads back ──

fn write_member(dir: &Path) {
    std::fs::create_dir_all(dir.join("proj").join("src")).unwrap();
    std::fs::write(dir.join("proj").join("src").join("a.txt"), "alpha\n").unwrap();
}

#[test]
fn tar_create_gzips_a_tar_gz_archive_and_round_trips() {
    let dir = scratch("tar_gz_round_trip");
    write_member(&dir);
    let proj = dir.join("proj");
    let created = ish_in(&proj, r#"tar_create("pack.tar.gz", "src")"#);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let bytes = std::fs::read(proj.join("pack.tar.gz")).unwrap();
    assert_eq!(&bytes[..2], &[0x1f, 0x8b], "the archive is gzip");

    let listed = ish_in(&proj, r#"tar_list("pack.tar.gz")"#);
    assert!(
        listed.status.success(),
        "tar_list must read it back: {}",
        String::from_utf8_lossy(&listed.stderr)
    );
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains("a.txt"),
        "{}",
        String::from_utf8_lossy(&listed.stdout)
    );

    let extracted = ish_in(&proj, r#"tar_extract("pack.tar.gz", "out")"#);
    assert!(
        extracted.status.success(),
        "{}",
        String::from_utf8_lossy(&extracted.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(proj.join("out").join("src").join("a.txt")).unwrap(),
        "alpha\n"
    );
}

#[test]
fn tar_create_writes_plain_tar_for_a_tar_name() {
    let dir = scratch("tar_plain");
    write_member(&dir);
    let proj = dir.join("proj");
    let created = ish_in(&proj, r#"tar_create("pack.tar", "src")"#);
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let bytes = std::fs::read(proj.join("pack.tar")).unwrap();
    assert_ne!(&bytes[..2], &[0x1f, 0x8b], "a .tar name is not gzipped");
    assert_eq!(&bytes[257..262], b"ustar", "a plain tar header");
}

#[test]
fn where_still_takes_a_truthy_value() {
    // where/&&/||/! keep truthiness: only `if` is strict.
    assert_eq!(
        run("[0, 1, 2] | where(fn(x) => x)"),
        Ok(Value::Array(vec![Value::Int(1), Value::Int(2)]))
    );
}

// ── Item 3: undo of a move puts the source back, as one step ──

#[test]
fn undo_of_a_move_puts_back_the_source_and_the_overwritten_destination() {
    let dir = scratch("mv_overwrite_undo");
    let ws = dir.join("proj");
    std::fs::write(ws.join("a.txt"), "new\n").unwrap();
    std::fs::write(ws.join("b.txt"), "old\n").unwrap();

    let first = ish_agent(&dir, r#"file_move("a.txt", "b.txt")"#, None);
    assert!(!first.status.success(), "the first move must be refused");
    let json = first_error(&first);
    assert_eq!(
        json["error"]["approval"]["blast_radius"]["overwrites"], true,
        "a move onto an existing file says so: {json}"
    );
    let token = token_from_refusal(&first);
    assert!(ws.join("a.txt").exists(), "a refused move must not move");

    let moved = ish_agent(&dir, r#"file_move("a.txt", "b.txt")"#, Some(&token));
    assert!(
        moved.status.success(),
        "the approved move must run: {}",
        String::from_utf8_lossy(&moved.stderr)
    );
    assert!(!ws.join("a.txt").exists());
    assert_eq!(std::fs::read_to_string(ws.join("b.txt")).unwrap(), "new\n");

    let undone = ish_agent(&dir, "undo()", None);
    assert!(
        undone.status.success(),
        "undo must run: {}",
        String::from_utf8_lossy(&undone.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("a.txt")).expect("undo puts the source back"),
        "new\n"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("b.txt")).unwrap(),
        "old\n",
        "and restores the overwritten destination"
    );
}

#[test]
fn undo_of_a_move_to_a_new_name_removes_the_destination_and_restores_the_source() {
    let dir = scratch("mv_new_name_undo");
    let ws = dir.join("proj");
    std::fs::write(ws.join("a.txt"), "only\n").unwrap();

    let first = ish_agent(&dir, r#"file_move("a.txt", "c.txt")"#, None);
    let token = token_from_refusal(&first);
    let json = first_error(&first);
    assert!(
        json["error"]["approval"]["blast_radius"]["overwrites"].is_null(),
        "a move to a new name overwrites nothing: {json}"
    );
    let moved = ish_agent(&dir, r#"file_move("a.txt", "c.txt")"#, Some(&token));
    assert!(moved.status.success());

    let undone = ish_agent(&dir, "undo()", None);
    assert!(undone.status.success());
    assert_eq!(std::fs::read_to_string(ws.join("a.txt")).unwrap(), "only\n");
    assert!(!ws.join("c.txt").exists(), "undo removes the new name");
}

#[test]
fn redo_of_an_undone_move_applies_both_ends_again() {
    let dir = scratch("mv_redo");
    let ws = dir.join("proj");
    std::fs::write(ws.join("a.txt"), "new\n").unwrap();
    std::fs::write(ws.join("b.txt"), "old\n").unwrap();
    let token = token_from_refusal(&ish_agent(&dir, r#"file_move("a.txt", "b.txt")"#, None));
    assert!(
        ish_agent(&dir, r#"file_move("a.txt", "b.txt")"#, Some(&token))
            .status
            .success()
    );
    assert!(ish_agent(&dir, "undo()", None).status.success());
    assert!(ws.join("a.txt").exists());

    let redone = ish_agent(&dir, "session_redo()", None);
    assert!(
        redone.status.success(),
        "{}",
        String::from_utf8_lossy(&redone.stderr)
    );
    assert!(!ws.join("a.txt").exists(), "redo moves the source again");
    assert_eq!(std::fs::read_to_string(ws.join("b.txt")).unwrap(), "new\n");
}

// ── Item 4: undo of a directory removal says it is not journaled ──

#[test]
fn undo_of_a_recursive_rmdir_says_the_removal_is_not_journaled() {
    let dir = scratch("rmdir_honest");
    let ws = dir.join("proj");
    std::fs::create_dir_all(ws.join("tree")).unwrap();
    std::fs::write(ws.join("tree").join("x.txt"), "kept\n").unwrap();

    let first = ish_agent(&dir, r#"rmdir("tree", true)"#, None);
    assert!(!first.status.success(), "the first removal must be refused");
    let json = first_error(&first);
    assert_eq!(json["error"]["approval"]["reversible"], false, "{json}");
    let token = token_from_refusal(&first);

    let removed = ish_agent(&dir, r#"rmdir("tree", true)"#, Some(&token));
    assert!(
        removed.status.success(),
        "the approved removal must run: {}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(!ws.join("tree").exists());

    let undone = ish_agent(&dir, "undo()", None);
    assert!(
        undone.status.success(),
        "undo must run: {}",
        String::from_utf8_lossy(&undone.stderr)
    );
    let said = String::from_utf8_lossy(&undone.stdout);
    assert!(
        said.contains("a directory removal is not journaled"),
        "undo names the real reason: {said}"
    );
    assert!(
        !said.contains("was absent before"),
        "the removed tree was present before: {said}"
    );
    assert!(
        !ws.join("tree").exists(),
        "undo does not restore a directory, and says so"
    );
}

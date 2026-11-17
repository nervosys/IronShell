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

/// `ish -b -c script` with an empty PATH, so no GNU program can answer.
fn ish_b(dir: &Path, script: &str) -> Output {
    let empty = dir.join("empty-path");
    std::fs::create_dir_all(&empty).expect("empty path dir");
    Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-b", "-c", script])
        .current_dir(dir.join("proj"))
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env("PATH", &empty)
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_AGENT")
        .env_remove("IRONSHELL_WORKSPACE")
        .output()
        .expect("run ish")
}

#[cfg(windows)]
#[test]
fn b2_find_refusal_names_the_operand_that_is_unsupported() {
    let dir = scratch("b2");
    let o = ish_b(&dir, r#"find src -name "*.rs" -o -name "*.py""#);
    let text = out(&o);
    assert!(text.contains("does not support -o;"), "{text}");
    let o = ish_b(&dir, r#"find src -type f -name "*.rs" -exec wc -l {} +"#);
    let text = out(&o);
    assert!(text.contains("does not support -exec;"), "{text}");
}

#[test]
fn b3_tar_extract_of_a_zip_says_zip_and_never_a_signal() {
    let dir = scratch("b3");
    let made = ish_in(&dir, r#"zip_create("a.zip", "src")"#);
    assert!(made.status.success(), "{}", out(&made));
    let o = ish_in(&dir, r#"tar_extract("a.zip", "ex")"#);
    let text = out(&o);
    assert!(!o.status.success(), "{text}");
    assert!(text.contains("zip archive"), "{text}");
    assert!(text.contains("zip_extract"), "{text}");
    assert!(!text.contains("signal"), "{text}");
    // A damaged tar is a read error, not a killed process.
    std::fs::write(dir.join("proj").join("bad.tar"), vec![b'x'; 2048]).expect("bad tar");
    let o = ish_in(&dir, r#"tar_extract("bad.tar", "ex2")"#);
    let text = out(&o);
    assert!(!o.status.success(), "{text}");
    assert!(!text.contains("signal"), "{text}");
}

#[test]
fn b4_zip_create_of_dot_does_not_store_itself_and_zip_list_has_path_and_kind() {
    let dir = scratch("b4");
    std::fs::create_dir_all(dir.join("proj").join("out")).expect("out dir");
    let made = ish_in(&dir, r#"zip_create("out/r.zip", ".")"#);
    assert!(made.status.success(), "{}", out(&made));
    let o = ish_in(
        &dir,
        r#"zip_list("out/r.zip") | map(fn(e) => e.path + ":" + e.kind) | join(",")"#,
    );
    let text = out(&o);
    assert!(o.status.success(), "{text}");
    assert!(text.contains("src/main.rs:file"), "{text}");
    assert!(
        text.contains("src:dir") || text.contains("src/:dir"),
        "{text}"
    );
    assert!(!text.contains("r.zip"), "{text}");
    // zip_add of the folder holding the archive skips the archive too.
    let o = ish_in(&dir, r#"zip_add("out/r.zip", "out")"#);
    assert!(o.status.success(), "{}", out(&o));
    let o = ish_in(
        &dir,
        r#"zip_list("out/r.zip") | map(fn(e) => e.name) | join(",")"#,
    );
    assert!(!out(&o).contains("r.zip"), "{}", out(&o));
}

#[test]
fn b6_file_replace_reports_the_path_as_written_in_both_modes() {
    let dir = scratch("b6");
    let src = "file.replace(\"src/main.rs\", \"main\", \"main2\").path";
    for agent in [false, true] {
        let o = ish_with(&dir, &["-c", src], agent);
        let text = out(&o);
        assert!(o.status.success(), "{text}");
        assert!(text.contains("src/main.rs"), "{text}");
        assert!(!text.contains("round50b_tools"), "{text}");
        // Put the text back for the next mode.
        let o = ish_with(
            &dir,
            &["-c", "file.replace(\"src/main.rs\", \"main2\", \"main\")"],
            agent,
        );
        assert!(o.status.success(), "{}", out(&o));
    }
}

#[test]
fn b7_journal_rows_carry_the_step_of_their_call() {
    let dir = scratch("b7");
    let code = r#"file.write("a.txt", "x"); file.write("b.txt", "y"); mv("a.txt", "c.txt"); journal().entries | map(fn(e) => "${e.builtin}:${e.step}") | join(",")"#;
    let o = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["-c", code])
        .current_dir(dir.join("proj"))
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("IRONSHELL_JOURNAL", "on")
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_AGENT")
        .env_remove("IRONSHELL_WORKSPACE")
        .output()
        .expect("run ish");
    let text = out(&o);
    assert!(o.status.success(), "{text}");
    // Every row has a step; the move's two rows share one.
    let rows: Vec<&str> = text.trim().trim_matches('"').split(',').collect();
    assert!(rows.len() >= 3, "{text}");
    let steps: Vec<&str> = rows.iter().filter_map(|r| r.rsplit(':').next()).collect();
    assert_eq!(steps.len(), rows.len(), "{text}");
    let moves: Vec<&&str> = rows
        .iter()
        .filter(|r| r.starts_with("mv:") || r.starts_with("file_move:"))
        .collect();
    if moves.len() == 2 {
        assert_eq!(moves[0], moves[1], "{text}");
    }
}

#[test]
fn b8_unmatched_glob_operand_reads_as_gnu_does() {
    let dir = scratch("b8a");
    let o = ish_b(&dir, "wc -l *.nomatch_b8");
    let text = out(&o);
    assert!(
        text.contains("wc: '*.nomatch_b8': No such file or directory"),
        "{text}"
    );
    assert_eq!(o.status.code(), Some(1), "{text}");
    let o = ish_b(&dir, "cat nosuch_b8.txt");
    assert!(
        out(&o).contains("cat: nosuch_b8.txt: No such file or directory"),
        "{}",
        out(&o)
    );
}

#[test]
fn b8_missing_cp_names_the_subset() {
    let dir = scratch("b8b");
    for cmd in ["cp a b", "mv a b", "rm a", "xargs echo", "unzip a.zip"] {
        let o = ish_b(&dir, cmd);
        let text = out(&o);
        let name = cmd.split(' ').next().unwrap();
        assert!(
            text.contains(&format!("{name}: command not found")),
            "{text}"
        );
        assert!(text.contains("in-process subset"), "{text}");
        assert_eq!(o.status.code(), Some(127), "{text}");
    }
}

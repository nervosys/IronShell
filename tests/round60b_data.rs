//! Round 60b: data and editing findings.
//!
//! Each test runs `ish` as a program, in a scratch working directory with its
//! own session directory, so the undo journal is never the real one.

use std::path::PathBuf;
use std::process::Command;

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let root = std::env::temp_dir().join(format!("ish-r60b-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cwd")).unwrap();
        std::fs::create_dir_all(root.join("session")).unwrap();
        Scratch { root }
    }

    fn cwd(&self) -> PathBuf {
        self.root.join("cwd")
    }

    fn put(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.cwd().join(name), bytes).unwrap();
    }

    fn get(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.cwd().join(name)).unwrap()
    }

    /// Canonical JSON of the value, or the error JSON from stderr.
    fn json(&self, expr: &str) -> String {
        let out = Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["--deterministic", "-c", expr])
            .current_dir(self.cwd())
            .env("NO_COLOR", "1")
            .env("IRONSHELL_JOURNAL", "on")
            .env_remove("IRONSHELL_ALLOW_SH")
            .env_remove("IRONSHELL_APPROVE")
            .env("IRONSHELL_SESSION_DIR", self.root.join("session"))
            .output()
            .expect("run ish");
        let text = if out.status.success() {
            String::from_utf8_lossy(&out.stdout).to_string()
        } else {
            String::from_utf8_lossy(&out.stderr).to_string()
        };
        text.trim().to_string()
    }

    fn code(&self, expr: &str) -> String {
        self.json(&format!("try {{ {expr} }} catch e {{ e.error.code }}"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// ---- 1. session_redo does not overwrite a file changed after the undo ----

#[test]
fn redo_refuses_a_file_edited_after_the_undo() {
    let s = Scratch::new("redo1");
    s.put("t.txt", b"a b c\n");
    s.json(r#"file.replace("t.txt", "a", "A")"#);
    assert_eq!(s.get("t.txt"), b"A b c\n");
    s.json("undo()");
    assert_eq!(s.get("t.txt"), b"a b c\n");
    s.put("t.txt", b"my edit\n");
    let err = s.json("session_redo()");
    assert!(err.contains("E_BAD_STATE"), "{err}");
    assert!(err.contains("t.txt"), "{err}");
    assert!(err.contains("changed after the undo"), "{err}");
    assert_eq!(s.get("t.txt"), b"my edit\n", "the edit survives");
    // The step stays on the redo stack, and force re-applies it.
    let forced = s.json("session_redo({force: true}).reapplied");
    assert_eq!(forced, "1");
    assert_eq!(s.get("t.txt"), b"A b c\n");
}

#[test]
fn redo_of_an_untouched_file_still_works_in_one_and_in_several_steps() {
    let s = Scratch::new("redo2");
    s.put("t.txt", b"a b c\n");
    s.json(r#"file.replace("t.txt", "a", "A")"#);
    s.json(r#"file.replace("t.txt", "b", "B")"#);
    s.json("undo(2)");
    assert_eq!(s.get("t.txt"), b"a b c\n");
    assert_eq!(s.json("session_redo(2).reapplied"), "2");
    assert_eq!(s.get("t.txt"), b"A B c\n");
    // Undo then redo again, the redo being itself journaled.
    s.json("undo()");
    assert_eq!(s.json("session_redo().reapplied"), "1");
    assert_eq!(s.get("t.txt"), b"A B c\n");
}

#[test]
fn redo_of_several_steps_stops_at_the_changed_file_and_lists_it() {
    let s = Scratch::new("redo3");
    s.put("one.txt", b"1\n");
    s.put("two.txt", b"2\n");
    s.json(r#"file.replace("one.txt", "1", "one")"#);
    s.json(r#"file.replace("two.txt", "2", "two")"#);
    s.json("undo(2)");
    // two.txt was undone first, so it is redone last: change it. The first
    // redo (one.txt) goes through and the second is listed as skipped.
    s.put("two.txt", b"mine\n");
    let rec = s.json("let r = session_redo(2); [r.reapplied, r.skipped, r.complete]");
    assert_eq!(rec, "[1,1,false]", "{rec}");
    assert_eq!(s.get("one.txt"), b"one\n");
    assert_eq!(s.get("two.txt"), b"mine\n");
}

#[test]
fn redo_is_refused_when_the_file_was_removed_after_the_undo() {
    let s = Scratch::new("redo4");
    s.put("t.txt", b"a\n");
    s.json(r#"file.replace("t.txt", "a", "b")"#);
    s.json("undo()");
    std::fs::remove_file(s.cwd().join("t.txt")).unwrap();
    let err = s.json("session_redo()");
    assert!(err.contains("E_BAD_STATE"), "{err}");
    assert!(err.contains("removed after the undo"), "{err}");
}

#[test]
fn undo_by_id_then_redo_is_guarded_the_same_way() {
    let s = Scratch::new("redo5");
    s.put("t.txt", b"a b c\n");
    s.json(r#"file.replace("t.txt", "a", "A")"#);
    let id = s.json("journal().entries | first | .id");
    s.json(&format!("undo({{id: {id}}})"));
    s.put("t.txt", b"hand edit\n");
    assert_eq!(s.code("session_redo()"), r#""E_BAD_STATE""#);
    assert_eq!(s.get("t.txt"), b"hand edit\n");
}

// ---- 2. sql sum/total/avg refuse text that is not a clean number ----

#[test]
fn sql_sum_refuses_text_that_sqlite_would_read_by_prefix() {
    let s = Scratch::new("sql1");
    for text in [
        "1,234.50", "2.000,10", "1 234", "$5", "1,234", "12x", "abc", "",
    ] {
        let q = format!(
            r#"try {{ [{{p: "{text}"}}] | sql("select sum(p) s from t") }} catch e {{ [e.error.code, e.error.message] }}"#
        );
        let out = s.json(&q);
        assert!(out.contains("E_BAD_ARG"), "{text:?}: {out}");
        assert!(out.contains("is not a clean number"), "{text:?}: {out}");
        assert!(out.contains(&format!("'{text}'")), "{text:?}: {out}");
    }
}

#[test]
fn sql_aggregate_error_has_a_hint_about_cleaning() {
    let s = Scratch::new("sql2");
    let hint =
        s.json(r#"try { [{p: "$5"}] | sql("select avg(p) a from t") } catch e { e.error.hint }"#);
    assert!(hint.contains("replace("), "{hint}");
    assert!(hint.contains("cast("), "{hint}");
}

#[test]
fn sql_csv_column_with_separators_is_refused_not_summed_wrong() {
    let s = Scratch::new("sql3");
    s.put(
        "m.csv",
        b"price\n\"1,234.50\"\n\"2.000,10\"\n1 234\n$5\n\"1,234\"\n",
    );
    let out = s.json(
        r#"try { open("m.csv") | sql("select sum(price) s from t") } catch e { e.error.code }"#,
    );
    assert_eq!(out, r#""E_BAD_ARG""#);
    // The documented way through works.
    let ok = s.json(
        r#"open("m.csv") | sql("select sum(cast(replace(replace(price, ',', ''), '$', '') as real)) s from t") | first | .s"#,
    );
    // 1234.50 + 2.00010 + 1 + 5 + 1234: what the cleaned text really sums to.
    assert!(ok.starts_with("2476."), "{ok}");
}

#[test]
fn sql_aggregates_over_numbers_and_clean_numeric_text_are_unchanged() {
    let s = Scratch::new("sql4");
    let q = |expr: &str| s.json(expr);
    assert_eq!(
        q(
            r#"[{n: 1}, {n: 2}, {n: 3}] | sql("select sum(n) s, total(n) t, avg(n) a from t") | first"#
        ),
        r#"{"a":2.0,"s":6,"t":6.0}"#
    );
    assert_eq!(
        q(r#"[{n: 0.1}, {n: 0.2}, {n: 0.3}] | sql("select sum(n) s from t") | first | .s"#),
        "0.6"
    );
    // NULLs are skipped; an empty set is null for sum and avg, 0.0 for total.
    assert_eq!(
        q(r#"[{n: 1}, {n: null}] | sql("select sum(n) s, avg(n) a from t") | first"#),
        r#"{"a":1.0,"s":1}"#
    );
    assert_eq!(
        q(r#"[{n: 1}] | sql("select sum(n) s, total(n) t, avg(n) a from t where n > 5") | first"#),
        r#"{"a":null,"s":null,"t":0.0}"#
    );
    // Clean text numbers are accepted, as SQLite accepts them.
    assert_eq!(
        q(r#"[{p: "12"}, {p: " 1.5 "}] | sql("select sum(p) s from t") | first | .s"#),
        "13.5"
    );
    // Groups, windows, DISTINCT, count and group_concat still work.
    assert_eq!(
        q(
            r#"[{g: "a", n: 1}, {g: "a", n: 2}, {g: "b", n: 5}] | sql("select g, sum(n) s, count(*) c from t group by g order by g")"#
        ),
        r#"[{"c":2,"g":"a","s":3},{"c":1,"g":"b","s":5}]"#
    );
    assert_eq!(
        q(
            r#"[{n: 1}, {n: 2}, {n: 3}] | sql("select sum(n) over (order by n rows between 1 preceding and current row) w from t order by n") | map(.w)"#
        ),
        "[1,3,5]"
    );
    assert_eq!(
        q(r#"[{n: 1}, {n: 1}, {n: 2}] | sql("select sum(distinct n) s from t") | first | .s"#),
        "3"
    );
    assert_eq!(
        q(
            r#"[{p: "x"}, {p: "y"}] | sql("select count(p) c, group_concat(p) g, min(p) lo, max(p) hi from t") | first"#
        ),
        r#"{"c":2,"g":"x,y","hi":"y","lo":"x"}"#
    );
}

#[test]
fn sql_value_uses_the_same_strict_aggregates() {
    let s = Scratch::new("sql5");
    s.put("m.json", br#"[{"p": "12x"}, {"p": "3"}]"#);
    assert_eq!(
        s.code(r#"sql_value("m.json", "select sum(p) from m")"#),
        r#""E_BAD_ARG""#
    );
    s.put("n.json", br#"[{"p": 2}, {"p": 3}]"#);
    assert_eq!(
        s.json(r#"sql_value("n.json", "select sum(p) from n")"#),
        "5"
    );
}

#[test]
fn sql_arithmetic_on_text_is_documented_as_unchanged() {
    let s = Scratch::new("sql6");
    // `p * 2` cannot be a function: SQLite reads the leading number.
    assert_eq!(
        s.json(r#"[{p: "12x"}] | sql("select p * 2 as x from t") | first | .x"#),
        "24"
    );
    let doc = s.json(r#"help("sql")"#);
    assert!(doc.to_lowercase().contains("arithmetic on text"), "{doc}");
}

// ---- 3. file.replace with \r\n in the pattern on a CRLF file ----

#[test]
fn file_replace_matches_crlf_spelled_either_way_and_keeps_crlf() {
    for (tag, find, replace) in [
        ("a", r"one\r\ntwo", r"1\n2"),
        ("b", r"one\ntwo", r"1\n2"),
        ("c", r"e\r\ntwo", r"E\r\nTWO"),
        ("d", r"e\ntwo", r"E\nTWO"),
        ("e", r"one\r\ntwo", r"1\r\n2"),
    ] {
        let s = Scratch::new(&format!("crlf{tag}"));
        s.put("c.txt", b"one\r\ntwo\r\nthree\r\n");
        let out = s.json(&format!(
            r#"file.replace("c.txt", "{find}", "{replace}") | .success"#
        ));
        assert_eq!(out, "true", "{find}: {out}");
        let text = String::from_utf8(s.get("c.txt")).unwrap();
        assert!(text.ends_with("\r\nthree\r\n"), "{find}: {text:?}");
        assert!(
            !text.replace("\r\n", "").contains('\n'),
            "{find}: bare LF in {text:?}"
        );
        assert!(!text.contains("one\r\ntwo"), "{find}: {text:?}");
    }
}

#[test]
fn file_patch_takes_crlf_patterns_too() {
    let s = Scratch::new("crlf2");
    s.put("c.txt", b"one\r\ntwo\r\none\r\ntwo\r\n");
    let out = s.json(r#"file.patch("c.txt", [{find: "one\r\ntwo", replace: "X\nY"}]) | .success"#);
    assert_eq!(out, "true", "{out}");
    let text = String::from_utf8(s.get("c.txt")).unwrap();
    assert_eq!(text, "X\r\nY\r\nX\r\nY\r\n");
}

// ---- 4. typed git history ----

fn git(dir: &std::path::Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Ann")
        .env("GIT_AUTHOR_EMAIL", "ann@example.com")
        .env("GIT_COMMITTER_NAME", "Ann")
        .env("GIT_COMMITTER_EMAIL", "ann@example.com")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "git {args:?}");
}

#[test]
fn git_log_options_return_typed_records_with_changed_files() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let s = Scratch::new("git1");
    let dir = s.cwd();
    git(&dir, &["init", "-q"]);
    s.put("a.txt", b"1\n");
    s.put("b.txt", b"1\n");
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "first"]);
    s.put("a.txt", b"2\n");
    git(&dir, &["commit", "-q", "-am", "second: a"]);
    // The string form is unchanged.
    let plain = s.json("git_log(2) | map(typeof)");
    assert_eq!(plain, r#"["String","String"]"#);
    let rows = s.json(
        r#"git_log({n: 5}) | map(fn(r) => [r.subject, r.author, len(r.hash), typeof(r.date)])"#,
    );
    assert_eq!(
        rows,
        r#"[["second: a","Ann",40,"String"],["first","Ann",40,"String"]]"#
    );
    let files = s.json(r#"git_log({n: 5, files: true}) | map(.files)"#);
    assert_eq!(files, r#"[["a.txt"],["a.txt","b.txt"]]"#);
    let both = s.json(r#"git_log(1, {files: true}) | first | .files"#);
    assert_eq!(both, r#"["a.txt"]"#);
    // Most changed files, the question the option exists for.
    let top = s.json(
        r#"git_log({n: 50, files: true}) | map(.files) | flatten | count_by(fn(f) => f) | first | fn(r) => [r.key, r.count]"#,
    );
    assert_eq!(top, r#"["a.txt",2]"#);
}

#[test]
fn git_log_options_are_validated() {
    let s = Scratch::new("git2");
    assert_eq!(s.code(r#"git_log({stat: true})"#), r#""E_BAD_ARG""#);
    assert_eq!(s.code(r#"git_log({files: 1})"#), r#""E_BAD_ARG""#);
    assert_eq!(s.code(r#"git_log(3, {n: 4})"#), r#""E_BAD_ARG""#);
    assert_eq!(s.code(r#"git_log({n: 0})"#), r#""E_BAD_ARG""#);
    let hint = s.json(r#"try { git_log({stat: true}) } catch e { e.error.hint }"#);
    assert!(hint.contains("files: true"), "{hint}");
}

#[test]
fn tool_search_finds_git_log_for_history_questions() {
    let s = Scratch::new("ts1");
    let names = s.json(r#"tool_search("most changed files git history") | where(.kind == "builtin") | take(3) | map(.name)"#);
    assert!(names.contains("git_log"), "{names}");
}

// ---- 5. discovery and the non-ASCII field name ----

#[test]
fn tool_search_read_a_file_ranks_cat_above_wc_and_bat() {
    let s = Scratch::new("ts2");
    let names = s.json(r#"tool_search("read a file") | where(.kind == "builtin") | map(.name)"#);
    let pos = |n: &str| {
        let key = format!("\"{n}\"");
        names.find(&key).unwrap_or(usize::MAX)
    };
    assert!(pos("cat") < pos("wc"), "{names}");
    assert!(pos("cat") < pos("bat"), "{names}");
}

#[test]
fn tool_search_disk_free_space_includes_df() {
    let s = Scratch::new("ts3");
    let names =
        s.json(r#"tool_search("disk free space") | where(.kind == "builtin") | map(.name)"#);
    assert!(names.contains("\"df\""), "{names}");
}

#[test]
fn a_non_ascii_field_name_after_a_dot_says_to_quote_it() {
    let s = Scratch::new("uni");
    let err = s.json("{a: 1} | .\u{65E5}\u{672C}");
    assert!(err.contains("E_PARSE"), "{err}");
    assert!(err.contains("r[\\\"name\\\"]"), "{err}");
    // The quoted spelling works.
    s.put("u.json", "[{\"\u{65E5}\u{672C}\": 7}]".as_bytes());
    assert_eq!(
        s.json("open(\"u.json\") | map(fn(r) => r[\"\u{65E5}\u{672C}\"])"),
        "[7]"
    );
}

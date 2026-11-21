//! Round 64b: refactor_rename and the extract refactors keep the source text,
//! grep bracket expressions, sed on CRLF files, headerless CSV, YAML nested
//! sequences and scalars, and git_status with non-ASCII names.

use std::path::{Path, PathBuf};
use std::process::Command;

fn dir(tag: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round64b_data")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("session")).unwrap();
    std::fs::create_dir_all(root.join("ws")).unwrap();
    root
}

/// Runs `code` with `--deterministic` in `<root>/ws`; returns stdout and stderr.
fn ish(root: &Path, code: &str) -> (String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--deterministic", "-c", code])
        .current_dir(root.join("ws"))
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", root.join("session"))
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_WORKSPACE")
        .output()
        .expect("run ish");
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    )
}

fn out(root: &Path, code: &str) -> String {
    ish(root, code).0
}

#[test]
fn refactor_rename_keeps_comments_layout_and_crlf() {
    let root = dir("rename");
    let src = "// a is the name\r\nfn a() {\r\n    // keep a\r\n    b(); /* a c */\r\n    let s = \"a\"; let ab = a(); let xa = 1;\r\n}\r\n";
    std::fs::write(root.join("ws").join("s.rs"), src).unwrap();
    let got = out(&root, r#"refactor_rename(cat("s.rs"), "a", "z")"#);
    let v: String = serde_json::from_str(&got).expect(&got);
    let want = "// a is the name\r\nfn z() {\r\n    // keep a\r\n    b(); /* a c */\r\n    let s = \"a\"; let ab = z(); let xa = 1;\r\n}\r\n";
    assert_eq!(v, want);
    // Multi-byte text before an identifier does not shift the spans.
    let got = out(
        &root,
        r#"refactor_rename("// é ü\nfn a() { let s = \"é\"; a(); }\n", "a", "z")"#,
    );
    assert_eq!(got, r#""// é ü\nfn z() { let s = \"é\"; z(); }\n""#);
    // The reported example.
    let got = out(
        &root,
        r#"refactor_rename("fn a() {\n    // keep\n    b(); /* c */\n}\n", "a", "z")"#,
    );
    assert_eq!(got, r#""fn z() {\n    // keep\n    b(); /* c */\n}\n""#);
}

#[test]
fn refactor_rename_is_rust_only() {
    let root = dir("rename_lang");
    let (_, err) = ish(
        &root,
        r##"refactor_rename("def a():\n  # x\n  pass\n", "a", "z")"##,
    );
    assert!(err.contains("E_BAD_ARG") && err.contains("Rust"), "{err}");
    let (_, err) = ish(
        &root,
        r#"refactor_rename("function a() { return 1 }", "a", "z")"#,
    );
    assert!(err.contains("E_BAD_ARG") && err.contains("Rust"), "{err}");
}

#[test]
fn refactor_extract_variable_and_constant_keep_the_source() {
    let root = dir("extract");
    let got = out(
        &root,
        r#"refactor_extract_variable("fn f(x: i32) -> i32 {\n    // note\n    let y = 1; // c\n    x + y /* t */\n}\n", "r")"#,
    );
    assert_eq!(
        got,
        r#""fn f(x: i32) -> i32 {\n    // note\n    let y = 1; // c\n    let r = x + y;\n    r /* t */\n}\n""#
    );
    let got = out(
        &root,
        r#"refactor_extract_constant("fn f() -> i32 {\r\n    // n\r\n    1 + 2\r\n}\r\n", "K", "i32")"#,
    );
    assert_eq!(
        got,
        r#""fn f() -> i32 {\r\n    // n\r\n    const K: i32 = 1 + 2;\r\n    K\r\n}\r\n""#
    );
}

#[test]
fn grep_bracket_expressions() {
    let root = dir("brackets");
    std::fs::write(root.join("ws").join("i.ini"), "[sec]\nk=1\n[b]\nx]y\n").unwrap();
    assert_eq!(out(&root, r#"grep("^[[]", "i.ini")"#), r#"["[sec]","[b]"]"#);
    assert_eq!(out(&root, r#"grep("[a[]", "i.ini")"#), r#"["[sec]","[b]"]"#);
    assert_eq!(
        out(&root, r#"grep("[]a]", "i.ini")"#),
        r#"["[sec]","[b]","x]y"]"#
    );
    assert_eq!(
        out(&root, r#"grep("^[^]a[]", "i.ini")"#),
        r#"["k=1","x]y"]"#
    );
    assert_eq!(out(&root, r#"grep("[[:alpha:]]=", "i.ini")"#), r#"["k=1"]"#);
    assert_eq!(
        out(&root, r#"grep("^[[]", "i.ini", {extended: true})"#),
        r#"["[sec]","[b]"]"#
    );
    assert_eq!(
        out(&root, r#"grep("[]a]y", "i.ini", {extended: true})"#),
        r#"["x]y"]"#
    );
    assert_eq!(
        out(
            &root,
            r#"grep("^[[:alpha:]]+=", "i.ini", {extended: true})"#
        ),
        r#"["k=1"]"#
    );
    let (_, err) = ish(&root, r#"grep("[z", "i.ini")"#);
    assert!(
        err.contains("E_BAD_ARG") && err.contains("closing ]"),
        "{err}"
    );
}

#[test]
fn sed_file_mode_on_crlf_files() {
    let root = dir("sedcrlf");
    let ws = root.join("ws");
    std::fs::write(ws.join("c.txt"), "x\r\ny\r\nx\r\n").unwrap();
    let r = out(&root, r#"sed("(?m)x$", "Z", "c.txt", {file: true})"#);
    assert_eq!(r, r#"{"file":"c.txt","matches":2,"status":"modified"}"#);
    assert_eq!(std::fs::read(ws.join("c.txt")).unwrap(), b"Z\r\ny\r\nZ\r\n");
    let r = out(&root, r#"sed("Z\ny", "Q", "c.txt", {file: true})"#);
    assert_eq!(r, r#"{"file":"c.txt","matches":1,"status":"modified"}"#);
    assert_eq!(std::fs::read(ws.join("c.txt")).unwrap(), b"Q\r\nZ\r\n");
    // A miss reports 0 matches and leaves the bytes alone.
    let r = out(&root, r#"sed("nope", "Q", "c.txt", {file: true})"#);
    assert_eq!(r, r#"{"file":"c.txt","matches":0,"status":"unchanged"}"#);
    assert_eq!(std::fs::read(ws.join("c.txt")).unwrap(), b"Q\r\nZ\r\n");
    // An LF file works as before.
    std::fs::write(ws.join("l.txt"), "x\ny\n").unwrap();
    let r = out(&root, r#"sed("(?m)x$", "Z", "l.txt", {file: true})"#);
    assert_eq!(r, r#"{"file":"l.txt","matches":1,"status":"modified"}"#);
    assert_eq!(std::fs::read(ws.join("l.txt")).unwrap(), b"Z\ny\n");
    // A file that mixes endings is edited as it is.
    std::fs::write(ws.join("m.txt"), "x\r\ny\nx\n").unwrap();
    out(&root, r#"sed("(?m)x$", "Z", "m.txt", {file: true})"#);
    assert_eq!(std::fs::read(ws.join("m.txt")).unwrap(), b"x\r\ny\nZ\n");
}

#[test]
fn from_csv_without_a_header_row() {
    let root = dir("csvheader");
    assert_eq!(
        out(&root, r#"from_csv("1,2\n3,4\n", {header: false})"#),
        r#"[{"c1":1,"c2":2},{"c1":3,"c2":4}]"#
    );
    assert_eq!(
        out(&root, r#"csv_columns("a,b,c\n1,2,3\n", {header: false})"#),
        r#"["c1","c2","c3"]"#
    );
    assert_eq!(out(&root, r#"csv_columns("a,b\n1,2\n")"#), r#"["a","b"]"#);
    assert_eq!(
        out(
            &root,
            r#"from_csv("a;b\n", {header: false, delimiter: ";"})"#
        ),
        r#"[{"c1":"a","c2":"b"}]"#
    );
    let (_, err) = ish(&root, r#"from_csv("a\n", {header: "no"})"#);
    assert!(err.contains("E_BAD_ARG"), "{err}");
    // A leading plus keeps its column text, as a leading zero does.
    assert_eq!(
        out(&root, r#"from_csv("p,n\n+2,1\n+3,2\n")"#),
        r#"[{"n":1,"p":"+2"},{"n":2,"p":"+3"}]"#
    );
    assert_eq!(out(&root, r#"from_csv("p\n-1\n")[0].p"#), "-1");
}

#[test]
fn yaml_nested_sequences_and_top_level_scalars() {
    let root = dir("yaml");
    assert_eq!(
        out(&root, r#"from_yaml("- - 1\n  - 2\n- - 3\n")"#),
        "[[1,2],[3]]"
    );
    assert_eq!(
        out(&root, r#"from_yaml("- - - a\n    - b\n- c\n")"#),
        r#"[[["a","b"]],"c"]"#
    );
    assert_eq!(
        out(&root, r#"from_yaml("- - k: 1\n    j: 2\n")"#),
        r#"[[{"j":2,"k":1}]]"#
    );
    for (v, json) in [
        ("5", "5"),
        (r#""x""#, r#""x""#),
        (r#""x: y""#, r#""x: y""#),
        (r#""null""#, r#""null""#),
        ("true", "true"),
        ("null", "null"),
        ("1.5", "1.5"),
    ] {
        assert_eq!(out(&root, &format!("from_yaml(to_yaml({v}))")), json, "{v}");
    }
}

#[test]
fn git_status_gives_real_non_ascii_names() {
    let root = dir("gitstatus");
    let ws = root.join("ws");
    let git = |args: &[&str]| {
        let o = Command::new("git")
            .args(["-c", "user.email=a@b", "-c", "user.name=n"])
            .args(args)
            .current_dir(&ws)
            .env("GIT_CONFIG_GLOBAL", "")
            .output()
            .expect("git");
        assert!(o.status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    std::fs::write(ws.join("\u{e9}.txt"), "1\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "one"]);
    std::fs::write(ws.join("\u{e9}.txt"), "2\n").unwrap();
    std::fs::write(ws.join("\u{4e2d}.txt"), "3\n").unwrap();
    let status = out(&root, "git_status()");
    assert!(status.contains("\u{e9}.txt"), "{status}");
    assert!(status.contains("\u{4e2d}.txt"), "{status}");
    assert!(!status.contains("\\\\303"), "{status}");
    let log = out(&root, "git_log({n: 5, files: true})");
    assert!(log.contains("\u{e9}.txt"), "{log}");
}

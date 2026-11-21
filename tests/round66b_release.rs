//! Round 66b: release engineering. Links in archives, semver, git tags and log
//! options, base64 digests, loopback in the network policy, zip mtimes.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;

fn dir(tag: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round66b_release")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("session")).unwrap();
    std::fs::create_dir_all(root.join("ws")).unwrap();
    root
}

struct Run {
    out: String,
    err: String,
    code: i32,
}

/// Runs `ish <flags> --deterministic -c code` in `cwd`, with `env` added.
fn run_in(root: &Path, cwd: &Path, flags: &[&str], env: &[(&str, &str)], code: &str) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ish"));
    cmd.args(flags)
        .args(["--deterministic", "-c", code])
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", root.join("session"))
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_AGENT")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_NET_ALLOW")
        .env_remove("IRONSHELL_POLICY")
        .env_remove("IRONSHELL_APPROVE")
        .env_remove("SOURCE_DATE_EPOCH");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let o = cmd.output().expect("run ish");
    Run {
        out: String::from_utf8_lossy(&o.stdout).trim().to_string(),
        err: String::from_utf8_lossy(&o.stderr).trim().to_string(),
        code: o.status.code().unwrap_or(-1),
    }
}

fn ish(root: &Path, code: &str) -> Run {
    run_in(root, &root.join("ws"), &[], &[], code)
}

fn out(root: &Path, code: &str) -> String {
    ish(root, code).out
}

// ---- archives: links ----

/// A link `at` to `to` (relative, as the link stores it): a symbolic link on
/// Unix; on Windows a junction (a folder target) or a symbolic link when the
/// session may create one. False when the platform refused.
fn make_link(at: &Path, to: &Path, folder: bool) -> bool {
    #[cfg(unix)]
    {
        let _ = folder;
        std::os::unix::fs::symlink(to, at).is_ok()
    }
    #[cfg(windows)]
    {
        let target = at.parent().unwrap().join(to);
        if folder {
            Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(at)
                .arg(&target)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        } else {
            std::os::windows::fs::symlink_file(to, at).is_ok()
        }
    }
}

fn tar_entries(path: &Path) -> Vec<(String, Option<String>)> {
    let f = std::fs::File::open(path).unwrap();
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    ar.entries()
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let name = e.path().unwrap().to_string_lossy().replace('\\', "/");
            let link = e
                .link_name()
                .unwrap()
                .map(|l| l.to_string_lossy().replace('\\', "/"));
            (name, link)
        })
        .collect()
}

#[test]
fn tar_create_stores_in_tree_links_with_relative_targets_and_extracts() {
    let root = dir("tar_links");
    let ws = root.join("ws");
    std::fs::create_dir_all(ws.join("t/a/deep")).unwrap();
    std::fs::create_dir_all(ws.join("t/b")).unwrap();
    std::fs::create_dir_all(ws.join("outside")).unwrap();
    std::fs::write(ws.join("t/a/f.txt"), "hi").unwrap();
    std::fs::write(ws.join("outside/o.txt"), "out").unwrap();
    let junction = make_link(&ws.join("t/ajunc"), Path::new("a"), true);
    let nested = make_link(&ws.join("t/b/up"), Path::new("../a/deep"), true);
    let leaving = make_link(&ws.join("t/leave"), Path::new("../outside"), true);
    let file_link = make_link(&ws.join("t/flink"), Path::new("a/f.txt"), false);
    if !junction {
        eprintln!("links cannot be made here; skipping");
        return;
    }
    assert!(nested);
    let r = run_in(
        &root,
        &ws,
        &["--agent", "--workspace", "."],
        &[],
        r#"tar_create("o.tar.gz", "t")"#,
    );
    assert_eq!(r.out, "true", "{}", r.err);
    let host = ws.to_string_lossy().replace('\\', "/");
    let entries = tar_entries(&ws.join("o.tar.gz"));
    let find = |n: &str| entries.iter().find(|(name, _)| name == n).cloned();
    assert_eq!(find("t/ajunc").and_then(|e| e.1).as_deref(), Some("a"));
    assert_eq!(
        find("t/b/up").and_then(|e| e.1).as_deref(),
        Some("../a/deep")
    );
    if file_link {
        assert_eq!(
            find("t/flink").and_then(|e| e.1).as_deref(),
            Some("a/f.txt")
        );
    }
    // No stored link carries a path of this machine, and the link that leads
    // outside the folder is left out and named on stderr.
    for (name, link) in &entries {
        if let Some(l) = link {
            assert!(
                !l.contains(&host) && !l.starts_with('/') && !l.contains(':'),
                "{name} -> {l}"
            );
        }
    }
    if leaving {
        assert!(find("t/leave").is_none());
        assert!(r.err.contains("skipped link"), "{}", r.err);
    }
    let listed = out(&root, r#"tar_list("o.tar.gz") | len"#);
    assert!(listed.parse::<i64>().unwrap() >= 6, "{listed}");
    // The archive extracts into a fresh folder, and the links lead to what
    // they led to.
    let r = ish(&root, r#"tar_extract("o.tar.gz", "ex")"#);
    assert_eq!(r.out, "true", "{}", r.err);
    assert_eq!(
        std::fs::read_to_string(ws.join("ex/t/a/f.txt")).unwrap(),
        "hi"
    );
    assert_eq!(
        std::fs::read_to_string(ws.join("ex/t/ajunc/f.txt")).unwrap(),
        "hi"
    );
    assert!(ws.join("ex/t/b/up").is_dir());
    if file_link {
        assert_eq!(
            std::fs::read_to_string(ws.join("ex/t/flink")).unwrap(),
            "hi"
        );
    }
}

#[test]
fn tar_extract_still_refuses_a_link_that_climbs_out() {
    use tar::{Builder, EntryType, Header};
    let root = dir("tar_escape");
    let ws = root.join("ws");
    for (target, label) in [
        ("../../etc", "up"),
        ("/etc", "abs"),
        ("a/../../../x", "dots"),
    ] {
        let f = std::fs::File::create(ws.join(format!("{label}.tar"))).unwrap();
        let mut b = Builder::new(f);
        let mut h = Header::new_gnu();
        h.set_entry_type(EntryType::Symlink);
        h.set_size(0);
        h.set_cksum();
        b.append_link(&mut h, "d/l", target).unwrap();
        b.into_inner().unwrap().flush().unwrap();
        let r = ish(
            &root,
            &format!(r#"tar_extract("{label}.tar", "ex_{label}")"#),
        );
        assert_eq!(r.code, 77, "{label}: {} {}", r.out, r.err);
        assert!(!ws.join(format!("ex_{label}")).join("d").exists());
    }
}

#[test]
fn tar_and_zip_store_file_mtimes_and_honour_source_date_epoch() {
    let root = dir("mtime");
    let ws = root.join("ws");
    std::fs::create_dir_all(ws.join("t")).unwrap();
    let file = ws.join("t/f.txt");
    std::fs::write(&file, "x").unwrap();
    // 2023-11-14 22:13:20 UTC
    let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(when)
        .unwrap();
    assert_eq!(out(&root, r#"zip_create("a.zip", "t")"#), "true");
    let mut z = zip::ZipArchive::new(std::fs::File::open(ws.join("a.zip")).unwrap()).unwrap();
    let t = z.by_name("t/f.txt").unwrap().last_modified().unwrap();
    assert_eq!(
        (t.year(), t.month(), t.day(), t.hour(), t.minute()),
        (2023, 11, 14, 22, 13)
    );
    drop(z);
    // SOURCE_DATE_EPOCH replaces it, for zip and for tar.
    let r = run_in(
        &root,
        &ws,
        &[],
        &[("SOURCE_DATE_EPOCH", "1000000000")],
        r#"zip_create("b.zip", "t"); tar_create("b.tar", "t")"#,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let mut z = zip::ZipArchive::new(std::fs::File::open(ws.join("b.zip")).unwrap()).unwrap();
    let t = z.by_name("t/f.txt").unwrap().last_modified().unwrap();
    // 2001-09-09 01:46:40 UTC
    assert_eq!((t.year(), t.month(), t.day()), (2001, 9, 9));
    let mut ar = tar::Archive::new(std::fs::File::open(ws.join("b.tar")).unwrap());
    for e in ar.entries().unwrap() {
        assert_eq!(e.unwrap().header().mtime().unwrap(), 1_000_000_000);
    }
    // The same tree and the same epoch give the same archive.
    let r = run_in(
        &root,
        &ws,
        &[],
        &[("SOURCE_DATE_EPOCH", "1000000000")],
        r#"zip_create("c.zip", "t")"#,
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(
        std::fs::read(ws.join("b.zip")).unwrap(),
        std::fs::read(ws.join("c.zip")).unwrap()
    );
}

// ---- semver ----

#[test]
fn semver_builtins() {
    let root = dir("semver");
    assert_eq!(
        out(&root, r#"semver_parse("v1.2.3-rc.1+b5")"#),
        r#"{"build":["b5"],"major":1,"minor":2,"patch":3,"prerelease":["rc",1]}"#
    );
    assert_eq!(
        out(&root, r#"semver_parse("0.0.4")"#),
        r#"{"build":[],"major":0,"minor":0,"patch":4,"prerelease":[]}"#
    );
    for bad in [
        "1.2", "01.2.3", "1.2.3-", "1.2.3-01", "1.2.3+", "x", "", "1.2.3.4",
    ] {
        let r = ish(&root, &format!(r#"semver_parse("{bad}")"#));
        assert!(r.err.contains("E_BAD_ARG"), "{bad}: {}", r.err);
    }
    // semver.org section 11, in order.
    let chain = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
        "2.0.0",
        "2.1.0",
        "2.1.1",
    ];
    for w in chain.windows(2) {
        assert_eq!(
            out(&root, &format!(r#"semver_compare("{}", "{}")"#, w[0], w[1])),
            "-1",
            "{w:?}"
        );
        assert_eq!(
            out(&root, &format!(r#"semver_compare("{}", "{}")"#, w[1], w[0])),
            "1"
        );
    }
    assert_eq!(out(&root, r#"semver_compare("1.0.0+a", "v1.0.0+b")"#), "0");
    assert_eq!(out(&root, r#"semver_compare("v1.10.0", "1.2.0")"#), "1");
    assert_eq!(
        out(&root, r#"("1.2.0-rc.1" | semver_compare("1.2.0"))"#),
        "-1"
    );
    // A shuffled chain sorts back into order, keeping the strings as given.
    let shuffled = r#"["2.1.1","1.0.0-beta.11","1.0.0","v1.0.0-alpha","1.0.0-rc.1","2.0.0","1.0.0-alpha.beta","1.0.0-beta","1.0.0-beta.2","1.0.0-alpha.1","2.1.0"]"#;
    assert_eq!(
        out(&root, &format!("{shuffled} | semver_sort")),
        r#"["v1.0.0-alpha","1.0.0-alpha.1","1.0.0-alpha.beta","1.0.0-beta","1.0.0-beta.2","1.0.0-beta.11","1.0.0-rc.1","1.0.0","2.0.0","2.1.0","2.1.1"]"#
    );
    // Stable for equal precedence.
    assert_eq!(
        out(&root, r#"["1.0.0+b", "1.0.0+a"] | semver_sort"#),
        r#"["1.0.0+b","1.0.0+a"]"#
    );
    // A non-version entry is refused and named, never dropped.
    let r = ish(&root, r#"["1.0.0", "latest"] | semver_sort"#);
    assert!(
        r.err.contains("E_BAD_ARG") && r.err.contains("entry 1"),
        "{}",
        r.err
    );
    assert_eq!(
        out(&root, r#"["1.0.0", "latest", 5] | where(semver_valid)"#),
        r#"["1.0.0"]"#
    );
    for (v, level, want) in [
        ("1.2.3", r#""major""#, "2.0.0"),
        ("1.2.3", r#""minor""#, "1.3.0"),
        ("1.2.3", r#""patch""#, "1.2.4"),
        ("v1.2.3", r#""patch""#, "v1.2.4"),
        ("1.2.3+b", r#""patch""#, "1.2.4"),
        ("2.0.0-rc.1", r#""major""#, "2.0.0"),
        ("1.3.0-rc.1", r#""minor""#, "1.3.0"),
        ("1.2.4-rc.1", r#""patch""#, "1.2.4"),
        ("1.2.3", r#""prerelease", "rc""#, "1.2.4-rc.0"),
        ("1.2.3", r#""prerelease""#, "1.2.4-0"),
        ("1.2.4-rc.0", r#""prerelease", "rc""#, "1.2.4-rc.1"),
        ("1.2.4-rc.1", r#""prerelease", "beta""#, "1.2.4-beta.0"),
        ("1.2.4-rc.9", r#""prerelease""#, "1.2.4-rc.10"),
    ] {
        assert_eq!(
            out(&root, &format!(r#"semver_bump("{v}", {level})"#)),
            format!("\"{want}\""),
            "{v} {level}"
        );
    }
    for bad in [
        r#"semver_bump("1.2.3", "huge")"#,
        r#"semver_bump("x", "patch")"#,
        r#"semver_bump("1.2.3", "patch", "rc")"#,
        r#"semver_bump("1.2.3", "prerelease", "a..b")"#,
    ] {
        assert!(ish(&root, bad).err.contains("E_BAD_ARG"), "{bad}");
    }
}

// ---- git ----

fn git(repo: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_DATE", "2026-03-01T12:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-03-01T12:00:00Z")
        .output()
        .expect("git");
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn repo(tag: &str) -> (PathBuf, PathBuf) {
    let root = dir(tag);
    let repo = root.join("ws");
    git(&repo, &["init", "-q", "."]);
    git(&repo, &["config", "user.name", "Test"]);
    git(&repo, &["config", "user.email", "t@example.com"]);
    git(&repo, &["config", "commit.gpgsign", "false"]);
    git(&repo, &["config", "tag.gpgsign", "false"]);
    (root, repo)
}

fn commit(repo: &Path, file: &str, message: &str) {
    std::fs::write(repo.join(file), message).unwrap();
    git(repo, &["add", file]);
    git(repo, &["commit", "-q", "-m", message]);
}

#[test]
fn git_tags_sort_semver_and_details() {
    let (root, repo) = repo("git_tags");
    commit(&repo, "a", "first");
    for t in ["v1.2.0", "v1.10.0", "v1.9.0", "v1.2.0-rc.1", "nightly"] {
        git(&repo, &["tag", t]);
    }
    commit(&repo, "b", "second");
    git(&repo, &["tag", "-a", "v2.0.0", "-m", "Release 2"]);
    let run = |code: &str| run_in(&root, &repo, &[], &[], code);
    assert_eq!(
        run("git_tags()").out,
        r#"["nightly","v1.10.0","v1.2.0","v1.2.0-rc.1","v1.9.0","v2.0.0"]"#
    );
    assert_eq!(
        run(r#"git_tags({sort: "semver"})"#).out,
        r#"["v1.2.0-rc.1","v1.2.0","v1.9.0","v1.10.0","v2.0.0","nightly"]"#
    );
    assert_eq!(
        run(r#"git_tags({sort: "semver"}) | where(semver_valid) | semver_sort | last"#).out,
        r#""v2.0.0""#
    );
    let first = git(&repo, &["rev-list", "--max-parents=0", "HEAD"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let rows = run(r#"git_tags({details: true, sort: "semver"})"#).out;
    assert!(
        rows.contains(&format!(
            r#"{{"annotated":false,"date":"2026-03-01T12:00:00Z","name":"v1.2.0","ref":"{first}"}}"#
        )),
        "{rows}"
    );
    // An annotated tag points at its commit, not at the tag object.
    assert!(
        rows.contains(r#""annotated":true"#)
            && rows.contains(&format!(r#""name":"v2.0.0","ref":"{head}""#)),
        "{rows}"
    );
    assert!(run(r#"git_tags({sort: "newest"})"#)
        .err
        .contains("E_BAD_ARG"));
    assert!(run(r#"git_tags({order: "semver"})"#)
        .err
        .contains("E_BAD_ARG"));
}

#[test]
fn git_tag_creates_lightweight_and_annotated_tags() {
    let (root, repo) = repo("git_tag");
    commit(&repo, "a", "first");
    commit(&repo, "b", "second");
    let first = git(&repo, &["rev-parse", "HEAD~1"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let run = |flags: &[&str], code: &str| run_in(&root, &repo, flags, &[], code);
    let r = run(&[], r#"git_tag("v1.0.0")"#);
    assert_eq!(
        r.out,
        format!(r#"{{"annotated":false,"name":"v1.0.0","ref":"{head}"}}"#),
        "{}",
        r.err
    );
    assert_eq!(git(&repo, &["cat-file", "-t", "v1.0.0"]), "commit");
    let r = run(
        &[],
        r#"git_tag("v0.9.0", {message: "Release 0.9.0", ref: "HEAD~1"})"#,
    );
    assert_eq!(
        r.out,
        format!(r#"{{"annotated":true,"name":"v0.9.0","ref":"{first}"}}"#),
        "{}",
        r.err
    );
    assert_eq!(git(&repo, &["cat-file", "-t", "v0.9.0"]), "tag");
    assert!(git(&repo, &["tag", "-n1", "-l", "v0.9.0"]).contains("Release 0.9.0"));
    // An existing tag is refused and left where it is.
    let r = run(&[], r#"git_tag("v1.0.0", {ref: "HEAD~1"})"#);
    assert!(r.err.contains("E_TOOL_FAILED"), "{}", r.err);
    assert_eq!(git(&repo, &["rev-parse", "v1.0.0"]), head);
    // Option injection and bad options.
    for bad in [
        r#"git_tag("--delete")"#,
        r#"git_tag("-f")"#,
        r#"git_tag("v2", {ref: "--all"})"#,
        r#"git_tag("v2", {message: ""})"#,
        r#"git_tag("v2", {note: "x"})"#,
        r#"git_tag("v2", 5)"#,
    ] {
        assert!(run(&[], bad).err.contains("E_BAD_ARG"), "{bad}");
    }
    assert!(!git(&repo, &["tag", "-l"]).contains("v2"));
    // Agent mode asks for approval (an Exec effect, as git_commit), and writes nothing.
    let r = run(&["--agent"], r#"git_tag("v3.0.0")"#);
    assert_eq!(r.code, 75, "{} {}", r.out, r.err);
    assert!(r.err.contains("E_NEEDS_APPROVAL"), "{}", r.err);
    assert!(!git(&repo, &["tag", "-l"]).contains("v3.0.0"));
    // A transaction refuses it: a rollback cannot delete a tag.
    let r = run(&[], r#"tx_begin(); git_tag("v4.0.0")"#);
    assert_ne!(r.code, 0);
    assert!(!git(&repo, &["tag", "-l"]).contains("v4.0.0"));
}

#[test]
fn git_log_range_since_and_body() {
    let (root, repo) = repo("git_log");
    commit(&repo, "a", "old one");
    git(&repo, &["tag", "v1.0.0"]);
    std::fs::write(repo.join("b"), "b").unwrap();
    git(&repo, &["add", "b"]);
    git(
        &repo,
        &[
            "commit",
            "-q",
            "-m",
            "feat!: new api",
            "-m",
            "Body line one.\nBody line two.",
            "-m",
            "BREAKING CHANGE: the api changed",
        ],
    );
    commit(&repo, "c", "fix: later");
    let run = |code: &str| run_in(&root, &repo, &[], &[], code);
    // Without options the default count is unchanged: the latest 10.
    assert_eq!(run("git_log({files: false}) | len").out, "3");
    assert_eq!(
        run(r#"git_log({range: "v1.0.0..HEAD"}) | map(fn(r) => r.subject)"#).out,
        r#"["fix: later","feat!: new api"]"#
    );
    let body = run(r#"git_log({range: "v1.0.0..HEAD", body: true}) | map(fn(r) => r.body)"#).out;
    assert!(body.contains("BREAKING CHANGE: the api changed"), "{body}");
    assert!(body.contains("Body line one.\\nBody line two."), "{body}");
    // body is only there when asked for.
    assert!(!run(r#"git_log({range: "v1.0.0..HEAD"}) | first"#)
        .out
        .contains("\"body\""));
    // A range with more than 10 commits is not cut at 10.
    for i in 0..12 {
        commit(&repo, &format!("f{i}"), &format!("chore: {i}"));
    }
    assert_eq!(run(r#"git_log({range: "v1.0.0..HEAD"}) | len"#).out, "14");
    assert_eq!(
        run(r#"git_log({range: "v1.0.0..HEAD", n: 3}) | len"#).out,
        "3"
    );
    assert_eq!(run(r#"git_log({since: "2020-01-01"}) | len"#).out, "15");
    assert_eq!(run(r#"git_log({since: "2026-03-02"}) | len"#).out, "0");
    // With files too, the body does not disturb the file list.
    let files = run(r#"git_log({range: "v1.0.0..HEAD", body: true, files: true, n: 14}) | map(fn(r) => len(r.files)) | sum"#).out;
    assert_eq!(files, "14");
    // Option injection through range or since.
    for bad in [
        r#"git_log({range: "--all"})"#,
        r#"git_log({range: "-p"})"#,
        r#"git_log({range: ""})"#,
        r#"git_log({range: 5})"#,
        r#"git_log({since: ""})"#,
        r#"git_log({body: "yes"})"#,
    ] {
        assert!(
            run(bad).err.contains("E_BAD_ARG"),
            "{bad}: {}",
            run(bad).err
        );
    }
    // A name that is not a revision is git's refusal, not a path listing.
    assert!(run(r#"git_log({range: "no-such-tag..HEAD"})"#)
        .err
        .contains("E_TOOL_FAILED"));
}

// ---- base64 digests ----

#[test]
fn digests_in_base64_and_sri() {
    let root = dir("b64");
    let b64_256 = "LPJNul+wow4m6DsqxbninhsWHlwfp0JecwQzYpOLmCQ=";
    let sri_384 = "sha384-WeF0h3dEjGnea4ANejO7+5/xtGPkQ1TDVTvNucZm+pASWjx5+QOXvfX2oT3oKGhP";
    assert_eq!(
        out(
            &root,
            r#"crypto_hash("sha256", "hello", {encoding: "base64"})"#
        ),
        format!("\"{b64_256}\"")
    );
    assert_eq!(
        out(
            &root,
            r#"crypto_hash("sha384", "hello", {encoding: "sri"})"#
        ),
        format!("\"{sri_384}\"")
    );
    assert_eq!(
        out(
            &root,
            r#""hello" | crypto_hash("sha384", {encoding: "sri"})"#
        ),
        format!("\"{sri_384}\"")
    );
    assert_eq!(
        out(
            &root,
            r#"crypto_hash("hello", "sha256", {encoding: "hex"})"#
        ),
        r#""2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824""#
    );
    std::fs::write(root.join("ws/f.bin"), b"hello").unwrap();
    assert_eq!(
        out(
            &root,
            r#"crypto_hash_file("f.bin", "sha384", {encoding: "sri"})"#
        ),
        format!("\"{sri_384}\"")
    );
    assert_eq!(
        out(
            &root,
            r#"crypto_hash_file("f.bin", "sha256", {encoding: "base64"})"#
        ),
        format!("\"{b64_256}\"")
    );
    // hex_to_base64 takes binary bytes, which crypto_base64_encode refuses.
    assert_eq!(out(&root, r#"hex_to_base64("ff00fe")"#), r#""/wD+""#);
    assert_eq!(
        out(&root, r#"crypto_hash_file("f.bin") | hex_to_base64"#),
        format!("\"{b64_256}\"")
    );
    assert!(ish(
        &root,
        r#"crypto_base64_encode(crypto_hex_decode("ff00fe"))"#
    )
    .err
    .contains("E_BAD_ARG"));
    for bad in [
        r#"hex_to_base64("abc")"#,
        r#"hex_to_base64("zz")"#,
        r#"crypto_hash("md5", "hello", {encoding: "sri"})"#,
        r#"crypto_hash("sha256", "hello", {encoding: "base32"})"#,
        r#"crypto_hash("sha256", "hello", {format: "base64"})"#,
        r#"crypto_hash_file("f.bin", "sha1", {encoding: "sri"})"#,
    ] {
        assert!(ish(&root, bad).err.contains("E_BAD_ARG"), "{bad}");
    }
}

// ---- network policy ----

/// A small HTTP server on a free loopback port. /ok is 200, /missing 404,
/// /redir a 301 to /ok, /big 3 MB, /slow answers after 6 s.
fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || answer(stream));
        }
    });
    port
}

fn answer(mut s: TcpStream) {
    let mut buf = [0u8; 2048];
    let n = s.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]).to_string();
    let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut reply = |status: &str, extra: &str, body: &[u8]| {
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n",
            body.len()
        );
        let _ = s.write_all(head.as_bytes());
        let _ = s.write_all(body);
    };
    match path.as_str() {
        "/ok" => reply("200 OK", "", b"hello local"),
        "/redir" => reply("301 Moved Permanently", "Location: /ok\r\n", b""),
        "/big" => reply("200 OK", "", &vec![b'x'; 3 * 1024 * 1024]),
        "/slow" => {
            std::thread::sleep(std::time::Duration::from_secs(6));
            reply("200 OK", "", b"late");
        }
        _ => reply("404 Not Found", "", b"no"),
    }
}

#[test]
fn human_mode_downloads_from_loopback() {
    let root = dir("net_human");
    let port = serve();
    let ws = root.join("ws");
    let get =
        |url: &str, file: &str| ish(&root, &format!(r#"web_download("http://{url}", "{file}")"#));
    for host in ["127.0.0.1", "localhost"] {
        let r = get(&format!("{host}:{port}/ok"), "ok.txt");
        assert_eq!(r.out, "true", "{host}: {}", r.err);
        assert_eq!(
            std::fs::read_to_string(ws.join("ok.txt")).unwrap(),
            "hello local"
        );
        let _ = std::fs::remove_file(ws.join("ok.txt"));
    }
    // A 404 is an error and leaves no file.
    let r = get(&format!("127.0.0.1:{port}/missing"), "m.txt");
    assert!(r.err.contains("404"), "{}", r.err);
    assert!(!ws.join("m.txt").exists());
    // A local redirect is followed to the 200.
    let r = get(&format!("127.0.0.1:{port}/redir"), "r.txt");
    assert_eq!(r.out, "true", "{}", r.err);
    assert_eq!(
        std::fs::read_to_string(ws.join("r.txt")).unwrap(),
        "hello local"
    );
    // A larger file arrives whole, and its checksum is that of the bytes sent.
    let r = get(&format!("127.0.0.1:{port}/big"), "big.bin");
    assert_eq!(r.out, "true", "{}", r.err);
    assert_eq!(
        std::fs::metadata(ws.join("big.bin")).unwrap().len(),
        3 * 1024 * 1024
    );
    std::fs::write(ws.join("expected.bin"), vec![b'x'; 3 * 1024 * 1024]).unwrap();
    assert_eq!(
        out(&root, r#"crypto_hash_file("big.bin")"#),
        out(&root, r#"crypto_hash_file("expected.bin")"#)
    );
    // http.get reads it too.
    let r = ish(
        &root,
        &format!(r#"http.get("http://127.0.0.1:{port}/ok").body"#),
    );
    assert_eq!(r.out, r#""hello local""#, "{}", r.err);
    // --timeout stops a slow response and says so.
    let r = run_in(
        &root,
        &ws,
        &["--timeout", "2"],
        &[],
        &format!(r#"web_download("http://127.0.0.1:{port}/slow", "s.txt")"#),
    );
    assert!(
        r.err.contains("E_BUDGET_EXCEEDED") && r.err.contains("--timeout of 2 s"),
        "{}",
        r.err
    );
    assert!(!ws.join("s.txt").exists());
}

#[test]
fn private_ranges_are_allowed_in_human_mode_and_metadata_never() {
    let root = dir("net_ranges");
    // Nothing listens on these: a refusal by policy is E_POLICY_DENY, a
    // connection failure is another code, so the code tells which it was.
    let probe = |url: &str, flags: &[&str], env: &[(&str, &str)]| {
        run_in(
            &root,
            &root.join("ws"),
            flags,
            env,
            &format!(r#"web_download("{url}", "p.txt")"#),
        )
    };
    for denied in [
        "http://169.254.169.254/x",
        "http://[fe80::1]/x",
        "http://[fd00:ec2::254]/x",
        "http://100.100.100.200/x",
        "http://metadata.google.internal/x",
        "http://0.0.0.0/x",
    ] {
        let r = probe(denied, &[], &[]);
        assert!(r.err.contains("E_POLICY_DENY"), "{denied}: {}", r.err);
        // Listing the host does not lift it.
        let host = denied
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap();
        let r = probe(denied, &["--agent"], &[("IRONSHELL_NET_ALLOW", host)]);
        assert!(
            r.err.contains("E_POLICY_DENY"),
            "{denied} listed: {}",
            r.err
        );
    }
    // Private addresses pass the policy in human mode; the request itself then
    // fails (or is refused for another reason), but never as an SSRF refusal.
    for private in [
        "http://10.255.255.1:9/x",
        "http://192.168.254.254:9/x",
        "http://[::1]:9/x",
    ] {
        let r = run_in(
            &root,
            &root.join("ws"),
            &["--timeout", "5"],
            &[],
            &format!(r#"web_download("{private}", "p.txt")"#),
        );
        assert!(!r.err.contains("SSRF"), "{private}: {}", r.err);
    }
}

#[test]
fn agent_mode_refuses_internal_hosts_unless_the_operator_lists_that_host() {
    let root = dir("net_agent");
    let port = serve();
    let ws = root.join("ws");
    let url = format!("http://127.0.0.1:{port}/ok");
    let code = format!(r#"web_download("{url}", "a.txt")"#);
    let agent =
        |env: &[(&str, &str)]| run_in(&root, &ws, &["--agent", "--workspace", "."], env, &code);
    // Not listed: refused, and the hint says how an operator allows it.
    let r = agent(&[]);
    assert!(r.err.contains("E_POLICY_DENY"), "{} {}", r.out, r.err);
    assert!(r.err.contains("lists that exact host"), "{}", r.err);
    assert!(!ws.join("a.txt").exists());
    // `*`, another host, a subdomain-style entry and another port do not admit it.
    for env in ["*", "localhost", "example.com", ".0.0.1", "127.0.0.1:9"] {
        let r = agent(&[("IRONSHELL_NET_ALLOW", env)]);
        assert!(
            r.err.contains("E_POLICY_DENY"),
            "{env}: {} {}",
            r.out,
            r.err
        );
        assert!(!ws.join("a.txt").exists(), "{env}");
    }
    // The exact host, with or without its port, admits it.
    for env in [
        "127.0.0.1",
        &format!("127.0.0.1:{port}"),
        &format!("example.com;127.0.0.1:{port}"),
    ] {
        let _ = std::fs::remove_file(ws.join("a.txt"));
        let r = agent(&[("IRONSHELL_NET_ALLOW", env)]);
        assert_eq!(r.out, "true", "{env}: {}", r.err);
        assert_eq!(
            std::fs::read_to_string(ws.join("a.txt")).unwrap(),
            "hello local"
        );
    }
    // An agent cannot set the variable.
    let r = run_in(
        &root,
        &ws,
        &["--agent", "--workspace", "."],
        &[],
        r#"env_set("IRONSHELL_NET_ALLOW", "127.0.0.1")"#,
    );
    assert!(r.err.contains("E_POLICY_DENY"), "{}", r.err);
}

#[test]
fn reach_of_addresses() {
    use iron_shell::security::{reach_of_ip, reach_of_url, Reach};
    let reach = |s: &str| reach_of_ip(&s.parse().unwrap());
    for ip in [
        "127.0.0.1",
        "127.9.9.9",
        "::1",
        "10.1.2.3",
        "172.16.0.1",
        "172.31.255.255",
        "192.168.1.1",
        "fc00::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
    ] {
        assert_eq!(reach(ip), Reach::Internal, "{ip}");
    }
    for ip in [
        "169.254.169.254",
        "169.254.0.1",
        "fe80::1",
        "fd00:ec2::254",
        "100.100.100.200",
        "::ffff:169.254.169.254",
    ] {
        assert_eq!(reach(ip), Reach::Metadata, "{ip}");
    }
    for ip in [
        "0.0.0.0",
        "::",
        "224.0.0.1",
        "100.64.0.1",
        "198.18.0.1",
        "240.0.0.1",
        "192.0.2.1",
    ] {
        assert_eq!(reach(ip), Reach::Reserved, "{ip}");
    }
    for ip in [
        "8.8.8.8",
        "172.32.0.1",
        "172.15.255.255",
        "1.1.1.1",
        "2606:4700::1111",
    ] {
        assert_eq!(reach(ip), Reach::Public, "{ip}");
    }
    assert_eq!(reach_of_url("http://localhost:8080/"), Reach::Internal);
    assert_eq!(reach_of_url("http://app.localhost/"), Reach::Internal);
    assert_eq!(reach_of_url("http://2130706433/"), Reach::Internal);
    assert_eq!(
        reach_of_url("http://metadata.google.internal/"),
        Reach::Metadata
    );
    assert_eq!(reach_of_url("https://example.com/"), Reach::Public);
}

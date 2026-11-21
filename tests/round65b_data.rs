//! Round 65b: multi-member gzip, multi-document YAML, gzip_read and .gz in
//! open, cron_next and cron_parse, the log timestamp in date_parse, and the
//! small hints (percentile of nothing, record previews).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

fn dir(tag: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round65b_data")
        .join(format!("{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("session")).unwrap();
    std::fs::create_dir_all(root.join("ws")).unwrap();
    root
}

/// Runs `ish <flags> --deterministic -c code` in `<root>/ws`; stdout and stderr.
fn run(root: &Path, flags: &[&str], code: &str) -> (String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(flags)
        .args(["--deterministic", "-c", code])
        .current_dir(root.join("ws"))
        .env("NO_COLOR", "1")
        .env("IRONSHELL_SESSION_DIR", root.join("session"))
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_WORKSPACE")
        .env_remove("IRONSHELL_SECRETS")
        .output()
        .expect("run ish");
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).trim().to_string(),
    )
}

fn ish(root: &Path, code: &str) -> (String, String) {
    run(root, &[], code)
}

fn out(root: &Path, code: &str) -> String {
    ish(root, code).0
}

fn gz(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

fn put(root: &Path, name: &str, bytes: &[u8]) {
    std::fs::write(root.join("ws").join(name), bytes).unwrap();
}

fn members(parts: &[&str]) -> Vec<u8> {
    parts.iter().flat_map(|p| gz(p.as_bytes())).collect()
}

// ---- gzip: every member ----

#[test]
fn gzip_read_and_decompress_read_all_members() {
    let root = dir("members");
    put(&root, "one.txt.gz", &members(&["a\n"]));
    put(&root, "two.txt.gz", &members(&["a\n", "b\n"]));
    put(&root, "three.txt.gz", &members(&["a\n", "b\n", "c\n"]));
    for (name, want) in [
        ("one", r#""a\n""#),
        ("two", r#""a\nb\n""#),
        ("three", r#""a\nb\nc\n""#),
    ] {
        assert_eq!(out(&root, &format!(r#"gzip_read("{name}.txt.gz")"#)), want);
        assert_eq!(
            out(&root, &format!(r#"gzip_decompress("{name}.txt.gz")"#)),
            "true"
        );
        let written = std::fs::read_to_string(root.join("ws").join(format!("{name}.txt"))).unwrap();
        assert_eq!(format!("{written:?}"), want, "{name}");
    }
}

#[test]
fn an_empty_member_adds_nothing_and_does_not_stop_the_read() {
    let root = dir("empty");
    put(&root, "m.gz", &members(&["a\n", "", "b\n"]));
    assert_eq!(out(&root, r#"gzip_read("m.gz")"#), r#""a\nb\n""#);
    put(&root, "e.gz", &members(&[""]));
    assert_eq!(out(&root, r#"gzip_read("e.gz")"#), r#""""#);
}

#[test]
fn bytes_after_the_last_member_are_refused_and_nothing_is_left() {
    let root = dir("garbage");
    let mut bytes = members(&["a\n"]);
    bytes.extend_from_slice(b"GARBAGEGARBAGE");
    put(&root, "g.txt.gz", &bytes);
    let (_, err) = ish(&root, r#"gzip_read("g.txt.gz")"#);
    assert!(
        err.contains("E_BAD_ARG") && err.contains("trailing garbage"),
        "{err}"
    );
    let (_, err) = ish(&root, r#"gzip_decompress("g.txt.gz")"#);
    assert!(
        err.contains("E_BAD_ARG") && err.contains("trailing garbage"),
        "{err}"
    );
    assert!(!root.join("ws").join("g.txt").exists(), "no partial output");
    // A file that is not gzip at all is the plain refusal.
    put(&root, "n.gz", b"plain text, not gzip");
    let (_, err) = ish(&root, r#"gzip_read("n.gz")"#);
    assert!(
        err.contains("E_BAD_ARG") && !err.contains("trailing garbage"),
        "{err}"
    );
}

#[test]
fn the_size_cap_counts_all_members() {
    let root = dir("cap");
    let part = "x".repeat(600);
    let both = root.join("ws").join("both.gz");
    std::fs::write(&both, members(&[&part, &part])).unwrap();
    let both = both.to_string_lossy().to_string();
    let err = iron_shell::builtins::__test_gzip_decompress_capped(&both, 1000)
        .unwrap_err()
        .downcast_ref::<iron_shell::safety::SafetyError>()
        .map(|s| s.code);
    assert_eq!(err, Some(iron_shell::safety::ErrorCode::BudgetExceeded));
    let _ = std::fs::remove_file(root.join("ws").join("both"));
    assert!(iron_shell::builtins::__test_gzip_decompress_capped(&both, 1200).is_ok());
}

#[test]
fn tar_gz_of_several_members_lists_and_extracts() {
    let root = dir("targz");
    let mut tar_bytes = Vec::new();
    {
        let mut b = tar::Builder::new(&mut tar_bytes);
        for (name, body) in [("a.txt", "alpha"), ("b.txt", "beta")] {
            let mut h = tar::Header::new_gnu();
            h.set_size(body.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, body.as_bytes()).unwrap();
        }
        b.finish().unwrap();
    }
    let half = tar_bytes.len() / 2;
    let mut file = gz(&tar_bytes[..half]);
    file.extend(gz(&tar_bytes[half..]));
    put(&root, "m.tar.gz", &file);
    assert_eq!(out(&root, r#"tar_list("m.tar.gz") | len"#), "2");
    assert_eq!(out(&root, r#"archive_test("m.tar.gz")"#), "true");
    assert_eq!(out(&root, r#"tar_extract("m.tar.gz", "x")"#), "true");
    assert_eq!(
        std::fs::read_to_string(root.join("ws/x/b.txt")).unwrap(),
        "beta"
    );
}

// ---- reading a .gz ----

#[test]
fn open_reads_gz_by_the_inner_extension() {
    let root = dir("open_gz");
    put(&root, "a.json.gz", &members(&["{\"a\":", "1}"]));
    put(
        &root,
        "b.jsonl.gz",
        &members(&["{\"n\":1}\n", "{\"n\":2}\n"]),
    );
    put(&root, "c.csv.gz", &members(&["id,v\n1,x\n", "2,y\n"]));
    put(&root, "d.log.gz", &members(&["l1\n", "l2\n"]));
    put(&root, "e.gz", &members(&["plain"]));
    assert_eq!(out(&root, r#"open("a.json.gz").a"#), "1");
    assert_eq!(out(&root, r#"open("b.jsonl.gz") | len"#), "2");
    assert_eq!(
        out(
            &root,
            r#"open("c.csv.gz") | map(fn(r) => r.id) | reduce(fn(a, b) => a + b, 0)"#
        ),
        "3"
    );
    assert_eq!(out(&root, r#"open("d.log.gz") | lines | len"#), "2");
    assert_eq!(out(&root, r#"open("e.gz")"#), r#""plain""#);
}

#[test]
fn cat_of_a_gz_names_gzip_read() {
    let root = dir("cat_gz");
    put(&root, "x.gz", &members(&["hello\n"]));
    let (_, err) = ish(&root, r#"cat("x.gz")"#);
    assert!(
        err.contains("E_BAD_VALUE") && err.contains("gzip_read"),
        "{err}"
    );
    // Data that is not UTF-8 once decompressed is E_BAD_VALUE too.
    put(&root, "bin.gz", &gz(&[0xff, 0xfe, 0x00, 0x80]));
    let (_, err) = ish(&root, r#"gzip_read("bin.gz")"#);
    assert!(err.contains("E_BAD_VALUE"), "{err}");
    // A path given piped in.
    assert_eq!(out(&root, r#""x.gz" | gzip_read"#), r#""hello\n""#);
}

#[test]
fn gzip_read_keeps_the_read_jail_and_the_key_refusal() {
    let root = dir("jail");
    std::fs::write(root.join("outside.gz"), members(&["secret\n"])).unwrap();
    let ws = root.join("ws").to_string_lossy().to_string();
    let (_, err) = run(
        &root,
        &["--agent", "--workspace", &ws],
        r#"gzip_read("../outside.gz")"#,
    );
    assert!(err.contains("E_OUTSIDE_WORKSPACE"), "{err}");
    let (_, err) = run(
        &root,
        &["--agent", "--workspace", &ws],
        r#"open("../outside.gz")"#,
    );
    assert!(err.contains("E_OUTSIDE_WORKSPACE"), "{err}");
    // The fixture is built from parts, so no key-looking text is committed.
    let (begin, end) = (
        format!("-----BEGIN {}-----", "PRIVATE KEY"),
        format!("-----END {}-----", "PRIVATE KEY"),
    );
    let key = format!("{begin}\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\n{end}\n");
    let key = key.as_str();
    put(&root, "k.gz", &members(&["note\n", key]));
    put(&root, "k.txt.gz", &members(&[key]));
    let (_, err) = run(
        &root,
        &["--agent", "--workspace", &ws],
        r#"gzip_read("k.gz")"#,
    );
    assert!(err.contains("E_POLICY_DENY"), "{err}");
    let (_, err) = run(
        &root,
        &["--agent", "--workspace", &ws],
        r#"open("k.txt.gz")"#,
    );
    assert!(err.contains("E_POLICY_DENY"), "{err}");
    // A key in the middle of a line, as in a service-account JSON file, is
    // found in the decompressed text as the file check finds it.
    let sa = format!(
        r#"{{"type": "service_account", "private_key": "{begin}\nMIIEvQIBADANBgkqhkiG9w0BAQEFAASC\n{end}\n"}}"#
    );
    put(&root, "sa.json.gz", &gz(sa.as_bytes()));
    let (_, err) = run(
        &root,
        &["--agent", "--workspace", &ws],
        r#"open("sa.json.gz")"#,
    );
    assert!(err.contains("E_POLICY_DENY"), "{err}");
    let (_, err) = run(
        &root,
        &["--agent", "--workspace", &ws],
        r#"gzip_read("sa.json.gz")"#,
    );
    assert!(err.contains("E_POLICY_DENY"), "{err}");
    // Not in agent mode.
    assert!(out(&root, r#"gzip_read("k.gz")"#).contains("PRIVATE KEY"));
    // A missing file and a directory.
    let (_, err) = ish(&root, r#"gzip_read("nope.gz")"#);
    assert!(err.contains("E_NOT_FOUND"), "{err}");
}

// ---- YAML documents ----

#[test]
fn from_yaml_all_reads_the_documents_as_pyyaml_does() {
    let root = dir("yaml_all");
    let all = |src: &str| out(&root, &format!("from_yaml_all({src}) | to_json"));
    assert_eq!(all(r#""a: 1\n---\nb: 2\n""#), r#""[{\"a\":1},{\"b\":2}]""#);
    assert_eq!(
        all(r#""---\na: 1\n---\nb: 2\n""#),
        r#""[{\"a\":1},{\"b\":2}]""#
    );
    assert_eq!(
        all(r#""---\r\na: 1\r\n---\r\nb: 2\r\n""#),
        r#""[{\"a\":1},{\"b\":2}]""#
    );
    assert_eq!(
        all(r#""a: 1\n...\n---\nb: 2\n...\n""#),
        r#""[{\"a\":1},{\"b\":2}]""#
    );
    assert_eq!(
        all(r##""# top\na: 1\n---\n# only a comment\n---\nb: 2\n""##),
        r#""[{\"a\":1},null,{\"b\":2}]""#
    );
    assert_eq!(all(r#""---\n---\na: 1\n""#), r#""[null,{\"a\":1}]""#);
    assert_eq!(all(r#""a: 1\n---\n""#), r#""[{\"a\":1},null]""#);
    assert_eq!(all(r#""""#), r#""[]""#);
    assert_eq!(all(r##""# nothing\n""##), r#""[]""#);
    assert_eq!(all(r#""- 1\n- 2\n""#), r#""[[1,2]]""#);
}

#[test]
fn from_yaml_refuses_several_documents_and_names_from_yaml_all() {
    let root = dir("yaml_one");
    let (_, err) = ish(&root, r#"from_yaml("a: 1\n---\nb: 2\n")"#);
    assert!(
        err.contains("E_BAD_VALUE") && err.contains("from_yaml_all"),
        "{err}"
    );
    // A single document is unchanged, with a leading marker too.
    assert_eq!(out(&root, r#"from_yaml("---\na: 1\n").a"#), "1");
    // An error keeps the line of the whole text.
    let (_, err) = ish(&root, r#"from_yaml_all("a: 1\n---\nb: 2\n  c: 3\n")"#);
    assert!(err.contains("line 4"), "{err}");
}

#[test]
fn open_of_a_yaml_file_with_several_documents_is_an_array() {
    let root = dir("yaml_open");
    put(
        &root,
        "k8s.yaml",
        b"---\r\nkind: A\r\n---\r\nkind: B\r\n---\r\nkind: C\r\n",
    );
    put(&root, "one.yml", b"kind: A\nn: 2\n");
    put(&root, "empty.yaml", b"# nothing\n");
    assert_eq!(
        out(
            &root,
            r#"open("k8s.yaml") | map(fn(d) => d.kind) | join(",")"#
        ),
        r#""A,B,C""#
    );
    assert_eq!(out(&root, r#"open("one.yml").n"#), "2");
    assert_eq!(out(&root, r#"open("empty.yaml")"#), "null");
}

// ---- cron ----

fn next(root: &Path, expr: &str, n: u32, from: &str) -> String {
    out(
        root,
        &format!(r#"cron_next("{expr}", {n}, "{from}") | join(" ")"#),
    )
    .replace('"', "")
}

#[test]
fn cron_next_steps_lists_ranges_and_names() {
    let root = dir("cron");
    assert_eq!(
        next(&root, "*/15 * * * *", 3, "2026-11-21T10:07:30Z"),
        "2026-11-21T10:15:00Z 2026-11-21T10:30:00Z 2026-11-21T10:45:00Z"
    );
    // A time equal to `from` is not returned.
    assert_eq!(
        next(&root, "0 9,17 * * *", 2, "2026-11-21T09:00:00Z"),
        "2026-11-21T17:00:00Z 2026-11-22T09:00:00Z"
    );
    assert_eq!(
        next(&root, "0 0 1 JAN-MAR *", 4, "2026-11-21T00:00:00Z"),
        "2027-01-01T00:00:00Z 2027-02-01T00:00:00Z 2027-03-01T00:00:00Z 2028-01-01T00:00:00Z"
    );
    assert_eq!(
        next(&root, "30 8 * * mon-fri", 3, "2026-11-20T09:00:00Z"),
        "2026-11-23T08:30:00Z 2026-11-24T08:30:00Z 2026-11-25T08:30:00Z"
    );
    // Sunday is 0 and 7.
    assert_eq!(
        next(&root, "0 0 * * 7", 1, "2026-11-21T00:00:00Z"),
        next(&root, "0 0 * * 0", 1, "2026-11-21T00:00:00Z")
    );
}

#[test]
fn cron_next_feb_29_month_ends_and_the_or_rule() {
    let root = dir("cron_cal");
    assert_eq!(
        next(&root, "0 0 29 2 *", 2, "2026-11-21T00:00:00Z"),
        "2028-02-29T00:00:00Z 2032-02-29T00:00:00Z"
    );
    // 2100 is not a leap year: the gap is eight years.
    assert_eq!(
        next(&root, "0 0 29 2 *", 1, "2096-03-01T00:00:00Z"),
        "2104-02-29T00:00:00Z"
    );
    assert_eq!(
        next(&root, "59 23 31 * *", 3, "2026-01-31T23:59:00Z"),
        "2026-03-31T23:59:00Z 2026-05-31T23:59:00Z 2026-07-31T23:59:00Z"
    );
    assert_eq!(
        next(&root, "59 23 31 12 *", 1, "2026-12-31T23:59:00Z"),
        "2027-12-31T23:59:00Z"
    );
    // Both day fields restricted: the 13th or a Friday.
    assert_eq!(
        next(&root, "0 0 13 * FRI", 3, "2026-11-01T00:00:00Z"),
        "2026-11-06T00:00:00Z 2026-11-13T00:00:00Z 2026-11-20T00:00:00Z"
    );
    // A star on one side: both must hold (Mondays in March).
    assert_eq!(
        next(&root, "0 0 * MAR MON", 1, "2026-11-21T00:00:00Z"),
        "2027-03-01T00:00:00Z"
    );
    // A day that no month has is refused, not searched forever.
    let (_, err) = ish(
        &root,
        r#"cron_next("0 0 31 2 *", 1, "2026-01-01T00:00:00Z")"#,
    );
    assert!(err.contains("E_BAD_VALUE"), "{err}");
}

#[test]
fn cron_shorthands_defaults_and_errors() {
    let root = dir("cron_misc");
    assert_eq!(
        next(&root, "@hourly", 1, "2026-11-21T10:00:00Z"),
        "2026-11-21T11:00:00Z"
    );
    assert_eq!(
        next(&root, "@daily", 1, "2026-11-21T10:00:00Z"),
        "2026-11-22T00:00:00Z"
    );
    assert_eq!(
        next(&root, "@weekly", 1, "2026-11-21T10:00:00Z"),
        "2026-11-22T00:00:00Z"
    );
    assert_eq!(
        next(&root, "@monthly", 1, "2026-11-21T10:00:00Z"),
        "2026-12-01T00:00:00Z"
    );
    assert_eq!(
        next(&root, "@yearly", 1, "2026-11-21T10:00:00Z"),
        "2027-01-01T00:00:00Z"
    );
    let (_, err) = ish(&root, r#"cron_next("@reboot")"#);
    assert!(
        err.contains("E_BAD_ARG") && err.contains("no next time"),
        "{err}"
    );
    for bad in [
        "* * * *",
        "60 * * * *",
        "*/0 * * * *",
        "5-1 * * * *",
        "* * * FOO *",
        "@sometimes",
    ] {
        let (_, err) = ish(&root, &format!(r#"cron_next("{bad}")"#));
        assert!(err.contains("E_BAD_ARG"), "{bad}: {err}");
    }
    let (_, err) = ish(&root, r#"cron_next("* * * * *", 0)"#);
    assert!(err.contains("E_BAD_ARG"), "{err}");
    // Defaults: one time, after now, to the minute.
    assert_eq!(out(&root, r#"cron_next("* * * * *") | len"#), "1");
    assert_eq!(out(&root, r#"cron_next("* * * * *", 5) | len"#), "5");
    assert_eq!(
        out(
            &root,
            r#"(cron_next("* * * * *")[0] | date_diff(now_iso(), "seconds")) <= 60"#
        ),
        "true"
    );
    assert_eq!(
        out(
            &root,
            r#""*/10 * * * *" | cron_next(2, "2026-11-21T10:00:00Z") | last"#
        ),
        r#""2026-11-21T10:20:00Z""#
    );
}

#[test]
fn cron_parse_expands_the_fields() {
    let root = dir("cron_parse");
    assert_eq!(
        out(
            &root,
            r#"cron_parse("*/20 9-10 * * MON-FRI").minute | to_json"#
        ),
        r#""[0,20,40]""#
    );
    assert_eq!(
        out(
            &root,
            r#"cron_parse("*/20 9-10 * * MON-FRI").day_of_week | to_json"#
        ),
        r#""[1,2,3,4,5]""#
    );
    assert_eq!(
        out(&root, r#"cron_parse("0 0 13 * FRI").day_rule"#),
        r#""or""#
    );
    assert_eq!(
        out(&root, r#"cron_parse("0 0 * * FRI").day_rule"#),
        r#""and""#
    );
    assert_eq!(
        out(&root, r#"cron_parse("0 0 1,15 jan,jul *").month | to_json"#),
        r#""[1,7]""#
    );
    assert_eq!(
        out(&root, r#"cron_parse("5/25 * * * *").minute | to_json"#),
        r#""[5,30,55]""#
    );
}

// ---- dates ----

#[test]
fn date_parse_reads_the_log_timestamp_and_the_hint_names_the_format_argument() {
    let root = dir("date");
    assert_eq!(
        out(&root, r#"date_parse("10/Oct/2026:13:55:36 +0000")"#),
        r#""2026-10-10T13:55:36Z""#
    );
    assert_eq!(
        out(&root, r#"date_parse("10/Oct/2026:13:55:36 +0200")"#),
        r#""2026-10-10T11:55:36Z""#
    );
    assert_eq!(
        out(&root, r#"date_parse("10/Oct/2026:13:55:36 -0530")"#),
        r#""2026-10-10T19:25:36Z""#
    );
    // The offset is required by the log layout: without it the text is not a date.
    let (_, err) = ish(&root, r#"date_parse("10/Oct/2026:13:55:36")"#);
    assert!(
        err.contains("%d/%b/%Y:%H:%M:%S %z") && err.contains("format"),
        "{err}"
    );
    let (_, err) = ish(&root, r#"date_parse("garbage")"#);
    assert!(err.contains("second argument"), "{err}");
}

// ---- hints ----

#[test]
fn percentile_of_nothing_asks_about_the_glob() {
    let root = dir("pct");
    assert_eq!(out(&root, r#"glob("nomatch*")"#), "[]");
    let (_, err) = ish(&root, r#"glob("nomatch*") | percentile(50)"#);
    assert!(err.contains("did a glob or filter match nothing"), "{err}");
    let (_, err) = ish(&root, r#"glob("nomatch*") | median"#);
    assert!(err.contains("did a glob or filter match nothing"), "{err}");
}

#[test]
fn a_record_preview_shows_scalar_values_and_says_types_are_types() {
    let root = dir("preview");
    // --deterministic keeps results whole, so this run is without it.
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args([
            "--agent",
            "-c",
            r#"{errors: 5, name: "x", bursts: arr.range(5000)}"#,
        ])
        .current_dir(root.join("ws"))
        .env("IRONSHELL_SESSION_DIR", root.join("session"))
        .env_remove("IRONSHELL_MODE")
        .env_remove("IRONSHELL_DETERMINISTIC")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("errors=5"), "{text}");
    assert!(text.contains(r#"name="x""#), "{text}");
    assert!(text.contains(r#"bursts="array<int>""#), "{text}");
    assert!(
        text.contains("@preview") && text.contains("shows its type"),
        "{text}"
    );
}

//! Round 31: zip archives are written and read in-process (`zip_extract` once
//! answered `true` and wrote nothing; `zip_create` dropped empty folders and
//! wrote no archive for a lone empty folder), a corrupt zip is E_BAD_ARG, and
//! the crypto encoders refuse binary hex, a lone algorithm name, accept
//! unpadded and URL-safe base64, and gain `url_encode` / `url_decode`.
//!
//! These drive the real binary in a fresh temporary directory; the session
//! journal lives in its own temporary directory, so a test never touches the
//! repository's journal.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    ws: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-round31-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            ws: root.join("ws"),
            session: root.join("session"),
            root,
        };
        std::fs::create_dir_all(&d.ws).unwrap();
        std::fs::create_dir_all(&d.session).unwrap();
        d
    }

    fn ish(&self, expr: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["-c", expr])
            .current_dir(&self.ws)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env("IRONSHELL_JOURNAL", "on")
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_AGENT")
            .env_remove("IRONSHELL_WORKSPACE")
            .output()
            .expect("run ish")
    }

    fn run(&self, expr: &str) -> Output {
        let out = self.ish(expr);
        assert!(out.status.success(), "{expr}: {}", both(&out));
        out
    }

    /// The printed value of `expr`, whitespace and quotes removed.
    fn value(&self, expr: &str) -> String {
        let out = self.run(expr);
        String::from_utf8_lossy(&out.stdout)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .trim_matches('"')
            .to_string()
    }

    /// The text of a failed call (stdout and stderr), which must have failed.
    fn refused(&self, expr: &str) -> String {
        let out = self.ish(expr);
        let text = both(&out);
        assert!(
            !out.status.success() || text.contains("E_"),
            "{expr} was not refused: {text}"
        );
        text
    }

    /// Write a zip with `entries` ((name, Some(content)) or (name, None) for a
    /// directory) at `ws/name`.
    fn make_zip(&self, name: &str, entries: &[(&str, Option<&str>)]) {
        let file = std::fs::File::create(self.ws.join(name)).unwrap();
        let mut w = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (n, content) in entries {
            match content {
                Some(c) => {
                    w.start_file(*n, opts).unwrap();
                    w.write_all(c.as_bytes()).unwrap();
                }
                None => w.add_directory(*n, opts).unwrap(),
            }
        }
        w.finish().unwrap();
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn both(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn zip_extract_writes_a_nested_file_and_an_empty_folder() {
    let d = Dirs::new("extract");
    d.make_zip(
        "a.zip",
        &[
            ("hello.txt", Some("hello zip")),
            ("deep/er/note.txt", Some("nested")),
            ("empty/", None),
        ],
    );
    assert_eq!(d.value(r#"zip_extract("a.zip", "out")"#), "true");
    assert_eq!(
        std::fs::read_to_string(d.ws.join("out/hello.txt")).unwrap(),
        "hello zip"
    );
    assert_eq!(
        std::fs::read_to_string(d.ws.join("out/deep/er/note.txt")).unwrap(),
        "nested"
    );
    assert!(d.ws.join("out/empty").is_dir(), "the empty folder is kept");
    // The alias behaves the same.
    assert_eq!(d.value(r#"unzip("a.zip", "out2")"#), "true");
    assert!(d.ws.join("out2/hello.txt").is_file());
}

#[test]
fn zip_extract_refuses_every_escaping_name_before_writing() {
    let d = Dirs::new("slip");
    for (i, bad) in [
        "../evil.txt",
        "a/../../evil.txt",
        "/abs.txt",
        "C:/drive.txt",
        "\\\\host\\share\\x.txt",
        "a\\..\\..\\evil.txt",
    ]
    .iter()
    .enumerate()
    {
        let name = format!("bad{i}.zip");
        d.make_zip(&name, &[("fine.txt", Some("ok")), (bad, Some("x"))]);
        let dest = format!("dest{i}");
        let text = d.refused(&format!(r#"zip_extract("{name}", "{dest}")"#));
        assert!(text.contains("E_POLICY_DENY"), "{bad}: {text}");
        assert!(
            !d.ws.join(&dest).join("fine.txt").exists(),
            "{bad}: nothing may be written before the refusal"
        );
    }
    assert!(!d.root.join("evil.txt").exists());
}

#[test]
fn zip_extract_of_a_corrupt_zip_is_a_bad_argument() {
    let d = Dirs::new("corrupt");
    std::fs::write(d.ws.join("junk.zip"), b"this is not a zip archive at all").unwrap();
    let text = d.refused(r#"zip_extract("junk.zip", "out")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("not a zip archive"), "{text}");
    assert!(!text.contains("service is running"), "{text}");
}

#[test]
fn zip_create_keeps_empty_folders_and_round_trips() {
    let d = Dirs::new("create");
    std::fs::create_dir_all(d.ws.join("tt/in/sub")).unwrap();
    std::fs::write(d.ws.join("tt/in/f.txt"), "file").unwrap();
    d.run(r#"zip_create("t.zip", "tt/in")"#);
    let listing = d.value(r#"zip_list("t.zip") | map(fn(e) => e.name)"#);
    assert!(listing.contains("in/sub/"), "empty folder kept: {listing}");
    assert!(listing.contains("in/f.txt"), "{listing}");
    d.run(r#"zip_extract("t.zip", "back")"#);
    assert_eq!(
        std::fs::read_to_string(d.ws.join("back/in/f.txt")).unwrap(),
        "file"
    );
    assert!(d.ws.join("back/in/sub").is_dir());
}

#[test]
fn zip_create_of_only_an_empty_folder_writes_an_archive() {
    let d = Dirs::new("emptyonly");
    std::fs::create_dir_all(d.ws.join("hollow")).unwrap();
    d.run(r#"zip_create("h.zip", "hollow")"#);
    assert!(d.ws.join("h.zip").is_file());
    assert!(d
        .value(r#"zip_list("h.zip") | map(fn(e) => e.name)"#)
        .contains("hollow/"));
}

#[test]
fn zip_create_names_a_missing_member_and_a_duplicate_cleanly() {
    let d = Dirs::new("errs");
    std::fs::write(d.ws.join("a.txt"), "a").unwrap();
    let text = d.refused(r#"zip_create("m.zip", "nope.txt")"#);
    assert!(text.contains("E_NOT_FOUND"), "{text}");
    assert!(!text.to_lowercase().contains("powershell"), "{text}");
    assert!(
        !d.ws.join("m.zip").exists(),
        "no archive for a refused call"
    );

    let text = d.refused(r#"zip_create("dup.zip", "a.txt", "a.txt")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(!text.to_lowercase().contains("powershell"), "{text}");
    assert!(!d.ws.join("dup.zip").exists());
}

#[test]
fn zip_add_refreshes_and_adds_and_a_missing_archive_says_so() {
    let d = Dirs::new("add");
    std::fs::write(d.ws.join("a.txt"), "one").unwrap();
    std::fs::write(d.ws.join("b.txt"), "two").unwrap();
    d.run(r#"zip_create("z.zip", "a.txt")"#);
    std::fs::write(d.ws.join("a.txt"), "ONE").unwrap();
    d.run(r#"zip_add("z.zip", "a.txt", "b.txt")"#);
    d.run(r#"zip_extract("z.zip", "x")"#);
    assert_eq!(
        std::fs::read_to_string(d.ws.join("x/a.txt")).unwrap(),
        "ONE"
    );
    assert_eq!(
        std::fs::read_to_string(d.ws.join("x/b.txt")).unwrap(),
        "two"
    );

    let text = d.refused(r#"zip_add("missing.zip", "a.txt")"#);
    assert!(text.contains("E_NOT_FOUND"), "{text}");
    assert!(!text.contains("list the available archives"), "{text}");
    assert!(text.contains("no archive at this path"), "{text}");
}

#[test]
fn hex_decode_refuses_binary_bytes() {
    let d = Dirs::new("hex");
    assert_eq!(d.value(r#"crypto.hex_decode("6869")"#), "hi");
    let text = d.refused(r#"crypto.hex_decode("ff00fe")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("binary"), "{text}");
}

#[test]
fn hash_with_only_an_algorithm_name_is_refused_but_piped_data_works() {
    let d = Dirs::new("hash");
    let text = d.refused(r#"crypto.hash("sha256")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
    assert!(text.contains("is an algorithm name"), "{text}");
    assert_eq!(
        d.value(r#""x" | crypto.hash("sha256")"#),
        "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881"
    );
    assert_eq!(
        d.value(r#"crypto.hash("sha256", "x")"#),
        "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881"
    );
}

#[test]
fn crypto_module_has_hash_file_checksum_and_random_string() {
    let d = Dirs::new("members");
    std::fs::write(d.ws.join("f.txt"), "x").unwrap();
    let want = "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881";
    assert_eq!(d.value(r#"crypto.hash_file("f.txt")"#), want);
    assert_eq!(d.value(r#"crypto.checksum("f.txt")"#), want);
    assert_eq!(d.value(r#"len(crypto.random_string(8))"#), "8");
}

#[test]
fn base64_decode_is_strict_about_padding_and_alphabet() {
    let d = Dirs::new("b64");
    assert_eq!(d.value(r#"base64_decode("aGVsbG8=")"#), "hello");
    // "???" is Pz8/ in the standard alphabet.
    assert_eq!(d.value(r#"base64_decode("Pz8/")"#), "???");
    // Missing or short padding is refused, as Python's b64decode(validate=True) does.
    for expr in [
        r#"base64_decode("aGVsbG8")"#,
        r#"base64_decode("YQ=")"#,
        r#"base64_decode("aGk")"#,
    ] {
        let text = d.refused(expr);
        assert!(text.contains("E_BAD_VALUE"), "{expr}: {text}");
        assert!(text.contains("multiple of 4"), "{expr}: {text}");
    }
    // The URL-safe alphabet is refused, and the message names it.
    let text = d.refused(r#"base64_decode("-_8=")"#);
    assert!(text.contains("E_BAD_VALUE"), "{text}");
    assert!(text.contains("URL-safe"), "{text}");
    let text = d.refused(r#"base64_decode("!!!!")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
}

#[test]
fn url_encode_and_decode_follow_rfc_3986() {
    let d = Dirs::new("url");
    assert_eq!(d.value(r#"url_encode("a b/c")"#), "a%20b%2Fc");
    assert_eq!(
        d.value(r#""a&b=c~d-e.f_g" | url_encode"#),
        "a%26b%3Dc~d-e.f_g"
    );
    assert_eq!(d.value(r#"str.url_encode("a b")"#), "a%20b");
    // `value` removes whitespace, so the decoded "a b/c" reads "ab/c".
    assert_eq!(d.value(r#"url_decode("a%20b%2Fc")"#), "ab/c");
    assert_eq!(d.value(r#"str.url_decode("%7E")"#), "~");
    assert_eq!(d.value(r#""%C3%A9" | url_decode | len"#), "1");
    // A + is a plus, not a space.
    assert_eq!(d.value(r#"url_decode("a+b")"#), "a+b");
    let text = d.refused(r#"url_decode("100%")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
    let text = d.refused(r#"url_decode("%zz")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
    let text = d.refused(r#"url_decode("%FF%FE")"#);
    assert!(text.contains("E_BAD_ARG"), "{text}");
}

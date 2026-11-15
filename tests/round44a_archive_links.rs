//! Round 44a: archive extraction does not write through a link or junction
//! inside the destination to a place outside it.
//!
//! `zip_extract` and `tar_extract` judged an entry name lexically (no `..`, not
//! absolute), so the entry `jout/secret.txt` passed and was written through a
//! link `jout` that pointed outside the workspace: it overwrote
//! `outside/secret.txt` and exited 0. Each landing place is now resolved as the
//! jail resolves a write, before anything is written. Under the jail the refusal
//! is E_OUTSIDE_WORKSPACE; in a plain human session it is E_POLICY_DENY; and
//! `--explain` predicts it.
//!
//! These drive the real binary on temporary directories only. The link is a
//! symbolic link on Unix and a junction on Windows (`mklink /J`, which needs no
//! privilege); a test skips with a message when it cannot be made.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Dirs {
    root: PathBuf,
    ws: PathBuf,
    outside: PathBuf,
    session: PathBuf,
}

impl Dirs {
    fn new(tag: &str) -> Dirs {
        let root = std::env::temp_dir().join(format!("ae-r44a-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let d = Dirs {
            ws: root.join("ws"),
            outside: root.join("outside"),
            session: root.join("session"),
            root,
        };
        for p in [&d.ws, &d.outside, &d.session] {
            std::fs::create_dir_all(p).unwrap();
        }
        d
    }

    /// `ws/jout` pointing at `outside`; false (and a message) when it cannot be made.
    fn link_out(&self) -> bool {
        link_dir(&self.outside, &self.ws.join("jout"))
    }

    fn ish(&self, jailed: bool, expr: &str) -> Output {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ish"));
        c.args(["-c", expr])
            .current_dir(&self.ws)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env("IRONSHELL_JOURNAL", "on")
            .env_remove("IRONSHELL_AGENT");
        if jailed {
            c.env("IRONSHELL_MODE", "agent")
                .env("IRONSHELL_WORKSPACE", &self.ws);
        } else {
            c.env_remove("IRONSHELL_MODE")
                .env_remove("IRONSHELL_WORKSPACE");
        }
        c.output().expect("run ish")
    }

    fn explain(&self, expr: &str) -> serde_json::Value {
        let out = Command::new(env!("CARGO_BIN_EXE_ish"))
            .args(["--explain", "-c", expr])
            .current_dir(&self.ws)
            .env("NO_COLOR", "1")
            .env("IRONSHELL_SESSION_DIR", &self.session)
            .env_remove("IRONSHELL_MODE")
            .env_remove("IRONSHELL_WORKSPACE")
            .output()
            .expect("run ish --explain");
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("explain prints JSON: {e}: {}", text(&out)))
    }

    fn make_zip(&self, name: &str, entries: &[(&str, &str)]) {
        let file = std::fs::File::create(self.ws.join(name)).unwrap();
        let mut w = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (n, content) in entries {
            w.start_file(*n, opts).unwrap();
            w.write_all(content.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }

    /// A tar of `files` (name, content) and `links` (name, target), in that order.
    fn tar_bytes(files: &[(&str, &str)], links: &[(&str, &str)]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (name, target) in links {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_path(name).unwrap();
            h.set_link_name(target).unwrap();
            h.set_size(0);
            h.set_mode(0o777);
            h.set_cksum();
            b.append(&h, std::io::empty()).unwrap();
        }
        for (name, content) in files {
            let mut h = tar::Header::new_gnu();
            h.set_path(name).unwrap();
            h.set_size(content.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append(&h, content.as_bytes()).unwrap();
        }
        b.into_inner().unwrap()
    }

    fn make_tar(&self, name: &str, files: &[(&str, &str)], links: &[(&str, &str)]) {
        std::fs::write(self.ws.join(name), Dirs::tar_bytes(files, links)).unwrap();
    }

    fn make_tar_gz(&self, name: &str, files: &[(&str, &str)], links: &[(&str, &str)]) {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&Dirs::tar_bytes(files, links)).unwrap();
        std::fs::write(self.ws.join(name), gz.finish().unwrap()).unwrap();
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        // Remove the link first: a recursive delete must not follow it.
        let jout = self.ws.join("jout");
        if std::fs::symlink_metadata(&jout).is_ok() {
            let _ = std::fs::remove_dir(&jout).or_else(|_| std::fs::remove_file(&jout));
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).expect("symlink");
    true
}

/// A junction, made with `mklink /J`: it needs no privilege.
#[cfg(windows)]
fn link_dir(target: &Path, link: &Path) -> bool {
    match Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
    {
        Ok(o) if o.status.success() => true,
        other => {
            eprintln!("skipped: mklink /J failed: {other:?}");
            false
        }
    }
}

fn present(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

const SECRET: &str = "keep me";

fn seed_secret(d: &Dirs) {
    std::fs::write(d.outside.join("secret.txt"), SECRET).unwrap();
}

fn secret_intact(d: &Dirs) -> bool {
    std::fs::read_to_string(d.outside.join("secret.txt")).is_ok_and(|s| s == SECRET)
}

#[test]
fn zip_extract_through_a_link_out_of_the_workspace_is_refused() {
    let d = Dirs::new("zip-jail");
    if !d.link_out() {
        return;
    }
    seed_secret(&d);
    d.make_zip("h.zip", &[("jout/secret.txt", "overwritten")]);
    let out = d.ish(true, "zip_extract('h.zip','.')");
    let shown = text(&out);
    assert!(!out.status.success(), "{shown}");
    assert!(shown.contains("E_OUTSIDE_WORKSPACE"), "{shown}");
    assert!(
        secret_intact(&d),
        "outside/secret.txt was overwritten: {shown}"
    );
}

#[test]
fn zip_extract_through_a_link_is_refused_without_a_jail_too() {
    let d = Dirs::new("zip-human");
    if !d.link_out() {
        return;
    }
    seed_secret(&d);
    d.make_zip("h.zip", &[("jout/secret.txt", "overwritten")]);
    let out = d.ish(false, "zip_extract('h.zip','.')");
    let shown = text(&out);
    assert!(!out.status.success(), "{shown}");
    assert!(shown.contains("E_POLICY_DENY"), "{shown}");
    assert!(
        secret_intact(&d),
        "outside/secret.txt was overwritten: {shown}"
    );
}

#[test]
fn zip_extract_refuses_before_writing_any_member() {
    let d = Dirs::new("zip-atomic");
    if !d.link_out() {
        return;
    }
    d.make_zip("h.zip", &[("first.txt", "a"), ("jout/evil.txt", "b")]);
    let out = d.ish(true, "zip_extract('h.zip','.')");
    assert!(!out.status.success(), "{}", text(&out));
    assert!(!present(&d.ws.join("first.txt")), "a member was written");
    assert!(!present(&d.outside.join("evil.txt")));
}

#[test]
fn tar_and_tar_gz_extract_through_a_link_are_refused() {
    for (tag, gz) in [("tar", false), ("targz", true)] {
        let d = Dirs::new(tag);
        if !d.link_out() {
            return;
        }
        let files = [("jout/evil.txt", "x")];
        let name = if gz { "h.tar.gz" } else { "h.tar" };
        if gz {
            d.make_tar_gz(name, &files, &[]);
        } else {
            d.make_tar(name, &files, &[]);
        }
        let jailed = d.ish(true, &format!("tar_extract('{name}','.')"));
        let shown = text(&jailed);
        assert!(!jailed.status.success(), "{name}: {shown}");
        assert!(shown.contains("E_OUTSIDE_WORKSPACE"), "{name}: {shown}");
        assert!(!present(&d.outside.join("evil.txt")), "{name}: {shown}");

        let human = d.ish(false, &format!("tar_extract('{name}','.')"));
        let shown = text(&human);
        assert!(!human.status.success(), "{name}: {shown}");
        assert!(shown.contains("E_POLICY_DENY"), "{name}: {shown}");
        assert!(!present(&d.outside.join("evil.txt")), "{name}: {shown}");
    }
}

#[test]
fn a_tar_symlink_that_points_through_a_link_is_refused() {
    // `x` is a symlink to `jout`, which leaves the workspace; `x/evil.txt` would
    // be written through both. The link's target is judged where it lands.
    if cfg!(windows) {
        // Creating a symlink entry needs a privilege there; the junction case
        // above covers the resolution.
        return;
    }
    let d = Dirs::new("tar-chain");
    if !d.link_out() {
        return;
    }
    d.make_tar("h.tar", &[("x/evil.txt", "x")], &[("x", "jout")]);
    let out = d.ish(true, "tar_extract('h.tar','.')");
    let shown = text(&out);
    assert!(!out.status.success(), "{shown}");
    assert!(shown.contains("E_OUTSIDE_WORKSPACE"), "{shown}");
    assert!(!present(&d.outside.join("evil.txt")), "{shown}");
    assert!(!present(&d.ws.join("x")), "a link was created: {shown}");
}

#[test]
fn the_unzip_alias_is_refused_too() {
    let d = Dirs::new("unzip");
    if !d.link_out() {
        return;
    }
    seed_secret(&d);
    d.make_zip("h.zip", &[("jout/secret.txt", "overwritten")]);
    let out = d.ish(true, "unzip('h.zip','.')");
    assert!(!out.status.success(), "{}", text(&out));
    assert!(secret_intact(&d));
}

#[test]
fn an_ordinary_extraction_still_works_beside_the_link() {
    let d = Dirs::new("ordinary");
    if !d.link_out() {
        return;
    }
    d.make_zip("h.zip", &[("sub/a.txt", "hello")]);
    let out = d.ish(true, "zip_extract('h.zip','.')");
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(
        std::fs::read_to_string(d.ws.join("sub").join("a.txt")).unwrap(),
        "hello"
    );
    d.make_tar("h.tar", &[("sub2/b.txt", "hi")], &[]);
    let out = d.ish(true, "tar_extract('h.tar','.')");
    assert!(out.status.success(), "{}", text(&out));
    assert!(present(&d.ws.join("sub2").join("b.txt")));
}

#[test]
fn explain_predicts_the_refusal() {
    let d = Dirs::new("explain");
    if !d.link_out() {
        return;
    }
    d.make_zip("h.zip", &[("jout/secret.txt", "overwritten")]);
    d.make_zip("ok.zip", &[("fine.txt", "x")]);
    let bad = d.explain("zip_extract('h.zip','.')");
    assert_eq!(bad["decision"], "deny", "{bad}");
    let ok = d.explain("zip_extract('ok.zip','.')");
    assert_ne!(ok["decision"], "deny", "{ok}");
}

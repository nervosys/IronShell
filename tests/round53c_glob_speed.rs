//! Round 53c: `glob("tree/**/*")` did a full link-walk jail check for every
//! entry it listed (1260 entries took 12 s while `fs_walk` took under a second).
//! A name listed inside a directory that was already judged is inside it, so
//! only a link needs its own check. The answer, and what the jail hides, are
//! unchanged.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("round53c_glob_speed")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("ws")).expect("ws");
    std::fs::create_dir_all(dir.join("outside")).expect("outside");
    std::fs::create_dir_all(dir.join("session")).expect("session");
    std::fs::write(dir.join("outside").join("secret.txt"), "s").expect("secret");
    dir
}

fn run(dir: &Path, agent: bool, code: &str) -> String {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ish"));
    c.args(["--deterministic", "-c", code])
        .current_dir(dir.join("ws"))
        .env("IRONSHELL_SESSION_DIR", dir.join("session"))
        .env("NO_COLOR", "1")
        .env_remove("IRONSHELL_AGENT")
        .env_remove("IRONSHELL_WORKSPACE");
    if agent {
        c.env("IRONSHELL_MODE", "agent")
            .env("IRONSHELL_WORKSPACE", dir.join("ws"));
    } else {
        c.env_remove("IRONSHELL_MODE");
    }
    let out = c.output().expect("run ish");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn build_tree(dir: &Path) {
    // 40 directories of 10 files each, three levels in.
    for d in 0..40 {
        let sub = dir
            .join("ws")
            .join("tree")
            .join(format!("a{}", d % 4))
            .join(format!("d{d}"));
        std::fs::create_dir_all(&sub).expect("subdir");
        for f in 0..10 {
            std::fs::write(sub.join(format!("f{f}.txt")), "x").expect("file");
        }
    }
}

#[test]
fn a_jailed_deep_glob_lists_every_file_and_directory() {
    let dir = scratch("count");
    build_tree(&dir);
    // 4 + 40 directories and 400 files below tree.
    let n = run(&dir, true, r#"len(glob("tree/**/*"))"#);
    assert_eq!(n, "444", "jailed count");
    let human = run(&dir, false, r#"len(glob("tree/**/*"))"#);
    assert_eq!(human, "444", "unjailed count");
    let files = run(&dir, true, r#"len(glob("tree/**/*.txt"))"#);
    assert_eq!(files, "400");
}

#[cfg(unix)]
#[test]
fn a_link_out_of_the_workspace_is_still_hidden() {
    let dir = scratch("link");
    build_tree(&dir);
    let tree = dir.join("ws").join("tree");
    std::os::unix::fs::symlink(dir.join("outside"), tree.join("a0").join("jout"))
        .expect("dir link");
    std::os::unix::fs::symlink(
        dir.join("outside").join("secret.txt"),
        tree.join("a0").join("d0").join("leak.txt"),
    )
    .expect("file link");
    let listed = run(&dir, true, r#"glob("tree/**/*")"#);
    assert!(
        !listed.contains("jout"),
        "a link out was listed: {}",
        listed.len()
    );
    assert!(!listed.contains("leak.txt"), "a file link out was listed");
    // The same tree without the jail shows them.
    let human = run(&dir, false, r#"glob("tree/**/*")"#);
    assert!(human.contains("jout") && human.contains("leak.txt"));
}

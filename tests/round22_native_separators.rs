//! Round 22: a relative path resolved under the jail uses the platform's
//! separator throughout, so the journal and the audit log record
//! `proj\src\a.py` on Windows rather than `proj\src/a.py`.
//!
//! One test in this binary: it sets process environment.

use iron_shell::safety::resolve_path_str;
use std::path::MAIN_SEPARATOR;

#[test]
fn a_jailed_relative_path_is_joined_with_native_separators() {
    let root = std::env::temp_dir().join(format!("ish_r22sep_{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("workspace");
    std::env::set_var("IRONSHELL_WORKSPACE", &root);
    let resolved = resolve_path_str("src/helpers.py");
    let expected = format!("src{MAIN_SEPARATOR}helpers.py");
    assert!(resolved.ends_with(&expected), "{resolved}");
    if cfg!(windows) {
        assert!(!resolved.contains('/'), "{resolved}");
    }

    let dotted = resolve_path_str("./a/./b.txt");
    assert!(
        dotted.ends_with(&format!("a{MAIN_SEPARATOR}b.txt")),
        "{dotted}"
    );
    assert!(
        !dotted.contains(&format!("{MAIN_SEPARATOR}.{MAIN_SEPARATOR}")),
        "{dotted}"
    );

    let dir = resolve_path_str("out/");
    assert!(
        dir.ends_with(MAIN_SEPARATOR),
        "a trailing separator is kept: {dir}"
    );

    let up = resolve_path_str("../x.txt");
    assert!(
        up.contains(".."),
        "`..` is left for the jail to judge: {up}"
    );
    std::env::remove_var("IRONSHELL_WORKSPACE");
}

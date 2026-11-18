//! A miss on a module member or a bare name suggests the builtin that does the
//! job: `file.is_file` -> `file.exists` / `fs_stat`, `hash_file` and
//! `sha256_file` -> `crypto_hash_file`.

use iron_shell::{eval::eval_program, parser::parse_program};

fn refusal(src: &str) -> String {
    let stmts = parse_program(src).expect("parse");
    match eval_program(&stmts, &mut iron_shell::modules::env_with_modules()) {
        Ok(_) => panic!("expected an error for {src:?}"),
        Err(e) => format!("{e}"),
    }
}

#[test]
fn a_file_type_test_suggests_file_exists_and_fs_stat() {
    for src in [r#"file.is_file("Cargo.toml")"#, r#"file.is_dir("src")"#] {
        let text = refusal(src);
        assert!(text.contains("file.exists"), "{src}: {text}");
        assert!(text.contains("fs_stat"), "{src}: {text}");
    }
}

#[test]
fn a_file_hash_name_suggests_crypto_hash_file() {
    for src in [
        r#"hash_file("Cargo.toml")"#,
        r#"sha256_file("Cargo.toml")"#,
        r#"file.hash("Cargo.toml")"#,
    ] {
        let text = refusal(src);
        assert!(text.contains("crypto_hash_file"), "{src}: {text}");
        assert!(!text.contains("patch_file"), "{src}: {text}");
    }
}

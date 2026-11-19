//! fs_walk results and ls rows' path use `/` on every platform, as glob does.

use iron_shell::value::Value;

fn run(src: &str) -> Value {
    let mut env = iron_shell::modules::env_with_modules();
    let stmts = iron_shell::parser::parse_program(src).expect("parses");
    iron_shell::eval::eval_program(&stmts, &mut env).expect("runs")
}

fn texts(v: Value) -> Vec<String> {
    match v {
        Value::Array(items) => items
            .into_iter()
            .map(|i| match i {
                Value::Str(s) => s,
                other => panic!("an element is a {}", other.type_name()),
            })
            .collect(),
        other => panic!("not an Array: {}", other.type_name()),
    }
}

#[test]
fn fs_walk_and_ls_paths_have_no_backslash() {
    let root = std::env::temp_dir().join(format!("ish-r55c5-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("tree").join("sub")).unwrap();
    std::fs::write(root.join("tree").join("sub").join("a.txt"), "x").unwrap();
    let tree = root.join("tree").to_string_lossy().replace('\\', "/");

    let walked = texts(run(&format!(r#"fs_walk("{tree}")"#)));
    let listed = texts(run(&format!(r#"ls("{tree}/sub") | map(fn(r) => r.path)"#)));
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        walked.iter().any(|p| p.ends_with("/sub/a.txt")),
        "{walked:?}"
    );
    assert!(walked.iter().all(|p| !p.contains('\\')), "{walked:?}");
    assert!(
        listed.iter().any(|p| p.ends_with("/sub/a.txt")),
        "{listed:?}"
    );
    assert!(listed.iter().all(|p| !p.contains('\\')), "{listed:?}");
}

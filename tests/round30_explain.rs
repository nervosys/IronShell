//! `--explain` / `explain_effects` must never call code that can write, delete,
//! execute or reach the network `allow`/`pure`.
//!
//! Found in round 30: `call("rm", "b.txt")`, a `${rm(...)}` interpolation hole,
//! an `import` whose module writes at top level, and a builtin passed as a value
//! were all reported as `allow` with the effect `pure`, and each ran. The
//! analysis may over-approximate (`unknown`, a stronger effect); it may not
//! under-approximate.

use iron_shell::explain::explain;
use iron_shell::value::Value;

struct Report {
    decision: String,
    effects: Vec<String>,
    unresolved: Vec<String>,
    builtins: Vec<String>,
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(xs)) => xs
            .iter()
            .filter_map(|x| match x {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn report(code: &str) -> Report {
    let Value::Record(r) = explain(code).expect("the code parses") else {
        panic!("explain returns a record")
    };
    let decision = match r.get("decision") {
        Some(Value::Str(s)) => s.clone(),
        other => panic!("no decision: {other:?}"),
    };
    let builtins = match r.get("calls") {
        Some(Value::Array(calls)) => calls
            .iter()
            .filter_map(|c| match c {
                Value::Record(c) => match c.get("builtin") {
                    Some(Value::Str(b)) => Some(b.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Report {
        decision,
        effects: strings(r.get("effects")),
        unresolved: strings(r.get("unresolved")),
        builtins,
    }
}

fn has_effect(r: &Report, e: &str) -> bool {
    r.effects.iter().any(|x| x == e)
}

fn assert_allow_pure(code: &str) {
    let r = report(code);
    assert_eq!(r.decision, "allow", "{code}: {:?}", r.effects);
    assert!(
        r.effects.iter().all(|e| e == "pure"),
        "{code}: effects {:?}",
        r.effects
    );
    assert!(r.unresolved.is_empty(), "{code}: {:?}", r.unresolved);
}

fn assert_unknown(code: &str) {
    let r = report(code);
    assert_eq!(r.decision, "unknown", "{code}: {:?}", r.effects);
    assert!(!r.unresolved.is_empty(), "{code}: nothing unresolved");
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ish_r30expl_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The path as an IronShell string literal body (forward slashes).
fn lit(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[test]
fn call_with_a_literal_name_is_that_builtin() {
    let r = report(r#"call("rm", "b.txt")"#);
    assert_eq!(r.decision, "approve");
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
    assert!(r.builtins.iter().any(|b| b == "rm"), "{:?}", r.builtins);
}

#[test]
fn call_through_a_variable_bound_to_a_literal_is_that_builtin() {
    let r = report("name = \"rm\"\ncall(name, \"x\")");
    assert_eq!(r.decision, "approve");
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
}

#[test]
fn call_with_a_computed_name_is_unknown() {
    assert_unknown("n = str.upper(\"rm\")\ncall(n, \"x\")");
    assert_unknown("call(\"r\" + \"m\", \"x\")");
    assert_unknown("mut n = \"len\"\ncall(n, [1])");
    // A parameter that shadows a literal binding is not that literal.
    assert_unknown("n = \"len\"\nf = fn(n) => call(n, \"x\")");
}

#[test]
fn call_of_call_is_followed() {
    let r = report(r#"call("call", "rm", "x")"#);
    assert_eq!(r.decision, "approve");
    assert!(has_effect(&r, "destructive"));
}

#[test]
fn interpolation_holes_are_analysed() {
    let r = report(r#"msg = "x ${rm("c.txt")}""#);
    assert_eq!(r.decision, "approve");
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
    // A nested hole, and a hole in a single-quoted string.
    let r = report(r#"m = "a ${"b ${rm("c")}"}""#);
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
    let r = report("m = 'a ${rm(\"c\")}'");
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
}

#[test]
fn an_interpolation_hole_that_does_not_parse_is_unknown() {
    assert_unknown(r#"m = "a ${1 +} b""#);
}

#[test]
fn an_import_contributes_the_modules_effects() {
    let dir = scratch("imp");
    let writer = dir.join("modw.ish");
    std::fs::write(&writer, "file.write(\"out.txt\", \"x\")\n").unwrap();
    let r = report(&format!("import \"{}\" as w", lit(&writer)));
    assert!(has_effect(&r, "write_local"), "{:?}", r.effects);

    // The effect of a function body in the module counts, and so does a
    // module the module imports (resolved against the module's folder).
    let inner = dir.join("inner.ish");
    std::fs::write(&inner, "pub let wipe = fn(p) => rm(p)\n").unwrap();
    let outer = dir.join("outer.ish");
    std::fs::write(&outer, "import \"inner.ish\" as i\n").unwrap();
    let r = report(&format!("import \"{}\" as o", lit(&outer)));
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
    let r = report(&format!("import {{ wipe }} from \"{}\"", lit(&inner)));
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
}

#[test]
fn an_import_cycle_ends() {
    let dir = scratch("cyc");
    let a = dir.join("a.ish");
    let b = dir.join("b.ish");
    std::fs::write(&a, format!("import \"{}\" as b\n", lit(&b))).unwrap();
    std::fs::write(
        &b,
        format!("import \"{}\" as a\nfile.write(\"o\", \"x\")\n", lit(&a)),
    )
    .unwrap();
    let r = report(&format!("import \"{}\" as a", lit(&a)));
    assert!(has_effect(&r, "write_local"), "{:?}", r.effects);
}

#[test]
fn an_import_that_cannot_be_read_or_parsed_is_unknown() {
    let dir = scratch("bad");
    let missing = dir.join("nope.ish");
    assert_unknown(&format!("import \"{}\" as m", lit(&missing)));
    let broken = dir.join("broken.ish");
    std::fs::write(&broken, "x = [1, 2\n").unwrap();
    assert_unknown(&format!("import \"{}\" as m", lit(&broken)));
}

#[test]
fn a_builtin_used_as_a_value_counts_as_a_call() {
    let r = report(r#"["a"] | map(rm)"#);
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
    assert_ne!(r.decision, "allow");
    let r = report("g = rm");
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
    let r = report("r = {f: rm}");
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
    let r = report("xs = [rm, 1]");
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
}

#[test]
fn builtins_that_run_code_are_not_pure() {
    let r = report(r#"try_repair("rm(\"x\")")"#);
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
    assert_unknown("s = \"1\"\ntry_repair(s)");
    assert_unknown("try_repair(\"1 +\")");
    let r = report(r#"sess_eval("s", "rm(\"x\")")"#);
    assert_ne!(r.decision, "allow");
    assert!(has_effect(&r, "destructive"), "{:?}", r.effects);
    // `eval` and `source` are no builtin: a call to them is unresolved.
    assert_unknown("eval(\"rm(1)\")");
    assert_unknown("source(\"x.ish\")");
}

#[test]
fn genuinely_pure_code_is_still_allowed() {
    assert_allow_pure("1 + 2");
    assert_allow_pure(r#""x ${1 + 1}""#);
    assert_allow_pure(r#"call("len", [1])"#);
    assert_allow_pure("[1, 2] | map(fn(x) => x * 2)");
    assert_allow_pure("f = fn(len) => len + 1");

    let dir = scratch("pure");
    let m = dir.join("modp.ish");
    std::fs::write(&m, "pub let double = fn(x) => x * 2\npub let k = 3\n").unwrap();
    assert_allow_pure(&format!("import \"{}\" as p", lit(&m)));
}

//! System-information builtins: types, field names and invariants only. No
//! assertion depends on the host's values.

use iron_shell::builtins::call;
use iron_shell::env::Env;
use iron_shell::safety::{effect_of, Effect};
use iron_shell::value::Value;
use std::process::{Command, Stdio};

fn run(name: &str, args: Vec<Value>) -> anyhow::Result<Value> {
    let mut env = Env::new();
    call(name, args, &mut env)
}

fn record(v: Value) -> std::collections::BTreeMap<String, Value> {
    match v {
        Value::Record(r) => r.into_iter().collect(),
        other => panic!("expected a Record, got {other:?}"),
    }
}

fn is_iso(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 20 && b[4] == b'-' && b[10] == b'T' && b[13] == b':' && b[19] == b'Z'
}

#[test]
fn net_interfaces_is_read_local_like_hw_network() {
    assert_eq!(effect_of("net_interfaces"), Effect::ReadLocal);
    assert_eq!(effect_of("hw_network"), Effect::ReadLocal);
    // The rest of the net family still sends or reads the network.
    assert_eq!(effect_of("net_ping"), Effect::Network);
}

#[test]
fn proc_list_and_proc_info_agree_on_name_and_memory_field() {
    let pid = i64::from(std::process::id());
    let Value::Array(rows) = run("proc_list", vec![]).expect("proc_list") else {
        panic!("proc_list is not an Array");
    };
    let mine = rows
        .into_iter()
        .map(record)
        .find(|r| r.get("pid") == Some(&Value::Int(pid)))
        .expect("this process is listed");
    assert!(matches!(mine.get("mem"), Some(Value::Int(_))));
    assert!(matches!(mine.get("memory_bytes"), Some(Value::Int(_))));
    assert_eq!(mine.get("mem"), mine.get("memory_bytes"));

    let info = record(run("proc_info", vec![Value::Int(pid)]).expect("proc_info"));
    assert!(matches!(info.get("memory_bytes"), Some(Value::Int(_))));
    assert_eq!(info.get("name"), mine.get("name"));
    match info.get("start_time") {
        Some(Value::Str(s)) => assert!(is_iso(s), "start_time {s:?} is not ISO 8601"),
        other => panic!("start_time is not an ISO String: {other:?}"),
    }
}

#[test]
fn proc_start_time_is_a_string_or_null_never_the_epoch() {
    let pid = i64::from(std::process::id());
    match run("proc_start_time", vec![Value::Int(pid)]).expect("proc_start_time") {
        Value::Str(s) => {
            assert!(is_iso(&s));
            assert_ne!(s, "1970-01-01T00:00:00Z");
        }
        Value::Null => {}
        other => panic!("proc_start_time answered {other:?}"),
    }
}

#[test]
fn an_unknown_pid_is_named_as_a_pid() {
    let e = run("proc_info", vec![Value::Int(4_000_000_000)]).expect_err("no such pid");
    let text = e.to_string();
    assert!(text.contains("E_NOT_FOUND"), "{text}");
    assert!(text.contains("no process with pid 4000000000"), "{text}");
    assert!(!text.contains("no process named"), "{text}");
}

#[test]
fn memory_records_carry_used_and_cpu_mhz_is_an_int() {
    for name in ["sys_mem_info", "hw_memory"] {
        let r = record(run(name, vec![]).expect(name));
        for key in ["total", "free", "available", "used"] {
            assert!(
                matches!(r.get(key), Some(Value::Int(n)) if *n >= 0),
                "{name}.{key}"
            );
        }
        let (Some(Value::Int(t)), Some(Value::Int(u))) = (r.get("total"), r.get("used")) else {
            unreachable!()
        };
        assert!(u <= t, "{name}: used {u} exceeds total {t}");
    }
    if let Ok(v) = run("sys_cpu_info", vec![]) {
        if let Some(mhz) = record(v).get("mhz") {
            assert!(matches!(mhz, Value::Int(_)), "mhz is {mhz:?}");
        }
    }
}

#[test]
fn swap_used_never_exceeds_total() {
    let r = record(run("sys_swap_info", vec![]).expect("sys_swap_info"));
    let (Some(Value::Int(t)), Some(Value::Int(u)), Some(Value::Int(f))) =
        (r.get("total"), r.get("used"), r.get("free"))
    else {
        panic!("sys_swap_info fields are not Ints: {r:?}");
    };
    assert!(*u >= 0 && u <= t && u + f == *t, "{r:?}");
}

#[test]
fn df_rows_have_numeric_twins_of_the_strings() {
    for name in ["df", "sys_disk_info", "hw_disk"] {
        let Value::Array(rows) = run(name, vec![]).expect(name) else {
            panic!("{name} is not an Array");
        };
        for row in rows.into_iter().map(record) {
            for key in ["size", "used", "available", "use_percent"] {
                if row.contains_key(key) {
                    assert!(matches!(row.get(key), Some(Value::Str(_))), "{name}.{key}");
                }
            }
            if let Some(size) = row.get("size_bytes") {
                assert!(matches!(size, Value::Int(n) if *n >= 0));
                assert!(matches!(row.get("used_bytes"), Some(Value::Int(_))));
                assert!(matches!(row.get("available_bytes"), Some(Value::Int(_))));
                assert!(matches!(row.get("use_pct"), Some(Value::Int(p)) if (0..=100).contains(p)));
            }
        }
    }
}

#[test]
fn boot_time_is_stable_across_calls() {
    let a = run("sys_boot_time", vec![]).expect("sys_boot_time");
    let b = run("sys_boot_time", vec![]).expect("sys_boot_time");
    assert_eq!(a, b);
}

#[test]
fn sys_info_documents_missing_as_an_array() {
    let r = record(run("sys_info", vec![]).expect("sys_info"));
    assert!(matches!(r.get("missing"), Some(Value::Array(_))));
    let doc = iron_shell::signature::signature_of("sys_info")
        .map(|s| s.doc)
        .expect("sys_info is declared");
    assert!(doc.contains("missing"), "{doc}");
}

#[test]
fn deterministic_errors_are_json_on_stderr() {
    let out = Command::new(env!("CARGO_BIN_EXE_ish"))
        .args(["--deterministic", "-c", "proc_info(4000000000)"])
        .stdin(Stdio::null())
        .env_remove("IRONSHELL_MODE")
        .env(
            "IRONSHELL_SESSION_DIR",
            std::env::temp_dir().join(format!("ish-r45c-{}", std::process::id())),
        )
        .output()
        .expect("run ish");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    let json: serde_json::Value =
        serde_json::from_str(err.trim()).unwrap_or_else(|_| panic!("stderr is not JSON: {err}"));
    assert_eq!(json["error"]["code"], "E_NOT_FOUND", "{err}");
}

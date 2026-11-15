//! Every `module.function` advertised in `modules.rs` must point at a builtin
//! that actually exists.
//!
//! `modules.rs` maps a user-facing name to a builtin name (`("verify",
//! "crypto_verify")`). Nothing checked that the right-hand side resolves, so
//! aliases drifted from the dispatch table and `crypto.verify(...)` failed with
//! `unknown builtin: crypto_verify` — an advertised API that cannot be called.
//!
//! The failure is *closed* (an error, and `Value::Error` is falsy), so this is a
//! correctness bug rather than a vulnerability. It still matters for the
//! security-relevant modules: `perm.acl_get` and the `sso.*` calls are in the
//! allowlist below, meaning IronShell advertises an access-control surface it
//! does not implement. Nothing should be written that depends on those
//! returning a meaningful answer.
//!
//! The whole `rbac.*` module used to be in that list -- all seven entries, so
//! `rbac.check(...)` answered `unknown builtin` while the README documented it
//! as working. The implementations existed under their own names the whole
//! time; the aliases now point at them, and `rbac.permissions` was withdrawn
//! because nothing implements it.
//!
//! As of 2026-07-30: 919 aliases, 71 dangling; as of 2026-08-24 the allowlist
//! is 64 (the `rbac.*` seven, wired up or withdrawn). The allowlist is the debt;
//! it should only ever shrink. Wiring up or withdrawing an entry means deleting
//! its line here.

use iron_shell::builtins::BUILTIN_LOOKUP;
use iron_shell::modules::all_modules;
use iron_shell::value::Value;

/// Builtins reachable through the fallback `match` at the end of
/// `builtins::call` but absent from `BUILTIN_LOOKUP`.
///
/// Dispatch has two layers: `BUILTIN_LOOKUP` maps a name to an index into a
/// function-pointer table, and a trailing `match` handles the rest. Only the
/// first is public, so these three resolve at runtime yet look missing to any
/// consumer that enumerates `BUILTIN_LOOKUP` — including `agent_api`'s dynamic
/// discovery, whose whole purpose is telling an agent what it may call. Worth
/// unifying; until then, they are dispatchable and must not fail this test.
const DISPATCHABLE_OUTSIDE_LOOKUP: &[&str] = &["agent", "ai", "swarm"];

/// Builtin names a module alias may point at without a builtin behind them.
/// Empty since round 44c: the phantom members were withdrawn from `modules.rs`,
/// and an alias to a builtin that does not exist is a failure again.
///
/// These are genuinely unimplemented features, not typos — the typos (nine of
/// them, e.g. `crypto_key_generate` for `crypto_generate_key` and
/// `gui_window_close` for `gui_close_window`) were corrected rather than
/// allowlisted.
const KNOWN_UNIMPLEMENTED: &[&str] = &[];

/// Whether `name` can actually be dispatched by `builtins::call`.
fn resolves(name: &str) -> bool {
    BUILTIN_LOOKUP.contains_key(name) || DISPATCHABLE_OUTSIDE_LOOKUP.contains(&name)
}

/// Walk every module and yield `(module, exposed_name, builtin_name)`.
fn aliases() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for (module_name, value) in all_modules() {
        let Value::Record(fields) = value else {
            panic!("module {module_name} is not a Record");
        };
        for (exposed, target) in fields {
            if let Value::Builtin(b) = target {
                out.push((module_name.to_string(), exposed, b.name));
            }
        }
    }
    out
}

#[test]
fn no_new_dangling_module_aliases() {
    let dangling: Vec<_> = aliases()
        .into_iter()
        .filter(|(_, _, target)| !resolves(target))
        .filter(|(_, _, target)| !KNOWN_UNIMPLEMENTED.contains(&target.as_str()))
        .collect();

    assert!(
        dangling.is_empty(),
        "module alias(es) point at builtins that do not exist — calling these \
         yields `unknown builtin`:\n{}",
        dangling
            .iter()
            .map(|(m, e, t)| format!("  {m}.{e} -> {t}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The allowlist must not rot: an entry that has since been implemented should
/// be deleted, so the list keeps measuring real debt rather than accumulating
/// stale names.
#[test]
fn allowlist_contains_no_entries_that_now_exist() {
    let stale: Vec<_> = KNOWN_UNIMPLEMENTED
        .iter()
        .filter(|name| resolves(name))
        .collect();

    assert!(
        stale.is_empty(),
        "these are implemented now — remove them from KNOWN_UNIMPLEMENTED: {stale:?}"
    );
}

/// Guard the fix itself: the nine corrected aliases must keep resolving.
#[test]
fn previously_broken_aliases_now_resolve() {
    for (module, exposed) in [
        ("crypto", "verify"),
        ("crypto", "key_generate"),
        ("gui", "window_close"),
        ("gui", "window_focus"),
        ("gui", "window_maximize"),
        ("gui", "window_minimize"),
        ("gui", "window_move"),
        ("gui", "window_resize"),
        ("input", "multi_select"),
    ] {
        let found = aliases()
            .into_iter()
            .find(|(m, e, _)| m == module && e == exposed);
        let Some((_, _, target)) = found else {
            // The alias may legitimately be renamed later; that is not this
            // test's business, so only assert when it is still exposed.
            continue;
        };
        assert!(
            resolves(&target),
            "{module}.{exposed} regressed to a dangling target: {target}"
        );
    }
}

/// What `schema.modules` lists (the ontology's module table) resolves: every
/// member names a builtin that exists, so no listed call answers "unknown
/// builtin".
#[test]
fn every_member_the_schema_lists_resolves_to_a_builtin() {
    let ontology = iron_shell::agent_api::build_language_ontology();
    assert!(!ontology.modules.is_empty());
    let mut phantom = Vec::new();
    for m in &ontology.modules {
        for f in &m.functions {
            if !resolves(&f.builtin) && !iron_shell::builtins::is_dispatched(&f.builtin) {
                phantom.push(format!("{}.{} -> {}", m.name, f.method, f.builtin));
            }
        }
    }
    assert!(phantom.is_empty(), "listed but not callable: {phantom:?}");
}

#[test]
fn the_withdrawn_phantom_members_stay_withdrawn() {
    let all = aliases();
    for (module, exposed) in [("crypto", "jwt_encode"), ("crypto", "key_derive")] {
        assert!(
            !all.iter().any(|(m, e, _)| m == module && e == exposed),
            "{module}.{exposed} is listed again"
        );
    }
}

//! `import` and `export`: file modules.
//!
//! Forms (as `parser.rs` parses them):
//!
//! ```text
//! import "lib/math.ish"                  # every export bound into the importer
//! import "lib/math.ish" as math          # a Record of the exports, bound as `math`
//! import { gcd, lcm as l } from "lib/math.ish"
//! export { a, b as c }                  # marks names in this file as exported
//! export { a } from "lib/math.ish"       # re-export one of another module's exports
//! pub let x = 1                         # `pub` also marks a name as exported
//! ```
//!
//! A module is evaluated in its own fresh environment, at most once per
//! process per canonical path. Only its exported names reach the importer.
//! Relative paths resolve against the directory of the file doing the import
//! (the script's directory, or the directory of the module that imports), or
//! against the current directory (or the workspace, when jailed) at the top
//! level of the REPL and `-c`. Every read goes through the same guard as
//! `file.read`, so agent mode cannot import from outside the workspace.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Result};

use crate::ast::{ExportItem, ImportItem};
use crate::env::Env;
use crate::safety::{self, Effect, ErrorCode, GuardCtx, SafetyError};
use crate::value::Value;

/// Deepest chain of modules that may be loading at once.
pub const MAX_IMPORT_DEPTH: usize = 32;
/// Largest module source accepted, in bytes.
pub const MAX_MODULE_BYTES: u64 = 4 * 1024 * 1024;

/// The exported names of a module, with their values.
type Exports = BTreeMap<String, Value>;

/// Modules evaluated so far in this process, by canonical path.
static CACHE: OnceLock<Mutex<HashMap<PathBuf, Exports>>> = OnceLock::new();

/// What a loaded module defines, under the source string it was imported by.
/// The import hints read it when a name is unknown; nothing else does.
struct ModuleFacts {
    source: String,
    /// Top-level names left without `pub`.
    private: Vec<String>,
    /// Names the module exports.
    exported: Vec<String>,
}

/// One entry per module loaded in this process.
static FACTS: Mutex<Vec<ModuleFacts>> = Mutex::new(Vec::new());

/// `import "path" as name` bindings, as (name, source).
static ALIASES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

thread_local! {
    /// Directory of each file being evaluated, innermost last. The entry
    /// script's directory is pushed by `enter_entry_file`; each module pushes its
    /// own while it runs. Empty at the top level of the REPL and `-c`.
    static BASE_DIRS: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    /// Canonical paths of the modules being loaded, outermost first.
    static LOADING: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

/// Keeps the entry file's directory in scope for the import resolver.
pub struct EntryFileGuard;

impl Drop for EntryFileGuard {
    fn drop(&mut self) {
        BASE_DIRS.with(|b| {
            b.borrow_mut().pop();
        });
    }
}

/// Make `file`'s directory the base for the imports it makes. Hold the guard
/// for as long as the file's statements run.
pub fn enter_entry_file(file: &Path) -> EntryFileGuard {
    let abs = if file.is_absolute() {
        file.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|c| c.join(file))
            .unwrap_or_else(|_| file.to_path_buf())
    };
    let dir = abs
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    BASE_DIRS.with(|b| b.borrow_mut().push(dir));
    EntryFileGuard
}

/// Pops the module's stacks on every exit path, including a panic.
struct LoadFrame;

impl Drop for LoadFrame {
    fn drop(&mut self) {
        LOADING.with(|l| {
            l.borrow_mut().pop();
        });
        BASE_DIRS.with(|b| {
            b.borrow_mut().pop();
        });
    }
}

/// `import` statement.
pub fn import(
    items: &[ImportItem],
    source: &str,
    alias: Option<&str>,
    env: &mut Env,
) -> Result<()> {
    let (path, module) = load(source)?;
    if !items.is_empty() {
        for item in items {
            let value = module
                .get(&item.name)
                .ok_or_else(|| missing_export("import", source, &path, &module, &item.name))?;
            let bound = item.alias.as_deref().unwrap_or(&item.name);
            env.set_var_unchecked(bound.to_string(), value.clone());
        }
    } else if let Some(name) = alias {
        ALIASES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((name.to_string(), source.to_string()));
        // An alias is a `let` binding: its fields cannot be assigned, as a
        // record's cannot. `set_var_unchecked` would mark it mutable.
        env.register_module(name, Value::Record(module));
    } else {
        for (name, value) in module {
            env.set_var_unchecked(name, value);
        }
    }
    Ok(())
}

/// `export { ... }` in the file being run: each name must be defined here and
/// is recorded as exported under its alias, if it has one.
pub fn export_local(items: &[ExportItem], env: &mut Env) -> Result<()> {
    for item in items {
        let value = env
            .get_var(&item.name)
            .cloned()
            .ok_or_else(|| safety::not_found("export", "definition", &item.name))?;
        let name = item.alias.as_deref().unwrap_or(&item.name);
        if name != item.name {
            env.set_var_unchecked(name.to_string(), value);
        }
        env.add_export(name);
    }
    Ok(())
}

/// `export { ... } from "path"`: bind and export names taken from another module.
pub fn reexport(items: &[ExportItem], source: &str, env: &mut Env) -> Result<()> {
    let (path, module) = load(source)?;
    for item in items {
        let value = module
            .get(&item.name)
            .ok_or_else(|| missing_export("export", source, &path, &module, &item.name))?;
        let name = item.alias.as_deref().unwrap_or(&item.name);
        env.set_var_unchecked(name.to_string(), value.clone());
        env.add_export(name);
    }
    Ok(())
}

fn remember_facts(source: &str, private: Vec<String>, exported: Vec<String>) {
    let mut facts = FACTS.lock().unwrap_or_else(|e| e.into_inner());
    facts.retain(|f| f.source != source);
    facts.push(ModuleFacts {
        source: source.to_string(),
        private,
        exported,
    });
}

/// The hint for a name that is unknown but defined in a loaded module without
/// `pub`: the module keeps it private, so it is not reachable from here.
pub(crate) fn unexported_hint(name: &str) -> Option<String> {
    let facts = FACTS.lock().unwrap_or_else(|e| e.into_inner());
    facts
        .iter()
        .find(|f| f.private.iter().any(|p| p == name) && !f.exported.iter().any(|e| e == name))
        .map(|f| {
            format!(
                "`{name}` is defined in {} but not exported; mark it `pub`",
                f.source
            )
        })
}

/// The refusal for a name that `source` does not export. A name the module defines
/// without `pub` says so, as the field form does; otherwise the exports are listed.
fn missing_export(
    builtin: &str,
    source: &str,
    path: &Path,
    module: &Exports,
    name: &str,
) -> anyhow::Error {
    let shown = display(path);
    let hint = if let Some(hint) = private_name_hint(source, name) {
        hint
    } else if module.is_empty() {
        format!("{shown} exports nothing: mark the definitions you want to use `pub`")
    } else {
        let names: Vec<&str> = module.keys().map(String::as_str).collect();
        format!(
            "the exports of {shown} are {}; use one of them",
            names.join(", ")
        )
    };
    anyhow::Error::new(SafetyError {
        code: ErrorCode::NotFound,
        message: format!("{builtin}: no export in module '{shown}' named '{name}'"),
        builtin: builtin.to_string(),
        hint,
        approval: None,
        did_you_mean: Vec::new(),
        expected: format!("an existing export in module '{shown}'"),
        got: name.to_string(),
        extra: serde_json::Map::new(),
    })
}

/// The hint for `name` when `source` defines it without `pub`: the same words the
/// `alias.name` form uses, so both paths say one thing.
fn private_name_hint(source: &str, name: &str) -> Option<String> {
    let facts = FACTS.lock().unwrap_or_else(|e| e.into_inner());
    facts
        .iter()
        .find(|f| {
            f.source == source
                && f.private.iter().any(|p| p == name)
                && !f.exported.iter().any(|e| e == name)
        })
        .map(|f| {
            format!(
                "`{name}` is defined in {} but not exported; mark it `pub`",
                f.source
            )
        })
}

/// A syntax error in a module keeps its `E_PARSE` form and exit status, with the
/// module's file named beside the line and column the parser reports.
fn parse_in_module(path: &Path, err: anyhow::Error) -> anyhow::Error {
    match err.downcast::<crate::parser::ParseError>() {
        Ok(pe) => anyhow::Error::new(crate::parser::ParseError::with_hint(
            format!("{} (in module '{}')", pe.message, site_label(path)),
            pe.hint,
        )),
        Err(other) => in_module(path, other),
    }
}

/// The hint for a missing field of `alias`, when `alias` names a module imported
/// with `import "..." as alias` and that module exports nothing.
pub(crate) fn empty_module_hint(alias: &str) -> Option<String> {
    let aliases = ALIASES.lock().unwrap_or_else(|e| e.into_inner());
    let source = aliases.iter().rev().find(|(a, _)| a == alias)?.1.clone();
    drop(aliases);
    let facts = FACTS.lock().unwrap_or_else(|e| e.into_inner());
    let module = facts.iter().find(|f| f.source == source)?;
    module.exported.is_empty().then(|| {
        format!(
            "`{alias}` is the module {source}, which exports nothing: mark the \
             definitions you want to use `pub`"
        )
    })
}

/// Load a module's exports, evaluating it on first use.
fn load(source: &str) -> Result<(PathBuf, Exports)> {
    if source.is_empty() || source.contains('\0') {
        return Err(safety::bad_arg(
            "import",
            "a module path",
            if source.is_empty() {
                "an empty string"
            } else {
                "a path with a NUL byte"
            },
        ));
    }
    if source.contains("://") {
        return Err(safety::bad_arg(
            "import",
            "a local .ish file path",
            "a URL; modules are read from the filesystem only",
        ));
    }
    if source.starts_with("pkg:") {
        return Err(safety::unimplemented(
            "import",
            "package imports (`pkg:`) are not wired to the module loader",
            "import the module file by its path",
        ));
    }

    let path = resolve(source)?;
    let key = path.clone();

    if let Some(exports) = cache().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok((path, exports.clone()));
    }

    let loading_now = LOADING.with(|l| l.borrow().clone());
    if loading_now.contains(&path) {
        // The file being loaded last is the one whose import closes the cycle.
        let importer = loading_now.last().map(|p| display(p)).unwrap_or_default();
        return Err(safety::bad_arg(
            "import",
            "an acyclic import graph",
            &format!(
                "{importer} imports {}, which is already being loaded; remove one of the two imports",
                display(&path)
            ),
        ));
    }
    if loading_now.len() >= MAX_IMPORT_DEPTH {
        return Err(safety::budget_exceeded(
            "import",
            format!(
                "import depth {} reached loading '{}'; the limit is {MAX_IMPORT_DEPTH}",
                loading_now.len(),
                display(&path)
            ),
            "flatten the module graph, or import fewer modules in a chain",
        ));
    }

    let source_text = read_module(&path)?;
    let (stmts, lines) = crate::parser::parse_program_with_lines(&source_text)
        .map_err(|e| parse_in_module(&path, e))?;

    let private: Vec<String> = stmts
        .iter()
        .filter_map(|stmt| match stmt {
            crate::ast::Stmt::Let {
                name, visibility, ..
            } if *visibility != crate::ast::Visibility::Pub => Some(name.clone()),
            _ => None,
        })
        .collect();
    let exports = evaluate(&path, &stmts, &lines).map_err(|e| in_module(&path, e))?;
    remember_facts(source, private, exports.keys().cloned().collect());

    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, exports.clone());
    Ok((path, exports))
}

fn evaluate(path: &Path, stmts: &[crate::ast::Stmt], lines: &[usize]) -> Result<Exports> {
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    LOADING.with(|l| l.borrow_mut().push(path.to_path_buf()));
    BASE_DIRS.with(|b| b.borrow_mut().push(dir));
    let _frame = LoadFrame;

    let mut module_env = fresh_module_env();
    // Each statement runs with its line and file set, so a lambda it binds and
    // a failure inside one can name where they were written. The statement that
    // imported this module gets its own site back afterwards.
    let file = site_label(path);
    let outer = crate::eval::set_statement_site(None);
    let run = (|| -> Result<()> {
        for (index, stmt) in stmts.iter().enumerate() {
            let site = lines.get(index).map(|&line| crate::eval::Site {
                line,
                file: Some(file.clone()),
            });
            crate::eval::set_statement_site(site);
            crate::eval::eval_program(std::slice::from_ref(stmt), &mut module_env)?;
        }
        Ok(())
    })();
    crate::eval::set_statement_site(outer);
    run?;
    // An exported lambda was written before some of the names it reads were
    // defined; it now captures those names' final values, so calling it from the
    // importer reaches a private helper defined after it.
    let mut exports = module_env.exported_vars();
    for value in exports.values_mut() {
        if let crate::value::Value::Lambda(lam) = value {
            crate::eval::complete_module_capture(lam, &module_env);
        }
    }
    Ok(exports)
}

#[cfg(feature = "native")]
fn fresh_module_env() -> Env {
    crate::modules::env_with_modules()
}

#[cfg(not(feature = "native"))]
fn fresh_module_env() -> Env {
    Env::new()
}

fn cache() -> &'static Mutex<HashMap<PathBuf, Exports>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The path a source string names before the file system is consulted.
fn module_path(source: &str) -> PathBuf {
    let p = Path::new(source);
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match BASE_DIRS.with(|b| b.borrow().last().cloned()) {
        Some(dir) => dir.join(p),
        None => PathBuf::from(safety::resolve_path_str(source)),
    }
}

/// Resolve a source string to the canonical file it names, guarded as a read.
fn resolve(source: &str) -> Result<PathBuf> {
    let mut candidate = module_path(source);
    if !candidate.exists() && candidate.extension().is_none() {
        candidate = PathBuf::from(format!("{}.ish", candidate.display()));
    }
    // Canonicalize when the file exists so a symlink cannot lead the jail
    // check somewhere the lexical path does not show.
    let target = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.clone());
    let shown = display(&target);

    safety::guard(GuardCtx {
        builtin: "import",
        effect: Effect::ReadLocal,
        what: "read",
        targets: vec![shown.clone()],
        blast_radius: serde_json::json!({ "path": shown }),
        reversible: true,
        fs_paths: true,
    })
    .map_err(anyhow::Error::new)?;

    // `guard` jails only writes: reads are unjailed by policy (see the note in
    // builtins.rs beside `web_upload_file`). A module is code the importer runs,
    // so an import is held to the workspace whenever the jail is on: agent mode,
    // or an explicit IRONSHELL_WORKSPACE.
    let jailed = safety::current_mode() == safety::Mode::Agent
        || std::env::var_os("IRONSHELL_WORKSPACE").is_some();
    if jailed && !safety::within_workspace(&shown) {
        return Err(anyhow::Error::new(SafetyError {
            code: ErrorCode::OutsideWorkspace,
            message: format!("import: '{shown}' is outside the workspace root"),
            builtin: "import".to_string(),
            hint: safety::outside_workspace_hint(),
            approval: None,
            did_you_mean: Vec::new(),
            expected: String::new(),
            got: String::new(),
            extra: serde_json::Map::new(),
        }));
    }

    if !target.exists() {
        // The path was resolved against the importing file's folder, so that is
        // the folder the hint names; a file imported from nowhere uses the
        // working directory.
        let dir = BASE_DIRS
            .with(|b| b.borrow().last().cloned())
            .unwrap_or_else(|| PathBuf::from("."));
        return Err(anyhow::Error::new(SafetyError {
            code: ErrorCode::NotFound,
            message: format!("import: no module named '{source}'"),
            builtin: "import".to_string(),
            hint: format!(
                "paths are relative to the importing file: {}",
                display(&dir)
            ),
            approval: None,
            did_you_mean: Vec::new(),
            expected: "an existing module file".to_string(),
            got: source.to_string(),
            extra: serde_json::Map::new(),
        }));
    }
    if target.is_dir() {
        return Err(safety::bad_arg(
            "import",
            "a .ish file",
            &format!("the directory '{shown}'"),
        ));
    }
    Ok(target)
}

/// For `--explain`: the file an import names, resolved as the loader resolves
/// it, and its source text. The module is read, never run, and no guard fires.
/// A module that cannot be found or read, or lies outside the workspace when
/// the jail is on, comes back as the reason, so the caller reports `unknown`.
pub(crate) fn read_for_explain(source: &str) -> std::result::Result<(PathBuf, String), String> {
    if source.is_empty()
        || source.contains('\0')
        || source.contains("://")
        || source.starts_with("pkg:")
    {
        return Err("not a local module path".to_string());
    }
    let mut candidate = module_path(source);
    if !candidate.exists() && candidate.extension().is_none() {
        candidate = PathBuf::from(format!("{}.ish", candidate.display()));
    }
    let target = candidate
        .canonicalize()
        .map_err(|_| "no such module".to_string())?;
    let shown = display(&target);
    let jailed = safety::current_mode() == safety::Mode::Agent
        || std::env::var_os("IRONSHELL_WORKSPACE").is_some();
    if jailed && !safety::within_workspace(&shown) {
        return Err("outside the workspace".to_string());
    }
    if !target.is_file() {
        return Err("not a file".to_string());
    }
    let text = read_module(&target).map_err(|e| e.to_string())?;
    Ok((target, text))
}

/// Read a module's source, refusing anything over `MAX_MODULE_BYTES`.
fn read_module(path: &Path) -> Result<String> {
    let shown = display(path);
    let file = File::open(path).map_err(|e| {
        safety::bad_state_with_hint(
            "import",
            &format!("cannot open module '{shown}': {e}"),
            "check that the file exists and is readable",
        )
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_MODULE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| {
            safety::bad_state_with_hint(
                "import",
                &format!("cannot read module '{shown}': {e}"),
                "check that the file is readable",
            )
        })?;
    if bytes.len() as u64 > MAX_MODULE_BYTES {
        return Err(safety::budget_exceeded(
            "import",
            format!("module '{shown}' is over the {MAX_MODULE_BYTES}-byte limit"),
            "split the module into smaller files",
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        safety::bad_arg(
            "import",
            "a UTF-8 source file",
            &format!("the non-UTF-8 file '{shown}'"),
        )
    })
}

/// Attach the module path to an error raised while loading or evaluating it.
fn in_module(path: &Path, err: anyhow::Error) -> anyhow::Error {
    let shown = display(path);
    match err.downcast::<SafetyError>() {
        Ok(mut coded) => {
            coded.message = format!("{} (in module '{shown}')", coded.message);
            anyhow::Error::new(coded)
        }
        Err(other) => anyhow!("in module '{shown}': {other:#}"),
    }
}

/// A path for messages: the `\\?\` prefix Windows canonicalization adds is noise.
/// A module file as an error note names it: relative to the working directory
/// when it is under it (`lib/bad.ish`), otherwise as displayed.
fn site_label(path: &Path) -> String {
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(relative) = path.strip_prefix(&cwd) {
            return relative.display().to_string();
        }
    }
    display(path)
}

fn display(path: &Path) -> String {
    let s = path.display().to_string();
    s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s)
}

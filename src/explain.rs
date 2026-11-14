//! What a program would do, decided before it runs.
//!
//! E4's containment is a runtime property: the gate fires when the call is
//! made, so an agent learns a pipeline needs approval by being refused halfway
//! through it. But `safety::effect_of` is static, so the effect set of a whole
//! program is computable from its syntax. This computes it
//! (`docs/LANGUAGE_FIRST_PRINCIPLES.md` §6):
//!
//! ```text
//! $ ish --explain -c 'ls("src") | where(fn(f) => f.size > 1000) | map(fn(f) => rm(f.path))'
//! ```
//!
//! reports `ls` as `read_local` and `rm` as `destructive`, and that
//! agent mode would ask for approval before the delete.
//!
//! It is conservative in one direction only. A call it cannot resolve to a
//! builtin -- through a variable, or a computed callee -- is listed as
//! unresolved and raises the verdict to `unknown`, never lowers it. What it
//! cannot see is `eval` of a string built at run time, which it reports as the
//! `eval` builtin's own effect and no more; that limit is stated in the output.

use crate::ast::{Expr, Stmt};
use crate::safety::{effect_of, effective_decision, Decision, Effect, Mode};
use crate::value::Value;
use anyhow::Result;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

/// Most modules one explanation will fold in; past it the verdict is `unknown`.
const MAX_MODULES: usize = 256;

struct Walk<'a> {
    env: &'a crate::env::Env,
    /// Names bound by `let`, which shadow builtins of the same name.
    locals: HashSet<String>,
    calls: Vec<(String, Option<String>)>,
    /// The literal String arguments of each call in `calls`, by index: the
    /// paths the jail would judge, when they are written out.
    literals: Vec<Vec<String>>,
    /// Names bound once, immutably, to a string literal: `name = "rm"`, so that
    /// `call(name, ...)` can be read as a call to that builtin.
    consts: HashMap<String, String>,
    /// Modules already folded in, by canonical path (cycles end here).
    visited: HashSet<PathBuf>,
}

/// The value of a string literal that has no interpolation hole.
fn plain_literal(s: &str) -> Option<String> {
    if s.contains("${") {
        None
    } else {
        Some(s.replace(crate::parser::ESCAPED_DOLLAR, "$"))
    }
}

/// The names a match pattern binds.
fn pattern_names(p: &crate::ast::Pattern, out: &mut Vec<String>) {
    use crate::ast::Pattern;
    match p {
        Pattern::Ident(n) => out.push(n.clone()),
        Pattern::Constructor { args, .. } => args.iter().for_each(|a| pattern_names(a, out)),
        Pattern::Array(xs) => xs.iter().for_each(|a| pattern_names(a, out)),
        Pattern::Record(fs) => fs.iter().for_each(|(k, a)| {
            out.push(k.clone());
            pattern_names(a, out);
        }),
        _ => {}
    }
}

impl Walk<'_> {
    /// A call the analysis cannot see into: listed as unresolved, so the
    /// verdict is at least `unknown`.
    fn unknown(&mut self, what: String) {
        self.calls.push((what, None));
        self.literals.push(Vec::new());
    }

    /// Run `f` with `names` bound as parameters: they shadow builtins and
    /// literal bindings of the same name for its duration.
    fn scoped(&mut self, names: &[String], f: impl FnOnce(&mut Self)) {
        let mut added = Vec::new();
        let mut shadowed = Vec::new();
        for n in names {
            if self.locals.insert(n.clone()) {
                added.push(n.clone());
            }
            if let Some(v) = self.consts.remove(n) {
                shadowed.push((n.clone(), v));
            }
        }
        f(self);
        for n in added {
            self.locals.remove(&n);
        }
        for (n, v) in shadowed {
            self.consts.insert(n, v);
        }
    }

    /// Analyse IronShell source that a builtin would run: each statement is
    /// walked like any other code, and source that does not parse is unknown.
    fn walk_source(&mut self, src: &str) {
        match crate::parser::parse_program(src) {
            Ok(stmts) => {
                for s in &stmts {
                    self.stmt(s);
                }
            }
            Err(_) => self.unknown("<code that does not parse>".to_string()),
        }
    }

    /// A builtin that evaluates a String as code: the code argument is analysed
    /// when it is a literal and is unknown otherwise.
    fn code_arg(&mut self, builtin: &str, args: &[Expr]) {
        let idx = match builtin {
            "try_repair" => 0,
            "sess_eval" => 1,
            _ => return,
        };
        match args.get(idx) {
            Some(Expr::LitStr(s)) if plain_literal(s).is_some() => {
                let code = s.clone();
                self.walk_source(&code);
            }
            _ => self.unknown(format!("{builtin}(<computed code>)")),
        }
    }

    /// `call(name, args...)` runs the builtin `name`: a literal name (or a
    /// name bound to one) is analysed as a call to that builtin, anything else
    /// is unknown.
    fn call_by_name(&mut self, args: &[Expr]) {
        let name = match args.first() {
            Some(Expr::LitStr(s)) => plain_literal(s),
            Some(Expr::Ident(n)) => self.consts.get(n).cloned(),
            _ => None,
        };
        let rest = args.get(1..).unwrap_or(&[]);
        match name {
            Some(n)
                if crate::builtins::is_dispatched(&n)
                    || crate::plugins::is_manifest_builtin(&n) =>
            {
                self.calls.push((n.clone(), Some(n.clone())));
                self.literals.push(
                    rest.iter()
                        .filter_map(|a| match a {
                            Expr::LitStr(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect(),
                );
                if n == "call" {
                    self.call_by_name(rest);
                } else {
                    self.code_arg(&n, rest);
                }
            }
            Some(n) => self.unknown(format!("call(\"{n}\")")),
            None => self.unknown("call(<computed name>)".to_string()),
        }
    }

    /// The `${...}` holes of a string literal are code: each is walked.
    fn string_holes(&mut self, s: &str) {
        if !s.contains("${") {
            return;
        }
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '$' && chars.get(i + 1) == Some(&'{') {
                let body = i + 2;
                let Some(end) = crate::parser::interp_hole_end(&chars, body) else {
                    self.unknown("<unclosed interpolation>".to_string());
                    return;
                };
                let src: String = chars[body..end - 1].iter().collect();
                i = end;
                match crate::parser::parse_program(&src) {
                    Ok(stmts) => match stmts.as_slice() {
                        [Stmt::Expr(e)] => self.expr(e),
                        _ => self.unknown(format!("<interpolation \"{src}\">")),
                    },
                    Err(_) => self.unknown(format!("<interpolation \"{src}\">")),
                }
            } else {
                i += 1;
            }
        }
    }

    /// An import runs the module's top-level statements, so they are folded
    /// into the verdict (the module is read and parsed, never run). A module
    /// that cannot be read or parsed is unknown, by name.
    fn import(&mut self, source: &str) {
        let (path, text) = match crate::module_loader::read_for_explain(source) {
            Ok(r) => r,
            Err(why) => {
                self.unknown(format!("import \"{source}\" ({why})"));
                return;
            }
        };
        if self.visited.contains(&path) {
            return;
        }
        if self.visited.len() >= MAX_MODULES {
            self.unknown(format!("import \"{source}\" (too many modules)"));
            return;
        }
        self.visited.insert(path.clone());
        let stmts = match crate::parser::parse_program(&text) {
            Ok(s) => s,
            Err(_) => {
                self.unknown(format!("import \"{source}\" (does not parse)"));
                return;
            }
        };
        // The module's own imports resolve against its folder, and its names
        // are its own.
        let _entry = crate::module_loader::enter_entry_file(&path);
        let locals = std::mem::take(&mut self.locals);
        let consts = std::mem::take(&mut self.consts);
        for s in &stmts {
            self.stmt(s);
        }
        self.locals = locals;
        self.consts = consts;
    }

    /// The builtin a callee names, if it names one.
    fn resolve(&self, callee: &Expr) -> (String, Option<String>) {
        match callee {
            Expr::Ident(n) => {
                if self.locals.contains(n) {
                    (n.clone(), None)
                } else if crate::builtins::is_dispatched(n)
                    || crate::plugins::is_manifest_builtin(n)
                {
                    // A plugin's manifest builtin is a builtin too: its body is not
                    // analysed, and effect_of classes it Exec.
                    (n.clone(), Some(n.clone()))
                } else {
                    (n.clone(), None)
                }
            }
            Expr::MemberAccess { object, field } => {
                if let Expr::Ident(m) = object.as_ref() {
                    let shown = format!("{m}.{field}");
                    if self.locals.contains(m) {
                        return (shown, None);
                    }
                    if let Some(Value::Record(r)) = self.env.get_var(m) {
                        if let Some(Value::Builtin(b)) = r.get(field) {
                            return (shown, Some(b.name.clone()));
                        }
                    }
                    return (shown, None);
                }
                ("<computed>".into(), None)
            }
            _ => ("<computed>".into(), None),
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let {
                name,
                value,
                is_mut,
                ..
            } => {
                self.expr(value);
                let literal = match value {
                    Expr::LitStr(s) if !*is_mut && !self.locals.contains(name) => plain_literal(s),
                    _ => None,
                };
                match literal {
                    Some(s) => {
                        self.consts.insert(name.clone(), s);
                    }
                    None => {
                        self.consts.remove(name);
                    }
                }
                self.locals.insert(name.clone());
            }
            Stmt::Expr(e) => self.expr(e),
            Stmt::SetField { value, .. } => self.expr(value),
            Stmt::Cfg { body, .. } => self.stmt(body),
            Stmt::Import { source, .. } => self.import(source),
            Stmt::Export { from_source, .. } => {
                if let Some(source) = from_source {
                    self.import(source);
                }
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Call {
                callee,
                args,
                named,
            } => {
                let r = self.resolve(callee);
                let target = r.1.clone();
                self.calls.push(r);
                self.literals.push(
                    args.iter()
                        .chain(named.iter().map(|(_, a)| a))
                        .filter_map(|a| match a {
                            Expr::LitStr(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect(),
                );
                if !matches!(callee.as_ref(), Expr::Ident(_) | Expr::MemberAccess { .. }) {
                    self.expr(callee);
                }
                for a in args {
                    self.expr(a);
                }
                for (_, a) in named {
                    self.expr(a);
                }
                // A builtin that runs another builtin, or code, by name.
                match target.as_deref() {
                    Some("call") => self.call_by_name(args),
                    Some(b) => self.code_arg(b, args),
                    None => {}
                }
            }
            Expr::Pipe { left, right } => {
                self.expr(left);
                // A bare name on the right of a pipe is a call: `xs | sort`.
                match right.as_ref() {
                    Expr::Ident(_) | Expr::MemberAccess { .. } => {
                        let r = self.resolve(right);
                        let target = r.1.clone();
                        self.calls.push(r);
                        self.literals.push(Vec::new());
                        if let Some(b) = target.as_deref() {
                            self.code_arg(b, &[]);
                        }
                    }
                    other => self.expr(other),
                }
            }
            Expr::Array(xs) => xs.iter().for_each(|x| self.expr(x)),
            Expr::Record(fs) => fs.iter().for_each(|(_, x)| self.expr(x)),
            Expr::Lambda { params, body } | Expr::AsyncLambda { params, body } => {
                self.scoped(params, |w| w.expr(body))
            }
            Expr::Await(x) | Expr::Throw(x) => self.expr(x),
            Expr::TryCatch {
                try_expr,
                catch_var,
                catch_expr,
            } => {
                self.expr(try_expr);
                let bound: Vec<String> = catch_var.iter().cloned().collect();
                self.scoped(&bound, |w| w.expr(catch_expr));
            }
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Unary { expr, .. } => self.expr(expr),
            Expr::MemberAccess { object, .. } => {
                // `m.f` used as a value (not called) is a reference to the
                // builtin: it may be called later, so it counts as a call.
                let r = self.resolve(e);
                if r.1.is_some() {
                    self.calls.push(r);
                    self.literals.push(Vec::new());
                }
                self.expr(object)
            }
            Expr::Index { object, index } => {
                self.expr(object);
                self.expr(index);
            }
            Expr::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for arm in arms {
                    let mut bound = Vec::new();
                    pattern_names(&arm.pattern, &mut bound);
                    self.scoped(&bound, |w| {
                        if let Some(g) = &arm.guard {
                            w.expr(g);
                        }
                        w.expr(&arm.body);
                    });
                }
            }
            Expr::LitStr(s) => self.string_holes(s),
            // A builtin named as a value (`map(rm)`, `{f: rm}`, `g = rm`) may be
            // called later, so it counts as a call.
            Expr::Ident(_) => {
                let r = self.resolve(e);
                if r.1.is_some() {
                    self.calls.push(r);
                    self.literals.push(Vec::new());
                }
            }
            Expr::LitInt(_) | Expr::LitFloat(_) | Expr::LitBool(_) | Expr::Null => {}
        }
    }
}

/// The agent-mode decision for one effect, as `safety::guard` applies it: the
/// policy table plus the egress rule (`safety::effective_decision`).
fn agent_decision(effect: Effect) -> &'static str {
    match effective_decision(effect, Mode::Agent) {
        Decision::Allow => "allow",
        Decision::Approve => "approve",
        Decision::Deny => "deny",
    }
}

/// A builtin the run refuses outright, before anything executes, unless an
/// opt-in is set: `sh` runs an arbitrary command, and the call checks
/// `validate_sh_allowed` first. Its decision is `deny` with the run's own hint,
/// not the `approve` its effect would get. `None` for any other builtin, and
/// for `sh` once the opt-in is set.
fn refused_outright(builtin: &str) -> Option<String> {
    if !matches!(builtin, "sh" | "shell") {
        return None;
    }
    let refusal = crate::security::validate_sh_allowed().err()?;
    Some(
        refusal
            .downcast_ref::<crate::safety::SafetyError>()
            .map(|se| se.hint.clone())
            .unwrap_or_default(),
    )
}

/// The jail's refusal of a call that reads or writes files, when one of its
/// literal String arguments names a path outside the workspace: the run refuses
/// it with E_OUTSIDE_WORKSPACE before its effect is judged, so `allow` or
/// `approve` would misstate it. Only paths written out are seen; an argument
/// that is not a path but reads as one outside (`"../x"` as a search pattern)
/// is judged as a path, which errs toward `deny`.
fn jail_refusal(effect: Effect, literals: &[String]) -> Option<String> {
    if !matches!(
        effect,
        Effect::ReadLocal | Effect::WriteLocal | Effect::Destructive
    ) {
        return None;
    }
    literals
        .iter()
        .filter(|s| !s.is_empty() && !s.contains('\n'))
        .find(|s| crate::security::outside_read_jail_as_agent(s))
        .map(|s| format!("'{s}' is outside the workspace root, so the run refuses it with E_OUTSIDE_WORKSPACE"))
}

fn rank(decision: &str) -> u8 {
    match decision {
        "allow" => 0,
        "unknown" => 1,
        "approve" => 2,
        _ => 3,
    }
}

/// Explain `code`: every call site, its effect and agent-mode decision, and
/// the strictest decision overall. Nothing is evaluated.
pub fn explain(code: &str) -> Result<Value> {
    let stmts = crate::parser::parse_program(code).map_err(|e| {
        crate::safety::bad_arg(
            "explain_effects",
            "IronShell source that parses",
            &e.to_string(),
        )
    })?;
    let env = crate::modules::env_with_modules();
    let mut w = Walk {
        env: &env,
        locals: HashSet::new(),
        calls: Vec::new(),
        literals: Vec::new(),
        consts: HashMap::new(),
        visited: HashSet::new(),
    };
    for s in &stmts {
        w.stmt(s);
    }

    let mut calls = Vec::new();
    let mut effects: BTreeMap<u8, &'static str> = BTreeMap::new();
    let mut unresolved = Vec::new();
    let mut verdict = "allow";
    for (i, (shown, builtin)) in w.calls.iter().enumerate() {
        let mut r = BTreeMap::new();
        r.insert("call".to_string(), Value::Str(shown.clone()));
        match builtin {
            Some(b) => {
                let effect = effect_of(b);
                let literals = w.literals.get(i).map(Vec::as_slice).unwrap_or(&[]);
                let refused = refused_outright(b).or_else(|| jail_refusal(effect, literals));
                let decision = if refused.is_some() {
                    "deny"
                } else {
                    agent_decision(effect)
                };
                effects.insert(effect as u8, effect.as_str());
                if rank(decision) > rank(verdict) {
                    verdict = decision;
                }
                r.insert("builtin".to_string(), Value::Str(b.clone()));
                r.insert(
                    "effect".to_string(),
                    Value::Str(effect.as_str().to_string()),
                );
                r.insert("decision".to_string(), Value::Str(decision.to_string()));
                if let Some(hint) = refused {
                    r.insert("hint".to_string(), Value::Str(hint));
                }
            }
            None => {
                if !unresolved.contains(shown) {
                    unresolved.push(shown.clone());
                }
                if rank("unknown") > rank(verdict) {
                    verdict = "unknown";
                }
                r.insert("decision".to_string(), Value::Str("unknown".to_string()));
            }
        }
        calls.push(Value::Record(r));
    }

    let mut out = BTreeMap::new();
    out.insert("calls".to_string(), Value::Array(calls));
    out.insert(
        "effects".to_string(),
        Value::Array(
            effects
                .values()
                .map(|e| Value::Str(e.to_string()))
                .collect(),
        ),
    );
    out.insert("decision".to_string(), Value::Str(verdict.to_string()));
    out.insert(
        "unresolved".to_string(),
        Value::Array(unresolved.into_iter().map(Value::Str).collect()),
    );
    out.insert(
        "note".to_string(),
        Value::Str(
            "static: decisions are agent mode's; a call through a variable is unresolved, \
             and code built at run time for eval is not seen"
                .to_string(),
        ),
    );
    Ok(Value::Record(out))
}

/// `--explain` for the POSIX subset (`ish -b`). Each command word the source
/// names is listed with the class the run gives it, and the decision is the
/// strictest of them. A word the subset runs itself is judged by its effect: a
/// utility that reads a path is `read_local`, the rest `pure`. An output file a
/// redirection names, and each file `tee` writes, is `write_local` with its
/// target, as `file.write` is. Any other word
/// runs as a program through the effect gate, as `exec`, which agent mode
/// approves; so `git --version` is `approve`, as the run is refused for approval.
/// Source outside the subset is run by the transpiler, which is not predicted,
/// so it is `unknown`. Nothing is run.
pub fn explain_posix(src: &str) -> Result<Value> {
    let in_subset = crate::posix::supports(src).is_ok();
    let mut calls = Vec::new();
    let mut effects: BTreeMap<u8, &'static str> = BTreeMap::new();
    let mut unresolved = Vec::new();
    let mut verdict = if in_subset { "allow" } else { "unknown" };
    for cmd in crate::posix::explain_commands(src) {
        // An output file is a write_local call of its own, as file.write is,
        // and so is each file `tee` writes when it runs in the subset. The
        // workspace jail is applied when the run opens the file, as it is for
        // file.write, so the decision here is the policy's.
        // `mkdir` and `touch` write their operands the same way.
        let head = cmd.head.clone().unwrap_or_default();
        // Files the run opens for reading outside a utility's operands: a `<`
        // redirection's file, and the directory `cd` enters. The run refuses
        // each outside the workspace, so each is judged here as it is there.
        let mut judged_reads: Vec<(String, String)> = cmd
            .reads
            .iter()
            .map(|f| (format!("< {f}"), f.clone()))
            .collect();
        if head == "cd" {
            if let Some(dir) = cmd.args.iter().find(|a| !a.starts_with('-')) {
                judged_reads.push((format!("cd {dir}"), dir.clone()));
            }
        }
        for (shown, f) in judged_reads {
            if f.is_empty() {
                if !unresolved.contains(&shown) {
                    unresolved.push(shown);
                }
                if rank("unknown") > rank(verdict) {
                    verdict = "unknown";
                }
                continue;
            }
            if crate::security::outside_read_jail(&f) {
                let mut fr = BTreeMap::new();
                fr.insert("call".to_string(), Value::Str(shown));
                fr.insert(
                    "effect".to_string(),
                    Value::Str(Effect::ReadLocal.as_str().to_string()),
                );
                fr.insert("target".to_string(), Value::Str(f.clone()));
                fr.insert("decision".to_string(), Value::Str("deny".to_string()));
                fr.insert(
                    "reason".to_string(),
                    Value::Str(format!("'{f}' is outside the workspace root")),
                );
                effects.insert(Effect::ReadLocal as u8, Effect::ReadLocal.as_str());
                verdict = "deny";
                calls.push(Value::Record(fr));
            }
        }
        let tee_files = crate::posix::write_operands(&head, &cmd.args);
        let tee_in_subset = tee_files.is_some();
        let mut writes: Vec<(String, String)> = cmd.writes.clone();
        for f in tee_files.unwrap_or_default() {
            if f != "/dev/null" {
                writes.push((head.clone(), f));
            }
        }
        for (op, target) in writes {
            let mut r = BTreeMap::new();
            r.insert("call".to_string(), Value::Str(format!("{op} {target}")));
            if target.is_empty() {
                // A target computed at run time: where it goes is not known here.
                let shown = format!("{op} <computed>");
                if !unresolved.contains(&shown) {
                    unresolved.push(shown);
                }
                if rank("unknown") > rank(verdict) {
                    verdict = "unknown";
                }
                r.insert("decision".to_string(), Value::Str("unknown".to_string()));
            } else {
                let effect = Effect::WriteLocal;
                // The jail judges the path the write goes to, as the run's
                // guard does for a `>` target or a `tee`, `mkdir` or `touch` file.
                let resolved = crate::safety::resolve_path_str(&target);
                let refusal = crate::safety::write_jail_refusal(&resolved);
                let decision = if refusal.is_some() {
                    "deny"
                } else {
                    agent_decision(effect)
                };
                effects.insert(effect as u8, effect.as_str());
                if rank(decision) > rank(verdict) {
                    verdict = decision;
                }
                r.insert(
                    "effect".to_string(),
                    Value::Str(effect.as_str().to_string()),
                );
                r.insert("target".to_string(), Value::Str(target.clone()));
                r.insert("decision".to_string(), Value::Str(decision.to_string()));
                if let Some(reason) = refusal {
                    r.insert("reason".to_string(), Value::Str(reason));
                }
            }
            calls.push(Value::Record(r));
        }
        let Some(head) = cmd.head else { continue };
        let mut r = BTreeMap::new();
        r.insert("call".to_string(), Value::Str(head.clone()));
        if head.is_empty() {
            // A command name computed at run time: its class is not known here.
            if !unresolved.contains(&head) {
                unresolved.push(head.clone());
            }
            if rank("unknown") > rank(verdict) {
                verdict = "unknown";
            }
            r.insert("decision".to_string(), Value::Str("unknown".to_string()));
        } else {
            let effect = if tee_in_subset {
                Effect::WriteLocal
            } else {
                match crate::posix::in_process_reads(&head) {
                    Some(true) => Effect::ReadLocal,
                    Some(false) => Effect::Pure,
                    None => Effect::Exec,
                }
            };
            let mut decision = agent_decision(effect);
            // A utility that reads files: each file operand is judged by the
            // read jail, as the run judges it when it opens the file. An operand
            // list the subset does not model is not predicted: `unknown`.
            if matches!(effect, Effect::ReadLocal) {
                match crate::posix::read_operands(&head, &cmd.args) {
                    Some(files) => {
                        for f in files.iter().filter(|f| f.as_str() != "-") {
                            if f.is_empty() {
                                // A file named at run time (`cat "$f"`): not known here.
                                if rank("unknown") > rank(verdict) {
                                    verdict = "unknown";
                                }
                                decision = "unknown";
                                continue;
                            }
                            if crate::security::outside_read_jail(f) {
                                let mut fr = BTreeMap::new();
                                fr.insert("call".to_string(), Value::Str(format!("{head} {f}")));
                                fr.insert(
                                    "effect".to_string(),
                                    Value::Str(Effect::ReadLocal.as_str().to_string()),
                                );
                                fr.insert("target".to_string(), Value::Str(f.clone()));
                                fr.insert("decision".to_string(), Value::Str("deny".to_string()));
                                fr.insert(
                                    "reason".to_string(),
                                    Value::Str(format!("'{f}' is outside the workspace root")),
                                );
                                effects.insert(Effect::ReadLocal as u8, Effect::ReadLocal.as_str());
                                if rank("deny") > rank(verdict) {
                                    verdict = "deny";
                                }
                                calls.push(Value::Record(fr));
                            }
                        }
                    }
                    None if cmd.args.iter().any(|a| !a.starts_with('-')) => {
                        decision = "unknown";
                    }
                    None => {}
                }
            }
            effects.insert(effect as u8, effect.as_str());
            if rank(decision) > rank(verdict) {
                verdict = decision;
            }
            r.insert(
                "effect".to_string(),
                Value::Str(effect.as_str().to_string()),
            );
            r.insert("decision".to_string(), Value::Str(decision.to_string()));
        }
        calls.push(Value::Record(r));
    }

    let mut out = BTreeMap::new();
    out.insert("calls".to_string(), Value::Array(calls));
    out.insert(
        "effects".to_string(),
        Value::Array(
            effects
                .values()
                .map(|e| Value::Str(e.to_string()))
                .collect(),
        ),
    );
    out.insert("decision".to_string(), Value::Str(verdict.to_string()));
    out.insert(
        "unresolved".to_string(),
        Value::Array(unresolved.into_iter().map(Value::Str).collect()),
    );
    out.insert(
        "note".to_string(),
        Value::Str(
            "static, POSIX subset: a program named here runs through the effect gate as exec; \
             a utility the subset runs itself is judged by its read effect, and a file it reads \
             outside the read jail is deny; an output file is write_local and is judged by the \
             write jail, as the run judges it; a utility that declines its flags runs as a \
             program and is not predicted"
                .to_string(),
        ),
    );
    Ok(Value::Record(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::net_allowlist;

    fn field<'a>(v: &'a Value, k: &str) -> &'a Value {
        match v {
            Value::Record(r) => r.get(k).unwrap_or_else(|| panic!("no {k}")),
            _ => panic!("not a record"),
        }
    }

    #[test]
    fn a_pure_pipeline_is_allowed() {
        let v = explain("[1, 2, 3] | map(fn(x) => x * 2) | sum").unwrap();
        assert_eq!(field(&v, "decision"), &Value::Str("allow".into()));
    }

    #[test]
    fn a_delete_inside_a_lambda_is_found() {
        let v = explain(r#"ls("src") | map(fn(f) => file.move(f.path, "old"))"#).unwrap();
        assert_eq!(field(&v, "decision"), &Value::Str("approve".into()));
        let Value::Array(effects) = field(&v, "effects") else {
            panic!()
        };
        assert!(
            effects.contains(&Value::Str("read_local".into())),
            "{effects:?}"
        );
    }

    #[test]
    fn a_call_through_a_variable_is_unknown_not_allowed() {
        let v = explain("let f = fn(x) => x\nf(1)").unwrap();
        assert_eq!(field(&v, "decision"), &Value::Str("unknown".into()));
    }

    #[test]
    fn network_asks_without_an_allowlist() {
        let v = explain(r#"http_get("https://example.com")"#).unwrap();
        // Unless the environment running the test allowlists hosts.
        if net_allowlist().is_none() {
            assert_eq!(field(&v, "decision"), &Value::Str("approve".into()));
        }
    }
}

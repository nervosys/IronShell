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
use crate::security::LiteralSlot;
use crate::value::Value;
use anyhow::Result;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

/// Most modules one explanation will fold in; past it the verdict is `unknown`.
const MAX_MODULES: usize = 256;

/// Most elements of an Array literal a lambda is judged against one by one.
const MAX_ELEMENTS: usize = 256;

/// Deepest nesting of function bodies entered for a call or an element.
const MAX_DEPTH: usize = 16;

struct Walk<'a> {
    env: &'a crate::env::Env,
    /// Names bound by `let`, which shadow builtins of the same name.
    locals: HashSet<String>,
    calls: Vec<(String, Option<String>)>,
    /// The literal String arguments of each call in `calls`, by index, with
    /// where each sits, and whether the call is a pipeline stage (its subject
    /// comes from the left): the paths the jail would judge, when they are
    /// written out.
    literals: Vec<(bool, Vec<(LiteralSlot, String)>)>,
    /// Set by a pipe for the call on its right.
    piped_next: bool,
    /// For a call that names its arguments in order, the plain String literal
    /// at each position (`None` for any other argument), by index into `calls`.
    /// A name check must know which argument is a path, and `literals` has
    /// lost the positions.
    positional: HashMap<usize, Vec<Option<String>>>,
    /// Names bound once, immutably, to a string literal: `name = "rm"`, so that
    /// `call(name, ...)` can be read as a call to that builtin.
    consts: HashMap<String, String>,
    /// Names bound once, immutably, to a fn literal: a call through one is
    /// judged by its body with the literal arguments bound.
    fns: HashMap<String, (Vec<String>, Expr)>,
    /// How deep the walk is inside bodies it entered for a call or an element.
    depth: usize,
    /// Set by a pipe for the `map`/`each`/`where` on its right: the strings of
    /// the literal Array on its left.
    elems: Option<Vec<String>>,
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

/// The value of an expression that is a String known without running: a literal,
/// one whose interpolation holes are all such values, or a name bound once to
/// one (`consts`).
fn const_str(consts: &HashMap<String, String>, e: &Expr) -> Option<String> {
    match e {
        Expr::LitStr(s) => fold_interpolation(consts, s),
        Expr::Ident(n) => consts.get(n).cloned(),
        _ => None,
    }
}

/// A string literal's value, its `${...}` holes replaced by their known values.
/// `None` when a hole is anything but a known String.
fn fold_interpolation(consts: &HashMap<String, String>, s: &str) -> Option<String> {
    if !s.contains("${") {
        return plain_literal(s);
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && chars.get(i + 1) == Some(&'{') {
            let body = i + 2;
            let end = crate::parser::interp_hole_end(&chars, body)?;
            let src: String = chars[body..end - 1].iter().collect();
            let stmts = crate::parser::parse_program(&src).ok()?;
            let value = match stmts.as_slice() {
                [Stmt::Expr(e)] => const_str(consts, e)?,
                _ => return None,
            };
            out.push_str(&value);
            i = end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    Some(out.replace(crate::parser::ESCAPED_DOLLAR, "$"))
}

/// The Strings of an Array literal whose elements are all known Strings.
fn const_array(consts: &HashMap<String, String>, e: &Expr) -> Option<Vec<String>> {
    match e {
        Expr::Array(items) if items.len() <= MAX_ELEMENTS => {
            items.iter().map(|x| const_str(consts, x)).collect()
        }
        _ => None,
    }
}

/// The plain String literal at each position of `args`, `None` for any other
/// argument (a computed value, a string with a hole that is not a known String).
fn positional_literals(consts: &HashMap<String, String>, args: &[Expr]) -> Vec<Option<String>> {
    args.iter().map(|a| const_str(consts, a)).collect()
}

/// The literal Strings among a call's arguments (and among the elements of an
/// Array argument), each with its slot. A name bound once to a String, and an
/// interpolation of known Strings, count as literals, with their value.
fn literal_args(
    consts: &HashMap<String, String>,
    args: &[Expr],
    named: &[(String, Expr)],
) -> Vec<(LiteralSlot, String)> {
    let mut out = Vec::new();
    let one = |a: &Expr| -> Option<String> {
        match const_str(consts, a) {
            Some(v) => Some(v),
            None => match a {
                Expr::LitStr(s) => Some(s.clone()),
                _ => None,
            },
        }
    };
    for (i, a) in args.iter().enumerate() {
        match a {
            Expr::Array(items) => {
                for it in items {
                    if let Some(s) = one(it) {
                        out.push((LiteralSlot::Position(i), s));
                    }
                }
            }
            _ => {
                if let Some(s) = one(a) {
                    out.push((LiteralSlot::Position(i), s));
                }
            }
        }
    }
    for (n, a) in named {
        if let Some(s) = one(a) {
            out.push((LiteralSlot::Named(n.clone()), s));
        }
    }
    out
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
        self.literals.push((false, Vec::new()));
    }

    /// Run `f` with `names` bound as parameters: they shadow builtins and
    /// literal bindings of the same name for its duration.
    fn scoped(&mut self, names: &[String], f: impl FnOnce(&mut Self)) {
        let binds: Vec<(String, Option<String>)> =
            names.iter().map(|n| (n.clone(), None)).collect();
        self.scoped_values(&binds, f);
    }

    /// Like `scoped`, with each name that has a value bound to that String for
    /// the duration, as a name bound once to a literal is.
    fn scoped_values(&mut self, binds: &[(String, Option<String>)], f: impl FnOnce(&mut Self)) {
        let mut added = Vec::new();
        let mut shadowed = Vec::new();
        let mut shadowed_fns = Vec::new();
        for (n, value) in binds {
            if self.locals.insert(n.clone()) {
                added.push(n.clone());
            }
            if let Some(v) = self.consts.remove(n) {
                shadowed.push((n.clone(), v));
            }
            if let Some(g) = self.fns.remove(n) {
                shadowed_fns.push((n.clone(), g));
            }
            if let Some(v) = value {
                self.consts.insert(n.clone(), v.clone());
            }
        }
        f(self);
        for (n, _) in binds {
            self.consts.remove(n);
        }
        for n in added {
            self.locals.remove(&n);
        }
        for (n, v) in shadowed {
            self.consts.insert(n, v);
        }
        for (n, g) in shadowed_fns {
            self.fns.insert(n, g);
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
        // `call("file.read", ...)` names the builtin as `file.read(...)` does.
        let name = name.map(|n| {
            if crate::builtins::is_dispatched(&n) || crate::plugins::is_manifest_builtin(&n) {
                n
            } else {
                crate::builtins::qualified_builtin(self.env, &n).unwrap_or(n)
            }
        });
        match name {
            Some(n)
                if crate::builtins::is_dispatched(&n)
                    || crate::plugins::is_manifest_builtin(&n) =>
            {
                self.calls.push((n.clone(), Some(n.clone())));
                self.positional.insert(
                    self.calls.len() - 1,
                    positional_literals(&self.consts, rest),
                );
                self.literals
                    .push((false, literal_args(&self.consts, rest, &[])));
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
        let fns = std::mem::take(&mut self.fns);
        for s in &stmts {
            self.stmt(s);
        }
        self.locals = locals;
        self.consts = consts;
        self.fns = fns;
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
                let fresh = !*is_mut && !self.locals.contains(name);
                let literal = if fresh {
                    const_str(&self.consts, value)
                } else {
                    None
                };
                match literal {
                    Some(s) => {
                        self.consts.insert(name.clone(), s);
                    }
                    None => {
                        self.consts.remove(name);
                    }
                }
                match value {
                    Expr::Lambda { params, body } if fresh => {
                        self.fns
                            .insert(name.clone(), (params.clone(), (**body).clone()));
                    }
                    _ => {
                        self.fns.remove(name);
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
                let piped = std::mem::take(&mut self.piped_next);
                let elems = self.elems.take();
                // A call through a name bound to a fn literal, with a known
                // String for every parameter: judged by the body with them bound.
                if let (Expr::Ident(n), false, true) =
                    (callee.as_ref(), piped, self.depth < MAX_DEPTH)
                {
                    if let Some((params, body)) = self.fns.get(n).cloned() {
                        let known: Vec<Option<String>> =
                            args.iter().map(|a| const_str(&self.consts, a)).collect();
                        if self.locals.contains(n)
                            && named.is_empty()
                            && params.len() == args.len()
                            && known.iter().all(|k| k.is_some())
                        {
                            for a in args {
                                self.expr(a);
                            }
                            let binds: Vec<(String, Option<String>)> =
                                params.iter().cloned().zip(known).collect();
                            self.depth += 1;
                            self.scoped_values(&binds, |w| w.expr(&body));
                            self.depth -= 1;
                            return;
                        }
                    }
                }
                let r = self.resolve(callee);
                let target = r.1.clone();
                // `map`/`each`/`where` over an Array literal of known Strings:
                // the lambda is judged with its parameter bound to each one.
                let source: Option<Vec<String>> = match target.as_deref() {
                    Some("map" | "each" | "where" | "filter") if self.depth < MAX_DEPTH => elems
                        .or_else(|| {
                            if piped {
                                None
                            } else {
                                args.first().and_then(|a| const_array(&self.consts, a))
                            }
                        }),
                    _ => None,
                };
                self.calls.push(r);
                self.positional.insert(
                    self.calls.len() - 1,
                    positional_literals(&self.consts, args),
                );
                self.literals
                    .push((piped, literal_args(&self.consts, args, named)));
                if !matches!(callee.as_ref(), Expr::Ident(_) | Expr::MemberAccess { .. }) {
                    self.expr(callee);
                }
                for a in args {
                    match (a, &source) {
                        (Expr::Lambda { params, body }, Some(items)) if !params.is_empty() => {
                            self.depth += 1;
                            for item in items {
                                let bind = [(params[0].clone(), Some(item.clone()))];
                                self.scoped_values(&bind, |w| w.expr(body));
                            }
                            self.depth -= 1;
                        }
                        _ => self.expr(a),
                    }
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
                        self.literals.push((true, Vec::new()));
                        if let Some(b) = target.as_deref() {
                            self.code_arg(b, &[]);
                        }
                    }
                    other => {
                        self.piped_next = matches!(other, Expr::Call { .. });
                        self.elems = if self.piped_next {
                            const_array(&self.consts, left)
                        } else {
                            None
                        };
                        self.expr(other);
                        self.piped_next = false;
                        self.elems = None;
                    }
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
                    self.literals.push((false, Vec::new()));
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
                    self.literals.push((false, Vec::new()));
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
fn jail_refusal(
    builtin: &str,
    effect: Effect,
    literals: &[(LiteralSlot, String)],
    piped: bool,
) -> Option<String> {
    if !matches!(
        effect,
        Effect::ReadLocal | Effect::WriteLocal | Effect::Destructive
    ) {
        return None;
    }
    crate::security::outside_path_literal(builtin, literals, piped).map(|s| {
        format!(
            "'{s}' is outside the workspace root, so the run refuses it with E_OUTSIDE_WORKSPACE"
        )
    })
}

/// The refusal of a path argument whose name Windows would not keep as written
/// (a device name, a trailing dot or space, a forbidden character, and for a
/// write a `:stream`), worded as the run words it. The run refuses it with
/// E_BAD_ARG before it consults the jail, so `a<b` is that and not an
/// outside-the-workspace path. Only on Windows, and only for an argument
/// written out as a literal in a position the builtin's signature calls a path;
/// a builtin that takes a piped subject is not predicted, as its arguments shift.
fn name_refusal(
    builtin: &str,
    effect: Effect,
    positional: Option<&Vec<Option<String>>>,
) -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let writes = matches!(effect, Effect::WriteLocal | Effect::Destructive);
    if !writes && !matches!(effect, Effect::ReadLocal | Effect::Exec) {
        return None;
    }
    let sig = crate::signature::signature_of(builtin)?;
    if sig.subject.is_some() {
        return None;
    }
    for (i, literal) in positional?.iter().enumerate() {
        let Some(s) = literal else { continue };
        let param = sig
            .params
            .get(i)
            .or_else(|| sig.params.last().filter(|p| p.variadic));
        let Some(param) = param else { break };
        if !crate::security::names_a_path_param(param.name)
            || s.is_empty()
            || s.len() > 4096
            || s.contains('\n')
            || s.contains('\0')
        {
            continue;
        }
        if let Some(e) = crate::safety::windows_param_name_refusal(builtin, s, writes) {
            return Some(e.message);
        }
        // The write guard judges the path a write goes to, `:stream` included.
        if writes && !matches!(param.name, "src" | "source" | "source_dir") {
            if let Some(e) = crate::safety::windows_name_refusal(builtin, s) {
                return Some(e.message);
            }
        }
    }
    None
}

/// The refusal of `rm` given a glob it would not expand: E_BAD_ARG, worded as the
/// run words it. The name is judged as the run judges it, so a file that really
/// has a `*` in its name is not a glob. `None` for any other builtin.
fn glob_refusal(builtin: &str, positional: Option<&Vec<Option<String>>>) -> Option<String> {
    if builtin != "rm" {
        return None;
    }
    let name = positional?.first()?.as_ref()?;
    if !name.contains(['*', '?'])
        || ["\\?\\", "\\.\\", "//?/", "//./"]
            .iter()
            .any(|p| name.starts_with(p))
        || crate::safety::names_a_remote_share(name)
        || std::fs::symlink_metadata(name).is_ok()
    {
        return None;
    }
    Some(
        "rm does not expand globs; list the files with glob(...) and remove each (the run refuses it with E_BAD_ARG)"
            .to_string(),
    )
}

/// The refusal of an archive extraction whose archive is named by a literal and
/// can be read here: an entry that lands outside the destination or the
/// workspace, through a link already on disk or by its name. The archive is read
/// and nothing is written. `None` for any other builtin, or an archive that
/// cannot be read.
fn extraction_refusal(builtin: &str, literals: &[String]) -> Option<String> {
    if !matches!(builtin, "zip_extract" | "unzip" | "tar_extract") {
        return None;
    }
    let archive = literals.first().filter(|a| !a.is_empty())?;
    let dest = literals.get(1).map(String::as_str).unwrap_or(".");
    crate::builtins::explain_extraction_refusal(builtin, archive, dest)
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
        positional: HashMap::new(),
        piped_next: false,
        consts: HashMap::new(),
        fns: HashMap::new(),
        depth: 0,
        elems: None,
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
                let (piped, literals) = match w.literals.get(i) {
                    Some((p, l)) => (*p, l.as_slice()),
                    None => (false, &[][..]),
                };
                let plain: Vec<String> = literals.iter().map(|(_, s)| s.clone()).collect();
                let refused = refused_outright(b)
                    .or_else(|| name_refusal(b, effect, w.positional.get(&i)))
                    .or_else(|| jail_refusal(b, effect, literals, piped))
                    .or_else(|| glob_refusal(b, w.positional.get(&i)))
                    .or_else(|| extraction_refusal(b, &plain));
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

    fn decision_of(code: &str) -> Value {
        field(&explain(code).unwrap(), "decision").clone()
    }

    #[test]
    fn text_that_reads_like_a_path_is_not_judged_as_one() {
        // The run reads no file for any of these, so none is denied.
        for code in [
            r#"from_json("{\"a\":1}")"#,
            r#"from_json("../x")"#,
            r#"sql_value([{n: 1}], "select count(*) from t")"#,
            r#"[{n: 1}] | sql("select * from t where s = '../x'")"#,
        ] {
            assert_ne!(decision_of(code), Value::Str("deny".into()), "{code}");
        }
    }

    #[test]
    fn a_path_argument_outside_the_workspace_is_still_denied() {
        assert_eq!(
            decision_of(r#"file.read("../x")"#),
            Value::Str("deny".into())
        );
        assert_eq!(
            decision_of(r#"read_text("../x")"#),
            Value::Str("deny".into())
        );
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

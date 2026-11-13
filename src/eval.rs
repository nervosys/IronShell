use crate::ast::{BinOp, CfgCondition, Expr, Stmt, UnOp, Visibility};
use crate::builtins;
use crate::env::Env;
use crate::value::{AsyncLambda, Future, Lambda, Value};
use anyhow::Result;
use std::collections::BTreeMap;

/// Evaluate a cfg condition at runtime
fn eval_cfg_condition(condition: &CfgCondition) -> Result<bool> {
    match condition {
        CfgCondition::Platform(platform) => {
            let current_os = std::env::consts::OS;
            Ok(match platform.as_str() {
                "windows" => current_os == "windows",
                "linux" => current_os == "linux",
                "macos" => current_os == "macos",
                "unix" => current_os != "windows",
                other => current_os == other,
            })
        }
        CfgCondition::Feature(feature) => {
            // Check environment variable IRONSHELL_FEATURES for enabled features
            let features = std::env::var("IRONSHELL_FEATURES").unwrap_or_default();
            Ok(features.split(',').any(|f| f.trim() == feature))
        }
        CfgCondition::Not(inner) => Ok(!eval_cfg_condition(inner)?),
        CfgCondition::All(conditions) => {
            for cond in conditions {
                if !eval_cfg_condition(cond)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        CfgCondition::Any(conditions) => {
            for cond in conditions {
                if eval_cfg_condition(cond)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

/// Where a top-level statement starts: its line, and the module file it was read
/// from (None for the script being run). Runtime errors name it.
#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    pub line: usize,
    pub file: Option<String>,
}

thread_local! {
    /// The top-level statement now running, when a caller that knows its line
    /// has set one (the script runner and module loader do; others leave None).
    static STATEMENT: std::cell::RefCell<Option<Site>> = const { std::cell::RefCell::new(None) };
    /// Named lambdas bound by a top-level `name = fn ...` statement, and where.
    static LAMBDA_SITES: std::cell::RefCell<BTreeMap<String, Site>> =
        const { std::cell::RefCell::new(BTreeMap::new()) };
    /// The lambda a failing call came out of, outermost-first kept: the first
    /// one recorded is the innermost, which is where the failure is.
    static FAILED_IN: std::cell::RefCell<Option<(String, Site)>> =
        const { std::cell::RefCell::new(None) };
}

/// Set the statement now running, returning the one it replaces. Clears the
/// record of a failed lambda call, which belongs to the statement before.
pub fn set_statement_site(site: Option<Site>) -> Option<Site> {
    FAILED_IN.with(|f| *f.borrow_mut() = None);
    STATEMENT.with(|s| std::mem::replace(&mut *s.borrow_mut(), site))
}

/// Forget a failed lambda call that was caught: a `try` handled it.
pub fn clear_failed_lambda() {
    FAILED_IN.with(|f| *f.borrow_mut() = None);
}

/// The note for the failing statement's error: which lambda it came out of and
/// where that lambda was written, when that is another statement or a module
/// file. Taken, so one error is noted once.
pub fn take_failed_lambda_note() -> Option<String> {
    let (name, site) = FAILED_IN.with(|f| f.borrow_mut().take())?;
    Some(match site.file {
        None => format!("(in `{name}`, defined at line {})", site.line),
        Some(file) => format!("(in `{name}`, defined at {file} line {})", site.line),
    })
}

/// A top-level `name = fn ...` statement that just ran: remember where it was
/// written, so a call that fails inside it can say so.
fn note_lambda_binding(stmt: &Stmt) {
    let Stmt::Let {
        name,
        value: Expr::Lambda { .. },
        ..
    } = stmt
    else {
        return;
    };
    if let Some(site) = STATEMENT.with(|s| s.borrow().clone()) {
        LAMBDA_SITES.with(|m| m.borrow_mut().insert(name.clone(), site));
    }
}

/// A call through the name `name` failed. When the lambda it names was written
/// somewhere other than the statement now running, record it, unless a deeper
/// lambda already is the record.
fn note_failed_lambda_call(name: &str) {
    let Some(site) = LAMBDA_SITES.with(|m| m.borrow().get(name).cloned()) else {
        return;
    };
    let Some(here) = STATEMENT.with(|s| s.borrow().clone()) else {
        return;
    };
    if site == here {
        return;
    }
    FAILED_IN.with(|f| {
        let mut f = f.borrow_mut();
        if f.is_none() {
            *f = Some((name.to_string(), site));
        }
    });
}

pub fn eval_program(stmts: &[Stmt], env: &mut Env) -> Result<Value> {
    let mut last = Value::Null;
    for (i, s) in stmts.iter().enumerate() {
        // Only the last statement's value is the program's, so a binding
        // before it does not copy its value out (`s = s + "x"` stays linear).
        last = eval_program_statement(s, i + 1 < stmts.len(), env)?;
    }
    Ok(last)
}

/// One statement of a program, as [`eval_program`] runs it. `discard` says the
/// statement's value is not the program's, because another statement follows
/// it: a binding then does not copy its value out. The REPL runs each top-level
/// statement on its own, and passes `discard` for all but the last, so a loop of
/// `a = a + [x]` lines is not a copy of `a` per line.
pub fn eval_program_statement(s: &Stmt, discard: bool, env: &mut Env) -> Result<Value> {
    // Clear any pipe input between statements to prevent leakage
    env.set_input(None);
    let value = match s {
        Stmt::Let {
            name,
            value,
            is_mut,
            visibility,
            ty,
            bare,
        } if discard => eval_let(
            name,
            value,
            *is_mut,
            *visibility,
            ty.as_deref(),
            *bare,
            env,
            false,
        )?,
        _ => eval_stmt(s, env)?,
    };
    note_lambda_binding(s);
    // An error value that a statement *produces* and nothing binds or
    // catches is an uncaught error. `throw` and `assert` produce error
    // values rather than unwinding, so without this `throw "x"` was
    // followed by the next statement and the program exited 0 --
    // `assert(false)` included. Bound with `let` or handled in `try`, an
    // error value is still an ordinary value, and naming one (`r` on its
    // own line) shows it rather than raising it.
    if let (Stmt::Expr(e), Value::Error(msg)) = (s, &value) {
        if !matches!(e, Expr::Ident(_) | Expr::MemberAccess { .. }) {
            return Err(anyhow::anyhow!("uncaught error: {msg}"));
        }
    }
    Ok(value)
}

/// Streaming evaluation (docs/AGENTIC_FIRST_DESIGN.md §6.3): evaluate `code` and
/// deliver results to `on_item` **incrementally**, rather than materializing the
/// whole value before chunking. When the final expression is a pipeline
/// `source | stage…` whose source is an `Array` and whose every stage is
/// element-independent (`map`/`where`/`filter`), each element is pushed through the
/// stage chain one at a time and emitted as soon as it survives — true
/// stage-by-stage streaming, computed lazily per element. Any other shape falls
/// back to eager evaluation, emitting array elements (or a single scalar) after.
/// Returns the number of items emitted.
///
/// A literal `take(n)` streams too, and is where laziness stops being a
/// description of the *output* and starts saving *work*: once the count is met
/// the source is abandoned unread, so `xs | map(f) | take(3)` calls `f` three
/// times rather than once per element. The early exit is withheld when any
/// upstream stage reaches a builtin that is not `Pure` — skipping work the
/// program asked for is not an optimisation — and such a pipeline still streams,
/// it just reads to the end. [`eval_stream_with_stats`] reports which happened.
///
/// A whole-collection stage (`sort`/`reduce`/`uniq`) ends the streamable region
/// rather than disqualifying the pipeline. It used to do the latter, which cost
/// laziness *retroactively*: `xs | map(f) | take(3) | sort` fell back entirely
/// and called `f` once per element of `xs` to sort the three that `sort` was
/// ever going to see. The prefix now streams and the barrier is handed exactly
/// the collection it would have received eagerly.
///
/// This is an **additive** path: the eager [`eval_expr`]/[`eval_program`] are
/// untouched, and each streamed stage is driven through the *same* pipe mechanism
/// (`env.set_input` + `eval_expr`) the normal evaluator uses, so element semantics
/// are identical — there is no second implementation of `map`/`where` to diverge.
pub fn eval_stream(code: &str, env: &mut Env, on_item: &mut dyn FnMut(Value)) -> Result<usize> {
    eval_stream_with_stats(code, env, on_item).map(|s| s.emitted)
}

/// What a streaming run actually did.
///
/// `emitted` is what the caller received. `pulled` is how many source elements
/// were *touched* -- the number that says whether laziness bought anything. For
/// `xs | map(f) | take(3)` over a thousand elements, an implementation that
/// materializes reports `pulled == 1000` and a lazy one reports `pulled == 3`;
/// without the number the two are indistinguishable from the outside, which is
/// how "streaming" stays a claim rather than a measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StreamStats {
    /// Values handed to the callback.
    pub emitted: usize,
    /// Source elements pushed through the stage chain.
    pub pulled: usize,
    /// Whether the element-wise path ran at all. `false` means the value went
    /// through the stages in a single batch, the way the eager evaluator does it.
    pub streamed: bool,
    /// Whether a `take` was satisfied and the remaining source abandoned.
    pub short_circuited: bool,
    /// Whether a whole-collection stage (`sort`/`reduce`/`uniq`) remained after
    /// the streamed prefix, so that prefix's output was materialised for it.
    /// The stages *ahead* of it still streamed — which is the whole point: a
    /// barrier at the end no longer costs laziness at the start.
    pub barrier_tail: bool,
}

/// [`eval_stream`], reporting what the run cost as well as what it produced.
pub fn eval_stream_with_stats(
    code: &str,
    env: &mut Env,
    on_item: &mut dyn FnMut(Value),
) -> Result<StreamStats> {
    let stmts = crate::parser::parse_program(code)?;
    let (last, init) = match stmts.split_last() {
        Some(parts) => parts,
        None => return Ok(StreamStats::default()),
    };
    // Evaluate the leading statements eagerly (bindings, imports, side effects).
    for s in init {
        env.set_input(None);
        // Their values are not the program's: a binding does not copy its value out.
        match s {
            Stmt::Let {
                name,
                value,
                is_mut,
                visibility,
                ty,
                bare,
            } => {
                eval_let(
                    name,
                    value,
                    *is_mut,
                    *visibility,
                    ty.as_deref(),
                    *bare,
                    env,
                    false,
                )?;
            }
            _ => {
                eval_stmt(s, env)?;
            }
        }
    }
    env.set_input(None);
    // Try to stream the final statement if it is a streamable pipeline.
    if let Stmt::Expr(expr) = last {
        if let Some(stats) = try_stream_pipeline(expr, env, on_item)? {
            return Ok(stats);
        }
    }
    // Fallback: eager-eval the final statement, then emit its elements.
    let v = eval_stmt(last, env)?;
    let emitted = emit_value(v, on_item);
    Ok(StreamStats {
        emitted,
        pulled: emitted,
        streamed: false,
        short_circuited: false,
        barrier_tail: false,
    })
}

fn emit_value(v: Value, on_item: &mut dyn FnMut(Value)) -> usize {
    match v {
        Value::Array(items) => {
            let n = items.len();
            for it in items {
                on_item(it);
            }
            n
        }
        Value::Null => 0,
        other => {
            on_item(other);
            1
        }
    }
}

/// What a pipeline stage needs in order to produce its output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StageKind {
    /// Transforms each element without reference to the others
    /// (`map`/`where`/`filter`), so it can be applied one element at a time.
    Elementwise,
    /// Passes the first `n` elements through and nothing after (`take(n)`).
    /// This is the stage that makes laziness worth having: once it is satisfied
    /// nothing downstream can ever emit again, so the source need not be read.
    Prefix(i64),
    /// Needs the whole collection (`sort`/`reduce`/`uniq`/anything unknown).
    ///
    /// A barrier ends the streamable region; it does **not** disqualify the
    /// stages ahead of it. Those still stream, and a `take` among them still
    /// stops the source — the barrier is simply handed the collection that
    /// reaches it, which is the same collection it would have been handed
    /// eagerly. Treating one barrier as poisoning the whole pipeline is what
    /// made `xs | map(f) | take(3) | sort` call `f` a thousand times.
    Barrier,
}

/// Classify one stage of a pipeline.
///
/// `take` counts only with a literal count: evaluating an argument expression
/// here would run it before the stages ahead of it, which is a different program
/// from the one that was written.
fn stage_kind(e: &Expr) -> StageKind {
    let Expr::Call { callee, args, .. } = e else {
        return StageKind::Barrier;
    };
    let Expr::Ident(name) = &**callee else {
        return StageKind::Barrier;
    };
    match name.as_str() {
        "map" | "where" | "filter" => StageKind::Elementwise,
        // A negative count is a barrier, so the builtin refuses it; as a prefix
        // it would take nothing and end the stream with no error.
        "take" => match args.first() {
            Some(Expr::LitInt(n)) if *n >= 0 => StageKind::Prefix(*n),
            _ => StageKind::Barrier,
        },
        _ => StageKind::Barrier,
    }
}

/// Whether evaluating `e` can be skipped without changing what the program does.
///
/// Abandoning the tail of a source is only sound when the work being skipped was
/// not the point. `xs | map(fn(x) => file_write(x, "…")) | take(3)` writes every
/// file when evaluated eagerly, and short-circuiting after three would silently
/// drop the rest -- so a stage that reaches anything but a `Pure` builtin
/// disqualifies the early exit, and the pipeline still streams, just without
/// abandoning the source.
///
/// The judgement is only as good as `safety::effect_of`, which is exactly the
/// classifier `tests/effect_ratchet.rs` holds to "no builtin that acts is
/// classified `Pure`". A name no builtin implements counts as unknown, not pure:
/// it may be a user-defined function whose body this walk cannot see.
///
/// "Implements" means either half of the dispatcher. This used to ask
/// `BUILTIN_LOOKUP` alone, so the ~113 names served by the fallback `match`
/// (`from_json`, `select`, `group_by`, `to_csv`, ...) read as unknown and any
/// `take` downstream of one withheld its short-circuit. That was fail-safe --
/// more work, never wrong work -- but it was fail-safe by accident, and the
/// reason it could not simply be widened was that nothing in `src/` knew what
/// the fallback served. `builtins::is_dispatched` is that missing answer, and
/// the ratchet now reads both halves too, so `Pure` means the same thing on
/// either side of the dispatcher.
fn is_effect_free(e: &Expr) -> bool {
    use crate::safety::{effect_of, Effect};
    match e {
        Expr::LitInt(_)
        | Expr::LitFloat(_)
        | Expr::LitStr(_)
        | Expr::LitBool(_)
        | Expr::Null
        | Expr::Ident(_) => true,
        Expr::Array(items) => items.iter().all(is_effect_free),
        Expr::Record(fields) => fields.iter().all(|(_, v)| is_effect_free(v)),
        Expr::Lambda { body, .. } => is_effect_free(body),
        Expr::Binary { left, right, .. } => is_effect_free(left) && is_effect_free(right),
        Expr::Unary { expr, .. } => is_effect_free(expr),
        Expr::MemberAccess { object, .. } => is_effect_free(object),
        Expr::Index { object, index } => is_effect_free(object) && is_effect_free(index),
        Expr::Pipe { left, right } => is_effect_free(left) && is_effect_free(right),
        Expr::Call {
            callee,
            args,
            named,
        } => {
            let callee_pure = match &**callee {
                Expr::Ident(name) => {
                    crate::builtins::is_dispatched(name) && effect_of(name) == Effect::Pure
                }
                // A computed callee, a member call, a user-defined function --
                // this walk cannot see the body, so it cannot vouch for it.
                _ => false,
            };
            callee_pure
                && args.iter().all(is_effect_free)
                && named.iter().all(|(_, v)| is_effect_free(v))
        }
        // Deliberately conservative: async work, thrown control flow and match
        // arms are not worth the analysis for the payoff, and being wrong here
        // means silently skipping work the program asked for.
        _ => false,
    }
}

/// Push one value through every stage of a pipe chain and return what comes out.
///
/// This is the eager evaluator's [`Expr::Pipe`] behaviour, reproduced for a
/// source that has already been evaluated: set the value as the pipe input,
/// evaluate the stage, repeat. `take` is deliberately *not* given the streaming
/// path's hand-rolled accounting here -- there is a single batch, so calling the
/// builtin is what eager evaluation would do, and matching it is the whole point.
fn apply_stages(mut batch: Value, stages: &[&Expr], env: &mut Env) -> Result<Value> {
    for stage in stages {
        let saved = env.take_input();
        env.set_input(Some(batch));
        let res = eval_expr(stage, env);
        match saved {
            Some(v) => env.set_input(Some(v)),
            None => env.set_input(None),
        }
        batch = res?;
    }
    Ok(batch)
}

/// If `expr` is a streamable pipeline, stream it element-by-element and return
/// `Some(stats)`; otherwise return `None` so the caller eager-evaluates.
fn try_stream_pipeline(
    expr: &Expr,
    env: &mut Env,
    on_item: &mut dyn FnMut(Value),
) -> Result<Option<StreamStats>> {
    // Flatten the left-associative pipe chain into source + ordered stages.
    let mut stages: Vec<&Expr> = Vec::new();
    let mut cur = expr;
    while let Expr::Pipe { left, right } = cur {
        stages.push(right);
        cur = left;
    }
    if stages.is_empty() {
        return Ok(None); // not a pipeline
    }
    stages.reverse();
    let all_kinds: Vec<StageKind> = stages.iter().map(|s| stage_kind(s)).collect();

    // A whole-collection stage ends the streamable region -- it cannot answer
    // until it has seen everything -- but it does not disqualify the stages
    // *ahead* of it, which is what returning `None` here used to do. That cost
    // more than tidiness: `xs | map(f) | take(3) | sort` fell back entirely and
    // called `f` once per element of `xs`, when three was all `sort` was ever
    // going to be given. The prefix streams; the barrier and everything after it
    // run once, eagerly, on what the prefix produced -- exactly the collection
    // they would have been handed anyway.
    let tail_at = all_kinds
        .iter()
        .position(|k| *k == StageKind::Barrier)
        .unwrap_or(stages.len());
    if tail_at == 0 {
        // Nothing streamable ahead of the barrier, so there is nothing to gain
        // and a needless copy to pay. `[3,1,2] | sort()` still falls back.
        return Ok(None);
    }
    let tail: Vec<&Expr> = stages[tail_at..].to_vec();
    let stages: Vec<&Expr> = stages[..tail_at].to_vec();
    let kinds: Vec<StageKind> = all_kinds[..tail_at].to_vec();
    // The source must be an array to stream element-wise.
    let src = eval_expr(cur, env)?;
    let items = match src {
        Value::Array(items) => items,
        // Not element-wise streamable -- but the source has *already run*, and
        // returning `Ok(None)` here would send the caller back to eager-evaluate
        // the whole statement, source included. `file_append(p, "x") | map(f)`
        // appended twice. A second side effect is a wrong answer, not a slow
        // one, so push the value that was already computed through the stages
        // instead of recomputing it -- which is exactly what the eager
        // evaluator's `Expr::Pipe` arm does with the same value.
        other => {
            // Every stage, not just the streamed head: a scalar has nothing to
            // stream, so the split above is irrelevant to it, and applying only
            // the head would silently drop stages the program wrote.
            let every: Vec<&Expr> = stages.iter().chain(tail.iter()).copied().collect();
            let out = apply_stages(other, &every, env)?;
            let emitted = emit_value(out, on_item);
            return Ok(Some(StreamStats {
                emitted,
                pulled: 1,
                streamed: false,
                short_circuited: false,
                barrier_tail: !tail.is_empty(),
            }));
        }
    };

    // A `take` may abandon the rest of the source only when everything upstream
    // of it is effect-free -- otherwise the skipped elements were work the
    // program asked for. Computed per `take`, since a later one may be preceded
    // by a stage an earlier one was not.
    let may_abandon: Vec<bool> = kinds
        .iter()
        .enumerate()
        .map(|(i, k)| {
            matches!(k, StageKind::Prefix(_)) && stages[..i].iter().all(|s| is_effect_free(s))
        })
        .collect();

    let mut taken = vec![0i64; stages.len()];
    let mut stats = StreamStats {
        streamed: true,
        barrier_tail: !tail.is_empty(),
        ..Default::default()
    };
    // With a barrier following, the head's output is what the barrier consumes,
    // so it is collected rather than emitted. With no barrier nothing is
    // buffered and each value reaches the callback as it is produced — the SSE
    // route depends on that, so the two cases stay genuinely different.
    let mut buffered: Vec<Value> = Vec::new();

    for x in items {
        // A satisfied `take` ends the run before the next element is read --
        // this, and not the emitting, is where laziness saves the work.
        let satisfied = kinds
            .iter()
            .enumerate()
            .any(|(i, k)| may_abandon[i] && matches!(k, StageKind::Prefix(n) if taken[i] >= *n));
        if satisfied {
            stats.short_circuited = true;
            break;
        }
        stats.pulled += 1;
        // Push this single element through the stage chain using the SAME pipe
        // mechanism the evaluator uses, so semantics are identical.
        let mut batch = Value::Array(vec![x]);
        for (i, stage) in stages.iter().enumerate() {
            if let StageKind::Prefix(n) = kinds[i] {
                // `take` is applied here rather than by calling the builtin: the
                // builtin sees one element at a time and would let every one of
                // them through.
                let width = match &batch {
                    Value::Array(items) => items.len() as i64,
                    Value::Null => 0,
                    _ => 1,
                };
                let room = (n - taken[i]).max(0);
                if room == 0 {
                    // Nothing downstream of a satisfied `take` can emit again.
                    batch = Value::Array(Vec::new());
                    break;
                }
                taken[i] += width.min(room);
                if width > room {
                    if let Value::Array(items) = batch {
                        batch = Value::Array(items.into_iter().take(room as usize).collect());
                    }
                }
                continue;
            }
            let saved = env.take_input();
            env.set_input(Some(batch));
            let res = eval_expr(stage, env);
            match saved {
                Some(v) => env.set_input(Some(v)),
                None => env.set_input(None),
            }
            batch = res?;
        }
        let mut deliver = |v: Value| {
            if tail.is_empty() {
                on_item(v);
                stats.emitted += 1;
            } else {
                buffered.push(v);
            }
        };
        match batch {
            Value::Array(out) => {
                for it in out {
                    deliver(it);
                }
            }
            Value::Null => {}
            other => deliver(other),
        }
    }

    // The barrier, and everything after it, once — on exactly the collection it
    // would have been handed by the eager evaluator.
    if !tail.is_empty() {
        let out = apply_stages(Value::Array(buffered), &tail, env)?;
        stats.emitted += emit_value(out, on_item);
    }
    Ok(Some(stats))
}

pub fn eval_stmt(stmt: &Stmt, env: &mut Env) -> Result<Value> {
    match stmt {
        Stmt::Let {
            name,
            value,
            is_mut,
            visibility,
            ty,
            bare,
        } => eval_let(
            name,
            value,
            *is_mut,
            *visibility,
            ty.as_deref(),
            *bare,
            env,
            true,
        ),
        Stmt::Expr(e) => eval_expr(e, env),
        Stmt::SetField { name, path, value } => {
            // The new value is evaluated while `name` is still bound, so
            // `r.a = r.a + 1` reads the old `a`. The Record is copied, changed
            // and stored back, so a refusal anywhere leaves `name` as it was.
            let v = eval_expr(value, env)?;
            let mut updated = match env.get_var(name) {
                Some(root) => root.clone(),
                None => return Err(unknown_name_error(name, env)),
            };
            set_field_path(&mut updated, name, path, v.clone())?;
            env.set_var(name, updated).map_err(|e| {
                crate::safety::bad_state_with_hint("assign", &e, &env.rebind_hint(name))
            })?;
            Ok(v)
        }
        Stmt::Import {
            items,
            source,
            alias,
        } => {
            crate::module_loader::import(items, source, alias.as_deref(), env)?;
            Ok(Value::Null)
        }
        Stmt::Export { items, from_source } => {
            match from_source {
                Some(source) => crate::module_loader::reexport(items, source, env)?,
                None => crate::module_loader::export_local(items, env)?,
            }
            Ok(Value::Null)
        }
        Stmt::Cfg { condition, body } => {
            // Evaluate the cfg condition at runtime
            if eval_cfg_condition(condition)? {
                eval_stmt(body, env)
            } else {
                Ok(Value::Null)
            }
        }
    }
}

/// `let name = value`, `name = value` and `name := value`. With `keep` the bound
/// value is returned; without it the result is Null, so a binding whose value
/// nothing reads is not copied out (see `eval_program`).
fn eval_let(
    name: &str,
    value: &Expr,
    is_mut: bool,
    visibility: Visibility,
    ty: Option<&str>,
    bare: bool,
    env: &mut Env,
    keep: bool,
) -> Result<Value> {
    if let Some(right) = in_place_append_operand(name, value, ty, visibility, env) {
        let operand = eval_expr(right, env)?;
        let operand = match append_in_place(env, name, operand) {
            Ok(()) => {
                let kept = if keep {
                    env.get_var(name).cloned().unwrap_or(Value::Null)
                } else {
                    Value::Null
                };
                return Ok(kept);
            }
            Err(operand) => operand,
        };
        // Not a case the fast path covers (a mismatched operand, or a string past
        // the size limit): the general rule for `+` decides it, and when it
        // refuses, `name` is left as it was.
        let lhs = env.get_var(name).cloned().unwrap_or(Value::Null);
        let v = binop(&BinOp::Add, lhs, operand)?;
        return bind_value(name, v, is_mut, visibility, bare, env, keep);
    }
    let v = eval_expr(value, env)?;
    if let Some(declared) = ty {
        check_declared_type(name, declared, &v)?;
    }
    bind_value(name, v, is_mut, visibility, bare, env, keep)
}

/// Stores `v` as `name`, marks it public when asked, and returns it when `keep`.
/// A `bare` statement (`name = value`, no `let`) that cannot rebind `name` is
/// reported as an assignment; a `let` is reported as a `let`.
fn bind_value(
    name: &str,
    v: Value,
    is_mut: bool,
    visibility: Visibility,
    bare: bool,
    env: &mut Env,
    keep: bool,
) -> Result<Value> {
    let kept = keep.then(|| v.clone());
    env.declare_var(name, v, is_mut).map_err(|e| {
        // `bad_state` would read this as a builtin to call first, which
        // is how "call `use a different name...` first" came about.
        if bare {
            crate::safety::bad_state_with_hint("assign", &e, &env.reassign_hint(name))
        } else {
            crate::safety::bad_state_with_hint("let", &e, &env.rebind_hint(name))
        }
    })?;

    // Mark as public if visibility is Pub
    if visibility == Visibility::Pub {
        env.set_public(name);
    }
    Ok(kept.unwrap_or(Value::Null))
}

/// The right operand of `name = name + e` when that statement may append in
/// place: a plain `=` (no annotation, not `pub`) to a mutable binding that holds
/// a String or an Array. Anything else takes the general path.
fn in_place_append_operand<'a>(
    name: &str,
    value: &'a Expr,
    ty: Option<&str>,
    visibility: Visibility,
    env: &Env,
) -> Option<&'a Expr> {
    let Expr::Binary {
        left,
        op: BinOp::Add,
        right,
    } = value
    else {
        return None;
    };
    let Expr::Ident(lhs) = &**left else {
        return None;
    };
    let fits = lhs.as_str() == name
        && ty.is_none()
        && visibility == Visibility::Private
        && env.is_mutable(name)
        && matches!(env.get_var(name), Some(Value::Str(_) | Value::Array(_)));
    fits.then_some(&**right)
}

/// Appends `operand` to the String or Array bound to `name`, in place, so the
/// binding is not copied out and back. Gives the operand back when the two do
/// not both fit: a different kind, or a string past the size limit.
/// Evaluate a lambda body whose parameters are bound in `env` and are not read
/// again afterwards. A body `acc + e`, where `acc` is a parameter that `e` does
/// not mention, moves the parameter's String or Array out and appends to it,
/// so `reduce(fn(acc, x) => acc + x, "")` is linear instead of copying the
/// accumulator at every step. Any other body is evaluated as usual.
pub(crate) fn eval_body_consuming_params(
    body: &Expr,
    params: &[String],
    env: &mut Env,
) -> Result<Value> {
    if let Expr::Binary {
        left,
        op: BinOp::Add,
        right,
    } = body
    {
        if let Expr::Ident(name) = left.as_ref() {
            if params.iter().any(|p| p == name) {
                let mut mentioned = std::collections::BTreeSet::new();
                free_idents(right, &mut Vec::new(), &mut mentioned);
                if !mentioned.contains(name.as_str()) {
                    let rhs = eval_expr(right, env)?;
                    match (env.get_var_mut(name), rhs) {
                        (Some(Value::Str(a)), Value::Str(b))
                            if a.len() + b.len() <= MAX_STRING_BYTES =>
                        {
                            let mut out = std::mem::take(a);
                            out.push_str(&b);
                            return Ok(Value::Str(out));
                        }
                        (Some(Value::Array(a)), Value::Array(b)) => {
                            let mut out = std::mem::take(a);
                            out.extend(b);
                            return Ok(Value::Array(out));
                        }
                        (Some(a), rhs) => return binop(&BinOp::Add, a.clone(), rhs),
                        (None, _) => {}
                    }
                }
            }
        }
    }
    eval_expr(body, env)
}

fn append_in_place(env: &mut Env, name: &str, operand: Value) -> std::result::Result<(), Value> {
    match (env.get_var_mut(name), operand) {
        (Some(Value::Str(a)), Value::Str(b)) if a.len() + b.len() <= MAX_STRING_BYTES => {
            a.push_str(&b);
            Ok(())
        }
        (Some(Value::Array(a)), Value::Array(b)) => {
            a.extend(b);
            Ok(())
        }
        (_, operand) => Err(operand),
    }
}

pub fn eval_expr(expr: &Expr, env: &mut Env) -> Result<Value> {
    // Cooperative cancellation for callers that set a deadline (the Agent API).
    // A no-op with none set — one thread-local read — which is every REPL
    // session, script and test. See `safety::check_deadline`.
    crate::safety::check_deadline()?;

    match expr {
        // ---------- literals ----------
        Expr::LitInt(n) => Ok(Value::Int(*n)),
        Expr::LitFloat(f) => Ok(Value::Float(*f)),
        Expr::LitStr(s) => {
            // Handle string interpolation: ${expr}
            if s.contains("${") {
                interpolate_string(s, env)
            } else {
                Ok(Value::Str(s.replace(crate::parser::ESCAPED_DOLLAR, "$")))
            }
        }
        Expr::LitBool(b) => Ok(Value::Bool(*b)),
        Expr::Null => Ok(Value::Null),

        // ---------- variables ----------
        Expr::Ident(name) => match env.get_var(name) {
            Some(value) => Ok(value.clone()),
            // A builtin name that nothing binds is a function value, the same
            // kind a module member is. `g = rm` then binds a callable, where it
            // used to bind Null and fail at `g(...)` with "got Null".
            None if builtins::is_dispatched(name) => Ok(crate::modules::builtin_ref(name)),
            // A name nothing binds is an error. It used to be null, so a typo
            // in a variable ran on with null and failed later, far from its cause.
            None => Err(unknown_name_error(name, env)),
        },

        // ---------- collections ----------
        Expr::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(eval_expr(it, env)?);
            }
            Ok(Value::Array(out))
        }
        Expr::Record(kvs) => {
            let mut m = BTreeMap::new();
            for (k, v) in kvs {
                m.insert(k.clone(), eval_expr(v, env)?);
            }
            Ok(Value::Record(m))
        }

        // ---------- lambda ----------
        Expr::Lambda { params, body } => Ok(Value::Lambda(Lambda {
            params: params.clone(),
            body: body.clone(),
            captured: capture_free_vars(params, body, env),
        })),

        // ---------- async lambda ----------
        Expr::AsyncLambda { params, body } => Ok(Value::AsyncLambda(AsyncLambda {
            params: params.clone(),
            body: body.clone(),
            captured: capture_free_vars(params, body, env),
        })),

        // ---------- await ----------
        Expr::Await(inner) => {
            let val = eval_expr(inner, env)?;
            match val {
                Value::Future(future) => {
                    // Bind the parameters in the *calling* environment and
                    // restore afterwards — the same shape `call_lambda1` uses
                    // for a plain lambda.
                    //
                    // This used to build a fresh `Env::new()`, so the body of an
                    // async lambda could see nothing but its own arguments:
                    //
                    //     let inner = async fn(x) => x + 1
                    //     let outer = async fn(n) => await inner(n)
                    //
                    // failed with `unknown builtin: inner`, because `inner` was
                    // not a builtin and the fresh environment held no bindings.
                    // Plain lambdas never had the problem, which is why it went
                    // unnoticed.
                    let _depth = crate::safety::enter_call()?;
                    let saved_pipe = env.take_input();
                    env.set_input(None);
                    let restore_caps = install_captured(&future.lambda.captured, env);

                    let mut saved: Vec<(String, Option<Value>)> = Vec::new();
                    for (param, arg) in future.lambda.params.iter().zip(future.args.iter()) {
                        saved.push((param.clone(), env.get_var(param).cloned()));
                        env.set_var_unchecked(param, arg.clone());
                    }

                    let out = eval_expr(&future.lambda.body, env);

                    for (name, old) in saved.into_iter().rev() {
                        match old {
                            Some(v) => env.set_var_unchecked(&name, v),
                            None => env.del_var(&name),
                        }
                    }
                    restore_captured(restore_caps, env);
                    env.set_input(saved_pipe);
                    out
                }
                // If it's not a future, just return the value (auto-await semantics)
                other => Ok(other),
            }
        }

        // ---------- try/catch ----------
        Expr::TryCatch {
            try_expr,
            catch_var,
            catch_expr,
        } => {
            // Try to evaluate the try expression
            match eval_expr(try_expr, env) {
                Ok(Value::Error(msg)) => {
                    // An error value was returned (from throw)
                    bind_caught(catch_var.as_deref(), Value::Str(msg), catch_expr, env)
                }
                Ok(val) => {
                    // Success - return the value
                    Ok(val)
                }
                Err(e) => {
                    // Runtime error - catch it. Safety refusals carry structured
                    // data, and the catch variable is bound to their record (see
                    // `caught_error_record`). Other errors stay as strings. A
                    // failed lambda call that is caught is not the statement's
                    // failure, so its note is dropped.
                    clear_failed_lambda();
                    let caught = match e.downcast_ref::<crate::safety::SafetyError>() {
                        Some(se) => Value::from_json(&caught_error_record(&se.to_json())),
                        None => Value::Str(e.to_string()),
                    };
                    bind_caught(catch_var.as_deref(), caught, catch_expr, env)
                }
            }
        }

        // ---------- throw ----------
        Expr::Throw(inner) => {
            let val = eval_expr(inner, env)?;
            let msg = match val {
                Value::Str(s) => s,
                other => format!("{:?}", other),
            };
            Ok(Value::Error(msg))
        }

        // ---------- call ----------
        Expr::Call {
            callee,
            args,
            named: _,
        } => {
            // The pipe input belongs to this call, not to calls nested in its
            // arguments. Evaluating the arguments with it still set handed it
            // to every inner call as well: `"hello" | slice(1, len("hello"))`
            // ran `len` with "hello" piped in *and* as its argument, and was
            // refused. A `.field` argument is already a lambda over the row, so
            // nothing in an argument needs the input.
            let pin = env.take_input();
            let mut vals = Vec::with_capacity(args.len());
            for a in args {
                match eval_expr(a, env) {
                    Ok(v) => vals.push(v),
                    Err(e) => {
                        env.set_input(pin);
                        return Err(e);
                    }
                }
            }
            env.set_input(pin.clone());

            // 1) If callee is a plain identifier, prefer builtin (unless a var is bound)
            if let Expr::Ident(name) = &**callee {
                if let Some(v) = env.get_var(name).cloned() {
                    // If the bound var is a Lambda or AsyncLambda, call it. Otherwise fall back
                    // to builtin behavior (e.g. when shadowing with a non-function).
                    match v {
                        Value::Lambda(_) | Value::AsyncLambda(_) | Value::Builtin(_) => {
                            return call_value_named(Some(name), v, pin, vals, env)
                        }
                        // A variable that is not a function and whose name is
                        // no builtin either: `let f = 5; f(1)` said "unknown
                        // builtin: f", which sends the reader looking for a
                        // missing builtin rather than at their variable.
                        other if !builtins::is_dispatched(name) => {
                            return Err(crate::safety::bad_arg(
                                &format!("{name}(...)"),
                                &format!("a function in `{name}`"),
                                other.type_name(),
                            ));
                        }
                        _ => {
                            return builtins::call_with_input(name, vals, pin, env);
                        }
                    }
                } else {
                    // Not a bound var → treat as builtin name
                    return builtins::call_with_input(name, vals, pin, env);
                }
            }

            // 2) Otherwise evaluate callee and dispatch
            if let Expr::MemberAccess { object, field } = &**callee {
                if let Some(e) = method_on_non_record(object, field, &vals, env) {
                    return Err(e);
                }
            }
            let f = eval_expr(callee, env)?;
            call_value_with_pipe(f, pin, vals, env)
        }

        // ---------- pipeline ----------
        Expr::Pipe { left, right } => {
            // Every stage's name is checked before the first one runs.
            check_pipeline_names(expr, env)?;
            let left_val = eval_expr(left, env)?;

            // `|.a.b` is parsed as `| map(fn(__) => __.a.b)`. A Record is one
            // row, so the stage reads the field from it; an Array keeps the map
            // path below. Anything else is named as the projection, not as map.
            if let Some((lambda, path)) = projection_stage(right) {
                if env.get_var("map").is_none() && !matches!(left_val, Value::Array(_)) {
                    if let Value::Record(_) = left_val {
                        let f = eval_expr(lambda, env)?;
                        return call_value_with_pipe(f, None, vec![left_val], env);
                    }
                    return Err(crate::safety::bad_arg_with_hint(
                        &path,
                        "an Array or a Record as the piped subject",
                        left_val.type_name(),
                        "a `|.field` stage reads a field of each row of an Array, or of one Record",
                    ));
                }
            }

            // If RHS is an identifier: prefer calling a bound lambda with the
            // entire left value as an explicit arg; if not bound, treat as
            // builtin and pass left as pipe input.
            if let Expr::Ident(name) = &**right {
                if let Some(v) = env.get_var(name).cloned() {
                    match v {
                        Value::Lambda(_) | Value::AsyncLambda(_) => {
                            // pass as explicit arg so the user lambda receives the whole array
                            return call_value_with_pipe(v, None, vec![left_val], env);
                        }
                        _ => {
                            return crate::builtins::call_with_input(
                                name,
                                Vec::new(),
                                Some(left_val),
                                env,
                            );
                        }
                    }
                } else {
                    return crate::builtins::call_with_input(name, Vec::new(), Some(left_val), env);
                }
            }

            // If RHS is a lambda literal, evaluate it and call with pipe input
            // — this is the shorthand mapping form: `[arr] | fn(x)=> ...`.
            if let Expr::Lambda { .. } = &**right {
                let f = eval_expr(right, env)?;
                return call_value_with_pipe(f, Some(left_val), Vec::new(), env);
            }

            // Otherwise set pipe input and evaluate right normally (this allows
            // call expressions to pick up env.input()). Restore afterwards.
            let saved = env.take_input();
            env.set_input(Some(left_val));
            let res = eval_expr(right, env);
            // Restore
            match saved {
                Some(v) => env.set_input(Some(v)),
                None => env.set_input(None),
            }
            res
        }

        // ---------- unary ----------
        Expr::Unary { op, expr } => {
            let v = eval_expr(expr, env)?;
            match (op, v) {
                // `-i64::MIN` is the one negation that does not fit.
                (UnOp::Neg, Value::Int(n)) => Ok(Value::Int(
                    n.checked_neg()
                        .ok_or_else(|| crate::safety::integer_overflow("Neg"))?,
                )),
                (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
                (UnOp::Not, v) => Ok(Value::Bool(!is_truthy(&v))),
                (_, other) => Err(crate::safety::bad_arg(
                    "unary -",
                    "an Int or Float",
                    other.type_name(),
                )),
            }
        }

        // ---------- binary ----------
        Expr::Binary { left, op, right } => {
            let a = eval_expr(left, env)?;
            // `&&` and `||` short-circuit, as in every shell and C-family
            // language: `x != "" && x >= 1` must not compare "" with 1.
            // Their operands must be Bool, as `if` and `where` require: `1 && 2`
            // used to be true, and `"false" && true` used to be true too.
            if matches!(op, BinOp::And | BinOp::Or) {
                let x = match a {
                    Value::Bool(x) => x,
                    other => return Err(logic_operand_error(op, &other)),
                };
                match op {
                    BinOp::And if !x => return Ok(Value::Bool(false)),
                    BinOp::Or if x => return Ok(Value::Bool(true)),
                    _ => {}
                }
                let b = eval_expr(right, env)?;
                return match b {
                    Value::Bool(y) => Ok(Value::Bool(y)),
                    other => Err(logic_operand_error(op, &other)),
                };
            }
            let b = eval_expr(right, env)?;
            binop(op, a, b)
        }

        // ---------- member access: record.field ----------
        Expr::MemberAccess { object, field } => {
            // Borrow a chain rooted in a variable. Cloning the object here
            // copied an entire module (or a row's unused payload) to read one
            // field. Only the selected value needs to become owned.
            if let Some(value) = borrowed_member(expr, env) {
                return value.cloned();
            }
            if let Some(e) = unbound_receiver_error(object, field, env) {
                return Err(e);
            }
            let obj = eval_expr(object, env)?;
            match obj {
                // A missing field is how a misspelled *module function* presents
                // (`file.raed` — modules are records in the env), so suggest from
                // the record's own keys rather than dead-ending on prose.
                Value::Record(mut map) => map
                    .remove(field)
                    .ok_or_else(|| module_field_error(field, &map, object)),
                // Uncoded (E_UNKNOWN), though it is the commonest type error
                // an agent makes: reading a field of a string or null.
                // Named after the expression when it is a plain name, so
                // `nope.read(...)` says `nope` -- the unbound name is the
                // commonest cause.
                other => Err(crate::safety::bad_arg(
                    &match object.as_ref() {
                        // `__` is the implicit row of `.field`; it is not written.
                        Expr::Ident(name) if name != "__" => format!("{name}.{field}"),
                        _ => format!(".{field}"),
                    },
                    "a Record (or a module)",
                    other.type_name(),
                )),
            }
        }

        // ---------- postfix index: expr[index] ----------
        Expr::Index { object, index } => {
            let idx = eval_expr(index, env)?;
            // Borrow a variable or field chain, as member access does, so that
            // reading one element does not copy the whole array.
            if let Some(target) = borrowed_member(object, env) {
                return index_value(target?, &idx, object);
            }
            let obj = eval_expr(object, env)?;
            index_value(&obj, &idx, object)
        }

        // ---------- pattern matching ----------
        Expr::Match { scrutinee, arms } => {
            let value = eval_expr(scrutinee, env)?;

            // `if` desugars to this two-arm shape (parse_if_expr). Its condition
            // must be a Bool; Null is false. Any other value used to fall through
            // to the else branch silently.
            if is_if_desugar(arms) && !matches!(value, Value::Bool(_) | Value::Null) {
                return Err(crate::safety::bad_arg_with_hint(
                    "if",
                    "a Bool condition",
                    value.type_name(),
                    if_condition_hint(&value),
                ));
            }

            for arm in arms {
                // Try to match the pattern
                if let Some(bindings) = match_pattern(&arm.pattern, &value) {
                    // Check guard if present
                    if let Some(guard_expr) = &arm.guard {
                        // Create temporary environment with pattern bindings
                        let mut temp_env = env.clone();
                        for (name, val) in bindings.iter() {
                            temp_env.set_var_unchecked(name, val.clone());
                        }

                        let guard_result = eval_expr(guard_expr, &mut temp_env)?;
                        if !is_truthy(&guard_result) {
                            continue; // Guard failed, try next arm
                        }
                    }

                    // Pattern matched (and guard passed if present), bind variables and evaluate body
                    for (name, val) in bindings {
                        env.set_var_unchecked(&name, val);
                    }
                    return eval_expr(&arm.body, env);
                }
            }

            Err(crate::safety::bad_arg_in(
                "match",
                format!("match: no arm matches {}", short_value(&value)),
                "add `_ => ...` as the last arm to catch the rest",
            ))
        }
    }
}

/// A value for an error message: its text, quoted when it is a string, on one
/// line and at most 40 characters.
fn short_value(value: &Value) -> String {
    let text = match value {
        Value::Str(s) => format!("\"{s}\""),
        other => other.to_display_string(),
    };
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 40 {
        flat
    } else {
        format!("{}...", flat.chars().take(37).collect::<String>())
    }
}

/// The unknown-module error for `name.member` when `name` is an unbound
/// identifier that is neither a module nor a builtin. Without it the access
/// reached the record check with Null and was refused as "expected a Record".
fn unbound_receiver_error(object: &Expr, field: &str, env: &Env) -> Option<anyhow::Error> {
    #[cfg(feature = "native")]
    {
        let Expr::Ident(name) = object else {
            return None;
        };
        if env.get_var(name).is_some()
            || crate::modules::is_module_name(name)
            || builtins::is_dispatched(name)
        {
            return None;
        }
        // A library's spelling (`re.findall`) names the builtin it means; the
        // nearest module names are only a guess for a misspelt module.
        let mapped = builtins::mapped_module_spelling(name, field);
        let candidates = if mapped.is_empty() {
            builtins::nearest_names(name, crate::modules::all_modules().iter().map(|(m, _)| *m))
        } else {
            mapped
        };
        Some(crate::safety::unknown_module(name, candidates))
    }
    #[cfg(not(feature = "native"))]
    {
        let _ = (object, field, env);
        None
    }
}

/// The error for a field a record lacks. When the record is a module imported
/// as `import "..." as name`, the hint says why the name is missing: the module
/// defines it without `pub`, or it exports nothing.
fn module_field_error(
    field: &str,
    map: &std::collections::BTreeMap<String, Value>,
    object: &Expr,
) -> anyhow::Error {
    // `str.pad_right` on the `str` module: the member is a builtin that does
    // not exist, named as such, with the module's own members as candidates.
    if let Expr::Ident(name) = object {
        if crate::modules::is_module_record(name, map) {
            let members = crate::modules::module_member_names(name);
            let candidates = builtins::module_member_candidates(name, field, &members);
            // `str.slice` names a builtin that is top-level, not a member: say
            // how to call it, since "did you mean: slice" does not.
            if builtins::is_dispatched(field) {
                let specific = format!(
                    "`{field}` is a top-level builtin, not a member of {name}: call {field}(s, ...) or s | {field}(...)"
                );
                return crate::safety::unknown_module_member_hinted(
                    name,
                    field,
                    candidates,
                    Some(specific),
                );
            }
            return crate::safety::unknown_module_member(name, field, candidates);
        }
    }
    let candidates = crate::builtins::nearest_names(field, map.keys().map(|k| k.as_str()));
    let specific = crate::module_loader::unexported_hint(field)
        .or_else(|| match object {
            Expr::Ident(name) => crate::module_loader::empty_module_hint(name),
            _ => None,
        })
        .or_else(|| {
            candidates
                .is_empty()
                .then(|| crate::safety::record_keys_hint(map.keys().map(|k| k.as_str())))
        });
    crate::safety::unknown_field_hinted(field, candidates, specific)
}

/// The error for a bare name that nothing binds. Its candidates are the nearest
/// bound variables and builtins, so a misspelt variable names its own fix.
fn unknown_name_error(name: &str, env: &Env) -> anyhow::Error {
    // The same suggestions as a misspelt builtin (`date` -> `now_iso` first),
    // with the bound variables added to the candidates.
    let candidates = builtins::did_you_mean_among(name, env.vars().keys().map(|k| k.as_str()));
    crate::safety::unknown_name(name, candidates)
}

/// Sets the field at `path` inside `target`, which is the Record `label` names.
/// Every field but the last must already hold a Record; the last is created
/// when missing. A refusal names the path up to the field that failed.
fn set_field_path(target: &mut Value, label: &str, path: &[String], new: Value) -> Result<()> {
    let Some((field, rest)) = path.split_first() else {
        *target = new;
        return Ok(());
    };
    let kind = target.type_name();
    let Value::Record(map) = target else {
        // `r.a.b = 5` with `r.a` an Int: the fix is to replace `r.a`, which
        // the hint shows as the record literal the path would need.
        let article = if matches!(kind.chars().next(), Some('A' | 'E' | 'I' | 'O' | 'U')) {
            "an"
        } else {
            "a"
        };
        let literal = path.iter().rev().fold(value_literal(&new), |inner, field| {
            format!("{{{field}: {inner}}}")
        });
        return Err(crate::safety::bad_arg_with_hint(
            label,
            "a Record",
            kind,
            &format!("{label} is {article} {kind}; assign {label} = {literal} to replace it"),
        ));
    };
    if rest.is_empty() {
        map.insert(field.clone(), new);
        return Ok(());
    }
    let child_label = format!("{label}.{field}");
    match map.get_mut(field) {
        Some(child) => set_field_path(child, &child_label, rest, new),
        None => {
            // `r.a.b = 1` with no `a` in `r`: the record for `a` has to be
            // created first, so the hint shows it.
            let literal = rest
                .iter()
                .rev()
                .fold(value_literal(&new), |inner, f| format!("{{{f}: {inner}}}"));
            Err(crate::safety::bad_arg_in(
                label,
                format!("{label} has no field {field} to assign into"),
                &format!("create it first: {child_label} = {literal}"),
            ))
        }
    }
}

/// A value as the record literal a path would need, for a hint: a number or
/// bool as it is, a string quoted, anything else as `...`.
fn value_literal(value: &Value) -> String {
    match value {
        Value::Int(_) | Value::Float(_) | Value::Bool(_) => value.to_display_string(),
        Value::Str(s) => format!("\"{s}\""),
        _ => "...".to_string(),
    }
}

/// Resolve only identifiers and field chains, which cannot mutate the env.
/// Other expressions keep the normal evaluator path and its evaluation order.
fn borrowed_member<'a>(expr: &Expr, env: &'a Env) -> Option<Result<&'a Value>> {
    match expr {
        Expr::Ident(name) => {
            Some(
                crate::safety::check_deadline().and_then(|()| match env.get_var(name) {
                    Some(value) => Ok(value),
                    None if builtins::is_dispatched(name) => Ok(&Value::Null),
                    None => Err(unknown_name_error(name, env)),
                }),
            )
        }
        Expr::MemberAccess { object, field } => {
            if let Some(e) = unbound_receiver_error(object, field, env) {
                return Some(Err(e));
            }
            let value = borrowed_member(object, env)?;
            Some(value.and_then(|value| {
                crate::safety::check_deadline()?;
                match value {
                    Value::Record(map) => map
                        .get(field)
                        .ok_or_else(|| module_field_error(field, map, object)),
                    other => Err(crate::safety::bad_arg(
                        &match object.as_ref() {
                            // `__` is the implicit row of `.field`; it is not written.
                            Expr::Ident(name) if name != "__" => format!("{name}.{field}"),
                            _ => format!(".{field}"),
                        },
                        "a Record (or a module)",
                        other.type_name(),
                    )),
                }
            }))
        }
        _ => None,
    }
}

/// `receiver.name(args)` where the receiver is a value that is not a Record and
/// `name` is a builtin. Such values have no methods; the hint names the call
/// that works, with the receiver's own name and the arguments as written
/// (`call contains(l, "bet") or l | contains("bet")`). `None` for anything
/// else, which keeps the ordinary member-access error.
fn method_on_non_record(
    object: &Expr,
    field: &str,
    args: &[Value],
    env: &Env,
) -> Option<anyhow::Error> {
    if !builtins::is_dispatched(field) {
        return None;
    }
    // `time.now()`: `time` is a builtin, not a module, so it has no members. An
    // unbound builtin name reads as Null, and the generic error did not say why.
    if let Expr::Ident(name) = object {
        if name != "__" && env.get_var(name).is_none() && builtins::is_dispatched(name) {
            return Some(crate::safety::bad_arg_with_hint(
                &format!("{name}.{field}"),
                "a Record (or a module)",
                "Null",
                &format!(
                    "`{name}` is a builtin, not a module: call {name}() (or {field}(), the builtin itself)"
                ),
            ));
        }
    }
    let receiver = match borrowed_member(object, env)? {
        Ok(Value::Record(_)) | Ok(Value::Null) | Err(_) => return None,
        Ok(value) => value,
    };
    // `__` is the implicit row of `.field`: it is named as the value it is.
    let recv = match object {
        Expr::Ident(name) if name != "__" => name.as_str(),
        _ => "value",
    };
    let ty = receiver.type_name();
    let article = if matches!(ty.chars().next(), Some('A' | 'E' | 'I' | 'O' | 'U')) {
        "an"
    } else {
        "a"
    };
    let rest: Vec<String> = args.iter().map(method_arg_text).collect();
    let with_recv = std::iter::once(recv.to_string())
        .chain(rest.iter().cloned())
        .collect::<Vec<_>>()
        .join(", ");
    let hint = format!(
        "{article} {ty} has no methods; call {field}({with_recv}) or {recv} | {field}({})",
        rest.join(", ")
    );
    Some(crate::safety::bad_arg_with_hint(
        &if matches!(object, Expr::Ident(n) if n == "__") {
            format!(".{field}")
        } else {
            format!("{recv}.{field}")
        },
        "a Record (or a module)",
        ty,
        &hint,
    ))
}

/// Refuse a pipeline whose stage names a plain builtin that nothing serves,
/// before any stage runs. Otherwise `file.write("p", "a") | nosuchfn()` wrote
/// `p` and only then failed. `pipe` is the outermost `Expr::Pipe`; its left
/// spine is walked too. A stage is checked only when it is a plain name: a
/// bound variable or function in scope is left to the evaluator.
fn check_pipeline_names(pipe: &Expr, env: &Env) -> Result<()> {
    let mut cur = pipe;
    while let Expr::Pipe { left, right } = cur {
        if let Some(name) = plain_stage_name(right) {
            if env.get_var(name).is_none() && !crate::builtins::is_served(name) {
                return Err(crate::safety::unknown_builtin_hinted(
                    name,
                    crate::builtins::did_you_mean(name),
                    crate::module_loader::unexported_hint(name),
                ));
            }
        }
        cur = &**left;
    }
    Ok(())
}

/// The name a pipeline stage calls when it is `name(...)` or a bare `name`.
fn plain_stage_name(stage: &Expr) -> Option<&str> {
    match stage {
        Expr::Call { callee, .. } => match &**callee {
            Expr::Ident(name) => Some(name.as_str()),
            _ => None,
        },
        Expr::Ident(name) => Some(name.as_str()),
        _ => None,
    }
}

/// The row lambda and the `.a.b` path of a `|.a.b` stage. The parser writes
/// that stage as `map(fn(__) => __.a.b)`, so the shape identifies it.
fn projection_stage(right: &Expr) -> Option<(&Expr, String)> {
    let Expr::Call {
        callee,
        args,
        named,
    } = right
    else {
        return None;
    };
    let (Expr::Ident(name), [lambda], true) = (callee.as_ref(), args.as_slice(), named.is_empty())
    else {
        return None;
    };
    let Expr::Lambda { params, body } = lambda else {
        return None;
    };
    let mut fields: Vec<&str> = Vec::new();
    let mut root = body.as_ref();
    while let Expr::MemberAccess { object, field } = root {
        fields.push(field.as_str());
        root = object.as_ref();
    }
    let rooted = !fields.is_empty() && matches!(root, Expr::Ident(n) if n == "__");
    if name != "map" || params.len() != 1 || params[0] != "__" || !rooted {
        return None;
    }
    fields.reverse();
    Some((lambda, format!(".{}", fields.join("."))))
}

/// A method argument as the script wrote it, for a hint: a String quoted.
fn method_arg_text(v: &Value) -> String {
    match v {
        Value::Str(s) => format!("{s:?}"),
        other => other.to_display_string(),
    }
}

/// `object[index]` on an evaluated value. Arrays and Strings take an Int (a
/// negative index counts from the end, and a String yields one character),
/// Records take a String key. Anything else, and any other index type, is an
/// `E_BAD_ARG` that names the object.
fn index_value(target: &Value, idx: &Value, object: &Expr) -> Result<Value> {
    let label = match object {
        Expr::Ident(name) => format!("{name}[...]"),
        _ => "[...]".to_string(),
    };
    match (target, idx) {
        (Value::Array(items), Value::Int(i)) => {
            let from_args = matches!(
                object,
                Expr::Call { callee, .. }
                    if matches!(callee.as_ref(), Expr::Ident(name) if name == "args")
            );
            let pos = resolve_index(*i, items.len(), "Array", from_args)?;
            Ok(items[pos].clone())
        }
        (Value::Str(s), Value::Int(i)) => {
            let chars: Vec<char> = s.chars().collect();
            let pos = resolve_index(*i, chars.len(), "String", false)?;
            Ok(Value::Str(chars[pos].to_string()))
        }
        (Value::Record(map), Value::Str(key)) => map.get(key).cloned().ok_or_else(|| {
            let candidates = crate::builtins::nearest_names(key, map.keys().map(|k| k.as_str()));
            let specific = candidates
                .is_empty()
                .then(|| crate::safety::record_keys_hint(map.keys().map(|k| k.as_str())));
            crate::safety::unknown_field_hinted(key, candidates, specific)
        }),
        (Value::Array(_) | Value::Str(_), other) => Err(crate::safety::bad_arg(
            &label,
            "an Int index",
            other.type_name(),
        )),
        (Value::Record(_), other) => Err(crate::safety::bad_arg(
            &label,
            "a String key",
            other.type_name(),
        )),
        (other, _) => Err(crate::safety::bad_arg(
            &label,
            "an Array, String or Record to index",
            other.type_name(),
        )),
    }
}

/// Turn a possibly negative index into a position in `0..len`, or an
/// out-of-range error that states the length.
fn resolve_index(index: i64, len: usize, kind: &str, from_args: bool) -> Result<usize> {
    let n = len as i64;
    let pos = if index < 0 { n + index } else { index };
    if pos < 0 || pos >= n {
        return Err(crate::safety::index_out_of_range(
            kind, index, len, from_args,
        ));
    }
    Ok(pos as usize)
}

/* ---------------- pattern matching helpers ---------------- */

use crate::ast::Pattern;
use std::collections::HashMap;

/// True for the two-arm shape `parse_if_expr` builds: `true => then, _ => else`,
/// with no guards. A user `match` written the same way is checked too.
fn is_if_desugar(arms: &[crate::ast::MatchArm]) -> bool {
    match arms {
        [first, second] => {
            first.guard.is_none()
                && second.guard.is_none()
                && matches!(first.pattern, Pattern::LitBool(true))
                && matches!(second.pattern, Pattern::Wildcard)
        }
        _ => false,
    }
}

/// How to turn a non-Bool `if` condition into a Bool, named per type.
fn if_condition_hint(value: &Value) -> &'static str {
    match value {
        Value::Record(_) => "test a field of the record, e.g. exists(p).exists",
        Value::Int(_) | Value::Float(_) => "compare it, e.g. n != 0",
        Value::Str(_) | Value::Uri(_) => "compare it, e.g. s != \"\"",
        Value::Array(_) | Value::Table(_) => "test the length, e.g. len(a) > 0",
        _ => "compare it to a value, e.g. x == \"ok\"",
    }
}

/// Attempt to match a pattern against a value.
/// Returns Some(bindings) if successful, None if no match.
fn match_pattern(pattern: &Pattern, value: &Value) -> Option<HashMap<String, Value>> {
    let mut bindings = HashMap::new();
    if match_pattern_impl(pattern, value, &mut bindings) {
        Some(bindings)
    } else {
        None
    }
}

fn match_pattern_impl(
    pattern: &Pattern,
    value: &Value,
    bindings: &mut HashMap<String, Value>,
) -> bool {
    match pattern {
        Pattern::Wildcard => true, // _ matches anything

        Pattern::Ident(name) => {
            // Variable binding - always matches and binds the value
            bindings.insert(name.clone(), value.clone());
            true
        }

        // Numbers match by `==`'s equality, so `1` matches `1.0` and back.
        Pattern::LitInt(n) => value_eq(&Value::Int(*n), value),
        Pattern::LitFloat(x) => value_eq(&Value::Float(*x), value),
        Pattern::LitStr(s) => matches!(value, Value::Str(v) if v == s),
        Pattern::LitBool(b) => matches!(value, Value::Bool(v) if v == b),
        Pattern::Null => matches!(value, Value::Null),

        Pattern::Constructor { name, args } => {
            // Match tagged records like Some(x) or None
            if let Value::Record(map) = value {
                // Check for _tag field
                if let Some(Value::Str(tag)) = map.get("_tag") {
                    if tag == name {
                        // Check arguments
                        if args.is_empty() {
                            // Constructor with no args (like None)
                            return true;
                        } else if args.len() == 1 {
                            // Constructor with one arg (like Some(x))
                            if let Some(inner) = map.get("_value") {
                                return match_pattern_impl(&args[0], inner, bindings);
                            }
                        }
                    }
                }
            }
            false
        }

        Pattern::Array(patterns) => {
            if let Value::Array(values) = value {
                if patterns.len() != values.len() {
                    return false;
                }
                for (pat, val) in patterns.iter().zip(values.iter()) {
                    if !match_pattern_impl(pat, val, bindings) {
                        return false;
                    }
                }
                true
            } else {
                false
            }
        }

        Pattern::Record(field_patterns) => {
            if let Value::Record(map) = value {
                for (field_name, field_pattern) in field_patterns {
                    if let Some(field_value) = map.get(field_name) {
                        if !match_pattern_impl(field_pattern, field_value, bindings) {
                            return false;
                        }
                    } else {
                        return false; // Required field missing
                    }
                }
                true
            } else {
                false
            }
        }
    }
}

/* ---------------- dispatch helpers ---------------- */

/// Dispatch calling a Value `f` with optional pipe input and explicit args.
///
/// Rules:
/// - Lambda:
///     - if pipe input Some(Array) and arity==1 -> map over it
///     - if pipe input Some(v) and arity==1 and no explicit args -> call with v
///     - if arity matches explicit args (plus maybe pipe) -> call
/// - String/Uri → builtin with (pipe_input ++ args)
/// - Null → error
/// - Other → error
pub(crate) fn call_value_with_pipe(
    f: Value,
    pin: Option<Value>,
    args: Vec<Value>,
    env: &mut Env,
) -> Result<Value> {
    call_value_named(None, f, pin, args, env)
}

/// [`call_value_with_pipe`], naming the variable the value was called through,
/// so an arity error can say `f takes 2 arguments` rather than `lambda takes`.
pub(crate) fn call_value_named(
    name: Option<&str>,
    f: Value,
    pin: Option<Value>,
    args: Vec<Value>,
    env: &mut Env,
) -> Result<Value> {
    let is_lambda = matches!(f, Value::Lambda(_));
    let result = call_value_named_inner(name, f, pin, args, env);
    if let (true, Err(_), Some(name)) = (is_lambda, &result, name) {
        note_failed_lambda_call(name);
    }
    result
}

fn call_value_named_inner(
    name: Option<&str>,
    f: Value,
    pin: Option<Value>,
    mut args: Vec<Value>,
    env: &mut Env,
) -> Result<Value> {
    match f {
        Value::Lambda(l) => {
            match (pin, l.params.len(), args.len()) {
                // Zero-arg lambda: no params, no input required, no explicit args
                (_, 0, 0) => call_lambda0(&l, env),
                // Map: pin is array, lambda 1-ary, and no explicit args
                (Some(Value::Array(arr)), 1, 0) => {
                    let mut out = Vec::with_capacity(arr.len());
                    for (i, x) in arr.into_iter().enumerate() {
                        out.push(call_lambda1(&l, x, i, env)?);
                    }
                    Ok(Value::Array(out))
                }
                // Single-arg: use pipe input if provided and no explicit arg given
                (Some(v), 1, 0) => call_lambda1(&l, v, 0, env),
                // Two-arg: if we have two explicit args, call directly
                (_, 2, 2) if args.len() == 2 => {
                    // SECURITY: Replace .unwrap() with proper error handling (CVSS 7.1)
                    let b = args.pop().ok_or_else(|| {
                        crate::safety::bad_arg("lambda call", "an argument", "nothing")
                    })?;
                    let a = args.pop().ok_or_else(|| {
                        crate::safety::bad_arg("lambda call", "an argument", "nothing")
                    })?;
                    call_lambda2(&l, a, b, 0, env)
                }
                // One-arg: if we have one explicit arg, call directly
                (_, 1, 1) => {
                    // SECURITY: Replace .unwrap() with proper error handling (CVSS 7.1)
                    let arg = args.pop().ok_or_else(|| {
                        crate::safety::bad_arg("lambda call", "an argument", "nothing")
                    })?;
                    call_lambda1(&l, arg, 0, env)
                }
                // N-arg (3+): if we have exact arity match, call with all args
                (_, n, m) if n >= 3 && n == m => call_lambda_n(&l, args, env),
                (pin, _, _) => Err(crate::safety::lambda_arity(
                    name,
                    &l.params,
                    args.len() + usize::from(pin.is_some()),
                )),
            }
        }
        // Async lambda: create a Future instead of executing immediately
        Value::AsyncLambda(al) => {
            // Prepend pipe input if present
            let all_args = if let Some(p) = pin {
                let mut all = Vec::with_capacity(1 + args.len());
                all.push(p);
                all.extend(args);
                all
            } else {
                args
            };
            Ok(Value::Future(Future {
                lambda: al,
                args: all_args,
            }))
        }
        // Builtin reference from module: call the builtin by name
        Value::Builtin(b) => builtins::call_with_input(&b.name, args, pin, env),
        Value::Str(name) | Value::Uri(name) => {
            if let Some(p) = pin {
                let mut all = Vec::with_capacity(1 + args.len());
                all.push(p);
                all.extend(args);
                builtins::call(&name, all, env)
            } else {
                builtins::call(&name, args, env)
            }
        }
        Value::Null => Err(crate::safety::bad_arg("call", "a function", "Null")),
        other => Err(crate::safety::bad_arg(
            "call",
            "a function",
            other.type_name(),
        )),
    }
}

/// Is this operand an exact numeric zero, in either numeric representation?
fn is_zero_divisor(v: &Value) -> bool {
    match v {
        Value::Int(0) => true,
        Value::Float(f) => *f == 0.0, // matches both +0.0 and -0.0
        _ => false,
    }
}

/// The refusal both `/` and `%` give for a zero divisor.
fn divide_by_zero(what: &str, operator: &str) -> anyhow::Error {
    // Not retryable: the divisor is data, and re-running with the same values
    // divides by the same zero. The hint says what to change.
    crate::safety::bad_value_in(
        operator,
        format!(
            "{what} by zero: the divisor evaluated to 0. Guard the divisor, or \
             filter the empty case before dividing"
        ),
        "check the divisor is not 0 before dividing, or filter zero divisors out first; \
         retrying with the same values fails again",
    )
}

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Int(n) => *n != 0,
        Value::Float(f) => *f != 0.0,
        Value::Str(s) => !s.is_empty(),
        Value::Uri(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Record(m) => !m.is_empty(),
        Value::Table(t) => !t.rows.is_empty(),
        Value::Lambda(_) => true,
        Value::AsyncLambda(_) => true,
        Value::Future(_) => true,
        Value::Error(_) => false,  // Errors are falsy
        Value::Builtin(_) => true, // Builtins are truthy
    }
}

/// The largest string a single operation may produce.
///
/// Applied to *every* string-producing operator, not just repetition. It was
/// briefly only on `*`, which made it a speed bump rather than a bound:
/// `"x" * 8_000_000` was allowed and `a + a` then produced 16 MB, walking
/// straight past the number the constant appeared to promise (AS-2026-10).
///
/// This bounds any single string value. It does not bound *total* memory — a
/// script can still hold many strings, or a large array — so it is a guard
/// against one value running away, not an allocation budget.
const MAX_STRING_BYTES: usize = 8 * 1024 * 1024;

/// Build a string value, refusing one that exceeds [`MAX_STRING_BYTES`].
fn checked_string(s: String) -> Result<Value> {
    if s.len() > MAX_STRING_BYTES {
        return Err(crate::safety::budget_exceeded(
            "string",
            format!(
                "string operation would produce {} bytes, over the {} byte limit",
                s.len(),
                MAX_STRING_BYTES
            ),
            "build the text in smaller pieces, or write it to a file",
        ));
    }
    Ok(Value::Str(s))
}

/// Bind the caught value to the `catch` variable, evaluate the handler, and put
/// the previous binding back.
///
/// This used to be `let _ = env.set_var(name, caught)`, which swallowed the
/// failure. `set_var` refuses to overwrite an immutable binding, so whenever a
/// variable of the same name already existed the handler silently saw the *old*
/// value instead of the error:
///
/// ```text
/// let e = "outer"
/// try { throw "boom" } catch e { e }     # -> "outer"
/// ```
///
/// An error handler reading a stale value is worse than one that fails loudly.
/// The catch variable is a binding the construct introduces, exactly like a
/// lambda parameter, so it is installed unconditionally and restored after.
fn bind_caught(
    catch_var: Option<&str>,
    caught: Value,
    catch_expr: &Expr,
    env: &mut Env,
) -> Result<Value> {
    let Some(name) = catch_var else {
        return eval_expr(catch_expr, env);
    };
    let previous = env.get_var(name).cloned();
    env.set_var_unchecked(name, caught);
    let out = eval_expr(catch_expr, env);
    match previous {
        Some(v) => env.set_var_unchecked(name, v),
        None => env.del_var(name),
    }
    out
}

/// Collect the free identifiers of an expression — the names it reads that it
/// does not itself bind.
///
/// This is what a lambda captures. It must cover every `Expr` variant: a
/// variant left out here is a name silently *not* captured, which reintroduces
/// exactly the bug the capture exists to fix, and does so only for the
/// construct that was missed. The `match` is therefore exhaustive with no
/// wildcard arm, so adding an `Expr` variant fails to compile until it is
/// handled here.
fn free_idents(expr: &Expr, bound: &mut Vec<String>, out: &mut std::collections::BTreeSet<String>) {
    match expr {
        Expr::LitInt(_) | Expr::LitFloat(_) | Expr::LitStr(_) | Expr::LitBool(_) | Expr::Null => {}

        Expr::Ident(name) => {
            if !bound.iter().any(|b| b == name) {
                out.insert(name.clone());
            }
        }

        Expr::Array(items) => {
            for e in items {
                free_idents(e, bound, out);
            }
        }
        Expr::Record(fields) => {
            for (_, e) in fields {
                free_idents(e, bound, out);
            }
        }

        // A nested lambda binds its own parameters; anything else it uses is
        // free in *this* one too, which is what makes currying work.
        Expr::Lambda { params, body } | Expr::AsyncLambda { params, body } => {
            let depth = bound.len();
            bound.extend(params.iter().cloned());
            free_idents(body, bound, out);
            bound.truncate(depth);
        }

        Expr::Await(inner) | Expr::Throw(inner) => free_idents(inner, bound, out),

        Expr::TryCatch {
            try_expr,
            catch_var,
            catch_expr,
        } => {
            free_idents(try_expr, bound, out);
            let depth = bound.len();
            if let Some(v) = catch_var {
                bound.push(v.clone());
            }
            free_idents(catch_expr, bound, out);
            bound.truncate(depth);
        }

        Expr::Call {
            callee,
            args,
            named,
        } => {
            free_idents(callee, bound, out);
            for e in args {
                free_idents(e, bound, out);
            }
            for (_, e) in named {
                free_idents(e, bound, out);
            }
        }

        Expr::Pipe { left, right } => {
            free_idents(left, bound, out);
            free_idents(right, bound, out);
        }

        Expr::Binary { left, right, .. } => {
            free_idents(left, bound, out);
            free_idents(right, bound, out);
        }
        Expr::Unary { expr, .. } => free_idents(expr, bound, out),

        // `object.field` reads `object`; the field name is not an identifier
        // in the environment.
        Expr::MemberAccess { object, .. } => free_idents(object, bound, out),

        Expr::Index { object, index } => {
            free_idents(object, bound, out);
            free_idents(index, bound, out);
        }

        Expr::Match { scrutinee, arms } => {
            free_idents(scrutinee, bound, out);
            for arm in arms {
                let depth = bound.len();
                pattern_bindings(&arm.pattern, bound);
                if let Some(g) = &arm.guard {
                    free_idents(g, bound, out);
                }
                free_idents(&arm.body, bound, out);
                bound.truncate(depth);
            }
        }
    }
}

/// Names a pattern introduces, which are bound inside that arm's guard and body.
fn pattern_bindings(p: &Pattern, bound: &mut Vec<String>) {
    match p {
        Pattern::Wildcard
        | Pattern::LitInt(_)
        | Pattern::LitFloat(_)
        | Pattern::LitStr(_)
        | Pattern::LitBool(_)
        | Pattern::Null => {}
        Pattern::Ident(name) => bound.push(name.clone()),
        Pattern::Constructor { args, .. } => {
            for a in args {
                pattern_bindings(a, bound);
            }
        }
        Pattern::Array(items) => {
            for a in items {
                pattern_bindings(a, bound);
            }
        }
        Pattern::Record(fields) => {
            for (name, sub) in fields {
                // `{x}` binds `x`; `{x: p}` binds whatever `p` binds.
                match sub {
                    Pattern::Wildcard => bound.push(name.clone()),
                    other => pattern_bindings(other, bound),
                }
            }
        }
    }
}

/// Snapshot the free variables of a lambda body that are bound right now.
///
/// Names that are not yet defined are deliberately left out: they stay dynamic
/// lookups, so a lambda that refers to a binding introduced later keeps
/// working. Module names are skipped because they are always in scope and
/// copying a whole module record into every lambda would be pure bloat.
fn capture_free_vars(
    params: &[String],
    body: &Expr,
    env: &Env,
) -> std::collections::BTreeMap<String, Value> {
    let mut bound: Vec<String> = params.to_vec();
    let mut free = std::collections::BTreeSet::new();
    free_idents(body, &mut bound, &mut free);

    let mut captured = std::collections::BTreeMap::new();
    for name in free {
        #[cfg(feature = "native")]
        if crate::modules::is_module_name(&name) {
            continue;
        }
        // A `let mut` binding is deliberately *not* captured.
        //
        // Capture is by value, so snapshotting a mutable variable would make a
        // later assignment invisible to the lambda:
        //
        //     let mut k = 1
        //     let f = fn(q) => k
        //     k = 2
        //     f(0)              # would be 1, and used to be 2
        //
        // That is a silent change to a behaviour scripts already rely on, to
        // fix a case (`let mut` closed over and then reassigned) that nobody
        // asked about. Mutable bindings stay dynamic lookups; immutable ones —
        // which is what a curried parameter is — are captured.
        if env.is_declared_mutable(&name) {
            continue;
        }
        if let Some(v) = env.get_var(&name) {
            captured.insert(name, v.clone());
        }
    }
    captured
}

/// Complete the captures of a lambda a module exports, from the module's final
/// bindings.
///
/// A lambda captures, when it is written, only the names that exist then. A name
/// defined later in the module (a private helper, or the other half of a mutual
/// recursion) was not captured, so the export could not reach it once another
/// file called it. Every module-level name the export reaches, directly or
/// through other module-level lambdas, is captured now with its final value. A
/// name already captured keeps its captured value, which is the final one, since
/// an immutable binding cannot change.
pub(crate) fn complete_module_capture(lam: &mut crate::value::Lambda, module: &Env) {
    let mut pending: Vec<String> = lambda_free_names(&lam.params, &lam.body)
        .into_iter()
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        #[cfg(feature = "native")]
        if crate::modules::is_module_name(&name) {
            continue;
        }
        if module.is_declared_mutable(&name) {
            continue;
        }
        let Some(value) = module.get_var(&name) else {
            continue;
        };
        if let Value::Lambda(inner) = value {
            pending.extend(lambda_free_names(&inner.params, &inner.body));
        }
        lam.captured.entry(name).or_insert_with(|| value.clone());
    }
}

/// The names a lambda with these parameters reads from outside itself.
fn lambda_free_names(params: &[String], body: &Expr) -> std::collections::BTreeSet<String> {
    let mut bound: Vec<String> = params.to_vec();
    let mut free = std::collections::BTreeSet::new();
    free_idents(body, &mut bound, &mut free);
    free
}

/// Install a lambda's captured free variables into `env`, returning what was
/// displaced so the caller can put it back.
///
/// Captured values are installed *under* the parameters: a parameter of the
/// same name must win, or a lambda could not shadow a name it also closes over.
pub(crate) fn install_captured(
    captured: &std::collections::BTreeMap<String, Value>,
    env: &mut Env,
) -> Vec<(String, Option<Value>)> {
    let mut saved = Vec::with_capacity(captured.len());
    for (name, value) in captured {
        saved.push((name.clone(), env.get_var(name).cloned()));
        env.set_var_unchecked(name, value.clone());
    }
    saved
}

/// Undo `install_captured`.
pub(crate) fn restore_captured(saved: Vec<(String, Option<Value>)>, env: &mut Env) {
    for (name, old) in saved.into_iter().rev() {
        match old {
            Some(v) => env.set_var_unchecked(&name, v),
            None => env.del_var(&name),
        }
    }
}

/// Call a zero-parameter lambda
fn call_lambda0(l: &Lambda, env: &mut Env) -> Result<Value> {
    let _depth = crate::safety::enter_call()?;
    let restore_caps = install_captured(&l.captured, env);
    // Save and clear pipe input to prevent leakage
    let saved_pipe = env.take_input();
    env.set_input(None);

    let out = eval_expr(&l.body, env);

    // Restore pipe input
    match saved_pipe {
        Some(v) => env.set_input(Some(v)),
        None => env.set_input(None),
    }

    restore_captured(restore_caps, env);

    out
}

fn call_lambda1(l: &Lambda, x: Value, i: usize, env: &mut Env) -> Result<Value> {
    let _depth = crate::safety::enter_call()?;
    let restore_caps = install_captured(&l.captured, env);
    let p = l
        .params
        .first()
        .ok_or_else(|| crate::safety::arg_err("lambda needs 1 param"))?
        .clone();

    // Save and clear pipe input to prevent leakage
    let saved_pipe = env.take_input();
    env.set_input(None);

    // Save prev bindings
    let old_p = env.get_var(&p).cloned();
    env.set_var_unchecked(&p, x);

    // Optional `i`
    let mut old_i: Option<Value> = None;
    if let Some(ip) = l.params.get(1) {
        if ip == "i" {
            old_i = env.get_var("i").cloned();
            env.set_var_unchecked("i", Value::Int(i as i64));
        }
    }

    let out = eval_expr(&l.body, env);

    // Debug: show result of evaluating the lambda body
    // Removed debug eprintln statements

    // Restore
    if let Some(v) = old_i {
        env.set_var_unchecked("i", v);
    } else if l.params.get(1).map(|s| s.as_str()) == Some("i") {
        env.del_var("i");
    }
    if let Some(v) = old_p {
        env.set_var_unchecked(&p, v);
    } else {
        env.del_var(&p);
    }

    // Restore pipe input
    match saved_pipe {
        Some(v) => env.set_input(Some(v)),
        None => env.set_input(None),
    }

    restore_captured(restore_caps, env);

    out
}

/// Call a lambda with N arguments (generic version for 3+ args)
fn call_lambda_n(l: &Lambda, args: Vec<Value>, env: &mut Env) -> Result<Value> {
    let _depth = crate::safety::enter_call()?;
    let restore_caps = install_captured(&l.captured, env);
    if args.len() != l.params.len() {
        return Err(crate::safety::bad_arg(
            "lambda call",
            &format!("{} argument(s)", l.params.len()),
            &format!("{}", args.len()),
        ));
    }

    // Save and clear pipe input to prevent leakage
    let saved_pipe = env.take_input();
    env.set_input(None);

    // Save old bindings and set new ones
    let mut old_bindings: Vec<(String, Option<Value>)> = Vec::with_capacity(l.params.len());
    for (param, arg) in l.params.iter().zip(args) {
        old_bindings.push((param.clone(), env.get_var(param).cloned()));
        env.set_var_unchecked(param, arg);
    }

    let out = eval_expr(&l.body, env);

    // Restore all bindings
    for (param, old_val) in old_bindings.into_iter().rev() {
        if let Some(v) = old_val {
            env.set_var_unchecked(&param, v);
        } else {
            env.del_var(&param);
        }
    }

    // Restore pipe input
    match saved_pipe {
        Some(v) => env.set_input(Some(v)),
        None => env.set_input(None),
    }

    restore_captured(restore_caps, env);

    out
}

fn call_lambda2(l: &Lambda, a: Value, b: Value, i: usize, env: &mut Env) -> Result<Value> {
    let _depth = crate::safety::enter_call()?;
    let restore_caps = install_captured(&l.captured, env);
    let p1 = l
        .params
        .first()
        .ok_or_else(|| crate::safety::arg_err("lambda needs 2 params"))?
        .clone();
    let p2 = l
        .params
        .get(1)
        .ok_or_else(|| crate::safety::arg_err("lambda needs 2 params"))?
        .clone();

    // Save and clear pipe input to prevent leakage
    let saved_pipe = env.take_input();
    env.set_input(None);

    let old_p1 = env.get_var(&p1).cloned();
    env.set_var_unchecked(&p1, a);
    let old_p2 = env.get_var(&p2).cloned();
    env.set_var_unchecked(&p2, b);

    let mut old_i: Option<Value> = None;
    if let Some(ip) = l.params.get(2) {
        if ip == "i" {
            old_i = env.get_var("i").cloned();
            env.set_var_unchecked("i", Value::Int(i as i64));
        }
    }

    let out = eval_expr(&l.body, env);

    if let Some(v) = old_i {
        env.set_var_unchecked("i", v);
    } else if l.params.get(2).map(|s| s.as_str()) == Some("i") {
        env.del_var("i");
    }

    if let Some(v) = old_p2 {
        env.set_var_unchecked(&p2, v);
    } else {
        env.del_var(&p2);
    }
    if let Some(v) = old_p1 {
        env.set_var_unchecked(&p1, v);
    } else {
        env.del_var(&p1);
    }

    // Restore pipe input
    match saved_pipe {
        Some(v) => env.set_input(Some(v)),
        None => env.set_input(None),
    }

    restore_captured(restore_caps, env);

    out
}

/* ---------------- binops & eq ---------------- */

fn value_eq(a: &Value, b: &Value) -> bool {
    use Value::*;
    match (a, b) {
        (Null, Null) => true,
        (Bool(x), Bool(y)) => x == y,
        (Int(x), Int(y)) => x == y,
        (Float(x), Float(y)) => x == y,
        (Int(x), Float(y)) => (*x as f64) == *y,
        (Float(x), Int(y)) => *x == (*y as f64),
        (Str(x), Str(y)) => x == y,
        (Uri(x), Uri(y)) => x == y,
        (Array(ax), Array(ay)) => {
            ax.len() == ay.len() && ax.iter().zip(ay).all(|(x, y)| value_eq(x, y))
        }
        (Record(rx), Record(ry)) => {
            rx.len() == ry.len()
                && rx
                    .iter()
                    .all(|(k, vx)| ry.get(k).is_some_and(|vy| value_eq(vx, vy)))
        }
        _ => false,
    }
}

fn binop(op: &BinOp, a: Value, b: Value) -> Result<Value> {
    use BinOp::*;
    Ok(match (op, a, b) {
        // Int results are checked: a result past 64 bits is an error, not a
        // wrapped number. See `checked_int`.
        (Add, Value::Int(x), Value::Int(y)) => Value::Int(checked_int(x.checked_add(y), "Add")?),
        (Add, Value::Float(x), Value::Float(y)) => Value::Float(x + y),
        (Add, Value::Int(x), Value::Float(y)) => Value::Float((x as f64) + y),
        (Add, Value::Float(x), Value::Int(y)) => Value::Float(x + (y as f64)),
        // String concatenation. A number and a String do not combine: see the
        // refusal arm below, which is what `"a" + 1` now reaches.
        // The left String is owned here, so it is extended rather than copied.
        (Add, Value::Str(mut x), Value::Str(y)) => {
            x.push_str(&y);
            checked_string(x)?
        }
        // Array concatenation: `[1, 2] + [3]` is `[1, 2, 3]`, a new Array.
        (Add, Value::Array(mut x), Value::Array(y)) => {
            x.extend(y);
            Value::Array(x)
        }

        (Sub, Value::Int(x), Value::Int(y)) => Value::Int(checked_int(x.checked_sub(y), "Sub")?),
        (Sub, Value::Float(x), Value::Float(y)) => Value::Float(x - y),
        (Sub, Value::Int(x), Value::Float(y)) => Value::Float((x as f64) - y),
        (Sub, Value::Float(x), Value::Int(y)) => Value::Float(x - (y as f64)),

        (Mul, Value::Int(x), Value::Int(y)) => Value::Int(checked_int(x.checked_mul(y), "Mul")?),
        (Mul, Value::Float(x), Value::Float(y)) => Value::Float(x * y),
        (Mul, Value::Int(x), Value::Float(y)) => Value::Float((x as f64) * y),
        (Mul, Value::Float(x), Value::Int(y)) => Value::Float(x * (y as f64)),

        // Division and remainder refuse a zero divisor rather than producing a
        // value the caller cannot distinguish from an answer.
        //
        // `1 / 0` used to evaluate to `inf` and exit 0, and `0 / 0` to `NaN`
        // and exit 0 -- an agent asked for a mean over an empty selection got
        // a float that flows through every later arithmetic step and lands in
        // a report. `1 % 0` was worse: it panicked the evaluator thread with
        // "attempt to calculate the remainder with a divisor of zero", killing
        // the process with a Rust backtrace and no structured error at all.
        //
        // jq and nushell both raise an error here, so refusing is also the
        // least surprising behaviour for anyone arriving from either.
        (Div, x, y) if is_zero_divisor(&y) && matches!(x, Value::Int(_) | Value::Float(_)) => {
            return Err(divide_by_zero("division", "operator Div"))
        }
        (Rem, x, y) if is_zero_divisor(&y) && matches!(x, Value::Int(_) | Value::Float(_)) => {
            return Err(divide_by_zero("remainder", "operator Rem"))
        }

        (Div, Value::Int(x), Value::Int(y)) => Value::Float((x as f64) / (y as f64)),
        (Div, Value::Float(x), Value::Float(y)) => Value::Float(x / y),
        (Div, Value::Int(x), Value::Float(y)) => Value::Float((x as f64) / y),
        (Div, Value::Float(x), Value::Int(y)) => Value::Float(x / (y as f64)),

        // `i64::MIN % -1` does not fit either, and panics in Rust unless checked.
        (Rem, Value::Int(x), Value::Int(y)) => Value::Int(checked_int(x.checked_rem(y), "Rem")?),

        (Eq, x, y) => Value::Bool(value_eq(&x, &y)),
        (Ne, x, y) => Value::Bool(!value_eq(&x, &y)),

        (Lt, Value::Int(x), Value::Int(y)) => Value::Bool(x < y),
        (Lt, Value::Float(x), Value::Float(y)) => Value::Bool(x < y),
        (Lte, Value::Int(x), Value::Int(y)) => Value::Bool(x <= y),
        (Lte, Value::Float(x), Value::Float(y)) => Value::Bool(x <= y),
        (Gt, Value::Int(x), Value::Int(y)) => Value::Bool(x > y),
        (Gt, Value::Float(x), Value::Float(y)) => Value::Bool(x > y),
        (Gte, Value::Int(x), Value::Int(y)) => Value::Bool(x >= y),
        (Gte, Value::Float(x), Value::Float(y)) => Value::Bool(x >= y),

        // Int and Float order together as f64, so `1.5 < 2` works as
        // `1.5 == 2` already did. `1.5 < 0` was refused before this.
        (Lt, Value::Int(x), Value::Float(y)) => Value::Bool((x as f64) < y),
        (Lt, Value::Float(x), Value::Int(y)) => Value::Bool(x < (y as f64)),
        (Lte, Value::Int(x), Value::Float(y)) => Value::Bool((x as f64) <= y),
        (Lte, Value::Float(x), Value::Int(y)) => Value::Bool(x <= (y as f64)),
        (Gt, Value::Int(x), Value::Float(y)) => Value::Bool((x as f64) > y),
        (Gt, Value::Float(x), Value::Int(y)) => Value::Bool(x > (y as f64)),
        (Gte, Value::Int(x), Value::Float(y)) => Value::Bool((x as f64) >= y),
        (Gte, Value::Float(x), Value::Int(y)) => Value::Bool(x >= (y as f64)),

        // Two Strings order by byte, as Rust's `str` Ord does: "a" < "b", and
        // "B" < "a" because 'B' is 0x42. Mixed String and number pairs are not
        // ordered; the refusal below names the conversions instead.
        (Lt, Value::Str(x), Value::Str(y)) => Value::Bool(x < y),
        (Lte, Value::Str(x), Value::Str(y)) => Value::Bool(x <= y),
        (Gt, Value::Str(x), Value::Str(y)) => Value::Bool(x > y),
        (Gte, Value::Str(x), Value::Str(y)) => Value::Bool(x >= y),

        (And, Value::Bool(x), Value::Bool(y)) => Value::Bool(x && y),
        (Or, Value::Bool(x), Value::Bool(y)) => Value::Bool(x || y),
        (And | Or, Value::Bool(_), b) => return Err(logic_operand_error(op, &b)),
        (And | Or, a, _) => return Err(logic_operand_error(op, &a)),

        // Power: numeric exponentiation. Promote to float when necessary.
        (Pow, Value::Int(x), Value::Int(y)) => {
            if y >= 0 {
                // use integer pow for non-negative integer exponents. An exponent
                // past u32 cannot be exact in 64 bits unless the base is 0 or 1.
                let exp = u32::try_from(y).map_err(|_| crate::safety::integer_overflow("Pow"))?;
                Value::Int(checked_int(x.checked_pow(exp), "Pow")?)
            } else {
                // negative exponent -> float result
                Value::Float((x as f64).powf(y as f64))
            }
        }
        (Pow, Value::Float(x), Value::Float(y)) => Value::Float(x.powf(y)),
        (Pow, Value::Int(x), Value::Float(y)) => Value::Float((x as f64).powf(y)),
        (Pow, Value::Float(x), Value::Int(y)) => Value::Float(x.powf(y as f64)),

        // String repetition. `"=" * 50` is how every shipped example draws a
        // rule, and it was an error.
        (Mul, Value::Str(s), Value::Int(n)) | (Mul, Value::Int(n), Value::Str(s)) => {
            if n <= 0 {
                Value::Str(String::new())
            } else {
                // Checked before allocating, not after: `"x" * 10_000_000_000`
                // must not be built and then rejected.
                let want = s.len().saturating_mul(n as usize);
                if want > MAX_STRING_BYTES {
                    return Err(crate::safety::budget_exceeded(
                        "string *",
                        format!(
                            "string repeat would produce {} bytes, over the {} byte limit",
                            want, MAX_STRING_BYTES
                        ),
                        "repeat fewer times",
                    ));
                }
                Value::Str(s.repeat(n as usize))
            }
        }

        // A number and a String never combine, in either order or under any
        // arithmetic operator. `"n: " + 1` used to be "n: 1", so an agent that
        // wrote `total + 1` where `total` was text got text back, silently.
        // String repetition (`"=" * 50`) is matched above, before this arm.
        (op, a, b) if is_number_string_mix(&op, &a, &b) => {
            return Err(number_string_error(&op, &a, &b))
        }

        // Concatenating a string with anything else renders it the way the user
        // would see it. `"cache hit: " + true` is the same as `"cache hit: ${true}"`.
        (Add, Value::Str(x), other) => {
            checked_string(format!("{}{}", x, other.to_display_string()))?
        }
        (Add, other, Value::Str(y)) => {
            checked_string(format!("{}{}", other.to_display_string(), y))?
        }

        // Uncoded (E_UNKNOWN), and printed whole values: `null - null` said
        // "unsupported op Sub on Null and Null", a record dumped its contents.
        (op, a, b) => {
            let hint = if is_empty_string_number_mix(&a, &b) {
                EMPTY_STRING_HINT
            } else if is_lambda(&a) || is_lambda(&b) {
                // `len > 0` after a pipe: the pipe binds loosest, so the
                // comparison ran on the function, not on the piped value.
                PIPE_PRECEDENCE_HINT
            } else {
                match op {
                    Lt | Lte | Gt | Gte => {
                        "compare two numbers or two strings; convert with int(x) or float(x)"
                    }
                    _ => "use numbers for arithmetic, or strings for + (concatenation); convert with int(x) or float(x)",
                }
            };
            return Err(crate::safety::bad_arg_with_hint(
                &format!("operator {op:?}"),
                "operands it applies to",
                &format!("{} and {}", a.type_name(), b.type_name()),
                hint,
            ));
        }
    })
}

/// The refusal for a non-Bool operand of `&&` or `||`. Truthiness is not used
/// for these operators, so `1 && 2` is refused rather than read as true.
fn logic_operand_error(op: &BinOp, got: &Value) -> anyhow::Error {
    crate::safety::bad_arg_with_hint(
        &format!("operator {op:?}"),
        "Bool operands",
        got.type_name(),
        "compare explicitly, e.g. x != 0, s != \"\", len(a) > 0",
    )
}

/// The Int result of an operation, or the overflow refusal when it does not fit.
fn checked_int(result: Option<i64>, operator: &str) -> Result<i64> {
    result.ok_or_else(|| crate::safety::integer_overflow(operator))
}

/// Whether an arithmetic operator is applied to a number and a String, in
/// either order. Comparison operators are not arithmetic and keep their own
/// refusal.
fn is_number_string_mix(op: &BinOp, a: &Value, b: &Value) -> bool {
    use BinOp::*;
    matches!(op, Add | Sub | Mul | Div | Rem)
        && matches!(
            (a, b),
            (Value::Str(_), Value::Int(_) | Value::Float(_))
                | (Value::Int(_) | Value::Float(_), Value::Str(_))
        )
}

/// The refusal for a number and a String in arithmetic. The hint names the
/// conversions that exist: interpolation, and `str()` (or its alias
/// `to_string()`), which turns a number into text.
fn number_string_error(op: &BinOp, a: &Value, b: &Value) -> anyhow::Error {
    let operator = format!("operator {op:?}");
    let hint = if is_empty_string_number_mix(a, b) {
        EMPTY_STRING_HINT
    } else {
        "use interpolation \"${x}\", or str(x) / to_string(x) to convert a number to text, or int(x) / float(x) to convert text to a number"
    };
    crate::safety::bad_arg_in(
        &operator,
        format!(
            "{operator}: cannot combine {} and {}",
            a.type_name(),
            b.type_name()
        ),
        hint,
    )
}

/// The hint when a number meets an empty String, which is usually a blank CSV
/// field. Arithmetic and comparison refusals both name it.
const EMPTY_STRING_HINT: &str =
    "the String is empty, often a blank CSV field; filter it out (where(.x != \"\")) or default it";

/// For a comparison or arithmetic operator given a function: `|` binds loosest,
/// so `a | f > 0` runs `f > 0` on the function, not on the piped value.
const PIPE_PRECEDENCE_HINT: &str =
    "`|` binds loosest: `a | f > 0` is `a | (f > 0)`; parenthesize the pipe: `(a | f) > 0`";

/// Whether an operand is a function value (a lambda, sync or async).
fn is_lambda(v: &Value) -> bool {
    matches!(v, Value::Lambda(_) | Value::AsyncLambda(_))
}

/// Whether a number meets an empty String, in either order.
fn is_empty_string_number_mix(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Str(s), Value::Int(_) | Value::Float(_))
        | (Value::Int(_) | Value::Float(_), Value::Str(s)) => s.is_empty(),
        _ => false,
    }
}

/// The record a caught error is bound to. Its fields sit at the top level, so
/// `e.code`, `e.message`, `e.hint`, `e.retryable` and `e.builtin` read as an
/// agent expects, and the same record is under `error`, so `e.error.code` keeps
/// working for scripts already written that way.
pub(crate) fn caught_error_record(wire: &serde_json::Value) -> serde_json::Value {
    let inner = wire
        .get("error")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let mut top = inner.as_object().cloned().unwrap_or_default();
    top.insert("error".to_string(), inner);
    serde_json::Value::Object(top)
}

/// The check an annotated `let` makes: the value must have the declared type.
///
/// Only the type names the language has a value for are checked (`Int`,
/// `Float`, `String`, `Bool`, `Array`, `Record`, `Null`). Any other annotation
/// is not checked, as it has never been. An Int is not accepted where a Float is
/// declared: the language does not widen at a binding, so the hint says to write
/// the decimal point.
fn check_declared_type(name: &str, declared: &str, value: &Value) -> Result<()> {
    let expected = match declared {
        "Int" | "Float" | "String" | "Bool" | "Array" | "Record" | "Null" => declared,
        _ => return Ok(()),
    };
    let actual = value.type_name();
    if actual == expected {
        return Ok(());
    }
    let hint = if expected == "Float" && actual == "Int" {
        format!("`{name}` is declared Float: write the literal with a decimal point, such as 1.0")
    } else {
        format!("`{name}` is declared {expected}: bind a {expected} value, or remove the `: {expected}` annotation")
    };
    Err(crate::safety::bad_arg_with_hint(
        &format!("let {name}"),
        expected,
        actual,
        &hint,
    ))
}

/// An unknown name that came from inside a `${...}` hole. The hint says what
/// the hole is, since `"${d}"` is interpolation and not a regex group: the way
/// to write the literal text is `\${`.
fn interpolation_name_error(e: anyhow::Error) -> anyhow::Error {
    match e.downcast::<crate::safety::SafetyError>() {
        Ok(mut se) => {
            se.hint = "${...} interpolates in strings; write \\${ for a literal ${".to_string();
            anyhow::Error::new(se)
        }
        Err(other) => other,
    }
}

/// A `${` that no `}` closes. The lexer refuses these in double-quoted strings,
/// so this is the check for a single-quoted one, which is not lexed for holes.
fn unclosed_interpolation_error() -> anyhow::Error {
    anyhow::Error::new(crate::parser::ParseError::with_hint(
        "unclosed interpolation `${`: no `}` closes it before the string ends",
        "close it with }, or write \\${ for a literal ${",
    ))
}

/// A `${...}` hole that is not one expression.
fn bad_hole_error(expr: &str) -> anyhow::Error {
    anyhow::Error::new(crate::parser::ParseError::with_hint(
        format!("the interpolation `${{{expr}}}` is not one expression"),
        "write one expression inside ${...}, or write \\${ for a literal ${",
    ))
}

/// Interpolate ${expr} patterns in a string
fn interpolate_string(s: &str, env: &mut Env) -> Result<Value> {
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '$' && chars.get(i + 1) == Some(&'{') {
            // The hole's end is found by the lexer's own rule, so a string
            // literal inside the hole can hold `}` or `${` without ending it.
            let body = i + 2;
            let expr_str = match crate::parser::interp_hole_end(&chars, body) {
                Some(end) => {
                    i = end;
                    chars[body..end - 1].iter().collect::<String>()
                }
                // A hole no `}` closes is refused, as the lexer refuses one in
                // a literal. A single-quoted string is not lexed for holes, so
                // this is where its unclosed `${` is caught.
                None => return Err(unclosed_interpolation_error()),
            };

            // Parse and evaluate the expression. A hole that does not parse, or
            // is not one expression, is refused: printing its text as if it had
            // been interpolated hid the mistake.
            let stmts = match crate::parser::parse_program(&expr_str) {
                Ok(stmts) => stmts,
                Err(_) => return Err(bad_hole_error(&expr_str)),
            };
            let expr = match stmts.as_slice() {
                [crate::ast::Stmt::Expr(expr)] => expr,
                _ => return Err(bad_hole_error(&expr_str)),
            };
            // An error in the hole is the error of the whole expression, so a
            // `try` around the string catches it. It used to be printed inside
            // the text as `${...} [error: ...]` and the string ran on.
            match eval_expr(expr, env) {
                Ok(val) => {
                    // `to_display_string` renders every variant the
                    // way a user wrote it. The fallback here used to
                    // be `format!("{:?}", other)`, so interpolating
                    // an array or a record printed Rust's Debug —
                    // `"x: ${r}"` came out as
                    // `x: Record({"a": Int(1)})`.
                    result.push_str(&val.to_display_string());
                }
                // A name nothing binds is an error of the string, not a
                // hole to print: `"${nosuch}"` must not come out as text.
                Err(e) if crate::safety::code_of(&e) == crate::safety::ErrorCode::UnknownName => {
                    return Err(interpolation_name_error(e));
                }
                Err(e) => return Err(e),
            }
        } else {
            result.push(if chars[i] == crate::parser::ESCAPED_DOLLAR {
                '$'
            } else {
                chars[i]
            });
            i += 1;
        }
    }

    Ok(Value::Str(result))
}

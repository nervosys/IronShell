use anyhow::Result;
use crossterm::style::{Color, Stylize};
use std::io::{self, Write};

use crate::{
    config::{get_config, Theme},
    env::Env,
    eval::eval_program,
    parser, // must expose `pub fn parse_program(&str) -> anyhow::Result<Vec<crate::ast::Stmt>>`
    value::Value,
};

/// Parse a hex color string to crossterm Color
fn parse_hex_color(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');
    if hex.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&hex[0..2], 16),
            u8::from_str_radix(&hex[2..4], 16),
            u8::from_str_radix(&hex[4..6], 16),
        ) {
            return Color::Rgb { r, g, b };
        }
    }
    Color::White // Fallback
}

/// Get the current theme colors
fn get_theme_colors() -> crate::config::CustomColors {
    let config = get_config();
    if config.colors.theme == "custom" {
        config.colors.custom.clone()
    } else {
        Theme::from_str(&config.colors.theme).colors()
    }
}

/// Load persisted history, oldest first.
///
/// A missing or unreadable history file is not an error — a fresh shell simply
/// starts with no suggestions.
fn load_history() -> Vec<String> {
    let config = get_config();
    if !config.history.enabled {
        return Vec::new();
    }
    let path = crate::config::ShellConfig::history_file();
    std::fs::read_to_string(path)
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Append one command to the history file, honoring the ignore rules.
fn append_history(command: &str, previous: Option<&String>) {
    let config = get_config();
    if !config.history.enabled {
        return;
    }
    if config.history.ignore_space && command.starts_with(' ') {
        return;
    }
    if config.history.ignore_duplicates && previous.is_some_and(|p| p == command) {
        return;
    }
    let path = crate::config::ShellConfig::history_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write as _;
        let _ = writeln!(f, "{command}");
    }
}

/// Interactive REPL. Ctrl-D exits or type 'exit'/'quit'.
pub fn run(env: &mut Env) -> Result<()> {
    use crate::line_editor::{read_line, EditAction, LineState};
    use crate::prompt::{render_line, render_transient, PromptContext};

    let mut stdout = io::stdout();
    let config = get_config();

    // Show banner if enabled
    if config.shell.show_banner {
        if config.colors.enabled {
            writeln!(
                stdout,
                "{}",
                "Æther REPL — type 'exit', 'quit', or Ctrl-D to exit".dark_grey()
            )?;
        } else {
            writeln!(
                stdout,
                "Æther REPL — type 'exit', 'quit', or Ctrl-D to exit"
            )?;
        }
        stdout.flush()?;
    }

    let mut history = load_history();
    // Exit status and duration of the previous command feed the prompt's
    // status/duration segments.
    let mut last_status = 0i32;
    let mut last_duration_ms = 0u64;

    loop {
        // Re-read per line, so a `config_reload()` typed here changes the prompt.
        let config = get_config();
        let ctx = PromptContext::current(&config.prompt).with_result(last_status, last_duration_ms);
        let rendered = render_line(&config.prompt, &ctx);

        let mut state = LineState::new(&config.prompt, history.clone());
        let action = read_line(&rendered, &mut state, config.colors.enabled)?;

        let code = match action {
            EditAction::Submit(line) => line,
            EditAction::Cancelled => {
                last_status = 130; // 128 + SIGINT, as a shell reports it
                continue;
            }
            EditAction::Eof => {
                writeln!(stdout)?;
                break;
            }
            // read_line only returns terminal actions; Continue/ClearScreen are
            // handled inside its event loop.
            _ => continue,
        };

        let code = code.trim();
        if code.is_empty() {
            continue;
        }
        if code == "exit" || code == "quit" {
            break;
        }

        // Replace the full prompt with a compact one now that the command has
        // been submitted, so scrollback stays readable (oh-my-posh's
        // "transient prompt").
        if config.prompt.transient && config.colors.enabled {
            write!(
                stdout,
                "\x1b[1A\r\x1b[2K{}{}\n",
                render_transient(&config.prompt, &ctx),
                code
            )?;
            stdout.flush()?;
        }

        append_history(code, history.last());
        history.push(code.to_string());

        let started = std::time::Instant::now();
        let LineOutcome {
            result,
            prints_itself,
            ..
        } = eval_line_noting_print(env, code);
        last_duration_ms = started.elapsed().as_millis() as u64;
        crate::record::note(
            crate::record::Surface::Repl,
            code,
            &result,
            started.elapsed(),
        );

        match result {
            Ok(_) if prints_itself => {
                last_status = 0;
            }
            Ok(v) => {
                last_status = 0;
                if let Some(out) = render_for_repl(&v) {
                    writeln!(stdout, "{out}")?;
                }
            }
            Err(e) => {
                last_status = 1;
                if config.colors.enabled {
                    writeln!(stdout, "{} {e}", "error:".red().bold())?;
                } else {
                    writeln!(stdout, "error: {e}")?;
                }
            }
        }
    }
    Ok(())
}

/// The most bytes a one-shot `-c` prints, from `IRONSHELL_MAX_OUTPUT_BYTES`.
/// Unset means no cap. A value that is not a number is ignored with a warning
/// on stderr, so a typo does not silently lift the cap.
fn max_output_bytes() -> Option<usize> {
    let raw = std::env::var("IRONSHELL_MAX_OUTPUT_BYTES").ok()?;
    match raw.trim().parse::<usize>() {
        Ok(max) => Some(max),
        Err(_) => {
            eprintln!(
                "ish: IRONSHELL_MAX_OUTPUT_BYTES={raw:?} is not a byte count; no cap applied"
            );
            None
        }
    }
}

/// Print the result of a one-shot `-c`, cut at [`max_output_bytes`].
///
/// A cut lands on the last UTF-8 boundary at or before the cap, so the output
/// is still valid text. The bytes left out are counted on stderr. The exit
/// status is decided before this runs, so a cut never changes it.
fn print_result(out: &str) {
    let Some(max) = max_output_bytes() else {
        println!("{out}");
        return;
    };
    if out.len() <= max {
        println!("{out}");
        return;
    }
    let mut keep = max;
    while !out.is_char_boundary(keep) {
        keep -= 1;
    }
    println!("{}", &out[..keep]);
    eprintln!(
        "ish: output cut at IRONSHELL_MAX_OUTPUT_BYTES={max}: {} bytes omitted",
        out.len() - keep
    );
}

/// Set when the code being run was transpiled from another shell without a line
/// map: its line numbers are the translation's, not the source's, so an error
/// names no line rather than a wrong one.
static SOURCE_LINES_UNKNOWN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Declare that the code about to run has no line map back to its source.
pub fn set_source_lines_unknown() {
    SOURCE_LINES_UNKNOWN.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// One-liner (e.g. `ish -c 'code'`)
pub fn run_one(env: &mut Env, code: &str) -> Result<i32> {
    run_one_as(env, code, crate::record::Surface::C)
}

/// [`run_one`] for a given surface, which only the recorder reads.
pub fn run_one_as(env: &mut Env, code: &str, surface: crate::record::Surface) -> Result<i32> {
    let started = std::time::Instant::now();
    let LineOutcome {
        result,
        prints_itself,
        failed_line,
        failed_in,
    } = eval_line_noting_print(env, code);
    crate::record::note(surface, code, &result, started.elapsed());
    match result {
        Ok(_) if prints_itself => Ok(0),
        Ok(v) => {
            let budget = std::env::var("IRONSHELL_TOKEN_BUDGET")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|m| *m > 0);
            // Deterministic mode (`--deterministic` / IRONSHELL_DETERMINISTIC) takes
            // precedence over every other renderer: canonical, byte-stable JSON for
            // snapshot tests / caching / diffs. The whole value is emitted (budget
            // is intentionally not applied — reproducibility wants the full result).
            let deterministic = std::env::var("IRONSHELL_DETERMINISTIC")
                .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            if deterministic {
                // Byte-stable output must not become the way around redaction:
                // in agent mode the canonical JSON is scrubbed as the agent
                // renderer's output is.
                let v = if crate::safety::current_mode() == crate::safety::Mode::Agent
                    && crate::safety::redaction_enabled()
                {
                    crate::builtins::redact_value(&v)
                } else {
                    v
                };
                if let Some(out) = crate::builtins::render_canonical(&v) {
                    print_result(&out);
                }
                return Ok(0);
            }
            // Agent mode renders results as compact, deterministic AECON by
            // default (keys once, the structural levers, no ANSI) — the token
            // savings happen automatically instead of requiring an explicit
            // `| aecon`. The human REPL keeps its colorized pretty-printer.
            if crate::safety::current_mode() == crate::safety::Mode::Agent {
                if let Some(out) = crate::builtins::render_agent(&v, budget) {
                    print_result(&out);
                }
                return Ok(0);
            }
            // Human path: apply an output token budget when IRONSHELL_TOKEN_BUDGET is set
            // (e.g. via `--budget`), then pretty-print.
            let v = match budget {
                Some(max) => crate::builtins::budget_value(&v, max, 0),
                None => v,
            };
            if let Some(out) = render_for_repl(&v) {
                print_result(&out);
            }
            Ok(0)
        }
        Err(e) => {
            // A one-line `-c` has no line to name: its only line is the one the
            // user typed.
            let failed_line = failed_line.filter(|_| {
                code.contains('\n')
                    && !SOURCE_LINES_UNKNOWN.load(std::sync::atomic::Ordering::Relaxed)
            });
            let failed_in = failed_in.filter(|_| failed_line.is_some());
            print_eval_error(&e, colors_enabled_stderr(), failed_line, failed_in);
            Ok(exit_status_for(&e))
        }
    }
}

/// The exit status a failure leaves behind.
///
/// Every failure used to exit 1, which scored 0 of 10 on exit-status
/// granularity against bash's 5 of 10 (`benches/agentic/errors.mjs`). The
/// taxonomy already existed; it just never reached the process boundary.
/// See [`crate::safety::ErrorCode::exit_code`] for the mapping and why two of
/// them borrow bash's numbers.
pub fn exit_status_for(e: &anyhow::Error) -> i32 {
    if let Some(se) = e.downcast_ref::<crate::safety::SafetyError>() {
        return se.code.exit_code();
    }
    // A parse failure is a `ParseError` rather than a `SafetyError`, because the
    // parser builds for wasm and `safety` does not. bash exits 2 on a syntax
    // error; so do we.
    if e.downcast_ref::<crate::parser::ParseError>().is_some() {
        return 2;
    }
    1
}

/// Render an error the way the REPL renders an uncaught one, for a refusal made
/// outside the REPL (`ish -b` before it runs anything).
pub fn report_error(e: &anyhow::Error) {
    print_eval_error(e, crate::config::colors_enabled_stderr(), None, None);
}

/// Render an uncaught evaluation error.
///
/// A [`crate::safety::SafetyError`]'s `Display` is JSON (so agents can branch on
/// it). For a **human** at the REPL that's noise, so we unpack it into legible
/// prose — `error[CODE]: message`, a `hint:` line, and (for an approvable action)
/// the exact re-run incantation. **Agent mode keeps the raw JSON** so the
/// structured `code`/`hint`/`approval` survive for programmatic self-correction.
/// Non-safety errors print as plain prose in both modes.
///
/// `line` is the line of the top-level statement that failed, when there is one.
/// It is added to the structured form as `line`, and to the prose as "at line".
fn print_eval_error(e: &anyhow::Error, color: bool, line: Option<usize>, note: Option<String>) {
    use crate::safety::{current_mode, Mode, SafetyError};

    if current_mode() == Mode::Agent {
        // Structured form is what an agent reads — emit it verbatim, with the
        // line of the failing statement in the error object.
        match (e.downcast_ref::<SafetyError>(), line) {
            (Some(se), Some(n)) => {
                let mut json = se.to_json();
                if let Some(err) = json.get_mut("error").and_then(|v| v.as_object_mut()) {
                    err.insert("line".to_string(), serde_json::Value::from(n));
                }
                eprintln!("{json}");
            }
            (_, Some(n)) => match &note {
                Some(t) => eprintln!("{e} (at line {n} {t})"),
                None => eprintln!("{e} (at line {n})"),
            },
            (_, None) => eprintln!("{e}"),
        }
        return;
    }
    let at = line.map(|n| match &note {
        Some(t) => format!("  at line {n} {t}"),
        None => format!("  at line {n}"),
    });

    // A parse failure is a `ParseError` rather than a `SafetyError`, because
    // the parser is built for wasm too and `safety` is not. It carries the same
    // code/message/hint contract, so unpack it the same way.
    if let Some(pe) = e.downcast_ref::<crate::parser::ParseError>() {
        let code = crate::parser::ParseError::CODE;
        if color {
            eprintln!(
                "{}{}{} {}",
                "error[".red().bold(),
                code.red().bold(),
                "]:".red().bold(),
                pe.message
            );
            eprintln!("  {} {}", "hint:".yellow().bold(), pe.hint);
        } else {
            eprintln!("error[{code}]: {}", pe.message);
            eprintln!("  hint: {}", pe.hint);
        }
        return;
    }

    if let Some(se) = e.downcast_ref::<SafetyError>() {
        let code = se.code.as_str();
        if color {
            eprintln!(
                "{}{}{} {}",
                "error[".red().bold(),
                code.red().bold(),
                "]:".red().bold(),
                se.message
            );
            if let Some(at) = &at {
                eprintln!("{at}");
            }
            if !se.hint.is_empty() {
                eprintln!("  {} {}", "hint:".yellow().bold(), se.hint);
            }
            print_extra_fields(se);
            if let Some(a) = &se.approval {
                eprintln!(
                    "  {} {}",
                    "approve:".cyan().bold(),
                    crate::safety::approval_instruction(&a.token)
                );
            }
        } else {
            eprintln!("error[{code}]: {}", se.message);
            if let Some(at) = &at {
                eprintln!("{at}");
            }
            if !se.hint.is_empty() {
                eprintln!("  hint: {}", se.hint);
            }
            print_extra_fields(se);
            if let Some(a) = &se.approval {
                eprintln!(
                    "  approve: {}",
                    crate::safety::approval_instruction(&a.token)
                );
            }
        }
        return;
    }

    if color {
        eprintln!("{} {e}", "error:".red().bold());
    } else {
        eprintln!("error: {e}");
    }
    if let Some(at) = &at {
        eprintln!("{at}");
    }
}

/// The fields a script added with `error(message, extra)`, one per line after
/// the hint, so the text report carries what the JSON form carries. Redacted as
/// the JSON form is.
fn print_extra_fields(se: &crate::safety::SafetyError) {
    for (key, value) in &se.extra {
        let shown = match value {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let shown = if crate::safety::redaction_enabled() {
            crate::safety::redact_str(&shown).0
        } else {
            shown
        };
        eprintln!("  {key}: {shown}");
    }
}

pub fn eval_line(env: &mut Env, code: &str) -> Result<Value> {
    let stmts = parser::parse_program(code)?;
    eval_program(&stmts, env)
}

/// The outcome of one line of code, as [`eval_line_noting_print`] reports it.
struct LineOutcome {
    result: Result<Value>,
    /// The final statement wrote its own output with `print` and produced
    /// exactly that text as its value; the caller must not echo it again.
    prints_itself: bool,
    /// The line the top-level statement that failed starts on, when one failed
    /// at run time (a parse error is not a run-time failure, so has none).
    failed_line: Option<usize>,
    /// Where the lambda that the failing call came out of was written, when that
    /// is another statement or a module file: `(in `bad`, defined at line 3)`.
    failed_in: Option<String>,
}

/// [`eval_line`], also reporting whether the program's final statement wrote
/// its own output with `print` and produced exactly that text as its value. The
/// caller must not echo such a value a second time.
///
/// Each top-level statement runs on its own, so the prints before the final one
/// are not counted, and a failure can be given the line its statement starts on.
/// A print anywhere inside the final statement counts: `print(1)` bare,
/// `if c { print(1) }`, a `try` body, or a lambda that prints and is called last.
fn eval_line_noting_print(env: &mut Env, code: &str) -> LineOutcome {
    let (stmts, lines) = match parser::parse_program_with_lines(code) {
        Ok(parsed) => parsed,
        Err(e) => {
            return LineOutcome {
                result: Err(e),
                prints_itself: false,
                failed_line: None,
                failed_in: None,
            }
        }
    };
    let mut last = Value::Null;
    let mut prints_itself = false;
    for (index, stmt) in stmts.iter().enumerate() {
        // The statement's line is what a failed lambda call is compared with.
        crate::eval::set_statement_site(
            lines
                .get(index)
                .map(|&line| crate::eval::Site { line, file: None }),
        );
        let (before, _) = crate::builtins::print_mark();
        // Not the last statement: its value is not the line's, so a binding
        // does not copy its value out (see `eval_program_statement`).
        match crate::eval::eval_program_statement(stmt, index + 1 < stmts.len(), env) {
            Ok(value) => {
                if index + 1 == stmts.len() {
                    let (after, printed) = crate::builtins::print_mark();
                    prints_itself =
                        after > before && matches!(&value, Value::Str(text) if *text == printed);
                }
                last = value;
            }
            Err(e) => {
                let failed_in = crate::eval::take_failed_lambda_note();
                crate::eval::set_statement_site(None);
                return LineOutcome {
                    result: Err(e),
                    prints_itself: false,
                    failed_line: lines.get(index).copied(),
                    failed_in,
                };
            }
        }
    }
    crate::eval::set_statement_site(None);
    LineOutcome {
        result: Ok(last),
        prints_itself,
        failed_line: None,
        failed_in: None,
    }
}

/// Should rendered values carry ANSI colour?
///
/// The config alone is not enough to answer this: until 10.0.1 it *was* the
/// whole answer, so `ish -c '1 + 2' > out` wrote `\x1b[38;2;180;142;173m3\x1b[39m`
/// into the file. Anything consuming the shell's output — a pipe, a
/// `$(...)` capture, the Homebrew formula's own `assert_equal "3"` — got
/// escape codes it did not ask for.
///
/// The order below is the one users expect from other tools:
///   * `NO_COLOR` set to anything non-empty wins (no-color.org),
///   * then an explicit force, for people piping into `less -R`,
///   * then: colour only when stdout is actually a terminal,
///   * and only then does the config get a say.
use crate::config::{colors_enabled, colors_enabled_stderr};

/// REPL rendering:
/// - Null => print nothing
/// - Str  => print raw (no quotes), so ANSI works
/// - else => compact colorized pretty-print (or plain if colors disabled)
fn render_for_repl(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::Str(s) => Some(s.clone()),
        _ => {
            if colors_enabled() {
                Some(pp_colored(v))
            } else {
                Some(pp(v))
            }
        }
    }
}

/// Apply a color from the theme to a string
fn colorize(s: &str, hex_color: &str) -> String {
    // Checked here as well as in `render_for_repl`, because `pp_colored` is
    // reachable from other callers and a helper that emits escapes regardless
    // of destination is the bug this pair of checks exists to prevent.
    if colors_enabled() && get_config().colors.true_color {
        format!("{}", s.with(parse_hex_color(hex_color)))
    } else {
        // Fallback to basic colors when true_color is disabled
        s.to_string()
    }
}

/// Colorized pretty-print using theme colors from config
fn pp_colored(v: &Value) -> String {
    pp_colored_at(v, false)
}

/// [`pp_colored`] for a value inside a container when `nested`. A string there
/// is a quoted, escaped literal, and a collection is shown in full.
fn pp_colored_at(v: &Value, nested: bool) -> String {
    let colors = get_theme_colors();
    match v {
        Value::Null => colorize("null", &colors.dim),
        Value::Bool(b) => colorize(&b.to_string(), &colors.boolean),
        Value::Int(n) => colorize(&n.to_string(), &colors.number),
        Value::Float(x) => colorize(&crate::value::float_text(*x), &colors.number),
        Value::Str(s) if nested => colorize(&crate::value::quote_str(s), &colors.string),
        Value::Str(s) => colorize(s, &colors.string),
        Value::Uri(u) => colorize(u, &colors.uri),
        Value::Array(items) => {
            let shown = if nested {
                items.len().min(crate::value::NESTED_ARRAY_SHOWN)
            } else {
                items.len()
            };
            let mut s = colorize("[", &colors.punctuation);
            for (i, it) in items[..shown].iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(&pp_colored_at(it, true));
            }
            if shown < items.len() {
                if shown > 0 {
                    s.push_str(", ");
                }
                s.push_str(&colorize(
                    &format!("... ({} more)", items.len() - shown),
                    &colors.dim,
                ));
            }
            s.push_str(&colorize("]", &colors.punctuation));
            s
        }
        Value::Record(map) => {
            let mut s = String::new();
            s.push_str(&colorize("{", &colors.punctuation));
            let mut first = true;
            for (k, v) in map {
                if !first {
                    s.push_str(", ");
                }
                first = false;
                s.push_str(&colorize(k, &colors.key));
                s.push_str(": ");
                s.push_str(&pp_colored_at(v, true));
            }
            s.push_str(&colorize("}", &colors.punctuation));
            s
        }
        Value::Table(t) => colorize(&format!("<Table rows={}>", t.rows.len()), &colors.dim),
        Value::Lambda(_) => colorize("<lambda>", &colors.dim),
        Value::AsyncLambda(_) => colorize("<async lambda>", &colors.dim),
        Value::Future(_) => colorize("<future>", &colors.dim),
        Value::Error(msg) => colorize(&format!("Error: {}", msg), &colors.error),
        Value::Builtin(b) => colorize(&format!("<builtin:{}>", b.name), &colors.dim),
    }
}

// Non-colored versions for when colors are disabled
fn pp(v: &Value) -> String {
    crate::value::plain_text(v, false)
}

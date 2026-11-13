//! Zsh → IronShell Shell transpiler (compat mode).
//!
//! Two routes, chosen by [`route`]:
//!
//!   1. **POSIX engine** (the default). Zsh source that stays inside the
//!      in-process POSIX subset (`src/posix.rs`, the engine `ish -b` uses)
//!      runs there, after a small rewrite of the zsh-only words the engine
//!      does not know (`setopt`, `print`, backslash escapes in `echo`). That
//!      gives GNU-compatible `grep`/`wc`/`sort`/`head`/`tr`/`find`/`date`, `;`,
//!      `&&`/`||`, flags and globs the way a shell has them.
//!   2. **Transpiler** ([`transpile_zsh_to_ae`]). Used for source the engine
//!      does not run: a single `**` recursive-glob command (zsh-only), or
//!      anything else once `IRONSHELL_ALLOW_SH` lets unsupported parts through.
//!      It maps a small set of commands to IronShell builtins, prints the
//!      output of `echo`/`print` and of value-producing commands as they run,
//!      splits `;`, refuses (before anything runs) a flag, glob, parameter
//!      expansion or array use it cannot translate faithfully, and records
//!      every construct it hands to `sh(...)` so the caller can refuse them by
//!      name and line.
//!
//! Variables whose names are IronShell module names (`arr`, `file`, `user`)
//! are renamed with an `sh_` prefix in the translation.

use super::{Fallback, Kind};
use anyhow::{anyhow, Result};

/* =============================================================================
Routing: POSIX engine or transpiler
============================================================================= */

/// Where a zsh source runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Run this (rewritten) text in the in-process POSIX engine. It has the
    /// same number of lines as the zsh source, so a refusal names the right one.
    Posix(String),
    /// Run through [`transpile_zsh_to_ae`].
    Transpile,
}

/// Why a zsh source is refused before anything runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub line: usize,
    pub construct: String,
    pub hint: String,
}

/// Decide how `src` runs, rewriting the zsh-only words the POSIX engine does
/// not know. `Err` is a refusal: nothing has run.
pub fn route(src: &str) -> std::result::Result<Route, Refusal> {
    // `**` (recursive glob) is zsh's own; the engine reads it as `*`. It runs
    // through the transpiler, alone.
    if let Some(at) = src.lines().position(|l| l.contains("**")) {
        let stmts = src
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .count();
        let one = src
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| split_commands(l).len() == 1 && split_statements(l).len() == 1)
            .unwrap_or(false);
        if stmts == 1 && one {
            return Ok(Route::Transpile);
        }
        return Err(Refusal {
            line: at + 1,
            construct: "`**` recursive glob together with other commands".into(),
            hint: "`**` is a zsh glob the POSIX engine does not have: run that command on its own, e.g. ish --zsh -c 'ls **/*.txt'".into(),
        });
    }

    let mut out: Vec<String> = Vec::new();
    let mut nullglob_line: Option<usize> = None;
    for (i, raw) in src.lines().enumerate() {
        let n = i + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            out.push(raw.to_string());
            continue;
        }
        if line.contains("${(") {
            return Err(Refusal {
                line: n,
                construct: "zsh parameter-expansion flag `${(...)...}`".into(),
                hint: "zsh's `${(f)x}`-style flags are not in the POSIX subset: split the text with tr or sed".into(),
            });
        }
        if (line.contains("$(") || line.contains('`'))
            && line.contains('\\')
            && (line.contains("echo") || line.contains("print"))
        {
            return Err(Refusal {
                line: n,
                construct: "echo/print with backslash escapes inside a command substitution".into(),
                hint:
                    "zsh's echo interprets the escapes and the POSIX engine's does not: use printf"
                        .into(),
            });
        }
        out.push(convert_line(raw, n, &mut nullglob_line)?);
    }
    if let Some(n) = nullglob_line {
        // nullglob makes an unmatched glob expand to nothing; the engine passes
        // the pattern through. Only a script with no glob at all is unaffected.
        let globs = out.iter().enumerate().any(|(i, l)| {
            i + 1 != n && !l.trim_start().starts_with('#') && l.chars().any(|c| "*?[".contains(c))
        });
        if globs {
            return Err(Refusal {
                line: n,
                construct: "`setopt nullglob` in a script that uses a glob".into(),
                hint: "an unmatched glob expands to nothing under nullglob but stays literal in the POSIX engine: test with ls first, or drop nullglob".into(),
            });
        }
    }
    Ok(Route::Posix(out.join("\n")))
}

/// Rewrite one zsh line into the POSIX subset's spelling.
fn convert_line(
    raw: &str,
    n: usize,
    nullglob: &mut Option<usize>,
) -> std::result::Result<String, Refusal> {
    let indent_len = raw.len() - raw.trim_start().len();
    let indent = &raw[..indent_len];
    let line = raw.trim();
    let mut out = String::new();
    for (sep, piece) in split_commands(line) {
        out.push_str(&sep);
        out.push_str(&convert_command(&piece, n, nullglob)?);
    }
    Ok(format!("{}{}", indent, out))
}

/// Keywords that may precede a command on the same line.
const LEADERS: [&str; 5] = ["then", "do", "else", "!", "{"];

fn convert_command(
    piece: &str,
    n: usize,
    nullglob: &mut Option<usize>,
) -> std::result::Result<String, Refusal> {
    let lead_len = piece.len() - piece.trim_start().len();
    let lead_ws = &piece[..lead_len];
    let mut body = piece.trim_start();
    let mut prefix = String::new();
    loop {
        let w = body.split_whitespace().next().unwrap_or("");
        if LEADERS.contains(&w) {
            prefix.push_str(w);
            prefix.push(' ');
            body = body[w.len()..].trim_start();
        } else {
            break;
        }
    }
    let first = body.split_whitespace().next().unwrap_or("");
    let rest_text = &body[first.len()..];
    let converted: String = match first {
        "setopt" | "unsetopt" => convert_setopt(first == "unsetopt", rest_text, n, nullglob)?,
        "autoload" | "compdef" | "compctl" | "zstyle" | "bindkey" => ":".to_string(),
        "print" => convert_print(rest_text, n)?,
        "echo" => {
            let (head, tail) = split_command_tail(rest_text);
            let words = raw_words(head);
            let mut suppress = false;
            for w in &words {
                if is_echo_flag(w) {
                    if w.contains('E') {
                        suppress = true;
                    }
                } else {
                    break;
                }
            }
            if head.contains('\\') && !suppress {
                format!("echo -e{}{}", head, tail)
            } else {
                body.to_string()
            }
        }
        _ => body.to_string(),
    };
    // Keep the original trailing whitespace out: the separator carries spacing.
    let trailing_len = piece.len() - piece.trim_end().len();
    let trailing = &piece[piece.len() - trailing_len..];
    Ok(format!(
        "{}{}{}{}",
        lead_ws,
        prefix,
        converted.trim_end(),
        trailing
    ))
}

fn is_echo_flag(w: &str) -> bool {
    w.len() > 1 && w.starts_with('-') && w[1..].chars().all(|c| matches!(c, 'n' | 'e' | 'E'))
}

/// `print [-lnr] args`: the flags zsh's print takes that have a POSIX spelling.
fn convert_print(rest: &str, n: usize) -> std::result::Result<String, Refusal> {
    let (head, tail) = split_command_tail(rest);
    let words = raw_words(head);
    let (mut l, mut nn, mut r) = (false, false, false);
    let mut k = 0;
    while k < words.len() {
        let w = &words[k];
        if w == "--" {
            k += 1;
            break;
        }
        if w.len() > 1 && w.starts_with('-') && !w.starts_with("--") {
            for c in w[1..].chars() {
                match c {
                    'l' => l = true,
                    'n' => nn = true,
                    'r' => r = true,
                    other => {
                        return Err(Refusal {
                            line: n,
                            construct: format!("`print -{}`", other),
                            hint: "only print -l, -n and -r are translated: use echo or printf for the rest".into(),
                        })
                    }
                }
            }
            k += 1;
        } else {
            break;
        }
    }
    let args = words[k..].join(" ");
    if args.contains('\\') && !r {
        return Err(Refusal {
            line: n,
            construct: "`print` with backslash escapes".into(),
            hint: "zsh's print interprets the escapes unless -r is given: use print -r, or printf"
                .into(),
        });
    }
    if let Some(first) = words.get(k) {
        if is_echo_flag(first) {
            return Err(Refusal {
                line: n,
                construct: format!("`print` of the word `{}`", first),
                hint: "the word would be read as an echo flag: use printf '%s\\n' instead".into(),
            });
        }
    }
    let body = if l {
        if args.is_empty() {
            "printf '%s\\n'".to_string()
        } else {
            format!("printf '%s\\n' {}", args)
        }
    } else {
        let flag = if nn { " -n" } else { "" };
        if args.is_empty() {
            format!("echo{}", flag)
        } else {
            format!("echo{} {}", flag, args)
        }
    };
    Ok(format!("{}{}", body, tail))
}

/// `setopt` / `unsetopt`: an option that changes nothing a script computes is
/// a no-op (`:`), one the engine has a spelling for becomes `set`, and any
/// other is refused by name.
fn convert_setopt(
    unset: bool,
    rest: &str,
    n: usize,
    nullglob: &mut Option<usize>,
) -> std::result::Result<String, Refusal> {
    let (head, tail) = split_command_tail(rest);
    let opts = raw_words(head);
    let cmd = if unset { "unsetopt" } else { "setopt" };
    if opts.is_empty()
        || opts
            .iter()
            .any(|o| o.starts_with('-') || o.starts_with('+'))
    {
        return Err(Refusal {
            line: n,
            construct: format!("`{}` without option names", cmd),
            hint: "name the options: setopt extendedglob".into(),
        });
    }
    let mut parts: Vec<String> = Vec::new();
    for o in &opts {
        let norm: String = o
            .chars()
            .filter(|c| *c != '_' && *c != '-')
            .collect::<String>()
            .to_lowercase();
        // `setopt noglob` is `unsetopt glob`; `notify` is an option, not a negation.
        let (eff, on) = match setopt_effect(&norm) {
            Some(e) => (Some(e), !unset),
            None => match norm.strip_prefix("no") {
                Some(b) => (setopt_effect(b), unset),
                None => (None, !unset),
            },
        };
        match eff {
            Some("errexit") => parts.push(format!("set {}e", if on { '-' } else { '+' })),
            Some("nounset") => parts.push(format!("set {}u", if on { '-' } else { '+' })),
            Some("pipefail") => {
                parts.push(format!("set {}o pipefail", if on { '-' } else { '+' }))
            }
            Some("nullglob") => {
                if on {
                    *nullglob = Some(n);
                }
                parts.push(":".to_string());
            }
            Some(_) => parts.push(":".to_string()),
            None => {
                return Err(Refusal {
                    line: n,
                    construct: format!("`{} {}`", cmd, o),
                    hint: "this option can change what the script computes and has no POSIX-engine spelling; only history, prompt, completion and beep options, errexit, nounset, pipefail and nullglob are accepted".into(),
                })
            }
        }
    }
    Ok(format!("{}{}", parts.join("; "), tail))
}

/// What a (normalised: lower-case, no underscores) option name does here:
/// `errexit`, `nounset`, `pipefail` and `nullglob` are themselves, `ignore`
/// changes nothing a script computes, `None` is refused.
fn setopt_effect(base: &str) -> Option<&'static str> {
    match base {
        "errexit" => Some("errexit"),
        "nounset" => Some("nounset"),
        "pipefail" => Some("pipefail"),
        "nullglob" => Some("nullglob"),
        "extendedglob"
        | "histignoredups"
        | "histignorespace"
        | "histignorealldups"
        | "sharehistory"
        | "appendhistory"
        | "incappendhistory"
        | "histverify"
        | "histreduceblanks"
        | "promptsubst"
        | "interactivecomments"
        | "beep"
        | "listbeep"
        | "hist"
        | "pushdsilent"
        | "pushdignoredups"
        | "autopushd"
        | "completeinword"
        | "alwaystoend"
        | "menucomplete"
        | "automenu"
        | "listambiguous"
        | "notify"
        | "longlistjobs"
        | "correct"
        | "autolist"
        | "globcomplete" => Some("ignore"),
        _ => None,
    }
}

/// Split `s` at the first unquoted `|`, `>`, `<` or `&`: the command words and
/// the redirection/pipe tail (kept verbatim).
fn split_command_tail(s: &str) -> (&str, &str) {
    let mut sq = false;
    let mut dq = false;
    let mut esc = false;
    for (i, ch) in s.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match ch {
            '\\' if !sq => esc = true,
            '\'' if !dq => sq = !sq,
            '"' if !sq => dq = !dq,
            '|' | '>' | '<' | '&' if !sq && !dq => return (&s[..i], &s[i..]),
            _ => {}
        }
    }
    (s, "")
}

/// Whitespace-separated words of `s`, quotes kept as written.
fn raw_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut sq = false;
    let mut dq = false;
    let mut esc = false;
    for ch in s.chars() {
        if esc {
            cur.push(ch);
            esc = false;
            continue;
        }
        match ch {
            '\\' if !sq => {
                cur.push(ch);
                esc = true;
            }
            '\'' if !dq => {
                sq = !sq;
                cur.push(ch);
            }
            '"' if !sq => {
                dq = !dq;
                cur.push(ch);
            }
            c if c.is_whitespace() && !sq && !dq => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// Split a line into commands at top-level `;`, `&&`, `||`, `|` and a single
/// `&`. Each item is (the separator that precedes it, the command text).
/// Quotes, `$(...)`, `${...}`, parentheses and braces nest. `;;` is not a
/// separator.
fn split_commands(line: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut sep = String::new();
    let mut cur = String::new();
    let mut sq = false;
    let mut dq = false;
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' && !sq && i + 1 < chars.len() {
            cur.push(ch);
            cur.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if ch == '\'' && !dq {
            sq = !sq;
        } else if ch == '"' && !sq {
            dq = !dq;
        } else if !sq && !dq {
            match ch {
                '(' | '{' => depth += 1,
                ')' | '}' => depth -= 1,
                _ => {}
            }
            if depth <= 0 {
                let next = chars.get(i + 1).copied();
                let two = match (ch, next) {
                    ('&', Some('&')) => Some("&&"),
                    ('|', Some('|')) => Some("||"),
                    (';', Some(';')) => {
                        cur.push_str(";;");
                        i += 2;
                        continue;
                    }
                    _ => None,
                };
                let is_redirect_amp = ch == '&'
                    && (next == Some('>')
                        || cur.ends_with('>')
                        || cur.ends_with('<')
                        || cur.ends_with('|'));
                if let Some(s) = two {
                    out.push((std::mem::take(&mut sep), std::mem::take(&mut cur)));
                    sep = s.to_string();
                    i += 2;
                    continue;
                }
                if (ch == ';' || ch == '|' || ch == '&') && !is_redirect_amp {
                    out.push((std::mem::take(&mut sep), std::mem::take(&mut cur)));
                    sep = ch.to_string();
                    i += 1;
                    continue;
                }
            }
        }
        cur.push(ch);
        i += 1;
    }
    out.push((sep, cur));
    out
}

/// The line without a trailing `# comment` (a `#` that starts a word, outside quotes).
fn strip_comment(line: &str) -> &str {
    let mut sq = false;
    let mut dq = false;
    let mut esc = false;
    let mut prev_ws = true;
    for (i, ch) in line.char_indices() {
        if esc {
            esc = false;
            prev_ws = false;
            continue;
        }
        match ch {
            '\\' if !sq => esc = true,
            '\'' if !dq => sq = !sq,
            '"' if !sq => dq = !dq,
            '#' if !sq && !dq && prev_ws => return line[..i].trim_end(),
            _ => {}
        }
        prev_ws = ch.is_whitespace();
    }
    line
}

/// Statements of a line: split at top-level `;` only.
fn split_statements(line: &str) -> Vec<String> {
    let line = strip_comment(line);
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (sep, piece) in split_commands(line) {
        if sep == ";" {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push_str(&sep);
        }
        cur.push_str(&piece);
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/* =============================================================================
Transpiler
============================================================================= */

/// Transpile a Zsh script (multi-line) to IronShell code.
pub fn transpile_zsh_to_ae(src: &str) -> Result<String> {
    transpile_zsh_with_report(src).map(|(code, _)| code)
}

struct Ctx {
    fb: Vec<Fallback>,
    arrays: Vec<String>,
}

/// [`transpile_zsh_to_ae`], plus every construct that was handed to `sh(...)`
/// rather than translated, with its line.
pub fn transpile_zsh_with_report(src: &str) -> Result<(String, Vec<Fallback>)> {
    let mut out = String::new();
    out.push_str("// Transpiled from Zsh → IronShell (compat mode)\n");

    let mut ctx = Ctx {
        fb: Vec::new(),
        arrays: Vec::new(),
    };
    let mut block_depth: i32 = 0;
    let mut block_lines: Vec<String> = Vec::new();
    let mut block_start: usize = 0;

    for (idx, raw_line) in src.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();

        if line.is_empty() {
            if block_depth > 0 {
                block_lines.push(raw_line.to_string());
            }
            continue;
        }
        if line.starts_with('#') {
            if block_depth > 0 {
                block_lines.push(raw_line.to_string());
            } else {
                out.push_str(&format!("// {}\n", line[1..].trim_start()));
            }
            continue;
        }

        let openers = count_block_openers(line);
        let closers = count_block_closers(line);

        if block_depth > 0 {
            block_lines.push(raw_line.to_string());
            block_depth += openers - closers;
            if block_depth <= 0 {
                let full_block = block_lines.join("\n");
                out.push_str(&render_fallback_zsh(&full_block));
                out.push('\n');
                ctx.fb.push(Fallback {
                    line: block_start,
                    construct: describe_block(&block_lines[0]),
                });
                block_lines.clear();
                block_depth = 0;
            }
            continue;
        }

        if openers > closers {
            block_depth = openers - closers;
            block_start = line_no;
            block_lines.push(raw_line.to_string());
            continue;
        }

        for stmt in split_statements(line) {
            emit_statement(&stmt, line_no, &mut out, &mut ctx)?;
        }
    }

    if !block_lines.is_empty() {
        let full_block = block_lines.join("\n");
        out.push_str(&render_fallback_zsh(&full_block));
        out.push('\n');
        ctx.fb.push(Fallback {
            line: block_start,
            construct: describe_block(&block_lines[0]),
        });
    }

    Ok((out, ctx.fb))
}

fn describe_block(first_line: &str) -> String {
    let t = first_line.trim();
    let w = t.split_whitespace().next().unwrap_or("");
    match w {
        "if" | "for" | "while" | "until" | "case" | "select" => format!("`{}` statement", w),
        "function" => "function definition".to_string(),
        _ => {
            if t.contains("()") {
                "function definition".to_string()
            } else {
                "block".to_string()
            }
        }
    }
}

fn describe_complex(s: &str) -> String {
    if s.contains("&&") || s.contains("||") {
        "`&&`/`||` list".to_string()
    } else if s.contains("$(") || s.contains('`') {
        "command substitution".to_string()
    } else if s.contains("<<") {
        "here-document or here-string".to_string()
    } else if s.contains('>') || s.contains('<') {
        "redirection".to_string()
    } else {
        "background job or `&`".to_string()
    }
}

/// Note `name=(...)` array assignments so a later `$name` can be refused.
fn note_array(ctx: &mut Ctx, line: &str) {
    let mut rest = line;
    loop {
        let w = rest.split_whitespace().next().unwrap_or("");
        if matches!(w, "local" | "typeset" | "declare" | "readonly") {
            rest = rest[w.len()..].trim_start();
        } else {
            break;
        }
    }
    if let Some(eq) = rest.find('=') {
        let name = rest[..eq].trim();
        if rest[eq + 1..].starts_with('(') && is_ident(name) {
            ctx.arrays.push(name.to_string());
        }
    }
}

fn emit_statement(stmt: &str, line_no: usize, out: &mut String, ctx: &mut Ctx) -> Result<()> {
    let line = stmt.trim();
    if line.is_empty() {
        return Ok(());
    }
    let first = line.split_whitespace().next().unwrap_or("");

    // Zsh-specific: setopt / unsetopt / completion setup → comment. The
    // transpiler ignores these options; the POSIX route is stricter.
    if matches!(
        first,
        "setopt"
            | "unsetopt"
            | "autoload"
            | "zparseopts"
            | "compdef"
            | "compctl"
            | "zstyle"
            | "bindkey"
    ) {
        out.push_str(&format!("// [zsh] {}\n", line));
        return Ok(());
    }

    // typeset -A (associative array) → empty record
    if line.starts_with("typeset -A ")
        || line.starts_with("declare -A ")
        || line.starts_with("local -A ")
    {
        let rest = line.split_once(" -A ").map(|x| x.1).unwrap_or("").trim();
        if !rest.is_empty() {
            let name = rest.split('=').next().unwrap_or(rest).trim();
            if is_ident(name) {
                out.push_str(&format!("let {} = {{}};\n", vname(name)));
                return Ok(());
            }
        }
    }

    // typeset / local / declare (non-flag) → let
    if (line.starts_with("local ") || line.starts_with("typeset ") || line.starts_with("declare "))
        && !line.contains(" -")
    {
        let rest = line.split_once(' ').map(|x| x.1).unwrap_or("").trim();
        if let Some(eq) = rest.find('=') {
            let name = rest[..eq].trim();
            let val = rest[eq + 1..].trim();
            if is_ident(name) {
                note_array(ctx, line);
                if val.starts_with('(') && val.ends_with(')') {
                    let inner = &val[1..val.len() - 1];
                    out.push_str(&format!(
                        "let {} = [{}];\n",
                        vname(name),
                        array_elems(inner)
                    ));
                } else if is_int_literal(val) || is_float_literal(val) {
                    out.push_str(&format!("let {} = {};\n", vname(name), val));
                } else {
                    let cleaned = val.trim_matches(|c: char| c == '\'' || c == '"');
                    out.push_str(&format!(
                        "let {} = {};\n",
                        vname(name),
                        json_string_literal(cleaned)
                    ));
                }
                return Ok(());
            }
        } else if is_ident(rest) {
            out.push_str(&format!("let {} = null;\n", vname(rest)));
            return Ok(());
        }
    }

    // export NAME=VALUE → let NAME = VALUE;
    if first == "export" {
        let rest = line[6..].trim();
        if let Some(eq) = rest.find('=') {
            let lhs = rest[..eq].trim();
            let rhs = rest[eq + 1..].trim();
            if is_ident(lhs) && !rhs.is_empty() {
                if is_int_literal(rhs) {
                    out.push_str(&format!("let {} = {};\n", vname(lhs), rhs));
                } else if rhs.starts_with('$') {
                    if let Some(var) = parse_var_ref(rhs) {
                        out.push_str(&format!("let {} = {};\n", vname(lhs), var));
                    } else {
                        out.push_str(&render_fallback_zsh(line));
                        out.push('\n');
                        ctx.fb.push(Fallback {
                            line: line_no,
                            construct: "`export` of a parameter expansion".into(),
                        });
                    }
                } else {
                    let cleaned = rhs.trim_matches(|c: char| c == '\'' || c == '"');
                    out.push_str(&format!(
                        "let {} = {};\n",
                        vname(lhs),
                        json_string_literal(cleaned)
                    ));
                }
                return Ok(());
            }
        }
    }

    if contains_zsh_complex(line) {
        out.push_str(&render_fallback_zsh(line));
        out.push('\n');
        ctx.fb.push(Fallback {
            line: line_no,
            construct: describe_complex(line),
        });
        return Ok(());
    }

    if let Some(assign) = parse_simple_assignment(line) {
        note_array(ctx, line);
        out.push_str(&assign);
        out.push('\n');
        return Ok(());
    }

    match parse_pipeline(line) {
        Ok(cmds) if !cmds.is_empty() => {
            let code = render_pipeline(&cmds, line_no, ctx)?;
            out.push_str(&code);
            out.push('\n');
            Ok(())
        }
        Ok(_) => Ok(()),
        Err(e) => Err(anyhow!("zsh line {}: {}", line_no, e)),
    }
}

fn array_elems(inner: &str) -> String {
    inner
        .split_whitespace()
        .map(|w| {
            let w = w.trim_matches(|c: char| c == '\'' || c == '"');
            if is_int_literal(w) || is_float_literal(w) {
                w.to_string()
            } else {
                json_string_literal(w)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/* =============================================================================
Block depth tracking
============================================================================= */

fn count_block_openers(line: &str) -> i32 {
    let mut count = 0i32;
    let words: Vec<&str> = line.split_whitespace().collect();
    let first = words.first().copied().unwrap_or("");

    match first {
        "if" | "for" | "while" | "until" | "case" | "select" => count += 1,
        "function" => count += 1,
        _ => {}
    }

    if first != "function" && (line.contains("() {") || line.contains("(){")) {
        count += 1;
    }

    count
}

fn count_block_closers(line: &str) -> i32 {
    let mut count = 0i32;
    let trimmed = line.trim().trim_end_matches(';').trim();
    let first_word = trimmed.split_whitespace().next().unwrap_or("");

    if trimmed == "fi" || trimmed == "done" || trimmed == "esac" || trimmed == "}" {
        count += 1;
    } else if first_word == "fi" || first_word == "done" || first_word == "esac" {
        count += 1;
    } else if trimmed.ends_with("; fi")
        || trimmed.ends_with("; done")
        || trimmed.ends_with("; esac")
        || trimmed.ends_with("; }")
    {
        count += 1;
    }

    count
}

/* =============================================================================
Representation
============================================================================= */

#[derive(Debug, Clone)]
struct Command {
    program: Token,
    args: Vec<Token>,
}

#[derive(Debug, Clone)]
enum Token {
    Interp(Vec<Piece>),
    Single(String),
}

#[derive(Debug, Clone)]
enum Piece {
    /// Unquoted text.
    Text(String),
    /// Text from inside double quotes: never a glob.
    QText(String),
    Var(String),
}

/* =============================================================================
Complexity detection
============================================================================= */

fn contains_zsh_complex(s: &str) -> bool {
    let suspicious = ['>', '<', '&'];
    s.chars().any(|c| suspicious.contains(&c))
        || s.contains("2>")
        || s.contains(">>")
        || s.contains("<<")
        || s.contains("|&")
        || s.contains("||")
        || s.contains("<<<")
        || s.contains(">|")
        || s.contains("$(")
        || s.contains("`")
        || s.contains("=~")
}

fn render_fallback_zsh(line: &str) -> String {
    let quoted = json_string_literal(line);
    format!("sh([\"zsh\",\"-c\", {}]);", quoted)
}

/* =============================================================================
Simple assignment
============================================================================= */

fn parse_simple_assignment(line: &str) -> Option<String> {
    if line.starts_with("export ") || line.starts_with("readonly ") {
        return None;
    }
    // Handle array assignment NAME=(a b c) → let NAME = ["a", "b", "c"];
    if let Some(eq) = line.find('=') {
        let rhs_start = &line[eq + 1..];
        if rhs_start.starts_with('(') && line.ends_with(')') {
            let lhs = line[..eq].trim();
            if !lhs.is_empty() && is_ident(lhs) {
                let inner = &rhs_start[1..rhs_start.len() - 1];
                return Some(format!("let {} = [{}];", vname(lhs), array_elems(inner)));
            }
        }
    }
    if line.split_whitespace().count() != 1 {
        return None;
    }
    let eq = line.find('=')?;
    let (lhs, rhs_with_eq) = line.split_at(eq);
    let lhs = lhs.trim();
    if lhs.is_empty() || !is_ident(lhs) {
        return None;
    }
    let rhs = rhs_with_eq[1..].trim();

    if rhs.is_empty() || rhs.contains(' ') || contains_zsh_complex(rhs) {
        return None;
    }
    let lhs = vname(lhs);

    if rhs.starts_with('"') || rhs.starts_with('\'') {
        if let Ok(tok) = parse_single_token(rhs) {
            let expr = render_arg_expr(&tok);
            return Some(format!("let {} = {};", lhs, expr));
        }
        None
    } else if rhs.starts_with('$') {
        parse_var_ref(rhs).map(|var| format!("let {} = {};", lhs, var))
    } else if is_int_literal(rhs) || is_float_literal(rhs) {
        Some(format!("let {} = {};", lhs, rhs))
    } else {
        Some(format!("let {} = {};", lhs, json_string_literal(rhs)))
    }
}
/* =============================================================================
Pipeline parsing
============================================================================= */

fn parse_pipeline(s: &str) -> Result<Vec<Command>> {
    let parts = split_unquoted_pipes(s)?;
    let mut cmds = Vec::new();
    for p in parts {
        let toks = split_shell_words(&p)?;
        if toks.is_empty() {
            continue;
        }
        let program = toks[0].clone();
        let args = toks[1..].to_vec();
        cmds.push(Command { program, args });
    }
    Ok(cmds)
}

fn split_unquoted_pipes(s: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut dq = false;
    let mut sq = false;
    for ch in s.chars() {
        match ch {
            '\'' if !dq => {
                sq = !sq;
                cur.push(ch);
            }
            '"' if !sq => {
                dq = !dq;
                cur.push(ch);
            }
            '|' if !sq && !dq => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    Ok(out)
}

/// Read a `$` reference, refusing the forms the transpiler cannot translate:
/// positional and special parameters, and `${name:-default}`-style expansions.
fn read_checked_var(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<Option<String>> {
    if let Some(&next) = chars.peek() {
        if matches!(next, '?' | '#' | '@' | '*' | '!' | '$' | '-') {
            return Err(anyhow!(
                "the special parameter `${}` is not translated (the POSIX engine runs it when the script has no `**` glob)",
                next
            ));
        }
    }
    match read_var(chars) {
        Some((var, _)) => {
            if is_ident(&var) {
                Ok(Some(var))
            } else {
                Err(anyhow!(
                    "the parameter expansion `${{{}}}` is not translated",
                    var
                ))
            }
        }
        None => Ok(None),
    }
}

fn split_shell_words(s: &str) -> Result<Vec<Token>> {
    let mut toks: Vec<Token> = Vec::new();
    let mut cur = Vec::<Piece>::new();
    let mut literal_sq = None::<String>;
    let mut dq = false;
    let mut sq = false;
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !dq => {
                if !sq {
                    sq = true;
                    literal_sq = Some(String::new());
                } else {
                    toks.push(Token::Single(literal_sq.take().unwrap_or_default()));
                    sq = false;
                }
            }
            '"' if !sq => {
                dq = !dq;
                if !dq && !cur.is_empty() {
                    toks.push(Token::Interp(std::mem::take(&mut cur)));
                }
            }
            c if sq => {
                if let Some(ref mut lit) = literal_sq {
                    lit.push(c);
                }
            }
            c if dq => {
                if c == '$' {
                    if let Some(var) = read_checked_var(&mut chars)? {
                        cur.push(Piece::Var(var));
                    } else {
                        cur.push(Piece::QText("$".into()));
                    }
                } else {
                    cur.push(Piece::QText(c.to_string()));
                }
            }
            c if c.is_whitespace() => {
                if !cur.is_empty() {
                    toks.push(Token::Interp(std::mem::take(&mut cur)));
                }
            }
            '$' => {
                if let Some(var) = read_checked_var(&mut chars)? {
                    cur.push(Piece::Var(var));
                } else {
                    cur.push(Piece::Text("$".into()));
                }
            }
            _ => {
                cur.push(Piece::Text(ch.to_string()));
            }
        }
    }
    if sq {
        toks.push(Token::Single(literal_sq.take().unwrap_or_default()));
    }
    if !cur.is_empty() {
        toks.push(Token::Interp(cur));
    }
    Ok(toks)
}

fn parse_single_token(s: &str) -> Result<Token> {
    let toks = split_shell_words(s)?;
    if toks.len() == 1 {
        Ok(toks
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("Expected single token"))?)
    } else {
        Err(anyhow!("expected single token, got {}", toks.len()))
    }
}

/* =============================================================================
Rendering
============================================================================= */

/// The builtin an unquoted word is translated to, and its kind.
fn map_zsh_builtin(name: &str) -> Option<(&'static str, Kind)> {
    ZSH_MAP
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, t, k)| (*t, *k))
}

fn is_output_command(name: &str) -> bool {
    name == "echo" || name == "print"
}

fn render_pipeline(cmds: &[Command], line_no: usize, ctx: &mut Ctx) -> Result<String> {
    let last = cmds.len() - 1;
    let mut stages: Vec<String> = Vec::new();
    // Whether the last stage already writes its output, or is run for effect.
    let mut settled = false;
    let mut quiet = false;
    for (i, c) in cmds.iter().enumerate() {
        reject_array_use(c, ctx, line_no)?;
        let name = token_to_plain_word(&c.program);
        let name_s = name.as_deref().unwrap_or("");
        if is_output_command(name_s) && (i == 0 || i == last) {
            check_args(name_s, &c.args, line_no, false)?;
            let text = joined_text_expr(&c.args, line_no)?;
            if i == last {
                stages.push(format!("print({})", text));
                settled = true;
            } else {
                stages.push(text);
            }
            continue;
        }
        if let Some((target, kind)) = map_zsh_builtin(name_s) {
            check_args(name_s, &c.args, line_no, name_s == "ls" || name_s == "cat")?;
            let args = c
                .args
                .iter()
                .map(render_arg_expr)
                .collect::<Vec<_>>()
                .join(", ");
            stages.push(format!("{}({})", target, args));
            if i == last {
                quiet = kind == Kind::Quiet;
            }
            continue;
        }
        // No faithful mapping: hand it to sh(), and say so.
        let what = if name_s.is_empty() {
            "command".to_string()
        } else if is_output_command(name_s) {
            format!("`{}` after a pipe", name_s)
        } else {
            format!("command `{}`", name_s)
        };
        ctx.fb.push(Fallback {
            line: line_no,
            construct: what,
        });
        stages.push(render_external_call(c));
        if i == last {
            settled = true;
        }
    }
    let mut code = stages.join(" | ");
    if !settled && !quiet {
        code.push_str(" | print()");
    }
    if quiet && !settled {
        code.push_str("\nnull;");
    }
    Ok(code)
}

/// Refuse a `$name` that holds an array: zsh prints its elements joined by
/// spaces, an IronShell Array prints as `[...]`.
fn reject_array_use(c: &Command, ctx: &Ctx, line_no: usize) -> Result<()> {
    let toks = std::iter::once(&c.program).chain(c.args.iter());
    for t in toks {
        if let Token::Interp(pcs) = t {
            for p in pcs {
                if let Piece::Var(v) = p {
                    if ctx.arrays.iter().any(|a| a == v) {
                        return Err(anyhow!(
                            "zsh line {}: the array `${}` is not translated (zsh would print its elements joined by spaces); nothing was run",
                            line_no,
                            v
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Refuse what the mapped builtins would read wrongly: a flag taken as data,
/// and (unless the builtin expands globs itself) an unexpanded glob.
fn check_args(name: &str, args: &[Token], line_no: usize, globs_ok: bool) -> Result<()> {
    for a in args {
        if let Token::Interp(pcs) = a {
            let mut unquoted = String::new();
            for p in pcs {
                if let Piece::Text(t) = p {
                    unquoted.push_str(t);
                }
            }
            if unquoted.len() > 1 && unquoted.starts_with('-') && !unquoted.starts_with("--") {
                return Err(anyhow!(
                    "zsh line {}: the flag `{}` of `{}` is not supported by the transpiler (the POSIX engine runs it when the script has no `**` glob); nothing was run",
                    line_no,
                    unquoted,
                    name
                ));
            }
            if unquoted.starts_with("--") && unquoted.len() > 2 {
                return Err(anyhow!(
                    "zsh line {}: the option `{}` of `{}` is not supported by the transpiler; nothing was run",
                    line_no,
                    unquoted,
                    name
                ));
            }
            if !globs_ok && unquoted.chars().any(|c| "*?[".contains(c)) {
                return Err(anyhow!(
                    "zsh line {}: the glob in `{}` is not expanded by the transpiler (zsh would expand it); nothing was run",
                    line_no,
                    name
                ));
            }
        }
    }
    Ok(())
}

/// The words of an `echo`/`print`, joined by single spaces, as one expression.
fn joined_text_expr(args: &[Token], line_no: usize) -> Result<String> {
    if args.is_empty() {
        return Ok("\"\"".to_string());
    }
    if args.len() == 1 {
        if let Token::Single(s) = &args[0] {
            if s.contains("${") {
                return Err(anyhow!(
                    "zsh line {}: single-quoted text containing `${{` would be interpolated by IronShell; nothing was run",
                    line_no
                ));
            }
        }
        return Ok(render_arg_expr(&args[0]));
    }
    let mut s = String::from("\"");
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        match a {
            Token::Single(t) => {
                if t.contains("${") {
                    return Err(anyhow!(
                        "zsh line {}: single-quoted text containing `${{` would be interpolated by IronShell; nothing was run",
                        line_no
                    ));
                }
                s.push_str(&dq_escape(t));
            }
            Token::Interp(pcs) => {
                for p in pcs {
                    match p {
                        Piece::Text(t) | Piece::QText(t) => s.push_str(&dq_escape(t)),
                        Piece::Var(v) => {
                            s.push_str("${");
                            s.push_str(&vname(v));
                            s.push('}');
                        }
                    }
                }
            }
        }
    }
    s.push('"');
    Ok(s)
}

fn dq_escape(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

fn render_external_call(cmd: &Command) -> String {
    let mut arr = Vec::new();
    let prog = token_to_plain_word(&cmd.program).unwrap_or_else(|| token_to_string(&cmd.program));
    arr.push(json_string_literal(&prog));
    for a in &cmd.args {
        arr.push(render_arg_expr(a));
    }
    format!("sh([{}])", arr.join(", "))
}

fn render_arg_expr(tok: &Token) -> String {
    match tok {
        Token::Single(s) => double_quote_with_escapes(s),
        Token::Interp(pcs) => {
            if pcs.len() == 1 {
                if let Piece::Var(v) = &pcs[0] {
                    if is_ident(v) {
                        return vname(v);
                    }
                }
            }
            let mut s = String::from("\"");
            for p in pcs {
                match p {
                    Piece::Text(t) | Piece::QText(t) => s.push_str(&escape_in_double_quotes(t)),
                    Piece::Var(v) => {
                        s.push_str("${");
                        s.push_str(&vname(v));
                        s.push('}');
                    }
                }
            }
            s.push('"');
            s
        }
    }
}
/* =============================================================================
Utilities
============================================================================= */

/// The IronShell name of a zsh variable: a module name (`arr`, `file`, `user`)
/// cannot be bound, so it gets an `sh_` prefix, everywhere it appears.
fn vname(n: &str) -> String {
    if crate::modules::is_module_name(n) {
        format!("sh_{}", n)
    } else {
        n.to_string()
    }
}

fn token_to_plain_word(tok: &Token) -> Option<String> {
    match tok {
        Token::Single(_) => None,
        Token::Interp(pcs) => {
            if pcs.iter().all(|p| matches!(p, Piece::Text(_))) {
                let s: String = pcs
                    .iter()
                    .map(|p| match p {
                        Piece::Text(t) => t.as_str(),
                        _ => "",
                    })
                    .collect();
                if !s.is_empty() && !s.chars().any(|c| c.is_whitespace()) {
                    return Some(s);
                }
            }
            None
        }
    }
}

fn token_to_string(tok: &Token) -> String {
    match tok {
        Token::Single(s) => s.clone(),
        Token::Interp(pcs) => {
            let mut s = String::new();
            for p in pcs {
                match p {
                    Piece::Text(t) | Piece::QText(t) => s.push_str(t),
                    Piece::Var(v) => {
                        s.push('$');
                        s.push_str(v);
                    }
                }
            }
            s
        }
    }
}

fn read_var(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<(String, usize)> {
    let mut consumed = 0usize;
    if let Some('{') = chars.peek().copied() {
        let _ = chars.next();
        consumed += 1;
        let mut name = String::new();
        while let Some(&ch) = chars.peek() {
            consumed += 1;
            let _ = chars.next();
            if ch == '}' {
                break;
            }
            name.push(ch);
        }
        if name.is_empty() {
            return None;
        }
        Some((name, consumed))
    } else {
        let mut name = String::new();
        while let Some(&ch) = chars.peek() {
            if ch == '_' || ch.is_ascii_alphanumeric() {
                name.push(ch);
                let _ = chars.next();
                consumed += 1;
            } else {
                break;
            }
        }
        if name.is_empty() {
            None
        } else {
            Some((name, consumed))
        }
    }
}

fn parse_var_ref(s: &str) -> Option<String> {
    if let Some(rest) = s.strip_prefix("${") {
        if let Some(end) = rest.find('}') {
            let name = &rest[..end];
            if is_ident(name) {
                return Some(vname(name));
            }
        }
        None
    } else if let Some(name) = s.strip_prefix('$') {
        if is_ident(name) {
            Some(vname(name))
        } else {
            None
        }
    } else {
        None
    }
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn is_int_literal(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn is_float_literal(s: &str) -> bool {
    s.contains('.')
        && s.chars().filter(|c| *c == '.').count() == 1
        && s.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn json_string_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn double_quote_with_escapes(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn escape_in_double_quotes(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(ch),
        }
    }
    out
}
/* =============================================================================
Builtin mapping
============================================================================= */

/// Command → (IronShell target, kind). Only commands whose IronShell target
/// takes the same arguments in the same order are listed; the rest are handed
/// to `sh(...)` and refused by name when `sh` is not allowed. Every target is
/// checked to resolve by `tests/round30_transpile.rs`.
pub const ZSH_MAP: &[(&str, &str, Kind)] = &[
    ("pwd", "pwd", Kind::Value),
    ("cd", "cd", Kind::Quiet),
    ("exit", "exit", Kind::Quiet),
    ("clear", "clear", Kind::Quiet),
    ("ls", "ls", Kind::Value),
    ("cat", "cat", Kind::Value),
    ("touch", "touch", Kind::Quiet),
    ("mkdir", "mkdir", Kind::Quiet),
    ("rmdir", "rmdir", Kind::Quiet),
    ("cp", "cp", Kind::Quiet),
    ("mv", "mv", Kind::Quiet),
    ("rm", "rm", Kind::Quiet),
    ("stat", "stat", Kind::Value),
    ("hostname", "hostname", Kind::Value),
    ("whoami", "whoami", Kind::Value),
    ("uptime", "uptime", Kind::Value),
    ("ps", "ps", Kind::Value),
    ("kill", "kill", Kind::Quiet),
    ("sleep", "sleep", Kind::Quiet),
    ("arch", "platform.arch", Kind::Value),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command() {
        let r = transpile_zsh_to_ae("echo hello").unwrap();
        assert!(r.contains("print(\"hello\")"), "got: {}", r);
    }

    #[test]
    fn test_assignment() {
        let r = transpile_zsh_to_ae("FOO=bar").unwrap();
        assert!(r.contains("let FOO ="), "got: {}", r);
    }

    #[test]
    fn test_array_literal() {
        let r = transpile_zsh_to_ae("arr=(a b c)").unwrap();
        assert!(r.contains("let sh_arr = ["), "got: {}", r);
    }

    #[test]
    fn test_setopt_comment() {
        let r = transpile_zsh_to_ae("setopt extended_glob").unwrap();
        assert!(r.contains("// [zsh]"), "got: {}", r);
    }

    #[test]
    fn test_pipeline() {
        let r = transpile_zsh_to_ae("ls | grep foo").unwrap();
        assert!(r.contains("ls()") && r.contains("|"), "got: {}", r);
    }

    #[test]
    fn test_export() {
        let r = transpile_zsh_to_ae("export PATH=/usr/bin").unwrap();
        assert!(r.contains("let PATH ="), "got: {}", r);
    }

    #[test]
    fn test_local_var() {
        let r = transpile_zsh_to_ae("local x=42").unwrap();
        assert!(r.contains("let x = 42;"), "got: {}", r);
    }

    #[test]
    fn test_if_block() {
        let script = "if true; then\n  echo yes\nfi";
        let r = transpile_zsh_to_ae(script).unwrap();
        assert!(r.contains("sh([\"zsh\",\"-c\""), "got: {}", r);
    }

    #[test]
    fn test_comment_preserved() {
        let r = transpile_zsh_to_ae("# hello world").unwrap();
        assert!(r.contains("// hello world"), "got: {}", r);
    }

    #[test]
    fn test_typeset_assoc() {
        let r = transpile_zsh_to_ae("typeset -A mymap").unwrap();
        assert!(r.contains("let mymap = {};"), "got: {}", r);
    }

    #[test]
    fn splits_semicolons_outside_quotes() {
        assert_eq!(split_statements("echo a; echo b"), vec!["echo a", "echo b"]);
        assert_eq!(split_statements("echo 'a;b'"), vec!["echo 'a;b'"]);
        assert_eq!(
            split_statements("echo \"a;b\"; ls"),
            vec!["echo \"a;b\"", "ls"]
        );
    }

    #[test]
    fn routes_plain_posix_to_the_engine() {
        match route("echo a; echo b") {
            Ok(Route::Posix(p)) => assert_eq!(p, "echo a; echo b"),
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn setopt_becomes_a_no_op_and_the_rest_runs() {
        match route("setopt nullglob; echo hi") {
            Ok(Route::Posix(p)) => assert_eq!(p, ":; echo hi"),
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn print_l_becomes_printf() {
        match route("print -l a b") {
            Ok(Route::Posix(p)) => assert_eq!(p, "printf '%s\\n' a b"),
            other => panic!("got {:?}", other),
        }
    }

    #[test]
    fn unknown_setopt_is_refused_by_name() {
        let r = route("setopt shwordsplit").unwrap_err();
        assert_eq!(r.line, 1);
        assert!(r.construct.contains("shwordsplit"), "got {:?}", r);
    }

    #[test]
    fn transpiler_refuses_a_flag_it_cannot_map() {
        let e = transpile_zsh_to_ae("ls -la").unwrap_err().to_string();
        assert!(e.contains("-la"), "got: {}", e);
    }

    #[test]
    fn an_unmapped_command_is_reported_with_its_line() {
        let (_, fb) = transpile_zsh_with_report("echo a\ngrep b f").unwrap();
        assert_eq!(fb.len(), 1);
        assert_eq!(fb[0].line, 2);
        assert!(fb[0].construct.contains("grep"), "got {:?}", fb);
    }
}

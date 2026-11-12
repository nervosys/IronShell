//! A POSIX-shell subset, executed in-process: what `ish -b` runs before it
//! falls back to the transpiler.
//!
//! E6 (`benches/agentic/bashcompat.mjs`) asked how much of the shell an agent
//! actually emits `ish -b` could run. The answer was 2 of 32: the transpiler
//! produced IronShell that did not parse, and everything with a loop, a
//! variable or `&&` was handed to `bash -lc` -- which bypasses every safety
//! property the rest of the shell has, because the effect gate sees one `sh`
//! call instead of the commands the script ran.
//!
//! The first thing a model writes is bash (`docs/LANGUAGE_FIRST_PRINCIPLES.md`
//! §3: borrow the syntax, own the semantics). So this runs the ordinary subset
//! itself: pipelines, `&&`/`||`/`;`, variables, `export`, `set -e`/`-u`/
//! `-o pipefail`, `$(…)`, quoting, globs, redirections,
//! `if`/`for`/`while`/`until`, `test`/`[`, and text-mode
//! versions of the utilities agents reach for, written to print what GNU
//! coreutils prints. A command it does not implement runs as that program,
//! directly -- never through a shell -- after `safety::guard_exec`, so agent
//! mode gates each one. A redirection that writes a file goes through the same
//! guard as `file.write`, jail included.
//!
//! What it does not parse (functions, `case`, arithmetic, here-documents,
//! subshells, background jobs) makes [`run`] return `None` before anything has
//! executed, and the caller falls back to the transpiler exactly as before.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;

// ════════════════════════════════════════════════════════════════════════
// Syntax
// ════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
enum Part {
    /// Literal text; `true` when it came from inside quotes (no globbing, no
    /// field splitting).
    Lit(String, bool),
    /// `$name` / `${name…}`: the name, an optional operator and its word, and
    /// whether the expansion was quoted.
    Var(String, Option<(String, Vec<Part>)>, bool),
    /// `${#name}`
    Len(String),
    /// `$(…)` or backquotes, and whether quoted.
    Cmd(String, bool),
}

type Word = Vec<Part>;

#[derive(Debug, Clone)]
enum Redir {
    /// `[n]>file`, `[n]>>file`
    Out { fd: u8, append: bool, target: Word },
    /// `<file`
    In(Word),
    /// `n>&m`
    Dup { from: u8, to: u8 },
    /// `&>file`
    Both(Word),
}

#[derive(Debug, Clone)]
enum Tok {
    Word(Word),
    Pipe,
    And,
    Or,
    Semi,
    DSemi,
    LParen,
    RParen,
    Nl,
    Redir(Redir),
    /// Where lexing stopped at a construct the subset refuses (see
    /// [`Lexer::tokens`]). The parser never gets past it.
    Bad,
}

#[derive(Debug, Clone)]
enum Cmd {
    Simple {
        assigns: Vec<(String, Word)>,
        words: Vec<Word>,
        redirs: Vec<Redir>,
    },
    If {
        branches: Vec<(List, List)>,
        otherwise: Option<List>,
        redirs: Vec<Redir>,
    },
    For {
        var: String,
        items: Vec<Word>,
        body: List,
        redirs: Vec<Redir>,
    },
    While {
        until: bool,
        cond: List,
        body: List,
        redirs: Vec<Redir>,
    },
    Group {
        body: List,
        redirs: Vec<Redir>,
    },
}

#[derive(Debug, Clone)]
struct Pipeline {
    negate: bool,
    cmds: Vec<Cmd>,
    /// The line the pipeline starts on, which an expansion error names.
    line: usize,
}

#[derive(Debug, Clone)]
struct AndOr {
    first: Pipeline,
    rest: Vec<(bool, Pipeline)>,
}

type List = Vec<AndOr>;

/// Why the subset declined a script. Never shown as an error: it is the signal
/// to fall back, or, for `ish -b`, the reason a refusal names.
#[derive(Debug)]
pub struct Unsupported {
    /// The construct, named as the user would name it: `case ... esac`. For a
    /// syntax error, the error itself: ``syntax error near unexpected token `fi'``.
    pub what: String,
    /// The character offset in the source where the construct starts, when the
    /// parser knows it.
    pub at: Option<usize>,
    /// Set when `what` is a syntax error, not an unsupported construct: the hint
    /// that says what the parser expected. A syntax error is wrong in every
    /// shell, so it is not a fallback case.
    pub syntax: Option<String>,
}

type PResult<T> = Result<T, Unsupported>;

fn unsupported<T>(what: &str) -> PResult<T> {
    Err(Unsupported {
        what: what.to_string(),
        at: None,
        syntax: None,
    })
}

/// [`unsupported`] placed at character offset `at`: the construct's own
/// position, not the start of the word it sits in.
fn unsupported_at<T>(what: &str, at: usize) -> PResult<T> {
    Err(Unsupported {
        what: what.to_string(),
        at: Some(at),
        syntax: None,
    })
}

/// A syntax error for input that ends inside a construct, placed at the
/// construct's opening character. It is wrong in every shell, so it is E_PARSE
/// and not a refusal of the subset: `syntax` says what closes it.
fn unterminated<T>(what: &str, open: usize, closing: &str) -> PResult<T> {
    Err(Unsupported {
        what: what.to_string(),
        at: Some(open),
        syntax: Some(format!("the construct is never closed: add {closing}")),
    })
}

/// `e` moved from the text it was found in (an operand or a substitution's
/// body, whose offsets count from `base`) to the outer source. An error with a
/// position of its own is at `base + offset`; one without is placed at
/// `fallback`, the expansion's own `$`.
fn relocate(mut e: Unsupported, base: usize, fallback: usize) -> Unsupported {
    e.at = Some(e.at.map_or(fallback, |off| base + off));
    e
}

/// Parses the body of a command substitution, which runs later as a script of
/// its own. A construct the subset refuses inside it is refused with the whole
/// script, before anything runs: its error is returned for the caller to place.
fn check_body(body: &str) -> PResult<()> {
    parse(body).map(|_| ())
}

// ── lexer ────────────────────────────────────────────────────────────────

struct Lexer<'a> {
    s: &'a [char],
    i: usize,
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

impl<'a> Lexer<'a> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }
    fn peek_at(&self, n: usize) -> Option<char> {
        self.s.get(self.i + n).copied()
    }

    /// The tokens of the source, the character offset each one starts at, and
    /// the first lexical refusal. Lexing stops at a refusal and leaves a
    /// [`Tok::Bad`] at its offset, so the parser still sees the constructs
    /// before it: a subshell at line 3 is named ahead of a `&` at line 5.
    fn tokens(mut self) -> (Vec<Tok>, Vec<usize>, Option<Unsupported>) {
        let mut out = Vec::new();
        let mut at = Vec::new();
        while let Some(c) = self.peek() {
            let start = self.i;
            let before = out.len();
            match self.token(c, &mut out) {
                Ok(()) => at.extend(std::iter::repeat_n(start, out.len() - before)),
                Err(mut e) => {
                    let bad_at = *e.at.get_or_insert(start);
                    out.truncate(before);
                    out.push(Tok::Bad);
                    // At the refusal's own offset, which may be inside the
                    // word (`n=$((` refuses at the `$`): placed at the word's
                    // start, the parser's "unexpected operator" on this token
                    // came first and hid the real refusal.
                    at.push(bad_at);
                    return (out, at, Some(e));
                }
            }
        }
        (out, at, None)
    }

    /// One token, or the whitespace or comment before it, at `self.i`.
    fn token(&mut self, c: char, out: &mut Vec<Tok>) -> PResult<()> {
        match c {
            ' ' | '\t' | '\r' => self.i += 1,
            '\\' if self.peek_at(1) == Some('\n') => self.i += 2,
            '#' => {
                while let Some(c) = self.peek() {
                    if c == '\n' {
                        break;
                    }
                    self.i += 1;
                }
            }
            '\n' => {
                self.i += 1;
                out.push(Tok::Nl);
            }
            ';' => {
                if self.peek_at(1) == Some(';') {
                    self.i += 2;
                    out.push(Tok::DSemi);
                } else {
                    self.i += 1;
                    out.push(Tok::Semi);
                }
            }
            '|' => {
                if self.peek_at(1) == Some('|') {
                    self.i += 2;
                    out.push(Tok::Or);
                } else if self.peek_at(1) == Some('&') {
                    return unsupported("|&");
                } else {
                    self.i += 1;
                    out.push(Tok::Pipe);
                }
            }
            '&' => {
                if self.peek_at(1) == Some('&') {
                    self.i += 2;
                    out.push(Tok::And);
                } else if self.peek_at(1) == Some('>') {
                    self.i += 2;
                    if self.peek() == Some('>') {
                        return unsupported("&>>");
                    }
                    let target = self.target()?;
                    out.push(Tok::Redir(Redir::Both(target)));
                } else {
                    return unsupported("background job");
                }
            }
            // A parenthesis is the parser's to refuse, where it knows whether
            // it opens a function definition, a subshell, or a case pattern.
            '(' => {
                self.i += 1;
                out.push(Tok::LParen);
            }
            ')' => {
                self.i += 1;
                out.push(Tok::RParen);
            }
            '<' | '>' => out.push(Tok::Redir(self.redir(None)?)),
            _ => {
                // A word; but digits immediately followed by a redirection
                // operator are that operator's descriptor.
                let start = self.i;
                let mut j = self.i;
                while self.s.get(j).is_some_and(|c| c.is_ascii_digit()) {
                    j += 1;
                }
                if j > start && matches!(self.s.get(j), Some('<') | Some('>')) {
                    let fd: u8 = self.s[start..j]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .map_err(|_| Unsupported {
                            what: "descriptor".into(),
                            at: None,
                            syntax: None,
                        })?;
                    self.i = j;
                    // A refusal of the redirection is at its operator, not at
                    // the descriptor digits before it.
                    let redir = self.redir(Some(fd)).map_err(|mut e| {
                        e.at.get_or_insert(j);
                        e
                    })?;
                    out.push(Tok::Redir(redir));
                    return Ok(());
                }
                out.push(Tok::Word(self.word()?));
            }
        }
        Ok(())
    }

    fn redir(&mut self, fd: Option<u8>) -> PResult<Redir> {
        let c = self.peek().unwrap_or(' ');
        self.i += 1;
        if c == '<' {
            match self.peek() {
                Some('<') => {
                    return if self.peek_at(1) == Some('<') {
                        unsupported("here-string (<<<)")
                    } else {
                        unsupported("here-document (<<)")
                    };
                }
                Some('(') => return unsupported("process substitution"),
                Some('&') | Some('>') => return unsupported("<& / <>"),
                _ => {}
            }
            if fd.is_some_and(|f| f != 0) {
                return unsupported("input descriptor");
            }
            return Ok(Redir::In(self.target()?));
        }
        let fd = fd.unwrap_or(1);
        let mut append = false;
        match self.peek() {
            Some('>') => {
                append = true;
                self.i += 1;
            }
            Some('&') => {
                self.i += 1;
                let d = self.peek();
                return match d {
                    Some(d @ '0'..='9') => {
                        self.i += 1;
                        Ok(Redir::Dup {
                            from: fd,
                            to: d as u8 - b'0',
                        })
                    }
                    _ => unsupported(">&word"),
                };
            }
            Some('(') => return unsupported("process substitution"),
            Some('|') => return unsupported(">|"),
            _ => {}
        }
        if fd > 2 {
            return unsupported("descriptor above 2");
        }
        Ok(Redir::Out {
            fd,
            append,
            target: self.target()?,
        })
    }

    fn target(&mut self) -> PResult<Word> {
        while matches!(self.peek(), Some(' ') | Some('\t')) {
            self.i += 1;
        }
        match self.peek() {
            None | Some('\n') | Some(';') | Some('|') | Some('&') | Some('<') | Some('>') => {
                Err(Unsupported {
                    what: "redirection without a target".into(),
                    at: None,
                    syntax: Some("a redirection needs a file name after its operator".into()),
                })
            }
            _ => self.word(),
        }
    }

    /// One shell word, up to an unquoted metacharacter.
    fn word(&mut self) -> PResult<Word> {
        let mut parts: Word = Vec::new();
        let mut lit = String::new();
        let flush = |lit: &mut String, parts: &mut Word| {
            if !lit.is_empty() {
                parts.push(Part::Lit(std::mem::take(lit), false));
            }
        };
        while let Some(c) = self.peek() {
            match c {
                ' ' | '\t' | '\r' | '\n' | ';' | '|' | '&' | '<' | '>' | '(' | ')' => break,
                '\\' => {
                    self.i += 1;
                    match self.peek() {
                        Some('\n') => self.i += 1,
                        Some(e) => {
                            flush(&mut lit, &mut parts);
                            parts.push(Part::Lit(e.to_string(), true));
                            self.i += 1;
                        }
                        None => lit.push('\\'),
                    }
                }
                '\'' => {
                    flush(&mut lit, &mut parts);
                    let open = self.i;
                    self.i += 1;
                    let mut q = String::new();
                    loop {
                        match self.peek() {
                            None => {
                                return unterminated("unterminated quote", open, "a `'`");
                            }
                            Some('\'') => {
                                self.i += 1;
                                break;
                            }
                            Some(ch) => {
                                q.push(ch);
                                self.i += 1;
                            }
                        }
                    }
                    parts.push(Part::Lit(q, true));
                }
                '"' => {
                    flush(&mut lit, &mut parts);
                    self.i += 1;
                    self.double_quoted(&mut parts)?;
                }
                '$' => {
                    flush(&mut lit, &mut parts);
                    self.dollar(&mut parts, false)?;
                }
                '`' => {
                    flush(&mut lit, &mut parts);
                    let body = self.backquote()?;
                    parts.push(Part::Cmd(body, false));
                }
                _ => {
                    lit.push(c);
                    self.i += 1;
                }
            }
        }
        flush(&mut lit, &mut parts);
        if parts.is_empty() {
            return unsupported("empty word");
        }
        Ok(parts)
    }

    fn double_quoted(&mut self, parts: &mut Word) -> PResult<()> {
        // Called just past the opening quote.
        let open = self.i.saturating_sub(1);
        let mut lit = String::new();
        // An empty "" is still an argument.
        let mut any = false;
        loop {
            match self.peek() {
                None => return unterminated("unterminated quote", open, "a `\"`"),
                Some('"') => {
                    self.i += 1;
                    break;
                }
                Some('\\') => {
                    self.i += 1;
                    match self.peek() {
                        Some(e @ ('"' | '\\' | '$' | '`')) => {
                            lit.push(e);
                            self.i += 1;
                        }
                        Some('\n') => self.i += 1,
                        Some(e) => {
                            lit.push('\\');
                            lit.push(e);
                            self.i += 1;
                        }
                        None => return unterminated("unterminated quote", open, "a `\"`"),
                    }
                }
                Some('$') => {
                    if !lit.is_empty() || !any {
                        parts.push(Part::Lit(std::mem::take(&mut lit), true));
                    }
                    self.dollar(parts, true)?;
                    any = true;
                }
                Some('`') => {
                    if !lit.is_empty() {
                        parts.push(Part::Lit(std::mem::take(&mut lit), true));
                    }
                    let body = self.backquote()?;
                    parts.push(Part::Cmd(body, true));
                    any = true;
                }
                Some(c) => {
                    lit.push(c);
                    self.i += 1;
                }
            }
        }
        if !lit.is_empty() || !any {
            parts.push(Part::Lit(lit, true));
        }
        Ok(())
    }

    fn backquote(&mut self) -> PResult<String> {
        let open = self.i;
        self.i += 1;
        let mut body = String::new();
        // The source offset of each character of `body`: its escapes are
        // gone, so a refusal inside it is mapped back through this.
        let mut from: Vec<usize> = Vec::new();
        loop {
            match self.peek() {
                None => return unterminated("unterminated backquote", open, "a closing backquote"),
                Some('`') => {
                    let close = self.i;
                    self.i += 1;
                    check_body(&body).map_err(|mut e| {
                        e.at = Some(match e.at {
                            Some(off) => from.get(off).copied().unwrap_or(close),
                            None => open,
                        });
                        e
                    })?;
                    return Ok(body);
                }
                Some('\\') if matches!(self.peek_at(1), Some('`') | Some('\\') | Some('$')) => {
                    body.push(self.peek_at(1).unwrap_or('\\'));
                    from.push(self.i + 1);
                    self.i += 2;
                }
                Some(c) => {
                    body.push(c);
                    from.push(self.i);
                    self.i += 1;
                }
            }
        }
    }

    /// `$…` at `self.i`.
    fn dollar(&mut self, parts: &mut Word, quoted: bool) -> PResult<()> {
        let dollar_at = self.i;
        self.i += 1;
        match self.peek() {
            Some('(') => {
                if self.peek_at(1) == Some('(') {
                    return unsupported_at("arithmetic expansion $((...))", dollar_at);
                }
                self.i += 1;
                let start = self.i;
                let mut depth = 1;
                let mut in_single = false;
                let mut in_double = false;
                while let Some(c) = self.peek() {
                    match c {
                        '\\' if !in_single => self.i += 1,
                        '\'' if !in_double => in_single = !in_single,
                        '"' if !in_single => in_double = !in_double,
                        '(' if !in_single && !in_double => depth += 1,
                        ')' if !in_single && !in_double => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    self.i += 1;
                }
                if self.peek() != Some(')') {
                    return unterminated("unterminated $(", dollar_at, "a `)`");
                }
                let body: String = self.s[start..self.i].iter().collect();
                // The body's offsets count from `start`.
                check_body(&body).map_err(|e| relocate(e, start, dollar_at))?;
                self.i += 1;
                parts.push(Part::Cmd(body, quoted));
            }
            Some('{') => {
                self.i += 1;
                if self.peek() == Some('#') && self.peek_at(1).is_some_and(is_name_start) {
                    self.i += 1;
                    let name = self.name();
                    if self.peek() != Some('}') {
                        return unsupported_at("${#...} form", dollar_at);
                    }
                    self.i += 1;
                    parts.push(Part::Len(name));
                    return Ok(());
                }
                let name = match self.peek() {
                    Some(c) if is_name_start(c) => self.name(),
                    // `${10}` is one parameter: every digit up to the brace.
                    // Without braces `$10` is `$1` followed by `0`.
                    Some(c) if c.is_ascii_digit() => {
                        let mut digits = String::new();
                        while let Some(d) = self.peek().filter(|d| d.is_ascii_digit()) {
                            digits.push(d);
                            self.i += 1;
                        }
                        digits
                    }
                    Some(c @ ('?' | '$' | '#')) => {
                        self.i += 1;
                        c.to_string()
                    }
                    _ => return unsupported_at("${...} form", dollar_at),
                };
                if self.peek() == Some('}') {
                    self.i += 1;
                    parts.push(Part::Var(name, None, quoted));
                    return Ok(());
                }
                let mut op = String::new();
                if self.peek() == Some(':') {
                    op.push(':');
                    self.i += 1;
                }
                match self.peek() {
                    Some(c @ ('-' | '=' | '+' | '?')) => {
                        op.push(c);
                        self.i += 1;
                    }
                    // `%`, `%%`, `#`, `##`: the word is a glob pattern.
                    Some(c @ ('%' | '#')) if op.is_empty() => {
                        op.push(c);
                        self.i += 1;
                        if self.peek() == Some(c) {
                            op.push(c);
                            self.i += 1;
                        }
                    }
                    _ => {
                        let named = param_operator(op.starts_with(':'), &self.s[self.i..]);
                        return unsupported_at(&format!("${{...}} operator `{named}`"), dollar_at);
                    }
                }
                // The operator's word, up to the matching brace.
                let start = self.i;
                let mut depth = 1;
                while let Some(c) = self.peek() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    self.i += 1;
                }
                if self.peek() != Some('}') {
                    return unterminated("unterminated ${", dollar_at, "a `}`");
                }
                let inner: Vec<char> = self.s[start..self.i].to_vec();
                self.i += 1;
                let word = if inner.is_empty() {
                    vec![Part::Lit(String::new(), true)]
                } else {
                    let mut sub = Lexer { s: &inner, i: 0 };
                    let mut w = Vec::new();
                    // Treat the operand like double-quoted text when the
                    // expansion itself is quoted, else as a word.
                    if quoted {
                        let mut chars = inner.clone();
                        chars.push('"');
                        let mut q = Lexer { s: &chars, i: 0 };
                        q.double_quoted(&mut w)
                            .map_err(|e| relocate(e, start, dollar_at))?;
                    } else {
                        // The operand runs to the closing brace, blanks
                        // included (`${1:?usage: f COLUMN}`), as in bash; its
                        // words are kept apart by the blanks between them.
                        w = sub.word().map_err(|e| relocate(e, start, dollar_at))?;
                        while sub.i < inner.len() {
                            let blanks: String = inner[sub.i..]
                                .iter()
                                .take_while(|c| matches!(c, ' ' | '\t'))
                                .collect();
                            if blanks.is_empty() {
                                return unsupported_at("${...} operand", dollar_at);
                            }
                            sub.i += blanks.chars().count();
                            w.push(Part::Lit(blanks, false));
                            let before = sub.i;
                            w.extend(sub.word().map_err(|e| relocate(e, start, dollar_at))?);
                            if sub.i == before && sub.i < inner.len() {
                                return unsupported_at("${...} operand", dollar_at);
                            }
                        }
                    }
                    w
                };
                parts.push(Part::Var(name, Some((op, word)), quoted));
            }
            Some(c) if is_name_start(c) => {
                let name = self.name();
                parts.push(Part::Var(name, None, quoted));
            }
            Some(c @ ('?' | '$' | '#' | '0'..='9' | '@' | '*' | '!' | '-')) => {
                self.i += 1;
                // `$@` and `$*` are the positional parameters; `$!` and `$-`
                // are not in the subset.
                if matches!(c, '!' | '-') {
                    return unsupported_at("special parameter", dollar_at);
                }
                parts.push(Part::Var(c.to_string(), None, quoted));
            }
            _ => parts.push(Part::Lit("$".into(), quoted)),
        }
        Ok(())
    }

    fn name(&mut self) -> String {
        let mut n = String::new();
        while let Some(c) = self.peek() {
            if is_name_char(c) {
                n.push(c);
                self.i += 1;
            } else {
                break;
            }
        }
        n
    }
}

// ── parser ───────────────────────────────────────────────────────────────

struct Parser {
    toks: Vec<Tok>,
    /// The character offset each token starts at.
    at: Vec<usize>,
    i: usize,
    /// The constructs open at the current token, innermost last, each as its
    /// keyword and the offset it starts at. Named in syntax-error hints.
    open: Vec<(&'static str, usize)>,
    /// The character offset of every newline, for line numbers.
    newlines: Vec<usize>,
    /// The length of the source: where the end of input is.
    end: usize,
}

/// The keyword that closes a construct opened by `kw`.
fn closer_of(kw: &str) -> &'static str {
    match kw {
        "if" => "fi",
        "{" => "}",
        _ => "done",
    }
}

/// A token as bash names it in a syntax error.
fn token_text(t: &Tok) -> String {
    match t {
        Tok::Word(w) => w
            .iter()
            .map(|p| match p {
                Part::Lit(s, _) => s.clone(),
                Part::Var(name, _, _) => format!("${name}"),
                Part::Len(name) => format!("${{#{name}}}"),
                Part::Cmd(..) => "$(...)".to_string(),
            })
            .collect(),
        Tok::Pipe => "|".into(),
        Tok::And => "&&".into(),
        Tok::Or => "||".into(),
        Tok::Semi => ";".into(),
        Tok::DSemi => ";;".into(),
        Tok::LParen => "(".into(),
        Tok::RParen => ")".into(),
        Tok::Nl => "newline".into(),
        Tok::Redir(_) => "redirection".into(),
        Tok::Bad => "&".into(),
    }
}

/// The operator of a `${name...}` form that the subset does not run, as its
/// refusal names it: `:offset` for `${name:2}`, else the operator's own
/// characters (`%`, `##`, `//`, `^`, `,`, `@`).
fn param_operator(colon: bool, rest: &[char]) -> String {
    if colon {
        return ":offset".into();
    }
    match rest {
        [a, b, ..] if a == b && matches!(*a, '%' | '#' | '/' | '^' | ',') => format!("{a}{b}"),
        [a, ..] => a.to_string(),
        [] => String::new(),
    }
}

/// What a stray operator token is, as the construct it belongs to.
fn operator_what(t: &Tok) -> &'static str {
    match t {
        Tok::LParen | Tok::RParen => "subshell",
        Tok::DSemi => "case ... esac",
        _ => "unexpected operator",
    }
}

/// What a reserved word that starts a construct the subset lacks is called.
fn keyword_what(k: &str) -> String {
    match k {
        "case" | "esac" => "case ... esac".into(),
        "function" => "function definition".into(),
        "[[" | "]]" => "[[ ... ]]".into(),
        "select" => "select loop".into(),
        _ => format!("`{k}`"),
    }
}

/// The text of a word that is a single unquoted literal, which is what a
/// reserved word must be to count as one.
fn plain(w: &Word) -> Option<&str> {
    match w.as_slice() {
        [Part::Lit(s, false)] => Some(s),
        _ => None,
    }
}

const RESERVED: &[&str] = &[
    "if", "then", "elif", "else", "fi", "for", "in", "do", "done", "while", "until", "{", "}", "!",
    "case", "esac", "function", "select", "[[", "]]",
];

impl Parser {
    /// The offset of the token at `self.i`, or the end of the source.
    fn offset(&self) -> usize {
        self.at.get(self.i).copied().unwrap_or(self.end)
    }

    /// The 1-based line of character offset `at`.
    fn line_of(&self, at: usize) -> usize {
        self.newlines.partition_point(|&n| n < at) + 1
    }

    /// A syntax error at the current token, with `hint` saying what was wanted.
    fn syntax<T>(&self, hint: String) -> PResult<T> {
        let what = match self.peek() {
            None => "syntax error: unexpected end of file".to_string(),
            Some(t) => format!("syntax error near unexpected token `{}'", token_text(t)),
        };
        Err(Unsupported {
            what,
            at: Some(self.offset()),
            syntax: Some(hint),
        })
    }

    /// A reserved word that closes nothing open, where a command starts. The
    /// hint names the construct it closes, when one is open.
    fn stray<T>(&self) -> PResult<T> {
        let hint = match self.open.last() {
            Some(&(kw, at)) => {
                let line = self.line_of(at);
                let close = closer_of(kw);
                if self.peek().is_none() {
                    format!("the `{kw}` at line {line} needs a closing `{close}`")
                } else {
                    format!("the `{kw}` at line {line} is closed by `{close}`")
                }
            }
            None => "there is no open `if`, loop or `{` for it to close".to_string(),
        };
        self.syntax(hint)
    }

    /// A `)` where a command starts or ends with nothing open for it to close.
    /// A subshell's `(` is refused as a construct; its `)` alone is a syntax
    /// error in every shell.
    fn stray_paren<T>(&self) -> PResult<T> {
        self.syntax("there is no open `(` for this `)` to close".to_string())
    }

    /// The keyword `kw` was wanted here, for the construct open at this point.
    fn expected<T>(&self, kw: &str) -> PResult<T> {
        let hint = match self.open.last() {
            Some(&(open, at)) => {
                format!(
                    "expected `{kw}` for the `{open}` at line {}",
                    self.line_of(at)
                )
            }
            None => format!("expected `{kw}`"),
        };
        self.syntax(hint)
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }

    fn peek_keyword(&self) -> Option<&str> {
        match self.peek() {
            Some(Tok::Word(w)) => plain(w).filter(|s| RESERVED.contains(s)),
            _ => None,
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Some(Tok::Nl) | Some(Tok::Semi)) {
            self.i += 1;
        }
    }

    fn expect_keyword(&mut self, kw: &str) -> PResult<()> {
        self.skip_newlines();
        if self.peek_keyword() == Some(kw) {
            self.i += 1;
            Ok(())
        } else {
            self.expected(kw)
        }
    }

    /// A list, stopping before any of `stop` (reserved words) or the end.
    fn list(&mut self, stop: &[&str]) -> PResult<List> {
        let mut out = Vec::new();
        loop {
            self.skip_newlines();
            match self.peek() {
                None => break,
                Some(Tok::Word(_)) => {
                    if let Some(k) = self.peek_keyword() {
                        if stop.contains(&k) {
                            break;
                        }
                    }
                }
                Some(Tok::Redir(_)) => {}
                Some(Tok::RParen) => return self.stray_paren(),
                Some(t) => return unsupported(operator_what(t)),
            }
            out.push(self.and_or()?);
            match self.peek() {
                None | Some(Tok::Nl) | Some(Tok::Semi) => {}
                Some(Tok::Word(w)) if plain(w).is_some_and(|k| stop.contains(&k)) => {}
                _ => return unsupported("unexpected token after command"),
            }
        }
        Ok(out)
    }

    fn and_or(&mut self) -> PResult<AndOr> {
        let first = self.pipeline()?;
        let mut rest = Vec::new();
        loop {
            let and = match self.peek() {
                Some(Tok::And) => true,
                Some(Tok::Or) => false,
                _ => break,
            };
            self.i += 1;
            while matches!(self.peek(), Some(Tok::Nl)) {
                self.i += 1;
            }
            rest.push((and, self.pipeline()?));
        }
        Ok(AndOr { first, rest })
    }

    fn pipeline(&mut self) -> PResult<Pipeline> {
        let line = self.line_of(self.offset());
        let mut negate = false;
        if self.peek_keyword() == Some("!") {
            negate = true;
            self.i += 1;
        }
        let mut cmds = vec![self.command()?];
        while matches!(self.peek(), Some(Tok::Pipe)) {
            self.i += 1;
            while matches!(self.peek(), Some(Tok::Nl)) {
                self.i += 1;
            }
            cmds.push(self.command()?);
        }
        Ok(Pipeline { negate, cmds, line })
    }

    fn trailing_redirs(&mut self) -> Vec<Redir> {
        let mut r = Vec::new();
        while let Some(Tok::Redir(x)) = self.peek() {
            r.push(x.clone());
            self.i += 1;
        }
        r
    }

    fn command(&mut self) -> PResult<Cmd> {
        match self.peek_keyword() {
            Some("if") => {
                let at = self.offset();
                self.open.push(("if", at));
                self.i += 1;
                let mut branches = Vec::new();
                let cond = self.list(&["then"])?;
                self.expect_keyword("then")?;
                let body = self.list(&["elif", "else", "fi"])?;
                branches.push((cond, body));
                let mut otherwise = None;
                loop {
                    self.skip_newlines();
                    match self.peek_keyword() {
                        Some("elif") => {
                            self.i += 1;
                            let c = self.list(&["then"])?;
                            self.expect_keyword("then")?;
                            let b = self.list(&["elif", "else", "fi"])?;
                            branches.push((c, b));
                        }
                        Some("else") => {
                            self.i += 1;
                            otherwise = Some(self.list(&["fi"])?);
                        }
                        Some("fi") => {
                            self.i += 1;
                            break;
                        }
                        _ => return self.stray(),
                    }
                }
                self.open.pop();
                Ok(Cmd::If {
                    branches,
                    otherwise,
                    redirs: self.trailing_redirs(),
                })
            }
            Some("for") => {
                let at = self.offset();
                self.open.push(("for", at));
                self.i += 1;
                let var = match self.peek() {
                    Some(Tok::Word(w)) => match plain(w) {
                        Some(n)
                            if n.chars().next().is_some_and(is_name_start)
                                && n.chars().all(is_name_char) =>
                        {
                            n.to_string()
                        }
                        _ => return unsupported("for variable"),
                    },
                    _ => return unsupported("for variable"),
                };
                self.i += 1;
                while matches!(self.peek(), Some(Tok::Nl)) {
                    self.i += 1;
                }
                if self.peek_keyword() != Some("in") {
                    return unsupported("for without in");
                }
                self.i += 1;
                let mut items = Vec::new();
                while let Some(Tok::Word(w)) = self.peek() {
                    if plain(w) == Some("do") {
                        break;
                    }
                    items.push(w.clone());
                    self.i += 1;
                }
                self.expect_keyword("do")?;
                let body = self.list(&["done"])?;
                self.expect_keyword("done")?;
                self.open.pop();
                Ok(Cmd::For {
                    var,
                    items,
                    body,
                    redirs: self.trailing_redirs(),
                })
            }
            Some(k @ ("while" | "until")) => {
                let until = k == "until";
                let kw: &'static str = if until { "until" } else { "while" };
                let at = self.offset();
                self.open.push((kw, at));
                self.i += 1;
                let cond = self.list(&["do"])?;
                self.expect_keyword("do")?;
                let body = self.list(&["done"])?;
                self.expect_keyword("done")?;
                self.open.pop();
                Ok(Cmd::While {
                    until,
                    cond,
                    body,
                    redirs: self.trailing_redirs(),
                })
            }
            Some("{") => {
                let at = self.offset();
                self.open.push(("{", at));
                self.i += 1;
                let body = self.list(&["}"])?;
                self.expect_keyword("}")?;
                self.open.pop();
                Ok(Cmd::Group {
                    body,
                    redirs: self.trailing_redirs(),
                })
            }
            // A closer that reaches a command's start has nothing to close.
            Some("fi" | "done" | "then" | "else" | "elif" | "do" | "esac" | "}") => self.stray(),
            Some(k) if !matches!(k, "!") => unsupported(&keyword_what(k)),
            _ => self.simple(),
        }
    }

    fn simple(&mut self) -> PResult<Cmd> {
        let start = self.offset();
        let mut assigns = Vec::new();
        let mut words: Vec<Word> = Vec::new();
        let mut redirs = Vec::new();
        loop {
            match self.peek() {
                Some(Tok::Word(w)) => {
                    if words.is_empty() {
                        if let Some(a) = assignment(w) {
                            assigns.push(a);
                            self.i += 1;
                            continue;
                        }
                        if let Some(Part::Lit(s, false)) = w.first() {
                            if s.ends_with("()") || s == "function" {
                                return unsupported_at("function definition", start);
                            }
                            if s == "[[" {
                                return unsupported("[[");
                            }
                        }
                    }
                    words.push(w.clone());
                    self.i += 1;
                }
                Some(Tok::Redir(r)) => {
                    redirs.push(r.clone());
                    self.i += 1;
                }
                // `greet() {`: the name is where the definition starts.
                Some(Tok::LParen)
                    if words.len() == 1 && assigns.is_empty() && redirs.is_empty() =>
                {
                    return unsupported_at("function definition", start);
                }
                // `name=(...)`: the assignment with an empty value is the word
                // before the `(`. Arrays are not in the subset.
                Some(Tok::LParen)
                    if words.is_empty()
                        && assigns.last().is_some_and(
                            |(_, v)| matches!(v.as_slice(), [Part::Lit(s, _)] if s.is_empty()),
                        ) =>
                {
                    return unsupported_at("array assignment", start);
                }
                Some(Tok::RParen) => return self.stray_paren(),
                Some(t @ (Tok::LParen | Tok::DSemi)) => {
                    return unsupported(operator_what(t));
                }
                _ => break,
            }
        }
        if assigns.is_empty() && words.is_empty() && redirs.is_empty() {
            return unsupported("empty command");
        }
        // A shell builtin the subset does not run is refused here, before any
        // command runs. Left alone it would be sent to PATH as a program.
        if let Some(what) = unsupported_builtin(&words) {
            return Err(Unsupported {
                what,
                at: Some(start),
                syntax: None,
            });
        }
        Ok(Cmd::Simple {
            assigns,
            words,
            redirs,
        })
    }
}

/// Shell builtins of POSIX and bash that the subset does not run. Each would
/// otherwise run as a program of the same name, or not at all. `set` and
/// `export` are checked for their options in [`unsupported_builtin`].
const UNSUPPORTED_BUILTINS: &[&str] = &[
    ".",
    "source",
    "eval",
    "exec",
    "trap",
    "return",
    "getopts",
    "local",
    "alias",
    "unalias",
    "readonly",
    "declare",
    "typeset",
    "let",
    "command",
    "builtin",
    "type",
    "hash",
    "umask",
    "ulimit",
    "wait",
    "jobs",
    "fg",
    "bg",
    "times",
    "fc",
    "newgrp",
    "shopt",
    "enable",
    "pushd",
    "popd",
    "dirs",
    "mapfile",
    "readarray",
    "help",
    "caller",
    "disown",
];

/// The construct a command word names when the subset does not run it: a
/// builtin in [`UNSUPPORTED_BUILTINS`], a `set` option the subset does not run
/// (see [`set_refusal`]), or `export` with an option. `None` for every other
/// command.
fn unsupported_builtin(words: &[Word]) -> Option<String> {
    let name = plain(words.first()?)?;
    let arg = words.get(1).and_then(plain);
    match name {
        "set" => set_refusal(&words[1..]),
        "export" => arg
            .filter(|a| a.len() > 1 && a.starts_with('-'))
            .map(|a| format!("`export {a}`")),
        n if UNSUPPORTED_BUILTINS.contains(&n) => Some(format!("`{n}` builtin")),
        _ => None,
    }
}

/// A `set` option the subset runs.
#[derive(Clone, Copy)]
enum SetOpt {
    Errexit,
    Nounset,
    Pipefail,
}

/// What one `set` command does: the options it turns on (`true`) or off, and
/// the new positional parameters when it has `--`.
struct SetArgs {
    changes: Vec<(SetOpt, bool)>,
    positional: Option<Vec<String>>,
}

/// The arguments of `set` the subset runs: `-e`/`+e`, `-u`/`+u`, and `-o`/`+o`
/// with `errexit`, `nounset` or `pipefail`, in clusters too (`-eu`, `-euo
/// pipefail`). `--` ends the options, and what follows is the new parameters.
/// Any other option or argument is an `Err` naming it, as the refusal does.
fn parse_set(args: &[String]) -> Result<SetArgs, String> {
    let mut out = SetArgs {
        changes: Vec::new(),
        positional: None,
    };
    let mut k = 0;
    while k < args.len() {
        let a = args[k].as_str();
        if a == "--" {
            out.positional = Some(args[k + 1..].to_vec());
            break;
        }
        let sign = a
            .chars()
            .next()
            .filter(|c| (*c == '-' || *c == '+') && a.len() > 1);
        let Some(sign) = sign else {
            return Err(format!("`set {a}`"));
        };
        let on = sign == '-';
        for c in a[1..].chars() {
            let opt = match c {
                'e' => SetOpt::Errexit,
                'u' => SetOpt::Nounset,
                'o' => {
                    // `-o name` takes the next argument as its name.
                    k += 1;
                    match args.get(k).map(String::as_str) {
                        Some("errexit") => SetOpt::Errexit,
                        Some("nounset") => SetOpt::Nounset,
                        Some("pipefail") => SetOpt::Pipefail,
                        Some(name) => return Err(format!("`set {sign}o {name}`")),
                        None => return Err(format!("`set {sign}o`")),
                    }
                }
                _ => return Err(format!("`set {sign}{c}`")),
            };
            out.changes.push((opt, on));
        }
        k += 1;
    }
    Ok(out)
}

/// Why a `set` command is refused before anything runs, or `None` when the
/// subset runs it. Only the options are looked at: the words after `--` are
/// the new parameters, whatever they are. A `set` with no arguments lists the
/// variables, which the subset does not do.
fn set_refusal(args: &[Word]) -> Option<String> {
    if args.is_empty() {
        return Some("`set` with no options".into());
    }
    let dashes = args.iter().position(|w| plain(w) == Some("--"));
    let mut words = Vec::new();
    for w in &args[..dashes.unwrap_or(args.len())] {
        match plain(w) {
            Some(s) => words.push(s.to_string()),
            None => return Some("`set` with options".into()),
        }
    }
    if dashes.is_some() {
        words.push("--".into());
    }
    parse_set(&words).err()
}

/// `NAME=value` at the start of a simple command.
fn assignment(w: &Word) -> Option<(String, Word)> {
    let Part::Lit(first, false) = w.first()? else {
        return None;
    };
    let eq = first.find('=')?;
    let name = &first[..eq];
    if name.is_empty()
        || !name.chars().next().is_some_and(is_name_start)
        || !name.chars().all(is_name_char)
    {
        return None;
    }
    if first[eq + 1..].starts_with('(') {
        return None;
    }
    let mut value: Word = Vec::new();
    let rest = &first[eq + 1..];
    if !rest.is_empty() {
        value.push(Part::Lit(rest.to_string(), false));
    }
    value.extend(w[1..].iter().cloned());
    if value.is_empty() {
        value.push(Part::Lit(String::new(), true));
    }
    Some((name.to_string(), value))
}

/// The 1-based line and column of character offset `at` in `src`.
fn line_col(src: &str, at: usize) -> (usize, usize) {
    let before: String = src.chars().take(at).collect();
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |s| s.chars().count()) + 1;
    (line, column)
}

fn parse(src: &str) -> PResult<List> {
    let chars: Vec<char> = src.chars().collect();
    let (toks, at, lexical) = Lexer { s: &chars, i: 0 }.tokens();
    let newlines: Vec<usize> = chars
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == '\n')
        .map(|(i, _)| i)
        .collect();
    let mut p = Parser {
        toks,
        at,
        i: 0,
        open: Vec::new(),
        newlines,
        end: chars.len(),
    };
    let parsed = match p.list(&[]) {
        Ok(list) if p.i == p.toks.len() => Ok(list),
        Ok(_) => Err(Unsupported {
            what: "trailing tokens".into(),
            at: Some(p.offset()),
            syntax: None,
        }),
        Err(mut e) => {
            e.at.get_or_insert_with(|| p.offset());
            Err(e)
        }
    };
    // The first refusal by position wins: the parser's error when it comes
    // before the lexer's, else the lexer's.
    match (parsed, lexical) {
        (Err(e), Some(bad)) if e.at.unwrap_or(0) < bad.at.unwrap_or(0) => Err(e),
        (_, Some(bad)) => Err(bad),
        (parsed, None) => parsed,
    }
}

// ════════════════════════════════════════════════════════════════════════
// Execution
// ════════════════════════════════════════════════════════════════════════

/// A command's standard input: a buffer, or the process's own stdin read on
/// demand (so a script that never reads it does not block on a terminal).
struct Input {
    data: Vec<u8>,
    pos: usize,
    real: bool,
    eof: bool,
}

/// The longest line `read` takes from real stdin.
const READ_LINE_MAX: usize = 16 * 1024 * 1024;

impl Input {
    fn buf(data: Vec<u8>) -> Self {
        Input {
            data,
            pos: 0,
            real: false,
            eof: true,
        }
    }
    fn real() -> Self {
        Input {
            data: Vec::new(),
            pos: 0,
            real: true,
            eof: false,
        }
    }
    fn fill_all(&mut self) {
        if self.real && !self.eof {
            let _ = std::io::stdin().read_to_end(&mut self.data);
            self.eof = true;
        }
    }
    fn read_all(&mut self) -> Vec<u8> {
        self.fill_all();
        let out = self.data[self.pos..].to_vec();
        self.pos = self.data.len();
        out
    }
    /// One line without its newline and whether a newline ended it, `Ok(None)`
    /// at end of input, or `Err` for a line from the terminal or a pipe longer
    /// than [`READ_LINE_MAX`]. A line was read whole however long, and one that
    /// was not UTF-8 read as the end of input; consumed lines are now dropped,
    /// not kept. A last line with no newline is returned with `false`.
    fn read_line(&mut self) -> Result<Option<(Vec<u8>, bool)>, String> {
        use std::io::BufRead;
        loop {
            if let Some(nl) = self.data[self.pos..].iter().position(|&b| b == b'\n') {
                let line = self.data[self.pos..self.pos + nl].to_vec();
                self.pos += nl + 1;
                if self.real {
                    self.data.drain(..self.pos);
                    self.pos = 0;
                }
                return Ok(Some((line, true)));
            }
            if self.real && !self.eof {
                let mut chunk = Vec::new();
                let limit = (READ_LINE_MAX + 1) as u64;
                match std::io::stdin()
                    .lock()
                    .take(limit)
                    .read_until(b'\n', &mut chunk)
                {
                    Ok(0) | Err(_) => self.eof = true,
                    Ok(_) if chunk.len() > READ_LINE_MAX && !chunk.ends_with(b"\n") => {
                        self.eof = true;
                        return Err(format!("a line longer than {READ_LINE_MAX} bytes"));
                    }
                    Ok(_) => self.data.extend_from_slice(&chunk),
                }
                continue;
            }
            if self.pos < self.data.len() {
                let line = self.data[self.pos..].to_vec();
                self.pos = self.data.len();
                return Ok(Some((line, false)));
            }
            return Ok(None);
        }
    }
    fn is_buffered(&self) -> bool {
        !self.real
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ErrMode {
    Real,
    Null,
    /// Into this sink's stdout (`2>&1`).
    Merge,
    /// Captured, for `2>file`.
    Capture,
}

/// Refuse, in agent mode, to set or export a variable that steers a child
/// program (see `safety::env_name_steers_children`). Prints the refusal to the
/// shell's error stream and returns true when the change must not be made.
fn refuse_steering(sink: &mut Sink, name: &str) -> bool {
    if !crate::safety::env_change_refused(name) {
        return false;
    }
    let refusal = crate::safety::policy_deny(
        "posix",
        format!(
            "{name}: an agent may not change this variable: it steers the programs, libraries or network destinations the shell uses"
        ),
        "variables that steer child programs are set by whoever launches the agent, not by the agent",
    );
    sink.eline(&refusal.to_string());
    true
}

struct Sink {
    out: Vec<u8>,
    err_mode: ErrMode,
    err: Vec<u8>,
}

impl Sink {
    fn new(err_mode: ErrMode) -> Self {
        Sink {
            out: Vec::new(),
            err_mode,
            err: Vec::new(),
        }
    }
    fn ewrite(&mut self, b: &[u8]) {
        match self.err_mode {
            ErrMode::Real => {
                let _ = std::io::stderr().write_all(&redacted(b));
            }
            ErrMode::Null => {}
            ErrMode::Merge => self.out.extend_from_slice(b),
            ErrMode::Capture => self.err.extend_from_slice(b),
        }
    }
    fn eline(&mut self, s: &str) {
        self.ewrite(format!("{s}\n").as_bytes());
    }
}

enum Flow {
    Next(i32),
    Break(u32),
    Continue(u32),
    Exit(i32),
}

/// The result of a utility that may decline its arguments, in which case the
/// real program runs instead.
enum Util {
    Done(i32),
    Decline,
    /// Declined, naming the part of the command the subset does not support
    /// (`the s command's flag `w``), so the refusal can say what it was.
    DeclineFor(String),
}

struct Shell {
    vars: HashMap<String, String>,
    exported: HashSet<String>,
    last: i32,
    /// A pathname expansion over the cap, set by `expand` (which cannot
    /// report errors itself) and taken by the command that contains it, which
    /// is then refused rather than run with part of its arguments.
    glob_refused: Option<String>,
    /// The script's arguments: `$1`..`$9` and `$#`. Empty unless the script was
    /// run with arguments (`ish -b script.sh a b`).
    positional: Vec<String>,
    /// `$0`: the script's path, or `ish` when there is no script file (`-c`).
    script_name: String,
    /// `set -e`: a failing command ends the script, unless `errexit_off` says
    /// it is exempt.
    errexit: bool,
    /// `set -u`: an unset parameter is an error (see [`Shell::note_unbound`]).
    nounset: bool,
    /// `set -o pipefail`: a pipeline fails when any of its commands does.
    pipefail: bool,
    /// How many enclosing contexts exempt the commands in them from errexit:
    /// an `if`/`elif`/`while`/`until` condition, every operand of a `&&`/`||`
    /// list but the last, and a `!` pipeline. Counted, not flagged, because
    /// these nest: a list inside a condition is exempt all the way down.
    errexit_off: u32,
    /// The first fatal expansion error: an unset parameter under `set -u`, or a
    /// `${name:?message}` that fails. Its message, ready to print. The command
    /// that made the expansion takes it and exits, before it runs.
    unbound: Option<String>,
    /// The line of the pipeline now running, for an expansion error's message.
    line: usize,
}

const SHELL_NAME: &str = "ish";

impl Shell {
    fn var(&self, name: &str) -> Option<String> {
        match name {
            "?" => return Some(self.last.to_string()),
            "$" => return Some(std::process::id().to_string()),
            "#" => return Some(self.positional.len().to_string()),
            "0" => return Some(self.script_name.clone()),
            _ => {}
        }
        if name.chars().all(|c| c.is_ascii_digit()) {
            // `$1`..`$9` and `${10}` name the script's arguments; past the last
            // one, unset.
            return name
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| self.positional.get(i))
                .cloned();
        }
        if let Some(v) = self.vars.get(name) {
            return Some(v.clone());
        }
        std::env::var(name).ok()
    }

    fn set_var(&mut self, name: &str, value: String) {
        // A steering name is not stored at all in agent mode, so no path into
        // here can make `$PATH` or `$HOME` something the agent chose. The callers
        // that can report it (assignment, export, for, read) refuse first; this
        // catches the rest, such as `${NAME:=value}`, which cannot.
        if crate::safety::env_change_refused(name) {
            return;
        }
        self.vars.insert(name.to_string(), value);
    }

    /// Under `set -u`, an unset parameter is noted for the command whose
    /// expansion met it, which then exits (see [`Shell::unbound_refusal`]).
    /// The first one wins.
    fn note_unbound(&mut self, name: &str) {
        if self.nounset && self.unbound.is_none() {
            self.unbound = Some(self.expansion_message(name, "unbound variable"));
        }
    }

    /// An expansion error in bash's shape: `script: line N: NAME: text`.
    fn expansion_message(&self, name: &str, text: &str) -> String {
        format!("{}: line {}: {name}: {text}", self.script_name, self.line)
    }

    /// The exit for a command whose expansion failed (see [`Shell::note_unbound`]):
    /// the message goes to stderr, and the script ends with status 1.
    fn unbound_refusal(&mut self, sink: &mut Sink) -> Option<Flow> {
        let msg = self.unbound.take()?;
        sink.eline(&msg);
        self.last = 1;
        Some(Flow::Exit(1))
    }

    // ── expansion ──

    /// Expand a word into fields: parameter and command substitution, field
    /// splitting of unquoted results, then pathname expansion.
    fn expand(&mut self, w: &Word, stdin: &mut Input) -> Vec<String> {
        // Each field is built as (text, glob-eligible mask per char).
        let mut fields: Vec<(String, Vec<bool>)> = Vec::new();
        let mut cur = String::new();
        let mut mask: Vec<bool> = Vec::new();
        let mut have = false;
        let tilde =
            matches!(w.first(), Some(Part::Lit(s, false)) if s == "~" || s.starts_with("~/"));
        // `"$@"` with no arguments is no word at all, even though the quotes
        // around it would otherwise make an empty one.
        if self.positional.is_empty()
            && w.iter()
                .any(|p| matches!(p, Part::Var(name, None, true) if name == "@"))
            && w.iter().all(|p| match p {
                Part::Var(name, None, true) => name == "@",
                Part::Lit(s, true) => s.is_empty(),
                _ => false,
            })
        {
            return Vec::new();
        }

        for (idx, part) in w.iter().enumerate() {
            match part {
                Part::Lit(s, quoted) => {
                    let s = if idx == 0 && tilde {
                        let home = self.var("HOME").unwrap_or_default();
                        format!("{home}{}", &s[1..])
                    } else {
                        s.clone()
                    };
                    for c in s.chars() {
                        cur.push(c);
                        mask.push(!quoted);
                    }
                    have = true;
                }
                Part::Len(name) => {
                    let value = self.var(name);
                    if value.is_none() {
                        self.note_unbound(name);
                    }
                    let n = value.unwrap_or_default().chars().count();
                    for c in n.to_string().chars() {
                        cur.push(c);
                        mask.push(false);
                    }
                    have = true;
                }
                // `"$*"` is one field, the arguments joined by a space.
                Part::Var(name, None, true) if name == "*" => {
                    let joined = self.positional.join(" ");
                    self.splice(&joined, true, &mut fields, &mut cur, &mut mask, &mut have);
                }
                // `"$@"` is each argument as its own field, and no field at all
                // when there are none.
                Part::Var(name, None, true) if name == "@" => {
                    let args = self.positional.clone();
                    for (k, arg) in args.iter().enumerate() {
                        if k > 0 {
                            fields.push((std::mem::take(&mut cur), std::mem::take(&mut mask)));
                        }
                        cur.push_str(arg);
                        mask.extend(std::iter::repeat_n(false, arg.chars().count()));
                        have = true;
                    }
                }
                // Unquoted `$@` and `$*` split each argument like any unquoted
                // expansion.
                Part::Var(name, None, false) if name == "@" || name == "*" => {
                    let args = self.positional.clone();
                    for (k, arg) in args.iter().enumerate() {
                        // Each argument ends the field before it: `$* x` with
                        // arguments a and b is three words, not `ab x`.
                        if k > 0 && have {
                            fields.push((std::mem::take(&mut cur), std::mem::take(&mut mask)));
                            have = false;
                        }
                        self.splice(arg, false, &mut fields, &mut cur, &mut mask, &mut have);
                    }
                }
                Part::Var(name, op, quoted) => {
                    let value = self.param(name, op, *quoted, stdin);
                    self.splice(&value, *quoted, &mut fields, &mut cur, &mut mask, &mut have);
                }
                Part::Cmd(body, quoted) => {
                    let value = self.substitute(body);
                    self.splice(&value, *quoted, &mut fields, &mut cur, &mut mask, &mut have);
                }
            }
        }
        if have {
            fields.push((cur, mask));
        }
        let mut out = Vec::new();
        for (text, mask) in fields {
            let globby = text
                .chars()
                .zip(mask.iter())
                .any(|(c, &m)| m && matches!(c, '*' | '?' | '['));
            if globby {
                match glob_expand(&text) {
                    Some(matches) if !matches.is_empty() => {
                        out.extend(matches);
                        continue;
                    }
                    Some(_) => {}
                    None => {
                        self.glob_refused.get_or_insert(format!(
                            "{SHELL_NAME}: {text}: more than {WALK_MAX} matches; nothing was expanded"
                        ));
                    }
                }
            }
            out.push(text);
        }
        out
    }

    /// The refusal for a command whose expansion went over the glob cap: the
    /// message goes to stderr, and the command does not run.
    fn glob_refusal(&mut self, sink: &mut Sink) -> Option<Flow> {
        let msg = self.glob_refused.take()?;
        sink.eline(&msg);
        self.last = 1;
        Some(Flow::Next(1))
    }

    /// Append an expansion's value, splitting it into fields when unquoted.
    fn splice(
        &self,
        value: &str,
        quoted: bool,
        fields: &mut Vec<(String, Vec<bool>)>,
        cur: &mut String,
        mask: &mut Vec<bool>,
        have: &mut bool,
    ) {
        if quoted {
            cur.push_str(value);
            mask.extend(std::iter::repeat_n(false, value.chars().count()));
            *have = true;
            return;
        }
        let starts_ws = value.starts_with(|c: char| c.is_ascii_whitespace());
        let ends_ws = value.ends_with(|c: char| c.is_ascii_whitespace());
        let pieces: Vec<&str> = value.split_ascii_whitespace().collect();
        if pieces.is_empty() {
            if starts_ws && *have {
                fields.push((std::mem::take(cur), std::mem::take(mask)));
                *have = false;
            }
            return;
        }
        for (k, p) in pieces.iter().enumerate() {
            if (k > 0 || starts_ws) && *have {
                fields.push((std::mem::take(cur), std::mem::take(mask)));
            }
            cur.push_str(p);
            // Unquoted expansion results are subject to globbing.
            mask.extend(std::iter::repeat_n(true, p.chars().count()));
            *have = true;
        }
        if ends_ws {
            fields.push((std::mem::take(cur), std::mem::take(mask)));
            *have = false;
        }
    }

    /// The value of `${name...}` (or `$name`). `quoted` is whether the
    /// expansion is inside double quotes, which is how its operand's
    /// quoting is read (see [`Shell::pattern`]).
    fn param(
        &mut self,
        name: &str,
        op: &Option<(String, Word)>,
        quoted: bool,
        stdin: &mut Input,
    ) -> String {
        let value = self.var(name);
        // Unset is an error under `set -u`, except for `$@` and `$*`, and for
        // the forms that supply their own value: `${x:-d}`, `${x-d}`, `${x:=d}`
        // and `${x+v}`; `${x?msg}` raises its own error below.
        let supplies = matches!(
            op,
            Some((o, _)) if matches!(o.trim_start_matches(':'), "-" | "=" | "+" | "?")
        );
        if value.is_none() && !supplies && name != "@" && name != "*" {
            self.note_unbound(name);
        }
        let Some((op, word)) = op else {
            return value.unwrap_or_default();
        };
        let colon = op.starts_with(':');
        let unset_or_null = match &value {
            None => true,
            Some(v) => colon && v.is_empty(),
        };
        let operand = |sh: &mut Shell, stdin: &mut Input| sh.expand(word, stdin).join(" ");
        match op.trim_start_matches(':') {
            "-" => {
                if unset_or_null {
                    operand(self, stdin)
                } else {
                    value.unwrap_or_default()
                }
            }
            "=" => {
                if unset_or_null {
                    let v = operand(self, stdin);
                    self.set_var(name, v.clone());
                    v
                } else {
                    value.unwrap_or_default()
                }
            }
            "+" => {
                if unset_or_null {
                    String::new()
                } else {
                    operand(self, stdin)
                }
            }
            // `${x%pat}`, `${x%%pat}`, `${x#pat}`, `${x##pat}`: the value less
            // the part the pattern matches. An unset name is empty (and an
            // error under `set -u`, noted above).
            "%" | "%%" | "#" | "##" => {
                let pat = self.pattern(word, quoted, stdin);
                value
                    .as_deref()
                    .map_or(String::new(), |v| trim_affix(v, &pat, op))
            }
            // `${x:?msg}` / `${x?msg}`: an error whatever `set -u` says, with
            // the message given, or bash's own when there is none. The command
            // that made the expansion does not run, and the script ends with 1.
            "?" => {
                if unset_or_null {
                    let given = operand(self, stdin);
                    let text = if !given.is_empty() {
                        given
                    } else if colon {
                        "parameter null or not set".to_string()
                    } else {
                        "parameter not set".to_string()
                    };
                    if self.unbound.is_none() {
                        self.unbound = Some(self.expansion_message(name, &text));
                    }
                    String::new()
                } else {
                    value.unwrap_or_default()
                }
            }
            _ => value.unwrap_or_default(),
        }
    }

    /// The glob pattern of a `${name%pat}`-style operand. Literal text quoted
    /// in the operand is literal, so its glob characters are escaped; unquoted
    /// text, and the unquoted results of expansions, keep theirs. Inside double
    /// quotes (`quoted`) the operand's text is still read as a pattern, since
    /// its quoting is not told apart here.
    fn pattern(&mut self, word: &Word, quoted: bool, stdin: &mut Input) -> String {
        let mut out = String::new();
        for part in word {
            match part {
                Part::Lit(s, lit_quoted) => {
                    if *lit_quoted && !quoted {
                        out.push_str(&escape_glob(s));
                    } else {
                        out.push_str(s);
                    }
                }
                Part::Var(_, _, part_quoted) | Part::Cmd(_, part_quoted) => {
                    let text = self.expand_assign(&vec![part.clone()], stdin);
                    if *part_quoted {
                        out.push_str(&escape_glob(&text));
                    } else {
                        out.push_str(&text);
                    }
                }
                Part::Len(_) => {
                    let text = self.expand_assign(&vec![part.clone()], stdin);
                    out.push_str(&text);
                }
            }
        }
        out
    }

    /// `$(…)`: run the body and take its stdout, less trailing newlines.
    fn substitute(&mut self, body: &str) -> String {
        let list = match parse(body) {
            Ok(l) => l,
            Err(Unsupported { what, .. }) => {
                let _ = writeln!(
                    std::io::stderr(),
                    "{SHELL_NAME}: command substitution uses {what}, which this shell does not run"
                );
                self.last = 2;
                return String::new();
            }
        };
        let mut sink = Sink::new(ErrMode::Real);
        let mut input = Input::buf(Vec::new());
        // Bash does not pass `set -e` into a command substitution (it clears
        // it unless `inherit_errexit` is set), so a failing command in one does
        // not end the script. `set -u` is inherited. An unbound parameter the
        // substitution's own commands meet is theirs; the caller's is kept.
        let errexit = std::mem::replace(&mut self.errexit, false);
        let unbound = self.unbound.take();
        // The body's lines count from its own start, so the caller's line is
        // restored: its later messages name its own line.
        let line = self.line;
        let flow = self.run_list(&list, &mut input, &mut sink);
        self.errexit = errexit;
        self.unbound = unbound;
        self.line = line;
        self.last = match flow {
            Flow::Next(c) | Flow::Exit(c) => c,
            _ => 0,
        };
        let mut s = String::from_utf8_lossy(&sink.out).into_owned();
        while s.ends_with('\n') {
            s.pop();
        }
        s
    }

    /// The value of an assignment (`x=word`). Bash does no field splitting,
    /// globbing or trimming here: the expansions are joined as text. A command
    /// substitution loses only its trailing newlines (see [`Shell::substitute`]).
    fn expand_assign(&mut self, w: &Word, stdin: &mut Input) -> String {
        let tilde =
            matches!(w.first(), Some(Part::Lit(s, false)) if s == "~" || s.starts_with("~/"));
        let mut out = String::new();
        for (idx, part) in w.iter().enumerate() {
            match part {
                Part::Lit(s, _) => {
                    if idx == 0 && tilde {
                        out.push_str(&self.var("HOME").unwrap_or_default());
                        out.push_str(&s[1..]);
                    } else {
                        out.push_str(s);
                    }
                }
                Part::Len(name) => {
                    let value = self.var(name);
                    if value.is_none() {
                        self.note_unbound(name);
                    }
                    out.push_str(&value.unwrap_or_default().chars().count().to_string());
                }
                Part::Var(name, None, _) if name == "@" || name == "*" => {
                    out.push_str(&self.positional.join(" "));
                }
                Part::Var(name, op, quoted) => {
                    let value = self.param(name, op, *quoted, stdin);
                    out.push_str(&value);
                }
                Part::Cmd(body, _) => {
                    let value = self.substitute(body);
                    out.push_str(&value);
                }
            }
        }
        out
    }

    // ── running ──

    fn run_list(&mut self, list: &List, stdin: &mut Input, sink: &mut Sink) -> Flow {
        let mut status = 0;
        for ao in list {
            match self.run_and_or(ao, stdin, sink) {
                Flow::Next(c) => status = c,
                other => return other,
            }
        }
        Flow::Next(status)
    }

    fn run_and_or(&mut self, ao: &AndOr, stdin: &mut Input, sink: &mut Sink) -> Flow {
        // Every operand of the list but the last is exempt from errexit: a
        // failure there is the list's business, not the script's.
        let n = ao.rest.len();
        let mut status = match self.run_exempt(&ao.first, n > 0, stdin, sink) {
            Flow::Next(c) => c,
            other => return other,
        };
        for (k, (and, p)) in ao.rest.iter().enumerate() {
            if (*and && status != 0) || (!*and && status == 0) {
                continue;
            }
            status = match self.run_exempt(p, k + 1 < n, stdin, sink) {
                Flow::Next(c) => c,
                other => return other,
            };
        }
        Flow::Next(status)
    }

    /// [`Shell::run_pipeline`], with errexit suppressed when `exempt`.
    fn run_exempt(
        &mut self,
        p: &Pipeline,
        exempt: bool,
        stdin: &mut Input,
        sink: &mut Sink,
    ) -> Flow {
        if exempt {
            self.errexit_off += 1;
        }
        let flow = self.run_pipeline(p, stdin, sink);
        if exempt {
            self.errexit_off -= 1;
        }
        flow
    }

    fn run_pipeline(&mut self, p: &Pipeline, stdin: &mut Input, sink: &mut Sink) -> Flow {
        self.line = p.line;
        // A `!` pipeline is exempt from errexit, and so is everything in it.
        if p.negate {
            self.errexit_off += 1;
        }
        let flow = if p.cmds.len() == 1 {
            self.run_cmd(&p.cmds[0], stdin, sink)
        } else {
            let mut data: Option<Vec<u8>> = None;
            let mut status = 0;
            let n = p.cmds.len();
            for (k, cmd) in p.cmds.iter().enumerate() {
                let mut local_in = match data.take() {
                    Some(d) => Input::buf(d),
                    None => Input::buf(Vec::new()),
                };
                let input: &mut Input = if k == 0 { &mut *stdin } else { &mut local_in };
                let mut stage = Sink::new(sink.err_mode);
                // Each stage of a pipeline is a subshell in bash: `exit` and
                // `break` end the stage, not the script.
                let c = match self.run_cmd(cmd, input, &mut stage) {
                    Flow::Next(c) | Flow::Exit(c) => c,
                    _ => 0,
                };
                // Under `pipefail` the status is the last failing stage's, or 0.
                if !self.pipefail || c != 0 {
                    status = c;
                }
                if stage.err_mode == ErrMode::Capture {
                    sink.err.extend_from_slice(&stage.err);
                }
                if k + 1 == n {
                    sink.out.extend_from_slice(&stage.out);
                } else {
                    data = Some(stage.out);
                }
            }
            Flow::Next(status)
        };
        if p.negate {
            self.errexit_off -= 1;
        }
        match flow {
            Flow::Next(c) => {
                let c = if p.negate { i32::from(c == 0) } else { c };
                self.last = c;
                // errexit: a failing pipeline ends the script, unless exempt.
                if c != 0 && !p.negate && self.errexit && self.errexit_off == 0 {
                    return Flow::Exit(c);
                }
                Flow::Next(c)
            }
            other => other,
        }
    }

    fn run_cmd(&mut self, cmd: &Cmd, stdin: &mut Input, sink: &mut Sink) -> Flow {
        // A refusal is the business of the command whose expansion set it. One
        // left by an expansion no site checks (a `case` word, a `${x:-...}`
        // operand) must not refuse this, unrelated, command.
        self.glob_refused = None;
        self.unbound = None;
        let redirs = match cmd {
            Cmd::Simple { redirs, .. }
            | Cmd::If { redirs, .. }
            | Cmd::For { redirs, .. }
            | Cmd::While { redirs, .. }
            | Cmd::Group { redirs, .. } => redirs,
        };
        if redirs.is_empty() {
            return self.run_cmd_inner(cmd, stdin, sink);
        }
        // Resolve redirections into a local input and sink.
        let mut local_in: Option<Input> = None;
        let mut out_file: Option<(String, bool)> = None;
        let mut err_file: Option<(String, bool)> = None;
        let mut out_null = false;
        let mut out_to_err = false;
        let mut err_mode = sink.err_mode;
        for r in redirs {
            match r {
                Redir::In(w) => {
                    let path = self.expand(w, stdin).join(" ");
                    if let Some(flow) = self.unbound_refusal(sink) {
                        return flow;
                    }
                    if let Some(refusal) = jail_refusal(SHELL_NAME, &path) {
                        sink.eline(&refusal);
                        self.last = 1;
                        return Flow::Next(1);
                    }
                    match std::fs::read(&path) {
                        Ok(d) => local_in = Some(Input::buf(d)),
                        Err(e) => {
                            sink.eline(&format!("{SHELL_NAME}: {path}: {}", io_reason(&e)));
                            self.last = 1;
                            return Flow::Next(1);
                        }
                    }
                }
                Redir::Out { fd, append, target } => {
                    let path = self.expand(target, stdin).join(" ");
                    if *fd == 2 {
                        if path == "/dev/null" {
                            err_mode = ErrMode::Null;
                        } else {
                            let resolved = match self.refuse_write(sink, &path) {
                                Ok(p) => p,
                                Err(flow) => return flow,
                            };
                            err_mode = ErrMode::Capture;
                            err_file = Some((resolved, *append));
                        }
                    } else if path == "/dev/null" {
                        out_null = true;
                        out_file = None;
                    } else {
                        let resolved = match self.refuse_write(sink, &path) {
                            Ok(p) => p,
                            Err(flow) => return flow,
                        };
                        out_null = false;
                        out_file = Some((resolved, *append));
                    }
                }
                Redir::Both(target) => {
                    let path = self.expand(target, stdin).join(" ");
                    if path == "/dev/null" {
                        out_null = true;
                        err_mode = ErrMode::Null;
                    } else {
                        let resolved = match self.refuse_write(sink, &path) {
                            Ok(p) => p,
                            Err(flow) => return flow,
                        };
                        out_file = Some((resolved, false));
                        err_mode = ErrMode::Merge;
                    }
                }
                Redir::Dup { from: 2, to: 1 } => err_mode = ErrMode::Merge,
                Redir::Dup { from: 1, to: 2 } => out_to_err = true,
                Redir::Dup { .. } => {}
            }
        }
        if let Some(flow) = self.unbound_refusal(sink) {
            return flow;
        }
        if let Some(flow) = self.glob_refusal(sink) {
            return flow;
        }
        let mut local = Sink::new(err_mode);
        let flow = match local_in.as_mut() {
            Some(i) => self.run_cmd_inner(cmd, i, &mut local),
            None => self.run_cmd_inner(cmd, stdin, &mut local),
        };
        if let Some((path, append)) = err_file {
            if let Err(msg) = write_file(&path, &local.err, append) {
                sink.eline(&msg);
            }
        }
        if let Some((path, append)) = out_file {
            if let Err(msg) = write_file(&path, &local.out, append) {
                sink.eline(&msg);
                self.last = 1;
                return Flow::Next(1);
            }
        } else if out_to_err {
            let out = std::mem::take(&mut local.out);
            sink.ewrite(&out);
        } else if !out_null {
            sink.out.extend_from_slice(&local.out);
        }
        flow
    }

    /// Judge a redirection's output file before its command runs. Bash opens the
    /// file first, so a refused file is a failed redirection: the command does
    /// not run and the status is 1 (and errexit applies, as for any failure).
    fn refuse_write(&mut self, sink: &mut Sink, path: &str) -> Result<String, Flow> {
        check_write_in_place(path).map_err(|msg| {
            sink.eline(&msg);
            self.last = 1;
            Flow::Next(1)
        })
    }

    fn run_cmd_inner(&mut self, cmd: &Cmd, stdin: &mut Input, sink: &mut Sink) -> Flow {
        match cmd {
            Cmd::Simple { assigns, words, .. } => self.run_simple(assigns, words, stdin, sink),
            Cmd::If {
                branches,
                otherwise,
                ..
            } => {
                for (cond, body) in branches {
                    // A condition is exempt from errexit.
                    self.errexit_off += 1;
                    let flow = self.run_list(cond, stdin, sink);
                    self.errexit_off -= 1;
                    match flow {
                        Flow::Next(0) => return self.run_list(body, stdin, sink),
                        Flow::Next(_) => {}
                        other => return other,
                    }
                }
                match otherwise {
                    Some(b) => self.run_list(b, stdin, sink),
                    None => Flow::Next(0),
                }
            }
            Cmd::For {
                var, items, body, ..
            } => {
                if refuse_steering(sink, var) {
                    return Flow::Next(126);
                }
                let mut values = Vec::new();
                for w in items {
                    values.extend(self.expand(w, stdin));
                }
                if let Some(flow) = self.unbound_refusal(sink) {
                    return flow;
                }
                if let Some(flow) = self.glob_refusal(sink) {
                    return flow;
                }
                let mut status = 0;
                for v in values {
                    self.set_var(var, v);
                    match self.run_list(body, stdin, sink) {
                        Flow::Next(c) => status = c,
                        Flow::Break(n) => {
                            if n > 1 {
                                return Flow::Break(n - 1);
                            }
                            break;
                        }
                        Flow::Continue(n) => {
                            if n > 1 {
                                return Flow::Continue(n - 1);
                            }
                        }
                        Flow::Exit(c) => return Flow::Exit(c),
                    }
                }
                Flow::Next(status)
            }
            Cmd::While {
                until, cond, body, ..
            } => {
                let mut status = 0;
                let mut guard = 0u64;
                loop {
                    guard += 1;
                    if guard > 10_000_000 {
                        sink.eline(&format!("{SHELL_NAME}: loop ran 10,000,000 times; stopped"));
                        return Flow::Next(1);
                    }
                    // The condition is exempt from errexit.
                    self.errexit_off += 1;
                    let flow = self.run_list(cond, stdin, sink);
                    self.errexit_off -= 1;
                    let c = match flow {
                        Flow::Next(c) => c,
                        other => return other,
                    };
                    if (c == 0) == *until {
                        break;
                    }
                    match self.run_list(body, stdin, sink) {
                        Flow::Next(c) => status = c,
                        Flow::Break(n) => {
                            if n > 1 {
                                return Flow::Break(n - 1);
                            }
                            break;
                        }
                        Flow::Continue(n) => {
                            if n > 1 {
                                return Flow::Continue(n - 1);
                            }
                        }
                        Flow::Exit(c) => return Flow::Exit(c),
                    }
                }
                Flow::Next(status)
            }
            Cmd::Group { body, .. } => self.run_list(body, stdin, sink),
        }
    }

    fn run_simple(
        &mut self,
        assigns: &[(String, Word)],
        words: &[Word],
        stdin: &mut Input,
        sink: &mut Sink,
    ) -> Flow {
        let mut argv = Vec::new();
        for w in words {
            argv.extend(self.expand(w, stdin));
        }
        let mut env_overrides = Vec::new();
        for (name, w) in assigns {
            if refuse_steering(sink, name) {
                return Flow::Next(126);
            }
            let v = self.expand_assign(w, stdin);
            if argv.is_empty() {
                self.set_var(name, v);
            } else {
                env_overrides.push((name.clone(), v));
            }
        }
        if let Some(flow) = self.unbound_refusal(sink) {
            return flow;
        }
        if let Some(flow) = self.glob_refusal(sink) {
            return flow;
        }
        if argv.is_empty() {
            // `$(cmd)` in an assignment leaves its status; a bare assignment 0.
            let has_subst = assigns
                .iter()
                .any(|(_, w)| w.iter().any(|p| matches!(p, Part::Cmd(..))));
            let c = if has_subst { self.last } else { 0 };
            self.last = c;
            return Flow::Next(c);
        }
        let flow = self.dispatch(&argv, &env_overrides, stdin, sink);
        if let Flow::Next(c) = flow {
            self.last = c;
        }
        flow
    }

    fn dispatch(
        &mut self,
        argv: &[String],
        env: &[(String, String)],
        stdin: &mut Input,
        sink: &mut Sink,
    ) -> Flow {
        let name = argv[0].as_str();
        let args = &argv[1..];
        // Shell builtins: they act on the shell itself.
        match name {
            "true" | ":" => return Flow::Next(0),
            "false" => return Flow::Next(1),
            "exit" => {
                let c = args
                    .first()
                    .and_then(|a| a.parse().ok())
                    .unwrap_or(self.last);
                return Flow::Exit(c);
            }
            "break" | "continue" => {
                let n = args
                    .first()
                    .and_then(|a| a.parse().ok())
                    .unwrap_or(1u32)
                    .max(1);
                return if name == "break" {
                    Flow::Break(n)
                } else {
                    Flow::Continue(n)
                };
            }
            "export" => {
                for a in args {
                    let (n, v) = match a.split_once('=') {
                        Some((n, v)) => (n.to_string(), Some(v.to_string())),
                        None => (a.clone(), None),
                    };
                    if refuse_steering(sink, &n) {
                        return Flow::Next(126);
                    }
                    if let Some(v) = v {
                        self.set_var(&n, v);
                    }
                    self.exported.insert(n);
                }
                return Flow::Next(0);
            }
            "unset" => {
                for a in args {
                    self.vars.remove(a);
                    self.exported.remove(a);
                }
                return Flow::Next(0);
            }
            // `set -e`, `-u`, `-o pipefail` and their `+` forms switch options;
            // `set -- a b` replaces the positional parameters, `set --` clears
            // them. Anything else was refused before the script ran (see
            // `set_refusal`), so the error here is a guard only.
            "set" => {
                return match parse_set(args) {
                    Ok(set) => {
                        for (opt, on) in set.changes {
                            match opt {
                                SetOpt::Errexit => self.errexit = on,
                                SetOpt::Nounset => self.nounset = on,
                                SetOpt::Pipefail => self.pipefail = on,
                            }
                        }
                        if let Some(params) = set.positional {
                            self.positional = params;
                        }
                        Flow::Next(0)
                    }
                    Err(what) => {
                        sink.eline(&format!("{SHELL_NAME}: {what} is not in the subset"));
                        Flow::Next(2)
                    }
                };
            }
            // `shift [N]` drops the first N positional parameters (1 by default).
            // A count past the last parameter changes nothing and fails, as bash.
            "shift" => {
                let n = match args.first() {
                    None => 1,
                    Some(a) => match a.parse::<usize>() {
                        Ok(n) => n,
                        Err(_) => {
                            sink.eline(&format!(
                                "{SHELL_NAME}: shift: {a}: numeric argument required"
                            ));
                            return Flow::Next(1);
                        }
                    },
                };
                if n > self.positional.len() {
                    sink.eline(&format!(
                        "{SHELL_NAME}: shift: {n}: shift count out of range"
                    ));
                    return Flow::Next(1);
                }
                self.positional.drain(..n);
                return Flow::Next(0);
            }
            "cd" => {
                let target = match args.first() {
                    Some(t) if t == "-" => self.var("OLDPWD").unwrap_or_default(),
                    Some(t) => t.clone(),
                    None => self.var("HOME").unwrap_or_default(),
                };
                // With --workspace set, a cd may not leave the workspace. Otherwise
                // pwd reports the outside folder while a relative write still
                // lands inside the jail, and the two disagree. The check is only
                // for an explicit workspace; the pinned agent root is unchanged.
                if std::env::var_os("IRONSHELL_WORKSPACE").is_some() {
                    let dest = match std::env::current_dir() {
                        Ok(here) => here.join(&target),
                        Err(_) => std::path::PathBuf::from(&target),
                    };
                    if !crate::safety::within_workspace(&dest.to_string_lossy()) {
                        sink.eline(&format!(
                            "{SHELL_NAME}: cd: {target}: outside the workspace; {}",
                            crate::safety::outside_workspace_hint()
                        ));
                        return Flow::Next(1);
                    }
                }
                let old = std::env::current_dir().ok();
                return match std::env::set_current_dir(&target) {
                    Ok(()) => {
                        if let Some(o) = old {
                            self.set_var("OLDPWD", o.to_string_lossy().into_owned());
                        }
                        if let Ok(d) = std::env::current_dir() {
                            self.set_var("PWD", d.to_string_lossy().into_owned());
                        }
                        Flow::Next(0)
                    }
                    Err(e) => {
                        sink.eline(&format!("{SHELL_NAME}: cd: {target}: {}", io_reason(&e)));
                        Flow::Next(1)
                    }
                };
            }
            "read" => return Flow::Next(self.read_builtin(args, env, stdin)),
            "pwd" => {
                sink.out.extend_from_slice(logical_pwd().as_bytes());
                sink.out.push(b'\n');
                return Flow::Next(0);
            }
            _ => {}
        }
        let mut why = None;
        if env.is_empty() {
            match utility(name, args, stdin, sink) {
                Util::Done(c) => return Flow::Next(c),
                Util::Decline => {}
                Util::DeclineFor(part) => why = Some(part),
            }
        }
        // A script file run as a command, or `sh FILE` / `bash FILE`, runs in a
        // child of this engine rather than through the platform's shell.
        if let Some((script, script_args)) = shell_script_command(argv) {
            return Flow::Next(self.run_script_child(script, script_args, env, stdin, sink));
        }
        Flow::Next(self.external(argv, env, stdin, sink, why.as_deref()))
    }

    /// `read [-r] [NAME...]`. The line is split into the names with the IFS in
    /// force for this command: a prefix assignment (`IFS=, read`) for this
    /// command only, else the shell's own IFS, else the default. Nothing is
    /// stored in IFS by `read` itself.
    fn read_builtin(
        &mut self,
        args: &[String],
        env: &[(String, String)],
        stdin: &mut Input,
    ) -> i32 {
        let mut raw = false;
        let mut names = Vec::new();
        for a in args {
            if a == "-r" {
                raw = true;
            } else if a.starts_with('-') {
                // -p, -t, -n …: honoured by ignoring would be a lie; say no.
                let _ = writeln!(
                    std::io::stderr(),
                    "{SHELL_NAME}: read: {a}: unsupported option"
                );
                return 2;
            } else {
                names.push(a.clone());
            }
        }
        // With no name, the line goes to REPLY whole, without field splitting.
        let default_reply = names.is_empty();
        if default_reply {
            names.push("REPLY".into());
        }
        if let Some(name) = names.iter().find(|n| crate::safety::env_change_refused(n)) {
            let refusal = crate::safety::policy_deny(
                "posix",
                format!(
                    "{name}: an agent may not read into this variable: it steers child programs"
                ),
                "variables that steer child programs are set by whoever launches the agent",
            );
            let _ = writeln!(std::io::stderr(), "{refusal}");
            return 126;
        }
        let (line, terminated) = match stdin.read_line() {
            Ok(Some(found)) => found,
            Ok(None) => {
                for n in &names {
                    self.set_var(n, String::new());
                }
                return 1;
            }
            Err(why) => {
                let _ = writeln!(std::io::stderr(), "{SHELL_NAME}: read: {why}");
                return 2;
            }
        };
        let mut line = String::from_utf8_lossy(&line).into_owned();
        if !raw {
            let mut s = String::new();
            let mut it = line.chars();
            while let Some(c) = it.next() {
                if c == '\\' {
                    if let Some(n) = it.next() {
                        s.push(n);
                    }
                } else {
                    s.push(c);
                }
            }
            line = s;
        }
        // As bash does, a last line with no newline is assigned but reads as
        // a failure, so `while read` does not run its body for it.
        let status = if terminated { 0 } else { 1 };
        if default_reply {
            self.set_var("REPLY", line);
            return status;
        }
        let ifs = match env.iter().rev().find(|(n, _)| n == "IFS") {
            Some((_, v)) => v.clone(),
            None => self
                .vars
                .get("IFS")
                .cloned()
                .unwrap_or_else(|| " \t\n".to_string()),
        };
        for (n, v) in names.iter().zip(read_fields(&line, &ifs, names.len())) {
            self.set_var(n, v);
        }
        status
    }

    /// Run a program that is not implemented here: directly, with no shell in
    /// between, and only after the effect gate has admitted it. `why` is the
    /// part of the command the subset declined, named if no GNU program runs it.
    fn external(
        &mut self,
        argv: &[String],
        env: &[(String, String)],
        stdin: &mut Input,
        sink: &mut Sink,
        why: Option<&str>,
    ) -> i32 {
        if let Some(message) = sed_missing_refusal(argv, why) {
            sink.eline(&message);
            return 2;
        }
        if let Err(e) = crate::safety::guard_exec("posix", argv.join(" ")) {
            sink.eline(&e.to_string());
            return e.code.exit_code();
        }
        if let Some(refusal) = self.steering_refusal(env) {
            sink.eline(&refusal);
            return 126;
        }
        let program = match resolve_program(argv, env, why) {
            Ok(p) => p,
            Err((code, message)) => {
                sink.eline(&message);
                return code;
            }
        };
        let mut cmd = crate::safety::child_command(&program);
        cmd.args(&argv[1..]);
        for n in &self.exported {
            if let Some(v) = self.vars.get(n) {
                cmd.env(n, v);
            }
        }
        for (n, v) in env {
            cmd.env(n, v);
        }
        crate::safety::strip_ironshell_env(&mut cmd);
        self.run_piped(cmd, &argv[0], stdin, sink)
    }

    /// The refusal, if any, for a steering variable in this command's
    /// environment: in agent mode no such variable reaches a child (see
    /// `external`). `None` when there is nothing to refuse.
    fn steering_refusal(&self, env: &[(String, String)]) -> Option<String> {
        if crate::safety::current_mode() != crate::safety::Mode::Agent {
            return None;
        }
        // The approval of argv does not cover what a child inherits from an
        // `IRONSHELL_*` variable set by an assignment prefix or an export, which
        // can set the gate of the child (`IRONSHELL_MODE=human ish -c 'apply(..)'`
        // is ungated), so no such variable reaches a child in agent mode.
        let name = self
            .exported
            .iter()
            .map(String::as_str)
            .chain(env.iter().map(|(n, _)| n.as_str()))
            .find(|n| crate::safety::env_name_steers_children(n))?;
        let refusal = crate::safety::policy_deny(
            "posix",
            format!(
                "{name}: an agent may not pass this variable to a child process: it steers the programs, libraries or network destinations the child uses"
            ),
            "variables that steer child programs are set by whoever launches the agent, not by the agent",
        );
        Some(refusal.to_string())
    }

    /// Run a shell script (see [`shell_script_command`]) in a child `ish -b`,
    /// and return its status.
    ///
    /// Windows cannot start a `#!/bin/sh` script, and `sh` or `bash` may not be
    /// installed, so the script runs in this same in-process engine. The child
    /// is this binary, so it is held as this shell is: `child_command` keeps
    /// `IRONSHELL_MODE`, `IRONSHELL_WORKSPACE` and the pinned workspace root (all
    /// in `CONFINING_ENV`), and in agent mode it sets `IRONSHELL_MODE=agent`. The
    /// script's commands are gated one by one in the child, exactly as they are
    /// here, so the script itself gets no Exec gate of its own: the workspace
    /// jail and every write check apply inside the child. The steering-variable
    /// refusal above still applies, as it does to any other command.
    fn run_script_child(
        &mut self,
        script: &str,
        args: &[String],
        env: &[(String, String)],
        stdin: &mut Input,
        sink: &mut Sink,
    ) -> i32 {
        // The script is read and run, so it is held to the read jail as `cat`
        // holds a file it reads: a script outside the workspace is refused
        // before anything runs, through a link or a junction or by an absolute
        // path alike (the jail resolves them). The refusal names `sh`, the
        // command the script runs as, not `cat`, which is not what was run.
        if let Some(refusal) = jail_refusal("sh", script) {
            sink.eline(&refusal);
            return 1;
        }
        if let Some(refusal) = self.steering_refusal(env) {
            sink.eline(&refusal);
            return 126;
        }
        // A script that runs itself would nest children until memory ran out.
        // Each child is told its depth, and a script at the limit starts none.
        let depth = match std::env::var_os(SCRIPT_DEPTH_ENV) {
            None => 0,
            // A depth that cannot be read is treated as the limit: when unsure, refuse.
            Some(text) => text
                .to_str()
                .and_then(|t| t.parse::<u32>().ok())
                .unwrap_or(MAX_SCRIPT_DEPTH),
        };
        if depth >= MAX_SCRIPT_DEPTH {
            let refusal = crate::safety::budget_exceeded(
                "posix",
                format!("{script}: scripts nest at most {MAX_SCRIPT_DEPTH} deep"),
                "a script is probably running itself; add a stop condition",
            );
            sink.eline(&refusal.to_string());
            return 1;
        }
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(e) => {
                sink.eline(&format!(
                    "{SHELL_NAME}: {script}: cannot find this shell to run the script: {e}"
                ));
                return 126;
            }
        };
        let mut cmd = crate::safety::child_command(&exe);
        cmd.arg("-b").arg(script).args(args);
        // Where this shell keeps its session (history, undo journal) is not
        // a confining setting, but the child would otherwise use the default,
        // so the same directory goes on to it.
        if let Some(session) = std::env::var_os("IRONSHELL_SESSION_DIR") {
            cmd.env("IRONSHELL_SESSION_DIR", session);
        }
        for n in &self.exported {
            if let Some(v) = self.vars.get(n) {
                cmd.env(n, v);
            }
        }
        for (n, v) in env {
            cmd.env(n, v);
        }
        // Set last, so no inherited or exported value can stand in for it.
        cmd.env(SCRIPT_DEPTH_ENV, (depth + 1).to_string());
        crate::safety::strip_ironshell_env(&mut cmd);
        self.run_piped(cmd, script, stdin, sink)
    }

    /// Run `cmd` with this shell's standard input, and gather its output into
    /// `sink` as a pipeline stage does. `name` labels its errors.
    fn run_piped(
        &mut self,
        mut cmd: std::process::Command,
        name: &str,
        stdin: &mut Input,
        sink: &mut Sink,
    ) -> i32 {
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(match sink.err_mode {
            ErrMode::Real => std::process::Stdio::inherit(),
            ErrMode::Null => std::process::Stdio::null(),
            _ => std::process::Stdio::piped(),
        });
        let feed = stdin.is_buffered();
        cmd.stdin(if feed {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::inherit()
        });
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                sink.eline(&format!("{SHELL_NAME}: {name}: command not found"));
                if let Some(hint) = windows_tools_hint() {
                    sink.eline(&hint);
                }
                return 127;
            }
            Err(e) => {
                sink.eline(&format!("{SHELL_NAME}: {name}: {}", io_reason(&e)));
                return 126;
            }
        };
        let writer = if feed {
            let data = stdin.read_all();
            child.stdin.take().map(|mut w| {
                std::thread::spawn(move || {
                    let _ = w.write_all(&data);
                })
            })
        } else {
            None
        };
        let out = match child.wait_with_output() {
            Ok(o) => o,
            Err(e) => {
                sink.eline(&format!("{SHELL_NAME}: {name}: {}", io_reason(&e)));
                return 1;
            }
        };
        if let Some(w) = writer {
            let _ = w.join();
        }
        sink.out.extend_from_slice(&out.stdout);
        if !out.stderr.is_empty() {
            sink.ewrite(&out.stderr);
        }
        out.status.code().unwrap_or(128)
    }
}

/// The environment variable that counts how deeply scripts nest: a script's
/// child `ish -b` gets one more than its own. It is an `IRONSHELL_` name, so an
/// agent cannot set or reset it (see `safety::env_name_steers_children`).
const SCRIPT_DEPTH_ENV: &str = "IRONSHELL_SCRIPT_DEPTH";

/// The deepest a script may nest: a script at this depth starts no child.
const MAX_SCRIPT_DEPTH: u32 = 32;

/// The script a command runs in a child shell (see `Shell::run_script_child`):
/// `FILE args...` where FILE is a path (it has a slash or a backslash) to a
/// shell script, or `sh FILE args...` / `bash FILE args...` where FILE is an
/// existing file. An option before FILE (`sh -c ...`) is not this form.
fn shell_script_command(argv: &[String]) -> Option<(&str, &[String])> {
    // Elsewhere a script runs as the real shell, as it always has; Windows
    // has no `sh` to run it, and `./x.sh` failed with a Win32 error.
    if !cfg!(windows) {
        return None;
    }
    let head = argv.first()?.as_str();
    if head.contains(['/', '\\']) {
        // An existing file outside the read jail is routed here whatever it
        // holds, so run_script_child refuses it before its head is read or it
        // runs. Without this the shebang probe below would read it first.
        if has_file(head) && crate::security::outside_read_jail(head) {
            return Some((head, &argv[1..]));
        }
        return has_shell_shebang(head).then_some((head, &argv[1..]));
    }
    if matches!(head, "sh" | "bash") {
        let file = argv.get(1)?.as_str();
        if !file.starts_with('-') && has_file(file) {
            return Some((file, &argv[2..]));
        }
    }
    None
}

/// Whether `path` names an existing file, never probing a remote share.
fn has_file(path: &str) -> bool {
    !crate::safety::names_a_remote_share(path) && std::path::Path::new(path).is_file()
}

/// Whether `path` is a file whose first line is a shebang for `sh` or `bash`:
/// `#!/bin/sh`, `#!/bin/bash`, or `#!/usr/bin/env` with `sh` or `bash`, in any
/// directory, and with no further option on the line.
fn has_shell_shebang(path: &str) -> bool {
    use std::io::Read;
    if !has_file(path) {
        return false;
    }
    let mut head = [0u8; 256];
    let n = std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut head))
        .unwrap_or(0);
    let head = &head[..n];
    let end = head.iter().position(|&b| b == b'\n').unwrap_or(head.len());
    let Ok(line) = std::str::from_utf8(&head[..end]) else {
        return false;
    };
    let Some(spec) = line.trim_end_matches('\r').strip_prefix("#!") else {
        return false;
    };
    let words: Vec<&str> = spec.split_whitespace().collect();
    let interpreter = match words.as_slice() {
        [program] => *program,
        [env, name] if env.rsplit('/').next() == Some("env") => *name,
        _ => return false,
    };
    matches!(interpreter.rsplit('/').next(), Some("sh" | "bash"))
}

/// The commands the subset implements. On Windows, a bare name from this list
/// that the subset declines runs only as a GNU-compatible program found on
/// PATH, because a `sort.exe` that is not GNU would be the wrong program.
const SUBSET_NAMES: &[&str] = &[
    "echo", "printf", "test", "[", "cat", "tee", "head", "tail", "wc", "grep", "sort", "uniq",
    "cut", "sed", "ls", "find", "date", "basename", "dirname", "seq", "mkdir", "touch", "tr",
];

/// The GNU-compatible program called `name` on PATH, if there is one (Windows).
///
/// Windows searches the current folder and then System32 before PATH, so a
/// bare `sort` ran `System32\sort.exe`, which is not GNU sort, and a bare `find`
/// ran `find.exe`, which is not GNU find. This walks PATH itself, skips the
/// Windows folder, never searches a relative PATH entry (such as `.`), and
/// accepts a program only when its `--version` says GNU.
fn gnu_program(name: &str) -> Option<std::path::PathBuf> {
    path_candidates(name).find(|candidate| {
        let probe = crate::safety::child_command(candidate)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        match probe {
            Ok(out) => {
                let first = String::from_utf8_lossy(&out.stdout);
                out.status.success() && first.lines().next().is_some_and(|l| l.contains("GNU"))
            }
            Err(_) => false,
        }
    })
}

/// The files on PATH that are called `name`, in PATH order, as far as a file
/// test can tell: absolute PATH entries only, and on Windows never the Windows
/// folder. Nothing is run here, so the exec gate can use it before it admits a
/// program.
fn path_candidates(name: &str) -> impl Iterator<Item = std::path::PathBuf> {
    let dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    let windows_dir = if cfg!(windows) {
        std::env::var_os("SystemRoot").map(|r| r.to_string_lossy().to_lowercase())
    } else {
        None
    };
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    dirs.into_iter()
        .filter(|dir| dir.is_absolute())
        .filter(move |dir| match &windows_dir {
            Some(root) => !dir
                .to_string_lossy()
                .to_lowercase()
                .starts_with(root.as_str()),
            None => true,
        })
        .map(move |dir| dir.join(&file))
        .filter(|candidate| candidate.is_file())
}

/// The first PATH program called `name`, from an absolute PATH entry only, so
/// a relative entry such as `.` never runs a program from the working folder.
fn path_program(name: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The program `argv` runs, or the status and message of why it cannot.
///
/// On Windows a subset command goes to a GNU-compatible program; with none, the
/// error names what the subset does not support and the status is 2. Elsewhere
/// it runs the PATH program of that name, as it did before, and a missing one is
/// "command not found" (127). Any other name runs as typed.
fn resolve_program(
    argv: &[String],
    env: &[(String, String)],
    why: Option<&str>,
) -> Result<String, (i32, String)> {
    let name = argv[0].as_str();
    if name.contains(['/', '\\']) || !SUBSET_NAMES.contains(&name) {
        return Ok(name.to_string());
    }
    if !cfg!(windows) {
        return match path_program(name) {
            Some(p) => Ok(p.to_string_lossy().into_owned()),
            None => Err((127, format!("{SHELL_NAME}: {name}: command not found"))),
        };
    }
    if let Some(p) = gnu_program(name) {
        return Ok(p.to_string_lossy().into_owned());
    }
    // A sed command the subset does not run is named as that, not as a missing
    // tool: the rest of the script has run, and this command is what is outside.
    if let Some(part) = why.filter(|p| name == "sed" && p.starts_with("sed command")) {
        return Err((
            2,
            format!("{SHELL_NAME}: sed: {part} is not in the in-process subset, which runs p, d, q and s///"),
        ));
    }
    let what = if !env.is_empty() {
        "a variable before the command".to_string()
    } else if let Some(part) = why {
        part.to_string()
    } else {
        match argv[1..].iter().find(|a| a.len() > 1 && a.starts_with('-')) {
            Some(flag) => flag.clone(),
            None => "this form".to_string(),
        }
    };
    let mut msg = format!(
        "{SHELL_NAME}: {name}: {what} is not supported by the in-process subset, and no GNU-compatible {name} is installed"
    );
    if let Some(hint) = windows_tools_hint() {
        msg.push('\n');
        msg.push_str(&hint);
    }
    Err((2, msg))
}

/// On Windows, the refusal for a sed command the subset declines when no sed is
/// on PATH. `resolve_program` gives the same refusal, but only after the exec
/// gate has asked for approval of a program that is not installed; this gives
/// it first. `None` when this is not that case. A sed that is on PATH still goes
/// through the gate, which then checks that it is GNU.
fn sed_missing_refusal(argv: &[String], why: Option<&str>) -> Option<String> {
    if !cfg!(windows) || argv[0] != "sed" {
        return None;
    }
    why?;
    if path_candidates("sed").next().is_some() {
        return None;
    }
    // With no sed on PATH, `resolve_program` runs nothing and gives the
    // refusal it would give after the gate, word for word.
    resolve_program(argv, &[], why).err().map(|(_, msg)| msg)
}

/// On Windows, where to find the GNU tools a script expects: Git for Windows
/// ships them, but its `usr\bin` folder is not on PATH by default.
fn windows_tools_hint() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    Some(
        r"  hint: Git for Windows ships GNU tools in C:\Program Files\Git\usr\bin; put that folder on PATH, or do this step in ish (ish -c)"
            .to_string(),
    )
}

/// Judge a redirection's output file through the same guard as `file.write`,
/// and return the path it judged.
///
/// This runs before the command, so a refused file never runs its command. The
/// write then goes to the path returned here. It must not go to the path as
/// given, which the OS resolves against the working directory while the guard,
/// in agent mode, resolved it against the workspace: with the two apart,
/// `echo x > f` was approved as `<workspace>/f` and wrote `<cwd>/f`, outside
/// the jail.
fn check_write(path: &str) -> Result<String, String> {
    let resolved = crate::safety::resolve_path_str(path);
    crate::safety::guard(crate::safety::GuardCtx {
        builtin: "posix",
        effect: crate::safety::Effect::WriteLocal,
        what: "write",
        targets: vec![resolved.clone()],
        blast_radius: serde_json::json!({ "path": resolved }),
        reversible: true,
        fs_paths: true,
    })
    .map_err(|e| e.to_string())?;
    Ok(resolved)
}

/// [`check_write`] for a writer that writes the existing file in place (a `>`
/// or `>>` redirection, `tee`, `touch`): it also refuses a file with other hard
/// links, since writing through one could change a file outside the workspace.
/// `mkdir` creates nothing in place and uses [`check_write`].
fn check_write_in_place(path: &str) -> Result<String, String> {
    let resolved = check_write(path)?;
    crate::safety::refuse_hard_linked_write("posix", path).map_err(|e| e.to_string())?;
    Ok(resolved)
}

/// Write a redirection's output to `path`, a path `check_write` has judged. The
/// prior contents are journalled first, as the dispatcher does for `file.write`,
/// so the `reversible` the guard is told is true.
fn write_file(path: &str, data: &[u8], append: bool) -> Result<(), String> {
    let mark = crate::journal::mark();
    crate::journal::record_before(
        "posix",
        crate::safety::Effect::WriteLocal,
        &[crate::value::Value::Str(path.to_string())],
    );
    // The handle is judged for hard links and truncated only after that check.
    let res = crate::safety::open_in_place("posix", Path::new(path), append)
        .and_then(|mut f| f.write_all(data).map_err(anyhow::Error::from));
    crate::journal::settle(mark, res.is_ok());
    res.map_err(|e| match e.downcast_ref::<std::io::Error>() {
        Some(io) => format!("{SHELL_NAME}: {path}: {}", io_reason(io)),
        None => e.to_string(),
    })
}

/// The message GNU tools print for an I/O error.
fn io_reason(e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        NotFound => "No such file or directory".into(),
        PermissionDenied => "Permission denied".into(),
        IsADirectory => "Is a directory".into(),
        NotADirectory => "Not a directory".into(),
        AlreadyExists => "File exists".into(),
        _ => {
            if e.raw_os_error() == Some(21) {
                return "Is a directory".into();
            }
            let s = e.to_string();
            s.split(" (os error").next().unwrap_or(&s).to_string()
        }
    }
}

fn logical_pwd() -> String {
    let phys = std::env::current_dir().ok();
    if let (Ok(pwd), Some(phys)) = (std::env::var("PWD"), phys.as_ref()) {
        if std::fs::canonicalize(&pwd).ok().as_deref()
            == std::fs::canonicalize(phys).ok().as_deref()
        {
            return pwd;
        }
    }
    phys.map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ════════════════════════════════════════════════════════════════════════
// Collation and globbing
// ════════════════════════════════════════════════════════════════════════

/// Whether sorting follows a natural-language locale rather than bytes. `ls`
/// and `sort` both use `strcoll`, so under `en_US.UTF-8` they do not sort by
/// byte, and output that is meant to match theirs cannot either.
fn locale_collates() -> bool {
    let loc = ["LC_ALL", "LC_COLLATE", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_default();
    if loc.is_empty() || loc == "C" || loc == "POSIX" || loc.starts_with("C.") {
        return false;
    }
    locale_loads(&loc)
}

/// Whether glibc can load the collation the environment names. When it cannot
/// (the locale was never generated), `ls` and `sort` fall back to bytes, so
/// output meant to match theirs must too. Cached per name: a sort asks once
/// per comparison.
#[cfg(target_os = "linux")]
fn locale_loads(name: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static SEEN: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&ok) = seen.as_ref().and_then(|m| m.get(name)) {
        return ok;
    }
    // `newlocale` with "" resolves the same variables `setlocale` would,
    // without touching the process locale.
    let ok = unsafe {
        let l = libc::newlocale(libc::LC_COLLATE_MASK, c"".as_ptr(), std::ptr::null_mut());
        if l.is_null() {
            false
        } else {
            libc::freelocale(l);
            true
        }
    };
    seen.get_or_insert_with(HashMap::new)
        .insert(name.to_string(), ok);
    ok
}

#[cfg(not(target_os = "linux"))]
fn locale_loads(_name: &str) -> bool {
    true
}

/// glibc's ISO 14651 ordering for ASCII, as measured against glibc 2.41's
/// `en_US.UTF-8` (`printf … | sort`): compare letters and digits ignoring case
/// and punctuation; then lower case before upper, position by position; then
/// the full strings with punctuation ranked below letters and digits. Beyond
/// ASCII this falls back to bytes, which is where it stops being exact.
fn collate(a: &str, b: &str) -> std::cmp::Ordering {
    collate_in(locale_collates(), a, b)
}

fn collate_in(natural: bool, a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    if !natural {
        return a.as_bytes().cmp(b.as_bytes());
    }
    let l1 = |s: &str| -> Vec<char> {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    let o = l1(a).cmp(&l1(b));
    if o != Ordering::Equal {
        return o;
    }
    let l3 = |s: &str| -> Vec<u8> {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .map(|c| u8::from(c.is_uppercase()))
            .collect()
    };
    let o = l3(a).cmp(&l3(b));
    if o != Ordering::Equal {
        return o;
    }
    let l4 = |s: &str| -> Vec<(u8, u32)> {
        s.chars()
            .map(|c| (u8::from(c.is_alphanumeric()), c as u32))
            .collect()
    };
    let o = l4(a).cmp(&l4(b));
    if o != Ordering::Equal {
        return o;
    }
    a.as_bytes().cmp(b.as_bytes())
}

/// `fnmatch` for `*`, `?` and `[…]` (with `!`/`^` negation and ranges).
fn glob_match(pat: &str, s: &str) -> bool {
    fn go(p: &[char], s: &[char]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some('*') => (0..=s.len()).any(|k| go(&p[1..], &s[k..])),
            Some('?') => !s.is_empty() && go(&p[1..], &s[1..]),
            Some('[') => {
                let Some(&c) = s.first() else { return false };
                let mut k = 1;
                let negate = matches!(p.get(1), Some('!') | Some('^'));
                if negate {
                    k += 1;
                }
                let mut matched = false;
                let mut first = true;
                while k < p.len() && (p[k] != ']' || first) {
                    first = false;
                    if k + 2 < p.len() && p[k + 1] == '-' && p[k + 2] != ']' {
                        if p[k] <= c && c <= p[k + 2] {
                            matched = true;
                        }
                        k += 3;
                    } else {
                        if p[k] == c {
                            matched = true;
                        }
                        k += 1;
                    }
                }
                if k >= p.len() {
                    // No closing bracket: a literal '['.
                    return c == '[' && go(&p[1..], &s[1..]);
                }
                matched != negate && go(&p[k + 1..], &s[1..])
            }
            Some('\\') if p.len() > 1 => s.first() == Some(&p[1]) && go(&p[2..], &s[1..]),
            Some(&c) => s.first() == Some(&c) && go(&p[1..], &s[1..]),
        }
    }
    let p: Vec<char> = pat.chars().collect();
    let s: Vec<char> = s.chars().collect();
    go(&p, &s)
}

/// `s` with its glob characters escaped, so a pattern matches them literally.
fn escape_glob(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if matches!(c, '*' | '?' | '[' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `value` less what `op` removes: `%` and `%%` take the shortest and the
/// longest suffix that matches `pat`, `#` and `##` the shortest and the longest
/// prefix, as bash does. `value` itself when nothing matches.
fn trim_affix(value: &str, pat: &str, op: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    let n = chars.len();
    let prefix = op.starts_with('#');
    let longest = op.len() == 2;
    // How many characters to remove, the shortest removal first.
    let counts: Vec<usize> = if longest {
        (0..=n).rev().collect()
    } else {
        (0..=n).collect()
    };
    for k in counts {
        let (head, tail) = chars.split_at(if prefix { k } else { n - k });
        let (removed, kept) = if prefix { (head, tail) } else { (tail, head) };
        if glob_match(pat, &removed.iter().collect::<String>()) {
            return kept.iter().collect();
        }
    }
    value.to_string()
}

/// Pathname expansion, component by component, sorted as `ls` would sort.
/// `**` is an ordinary `*` here, as in bash without `globstar`.
/// `None` when some stage gathers more than `WALK_MAX_ENTRIES` names: the
/// caller refuses the expansion rather than run with part of it.
fn glob_expand(pattern: &str) -> Option<Vec<String>> {
    let (v, over) = expand_pattern(pattern, false, |a, b| collate(a, b), WALK_MAX);
    (!over).then_some(v)
}

/// Pathname expansion for the typed builtins (`glob`, `cat`, `ls`): `**`
/// matches any number of directories, and results sort by bytes, so the
/// answer does not depend on the locale the shell happens to run under.
pub fn glob_paths(pattern: &str) -> Vec<String> {
    glob_paths_capped(pattern, usize::MAX).0
}

/// `glob_paths`, but it stops gathering once more than `max` distinct names
/// have turned up, and says so: `(paths, true)` then holds only what was
/// gathered before the stop, and the caller must refuse. Below the cap the
/// paths are the same, sorted and without duplicates, as `glob_paths` gives.
pub fn glob_paths_capped(pattern: &str, max: usize) -> (Vec<String>, bool) {
    let (mut v, over) = expand_pattern(pattern, true, |a, b| a.as_bytes().cmp(b.as_bytes()), max);
    if over {
        return (Vec::new(), true);
    }
    v.sort();
    v.dedup();
    (v, false)
}

/// The cap the POSIX subset applies to a glob, the same as the typed
/// builtins' walks.
const WALK_MAX: usize = crate::builtins::WALK_MAX_ENTRIES;

/// The names one expansion stage has gathered. A stage that gathers more
/// than `max` distinct names is over, and stops as soon as it knows that.
/// Under `dedup` (for `**`, whose stages can repeat a name), duplicates are
/// removed before the count, so only distinct names count against the cap;
/// the list is kept small by settling it when it has doubled since the last
/// settle, not on every push.
struct Gather {
    out: Vec<String>,
    max: usize,
    dedup: bool,
    settle_at: usize,
    over: bool,
}

impl Gather {
    fn new(max: usize, dedup: bool) -> Self {
        Gather {
            out: Vec::new(),
            max,
            dedup,
            settle_at: max,
            over: false,
        }
    }

    fn push(&mut self, name: String) {
        if self.over {
            return;
        }
        self.out.push(name);
        if self.out.len() > self.settle_at {
            self.settle();
        }
    }

    fn settle(&mut self) {
        if self.dedup {
            self.out.sort_unstable();
            self.out.dedup();
        }
        if self.out.len() > self.max {
            self.over = true;
            self.out.clear();
        }
        self.settle_at = self.out.len().max(self.max).saturating_mul(2);
    }

    /// The stage's names, or `None` when it is over.
    fn finish(mut self) -> Option<Vec<String>> {
        self.settle();
        (!self.over).then_some(self.out)
    }
}

/// Whether a string contains glob metacharacters.
pub fn has_glob_meta(s: &str) -> bool {
    s.contains(['*', '?', '['])
}

/// The directories beneath `dir`, depth first, not following symbolic links
/// and skipping hidden names, as bash's `globstar` does. Stops once `out` is
/// over the cap.
fn descendant_dirs(dir: &str, out: &mut Gather) {
    let Ok(rd) = std::fs::read_dir(if dir.is_empty() { "." } else { dir }) else {
        return;
    };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    for n in names {
        if out.over {
            return;
        }
        let p = if dir.is_empty() {
            n
        } else if dir.ends_with('/') {
            format!("{dir}{n}")
        } else {
            format!("{dir}/{n}")
        };
        out.push(p.clone());
        descendant_dirs(&p, out);
    }
}

/// Expands `pattern` one component at a time. The second value is true when
/// some stage gathered more than `max` names; the names are then not
/// returned, and the caller must refuse.
fn expand_pattern(
    pattern: &str,
    globstar: bool,
    order: impl Fn(&String, &String) -> std::cmp::Ordering,
    max: usize,
) -> (Vec<String>, bool) {
    let absolute = pattern.starts_with('/');
    let comps: Vec<&str> = pattern.split('/').filter(|c| !c.is_empty()).collect();
    let mut bases: Vec<String> = vec![if absolute { "/".into() } else { String::new() }];
    for (i, comp) in comps.iter().enumerate() {
        let last = i + 1 == comps.len();
        let mut next = Gather::new(max, globstar);
        if globstar && *comp == "**" {
            // Zero or more directories: each base, and everything below it.
            for base in &bases {
                // A base outside the workspace jail is never walked (`/**` would
                // list the whole disk).
                if crate::security::outside_read_jail(base) {
                    continue;
                }
                next.push(base.clone());
                descendant_dirs(base, &mut next);
                if next.over {
                    return (Vec::new(), true);
                }
            }
            if last {
                // A trailing `**` also names the files in those directories.
                let dirs = next.out.clone();
                for d in dirs {
                    let dir = if d.is_empty() {
                        ".".to_string()
                    } else {
                        d.clone()
                    };
                    if let Ok(rd) = std::fs::read_dir(&dir) {
                        for e in rd.filter_map(|e| e.ok()) {
                            let n = e.file_name().to_string_lossy().into_owned();
                            if !n.starts_with('.')
                                && e.file_type().map(|t| !t.is_dir()).unwrap_or(false)
                            {
                                let p = if d.is_empty() { n } else { format!("{d}/{n}") };
                                if !crate::security::outside_read_jail(&p) {
                                    next.push(p);
                                }
                            }
                        }
                    }
                    if next.over {
                        return (Vec::new(), true);
                    }
                }
                next.out.retain(|p| !p.is_empty());
            }
            bases = match next.finish() {
                Some(v) => v,
                None => return (Vec::new(), true),
            };
            continue;
        }
        let has_meta = has_glob_meta(comp);
        for base in &bases {
            if next.over {
                break;
            }
            let join = |name: &str| {
                if base.is_empty() {
                    name.to_string()
                } else if base.ends_with('/') {
                    format!("{base}{name}")
                } else {
                    format!("{base}/{name}")
                }
            };
            if !has_meta {
                let p = join(comp);
                // The jail is judged before the path is probed, so the answer does
                // not say whether an outside path exists.
                if crate::security::outside_read_jail(&p) {
                    continue;
                }
                let path = Path::new(&p);
                if (last && path.exists()) || (!last && path.is_dir()) {
                    next.push(p);
                }
                continue;
            }
            let dir = if base.is_empty() { "." } else { base.as_str() };
            if crate::security::outside_read_jail(dir) {
                continue;
            }
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue;
            };
            let mut names: Vec<String> = rd
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.') || comp.starts_with('.'))
                .filter(|n| glob_match(comp, n))
                .collect();
            names.sort_by(&order);
            for n in names {
                let p = join(&n);
                // A link inside the workspace can name a target outside it.
                if crate::security::outside_read_jail(&p) {
                    continue;
                }
                if last || Path::new(&p).is_dir() {
                    next.push(p);
                }
            }
        }
        bases = match next.finish() {
            Some(v) => v,
            None => return (Vec::new(), true),
        };
    }
    if pattern.ends_with('/') {
        bases = bases.into_iter().map(|b| format!("{b}/")).collect();
    }
    (bases, false)
}

// ════════════════════════════════════════════════════════════════════════
// Utilities
// ════════════════════════════════════════════════════════════════════════

/// Read a utility's inputs: each operand in turn (`-` is stdin), or stdin
/// when there are none. Missing files are reported the GNU way and skipped.
/// The refusal for a utility's read of `path` outside the workspace jail, as
/// the JSON error every refusal uses. `None` when the jail lets the read
/// through, which it always does outside agent mode with no workspace set.
/// The bytes a utility prints, with secret shapes redacted when ish redacts
/// builtin output (`safety::redaction_enabled`), as `file.read` output is. Bytes
/// that are not UTF-8 are passed through: the redaction works on text.
fn redacted(bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
    if !crate::safety::redaction_enabled() {
        return std::borrow::Cow::Borrowed(bytes);
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return std::borrow::Cow::Borrowed(bytes);
    };
    let (out, _) = crate::safety::redact_str(text);
    if out == text {
        std::borrow::Cow::Borrowed(bytes)
    } else {
        std::borrow::Cow::Owned(out.into_bytes())
    }
}

fn jail_refusal(tool: &str, path: &str) -> Option<String> {
    if crate::security::outside_read_jail(path) {
        crate::security::audit_read_refusal(tool, path);
        return Some(crate::safety::outside_workspace(tool, path).to_string());
    }
    // Off unless the operator set IRONSHELL_REFUSE_LINKED_READS=1.
    crate::safety::refuse_hard_linked_read(tool, path)
        .err()
        .map(|e| e.to_string())
}

fn read_inputs(
    tool: &str,
    files: &[String],
    stdin: &mut Input,
    sink: &mut Sink,
    status: &mut i32,
) -> Vec<(String, Vec<u8>)> {
    if files.is_empty() {
        return vec![("(standard input)".into(), stdin.read_all())];
    }
    let mut out = Vec::new();
    for f in files {
        if f == "-" {
            out.push(("(standard input)".into(), stdin.read_all()));
            continue;
        }
        if let Some(refusal) = jail_refusal(tool, f) {
            sink.eline(&refusal);
            *status = 1;
            continue;
        }
        match std::fs::read(f) {
            Ok(d) => out.push((f.clone(), d)),
            Err(e) => {
                sink.eline(&format!("{tool}: {f}: {}", io_reason(&e)));
                *status = 1;
            }
        }
    }
    out
}

fn lines_of(data: &[u8]) -> Vec<&[u8]> {
    if data.is_empty() {
        return Vec::new();
    }
    let mut v: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();
    if data.ends_with(b"\n") {
        v.pop();
    }
    v
}

/// Split `-abc` style flags; returns (flags, operands) or `None` when an
/// option takes a value we do not model.
fn short_flags(args: &[String], allowed: &str) -> Option<(HashSet<char>, Vec<String>)> {
    let mut flags = HashSet::new();
    let mut ops = Vec::new();
    let mut done = false;
    for a in args {
        if !done && a == "--" {
            done = true;
        } else if !done && a.len() > 1 && a.starts_with('-') {
            for c in a[1..].chars() {
                if !allowed.contains(c) {
                    return None;
                }
                flags.insert(c);
            }
        } else {
            ops.push(a.clone());
        }
    }
    Some((flags, ops))
}

fn utility(name: &str, args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    match name {
        "echo" => echo(args, sink),
        "printf" => printf(args, sink),
        "test" => Util::Done(test(args, sink)),
        "[" => {
            if args.last().map(String::as_str) != Some("]") {
                sink.eline(&format!("{SHELL_NAME}: [: missing `]'"));
                return Util::Done(2);
            }
            Util::Done(test(&args[..args.len() - 1], sink))
        }
        "cat" => cat(args, stdin, sink),
        "tee" => tee(args, stdin, sink),
        "mkdir" => mkdir(args, sink),
        "touch" => touch(args, sink),
        "tr" => tr(args, stdin, sink),
        "head" => head_tail(true, args, stdin, sink),
        "tail" => head_tail(false, args, stdin, sink),
        "wc" => wc(args, stdin, sink),
        "grep" => grep(args, stdin, sink),
        "sort" => sort(args, stdin, sink),
        "uniq" => uniq(args, stdin, sink),
        "cut" => cut(args, stdin, sink),
        "sed" => sed(args, stdin, sink),
        "ls" => ls(args, sink),
        "find" => find(args, sink),
        "date" => date(args, sink),
        "basename" => match args {
            [p] | [p, _] => {
                let trimmed = p.trim_end_matches('/');
                let mut b = trimmed.rsplit('/').next().unwrap_or("").to_string();
                if trimmed.is_empty() && p.starts_with('/') {
                    b = "/".into();
                }
                if let [_, suffix] = args {
                    if b.len() > suffix.len() && b.ends_with(suffix.as_str()) {
                        b.truncate(b.len() - suffix.len());
                    }
                }
                sink.out.extend_from_slice(format!("{b}\n").as_bytes());
                Util::Done(0)
            }
            _ => Util::Decline,
        },
        "dirname" => match args {
            [p] => {
                let t = p.trim_end_matches('/');
                let d = match t.rfind('/') {
                    Some(0) => "/".to_string(),
                    Some(i) => t[..i].trim_end_matches('/').to_string(),
                    None => ".".to_string(),
                };
                let d = if t.is_empty() && p.starts_with('/') {
                    "/".into()
                } else {
                    d
                };
                sink.out.extend_from_slice(format!("{d}\n").as_bytes());
                Util::Done(0)
            }
            _ => Util::Decline,
        },
        "seq" => {
            let nums: Option<Vec<i64>> = args.iter().map(|a| a.parse().ok()).collect();
            let (first, step, last) = match nums.as_deref() {
                Some([l]) => (1, 1, *l),
                Some([f, l]) => (*f, 1, *l),
                Some([f, s, l]) if *s != 0 => (*f, *s, *l),
                _ => return Util::Decline,
            };
            // The whole output is gathered before it is written, so `seq 1
            // 9999999999` grew without limit; and `i += step` overflowed
            // near i64::MAX.
            let count = if (step > 0 && first > last) || (step < 0 && first < last) {
                0
            } else {
                (last as i128 - first as i128) / step as i128 + 1
            };
            if count > SEQ_MAX as i128 {
                sink.eline(&format!(
                    "seq: {count} numbers is more than {SEQ_MAX}; nothing was printed"
                ));
                return Util::Done(1);
            }
            let mut i = first;
            for _ in 0..count {
                sink.out.extend_from_slice(format!("{i}\n").as_bytes());
                match i.checked_add(step) {
                    Some(n) => i = n,
                    None => break,
                }
            }
            Util::Done(0)
        }
        _ => Util::Decline,
    }
}

/// The bytes of `s` with its backslash escapes decoded, as `echo -e` and `%b`
/// decode them, and whether a `\c` stopped the text. A `\xHH` (one or two hex
/// digits) or `\0NNN` is one raw byte, so `\xc3\xa9` is the UTF-8 for `é`. A
/// `\x` with no hex digit after it is kept as written, as bash does.
fn unescape_echo(s: &str) -> (Vec<u8>, bool) {
    let mut out: Vec<u8> = Vec::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            let mut utf8 = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
            continue;
        }
        match it.next() {
            Some('n') => out.push(b'\n'),
            Some('t') => out.push(b'\t'),
            Some('r') => out.push(b'\r'),
            Some('\\') => out.push(b'\\'),
            Some('a') => out.push(0x07),
            Some('b') => out.push(0x08),
            Some('e') => out.push(0x1b),
            Some('f') => out.push(0x0c),
            Some('v') => out.push(0x0b),
            Some('c') => return (out, true),
            Some('x') => {
                let mut v = 0u32;
                let mut digits = 0;
                while digits < 2 {
                    match it.peek().and_then(|d| d.to_digit(16)) {
                        Some(d) => {
                            v = v * 16 + d;
                            it.next();
                            digits += 1;
                        }
                        None => break,
                    }
                }
                if digits == 0 {
                    out.extend_from_slice(b"\\x");
                } else {
                    out.push(v as u8);
                }
            }
            Some('0') => {
                let mut v = 0u32;
                for _ in 0..3 {
                    match it.peek() {
                        Some(d @ '0'..='7') => {
                            v = v * 8 + (*d as u32 - '0' as u32);
                            it.next();
                        }
                        _ => break,
                    }
                }
                out.push(v as u8);
            }
            Some(o) => {
                out.push(b'\\');
                let mut utf8 = [0u8; 4];
                out.extend_from_slice(o.encode_utf8(&mut utf8).as_bytes());
            }
            None => out.push(b'\\'),
        }
    }
    (out, false)
}

/// `read`'s field splitting, as POSIX gives it: the line as `count` values.
/// Leading and trailing IFS white space is dropped; each run of IFS white space
/// delimits, and each non-white IFS character delimits on its own, so two in a
/// row make an empty field. The last value takes the rest of the line with its
/// trailing IFS white space dropped. An empty IFS splits nothing.
fn read_fields(line: &str, ifs: &str, count: usize) -> Vec<String> {
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n') && ifs.contains(c);
    let is_sep = |c: char| ifs.contains(c);
    let mut out = Vec::with_capacity(count);
    let mut rest = line.trim_start_matches(|c: char| is_ws(c));
    for _ in 1..count {
        let end = rest.find(|c: char| is_sep(c)).unwrap_or(rest.len());
        out.push(rest[..end].to_string());
        rest = rest[end..].trim_start_matches(|c: char| is_ws(c));
        if let Some(c) = rest.chars().next() {
            if is_sep(c) && !is_ws(c) {
                rest = rest[c.len_utf8()..].trim_start_matches(|c: char| is_ws(c));
            }
        }
    }
    out.push(rest.trim_end_matches(|c: char| is_ws(c)).to_string());
    out
}

fn echo(args: &[String], sink: &mut Sink) -> Util {
    let mut newline = true;
    let mut escapes = false;
    let mut k = 0;
    while let Some(a) = args.get(k) {
        if a.len() > 1 && a.starts_with('-') && a[1..].chars().all(|c| matches!(c, 'n' | 'e' | 'E'))
        {
            for c in a[1..].chars() {
                match c {
                    'n' => newline = false,
                    'e' => escapes = true,
                    _ => escapes = false,
                }
            }
            k += 1;
        } else {
            break;
        }
    }
    let text = args[k..].join(" ");
    let bytes = if escapes {
        let (t, stop) = unescape_echo(&text);
        if stop {
            newline = false;
        }
        t
    } else {
        text.into_bytes()
    };
    sink.out.extend_from_slice(&bytes);
    if newline {
        sink.out.push(b'\n');
    }
    Util::Done(0)
}

/// The most numbers `seq` prints: its output is gathered whole before it is
/// written.
const SEQ_MAX: usize = 10_000_000;
/// The widest `printf` field, as awk's printf allows.
const PRINTF_WIDTH_MAX: usize = 4096;

fn printf(args: &[String], sink: &mut Sink) -> Util {
    let Some(fmt) = args.first() else {
        sink.eline("printf: usage: printf format [arguments]");
        return Util::Done(2);
    };
    let fmt: Vec<char> = fmt.chars().collect();
    let mut rest = args[1..].iter();
    let mut out = String::new();
    loop {
        let mut consumed = false;
        let mut i = 0;
        while i < fmt.len() {
            let c = fmt[i];
            if c == '\\' {
                // `\xHH` takes up to two hex digits after the `x`.
                let mut end = (i + 2).min(fmt.len());
                if fmt.get(i + 1) == Some(&'x') {
                    while end < fmt.len() && end < i + 4 && fmt[end].is_ascii_hexdigit() {
                        end += 1;
                    }
                }
                let (e, _) = unescape_echo(&fmt[i..end].iter().collect::<String>());
                out.push_str(&String::from_utf8_lossy(&e));
                i = end;
                continue;
            }
            if c != '%' {
                out.push(c);
                i += 1;
                continue;
            }
            i += 1;
            if fmt.get(i) == Some(&'%') {
                out.push('%');
                i += 1;
                continue;
            }
            let mut spec = String::new();
            while let Some(&f) = fmt.get(i) {
                if f == '-' || f == '0' || f.is_ascii_digit() || f == '.' {
                    spec.push(f);
                    i += 1;
                } else {
                    break;
                }
            }
            let conv = fmt.get(i).copied().unwrap_or('s');
            i += 1;
            let arg = rest.next();
            consumed |= arg.is_some();
            let arg = arg.cloned().unwrap_or_default();
            let left = spec.starts_with('-');
            let zero = spec.trim_start_matches('-').starts_with('0');
            // A width is padding allocated up front: `%999999999999s` asked
            // for a terabyte, and a width too long to parse was read as 0.
            let digits = spec
                .trim_start_matches(['-', '0'])
                .split('.')
                .next()
                .unwrap_or("");
            let width: usize = if digits.is_empty() {
                0
            } else {
                match digits.parse::<usize>() {
                    Ok(w) if w <= PRINTF_WIDTH_MAX => w,
                    _ => {
                        sink.eline(&format!(
                            "printf: field width {digits} is more than {PRINTF_WIDTH_MAX}"
                        ));
                        return Util::Done(1);
                    }
                }
            };
            // `.N` is a precision: how many characters of a string, how many
            // decimals of a float. It has no meaning for an integer here, so an
            // integer with one is declined.
            let precision = match spec.split_once('.') {
                None => None,
                Some((_, "")) => Some(0),
                Some((_, p)) => match p.parse::<usize>() {
                    Ok(p) if p <= PRINTF_WIDTH_MAX => Some(p),
                    _ => return Util::Decline,
                },
            };
            let body = match conv {
                's' | 'b' => {
                    let text = if conv == 'b' {
                        String::from_utf8_lossy(&unescape_echo(&arg).0).into_owned()
                    } else {
                        arg
                    };
                    match precision {
                        Some(p) => text.chars().take(p).collect(),
                        None => text,
                    }
                }
                'd' | 'i' | 'x' | 'X' | 'o' if precision.is_some() => return Util::Decline,
                'd' | 'i' => match arg.trim().parse::<i64>() {
                    Ok(n) => n.to_string(),
                    Err(_) if arg.is_empty() => "0".into(),
                    Err(_) => return Util::Decline,
                },
                'x' | 'X' | 'o' => match arg.trim().parse::<i64>() {
                    // As bash does, a negative number is shown as 64 bits.
                    Ok(n) => match conv {
                        'x' => format!("{:x}", n as u64),
                        'X' => format!("{:X}", n as u64),
                        _ => format!("{:o}", n as u64),
                    },
                    Err(_) if arg.is_empty() => "0".into(),
                    Err(_) => return Util::Decline,
                },
                'f' => match arg.trim().parse::<f64>() {
                    Ok(v) if v.is_finite() => format!("{:.*}", precision.unwrap_or(6), v),
                    Err(_) if arg.is_empty() => format!("{:.*}", precision.unwrap_or(6), 0.0),
                    _ => return Util::Decline,
                },
                'e' | 'E' | 'g' | 'G' => {
                    let value = match arg.trim().parse::<f64>() {
                        Ok(v) if v.is_finite() => v,
                        Err(_) if arg.is_empty() => 0.0,
                        _ => return Util::Decline,
                    };
                    let upper = conv.is_ascii_uppercase();
                    if matches!(conv, 'e' | 'E') {
                        c_exp(value, precision.unwrap_or(6), upper)
                    } else {
                        c_general(value, precision.unwrap_or(6), upper)
                    }
                }
                'c' => arg.chars().next().map(String::from).unwrap_or_default(),
                _ => return Util::Decline,
            };
            let pad = width.saturating_sub(body.chars().count());
            if left {
                out.push_str(&body);
                out.push_str(&" ".repeat(pad));
            } else if zero
                && matches!(
                    conv,
                    'd' | 'i' | 'x' | 'X' | 'o' | 'f' | 'e' | 'E' | 'g' | 'G'
                )
            {
                let (sign, digits) = body
                    .strip_prefix('-')
                    .map_or(("", body.as_str()), |d| ("-", d));
                out.push_str(sign);
                out.push_str(&"0".repeat(pad));
                out.push_str(digits);
            } else {
                out.push_str(&" ".repeat(pad));
                out.push_str(&body);
            }
        }
        if !consumed || rest.len() == 0 {
            break;
        }
    }
    sink.out.extend_from_slice(out.as_bytes());
    Util::Done(0)
}

/// C's `%e`: one digit, the decimals, then `e`, a sign and at least two
/// exponent digits (`1.234568e+04`). Rust's `{:e}` writes `1.234568e4`, so the
/// exponent is rewritten.
fn c_exp(v: f64, precision: usize, upper: bool) -> String {
    let s = format!("{:.*e}", precision, v);
    let (mantissa, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    format!(
        "{mantissa}{}{}{:02}",
        if upper { 'E' } else { 'e' },
        if exp < 0 { '-' } else { '+' },
        exp.abs()
    )
}

/// C's `%g`: `precision` significant digits (6 by default, 1 for 0). The `%e`
/// form is used when the exponent is below -4 or at least the precision, else
/// `%f`; trailing zeros of the fraction are removed (the `#` flag is not in the
/// subset).
fn c_general(v: f64, precision: usize, upper: bool) -> String {
    let p = precision.max(1);
    // The exponent `%e` would show with this precision decides the form.
    let probe = format!("{:.*e}", p - 1, v);
    let x: i64 = probe
        .split_once('e')
        .and_then(|(_, e)| e.parse().ok())
        .unwrap_or(0);
    fn strip_zeros(s: &str) -> String {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s.to_string()
        }
    }
    if x < -4 || x >= p as i64 {
        let s = c_exp(v, p - 1, upper);
        let at = s.find(['e', 'E']).unwrap_or(s.len());
        format!("{}{}", strip_zeros(&s[..at]), &s[at..])
    } else {
        let decimals = (p as i64 - 1 - x) as usize;
        strip_zeros(&format!("{:.*}", decimals, v))
    }
}

fn test(args: &[String], sink: &mut Sink) -> i32 {
    fn unary(op: &str, a: &str) -> Option<bool> {
        let md = std::fs::metadata(a);
        Some(match op {
            "-e" => md.is_ok(),
            "-f" => md.map(|m| m.is_file()).unwrap_or(false),
            "-d" => md.map(|m| m.is_dir()).unwrap_or(false),
            "-s" => md.map(|m| m.len() > 0).unwrap_or(false),
            "-r" | "-w" | "-x" => md.is_ok(),
            "-L" | "-h" => std::fs::symlink_metadata(a)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false),
            "-z" => a.is_empty(),
            "-n" => !a.is_empty(),
            _ => return None,
        })
    }
    fn binary(a: &str, op: &str, b: &str) -> Option<Result<bool, String>> {
        let num = |s: &str| {
            s.trim()
                .parse::<i64>()
                .map_err(|_| format!("{s}: integer expression expected"))
        };
        Some(match op {
            "=" | "==" => Ok(a == b),
            "!=" => Ok(a != b),
            "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge" => num(a).and_then(|x| {
                num(b).map(|y| match op {
                    "-eq" => x == y,
                    "-ne" => x != y,
                    "-lt" => x < y,
                    "-le" => x <= y,
                    "-gt" => x > y,
                    _ => x >= y,
                })
            }),
            _ => return None,
        })
    }
    fn eval(a: &[String], sink: &mut Sink) -> Result<bool, i32> {
        // -a / -o, lowest precedence first.
        if let Some(p) = a.iter().rposition(|x| x == "-o") {
            if p > 0 && p + 1 < a.len() {
                return Ok(eval(&a[..p], sink)? || eval(&a[p + 1..], sink)?);
            }
        }
        if let Some(p) = a.iter().rposition(|x| x == "-a") {
            if p > 0 && p + 1 < a.len() {
                return Ok(eval(&a[..p], sink)? && eval(&a[p + 1..], sink)?);
            }
        }
        match a {
            [] => Ok(false),
            [x] => Ok(!x.is_empty()),
            [bang, rest @ ..] if bang == "!" => Ok(!eval(rest, sink)?),
            [op, x] => {
                // A file test on a path outside the jail is refused, not answered:
                // the answer would say whether the path exists outside.
                let file_test = matches!(
                    op.as_str(),
                    "-e" | "-f" | "-d" | "-s" | "-r" | "-w" | "-x" | "-L" | "-h"
                );
                if let Some(refusal) = file_test.then(|| jail_refusal("test", x)).flatten() {
                    sink.eline(&refusal);
                    return Err(2);
                }
                match unary(op, x) {
                    Some(v) => Ok(v),
                    None => {
                        sink.eline(&format!(
                            "{SHELL_NAME}: test: {op}: unary operator expected"
                        ));
                        Err(2)
                    }
                }
            }
            [x, op, y] => match binary(x, op, y) {
                Some(Ok(v)) => Ok(v),
                Some(Err(m)) => {
                    sink.eline(&format!("{SHELL_NAME}: test: {m}"));
                    Err(2)
                }
                None => {
                    sink.eline(&format!(
                        "{SHELL_NAME}: test: {op}: binary operator expected"
                    ));
                    Err(2)
                }
            },
            _ => {
                sink.eline(&format!("{SHELL_NAME}: test: too many arguments"));
                Err(2)
            }
        }
    }
    match eval(args, sink) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(c) => c,
    }
}

fn cat(args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    let Some((flags, files)) = short_flags(args, "n") else {
        return Util::Decline;
    };
    let number = flags.contains(&'n');
    let mut status = 0;
    let mut line_no = 0usize;
    for (_, data) in read_inputs_dirs("cat", &files, stdin, sink, &mut status) {
        if !number {
            sink.out.extend_from_slice(&data);
            continue;
        }
        // GNU `cat -n`: a number six wide and a tab before each line, counted
        // across every file. A last line with no newline gets none added.
        for line in data.split_inclusive(|&b| b == b'\n') {
            line_no += 1;
            sink.out
                .extend_from_slice(format!("{line_no:>6}\t").as_bytes());
            sink.out.extend_from_slice(line);
        }
    }
    Util::Done(status)
}

/// `tee [-a] FILE...`: copy stdin to stdout and to each FILE.
///
/// A FILE is written only after the same guard a `>` redirection passes
/// ([`check_write`]), so `tee ../f` is refused and writes nothing, and the other
/// FILEs and stdout still get their copy, as GNU tee does. `/dev/null` is
/// special-cased as a redirection is. `-a` appends.
fn tee(args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    let Some((flags, files)) = short_flags(args, "a") else {
        return Util::Decline;
    };
    let append = flags.contains(&'a');
    let data = stdin.read_all();
    sink.out.extend_from_slice(&data);
    let mut status = 0;
    for f in &files {
        if f == "/dev/null" {
            continue;
        }
        let resolved = match check_write_in_place(f) {
            Ok(p) => p,
            Err(msg) => {
                sink.eline(&msg);
                status = 1;
                continue;
            }
        };
        if let Err(msg) = write_file(&resolved, &data, append) {
            sink.eline(&msg);
            status = 1;
        }
    }
    Util::Done(status)
}

/// `mkdir [-p] DIR...`: each directory is judged by the jail as a `>` target is,
/// so a directory outside the workspace is refused and nothing is made.
fn mkdir(args: &[String], sink: &mut Sink) -> Util {
    let Some((flags, dirs)) = short_flags(args, "p") else {
        return Util::Decline;
    };
    if dirs.is_empty() {
        sink.eline("mkdir: missing operand");
        return Util::Done(1);
    }
    let parents = flags.contains(&'p');
    let mut status = 0;
    for d in &dirs {
        let resolved = match check_write(d) {
            Ok(p) => p,
            Err(msg) => {
                sink.eline(&msg);
                status = 1;
                continue;
            }
        };
        let res = if parents {
            std::fs::create_dir_all(&resolved)
        } else {
            std::fs::create_dir(&resolved)
        };
        if let Err(e) = res {
            let why = if e.kind() == std::io::ErrorKind::AlreadyExists {
                "File exists".to_string()
            } else {
                io_reason(&e)
            };
            sink.eline(&format!("mkdir: cannot create directory '{d}': {why}"));
            status = 1;
        }
    }
    Util::Done(status)
}

/// `touch FILE...`: a missing file is created empty, an existing one gets the
/// current modification time. Each file is judged by the jail as a `>` target.
fn touch(args: &[String], sink: &mut Sink) -> Util {
    let Some((_, files)) = short_flags(args, "") else {
        return Util::Decline;
    };
    if files.is_empty() {
        sink.eline("touch: missing file operand");
        return Util::Done(1);
    }
    let mut status = 0;
    for f in &files {
        let resolved = match check_write_in_place(f) {
            Ok(p) => p,
            Err(msg) => {
                sink.eline(&msg);
                status = 1;
                continue;
            }
        };
        let res = if Path::new(&resolved).exists() {
            crate::safety::open_in_place("posix", Path::new(&resolved), true)
                .and_then(|file| {
                    file.set_modified(std::time::SystemTime::now())
                        .map_err(Into::into)
                })
                .map_err(|e| match e.downcast_ref::<std::io::Error>() {
                    Some(io) => format!("touch: cannot touch '{f}': {}", io_reason(io)),
                    None => e.to_string(),
                })
        } else {
            write_file(&resolved, b"", true)
                .map_err(|_| format!("touch: cannot touch '{f}': No such file or directory"))
        };
        if let Err(msg) = res {
            sink.eline(&msg);
            status = 1;
        }
    }
    Util::Done(status)
}

/// The characters of a `tr` set: ranges (`a-z`), the classes `[:upper:]`,
/// `[:lower:]`, `[:digit:]`, `[:space:]`, `[:alpha:]`, `[:alnum:]`, and the
/// escapes `\n`, `\t`, `\r`, `\\`. `None` for anything else (`[=c=]`, `[c*n]`,
/// octal escapes), so the caller declines.
fn tr_set(spec: &str) -> Option<Vec<char>> {
    let chars: Vec<char> = spec.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' && chars.get(i + 1) == Some(&':') {
            let rest: String = chars[i..].iter().collect();
            let end = rest.find(":]")?;
            let class = &rest[2..end];
            let members: Vec<char> = match class {
                "upper" => ('A'..='Z').collect(),
                "lower" => ('a'..='z').collect(),
                "digit" => ('0'..='9').collect(),
                "space" => vec![' ', '\t', '\n', '\r', '\x0b', '\x0c'],
                "alpha" => ('A'..='Z').chain('a'..='z').collect(),
                "alnum" => ('0'..='9').chain('A'..='Z').chain('a'..='z').collect(),
                _ => return None,
            };
            out.extend(members);
            i += rest[..end + 2].chars().count();
            continue;
        }
        if chars[i] == '[' {
            // `[c*n]` and `[=c=]` are not supported.
            if chars.get(i + 1) == Some(&'=') || chars[i + 1..].contains(&'*') {
                return None;
            }
        }
        let (c, width) = if chars[i] == '\\' {
            let e = match chars.get(i + 1)? {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                '\\' => '\\',
                '-' => '-',
                _ => return None,
            };
            (e, 2)
        } else {
            (chars[i], 1)
        };
        if chars.get(i + width) == Some(&'-') && i + width + 1 < chars.len() {
            let hi = chars[i + width + 1];
            if hi == '\\' || hi < c {
                return None;
            }
            out.extend(c..=hi);
            i += width + 2;
        } else {
            out.push(c);
            i += width;
        }
    }
    Some(out)
}

/// `tr SET1 SET2`, `tr -d SET1`, `tr -s SET1` and `tr -s SET1 SET2`, over
/// characters. SET2 shorter than SET1 is padded with its last character, as
/// GNU tr does. Other options (`-c`, `-t`) and set forms decline to the real
/// program.
fn tr(args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    let Some((flags, sets)) = short_flags(args, "ds") else {
        return Util::Decline;
    };
    let delete = flags.contains(&'d');
    let squeeze = flags.contains(&'s');
    let parsed: Option<Vec<Vec<char>>> = sets.iter().map(|s| tr_set(s)).collect();
    let Some(parsed) = parsed else {
        return Util::Decline;
    };
    let (set1, set2) = match (delete, squeeze, parsed.as_slice()) {
        (true, false, [a]) => (a.clone(), Vec::new()),
        (false, true, [a]) => (a.clone(), Vec::new()),
        (false, _, [a, b]) if !b.is_empty() => (a.clone(), b.clone()),
        (false, false, [_]) => {
            sink.eline(&format!(
                "tr: missing operand after '{}'\nTry 'tr --help' for more information.",
                sets[0]
            ));
            return Util::Done(1);
        }
        _ => return Util::Decline,
    };
    let text = String::from_utf8_lossy(&stdin.read_all()).into_owned();
    let translate = !delete && !set2.is_empty();
    let mut map = std::collections::HashMap::new();
    if translate {
        let last = *set2.last().unwrap_or(&' ');
        for (k, c) in set1.iter().enumerate() {
            // The first mapping of a repeated character wins, as in GNU tr.
            map.entry(*c).or_insert(*set2.get(k).unwrap_or(&last));
        }
    }
    // -s squeezes runs of the characters of the last set given.
    let squeeze_set: &[char] = if translate { &set2 } else { &set1 };
    let mut out = String::with_capacity(text.len());
    let mut prev: Option<char> = None;
    for c in text.chars() {
        if delete && set1.contains(&c) {
            continue;
        }
        let c = if translate {
            *map.get(&c).unwrap_or(&c)
        } else {
            c
        };
        if squeeze && prev == Some(c) && squeeze_set.contains(&c) {
            continue;
        }
        out.push(c);
        prev = Some(c);
    }
    sink.out.extend_from_slice(out.as_bytes());
    Util::Done(0)
}

/// Like [`read_inputs`], but a directory operand is the GNU "Is a directory"
/// error rather than an unreadable file.
fn read_inputs_dirs(
    tool: &str,
    files: &[String],
    stdin: &mut Input,
    sink: &mut Sink,
    status: &mut i32,
) -> Vec<(String, Vec<u8>)> {
    let mut ok = Vec::new();
    for f in files {
        if let Some(refusal) = jail_refusal(tool, f) {
            sink.eline(&refusal);
            *status = 1;
        } else if Path::new(f).is_dir() {
            sink.eline(&format!("{tool}: {f}: Is a directory"));
            *status = 1;
        } else {
            ok.push(f.clone());
        }
    }
    if !files.is_empty() && ok.is_empty() {
        return Vec::new();
    }
    read_inputs(tool, &ok, stdin, sink, status)
}

/// `head` and `tail`: `-n N`, `-nN`, `-N`, `-c N`, and `tail -n +N`.
fn head_tail(head: bool, args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    let tool = if head { "head" } else { "tail" };
    let mut count: i64 = 10;
    let mut from_start = false;
    let mut bytes = false;
    let mut quiet = false;
    let mut files = Vec::new();
    let mut i = 0;
    let parse_n = |s: &str, from_start: &mut bool| -> Option<i64> {
        let s = if let Some(r) = s.strip_prefix('+') {
            *from_start = true;
            r
        } else {
            s
        };
        s.parse::<i64>().ok()
    };
    while i < args.len() {
        let a = &args[i];
        if a == "-n" || a == "-c" {
            bytes = a == "-c";
            let v = args.get(i + 1).and_then(|v| parse_n(v, &mut from_start));
            match v {
                Some(v) => count = v,
                None => return Util::Decline,
            }
            i += 2;
            continue;
        }
        if let Some(v) = a.strip_prefix("-n").or_else(|| a.strip_prefix("-c")) {
            if !v.is_empty() {
                bytes = a.starts_with("-c");
                match parse_n(v, &mut from_start) {
                    Some(v) => count = v,
                    None => return Util::Decline,
                }
                i += 1;
                continue;
            }
        }
        if a == "-q" {
            quiet = true;
        } else if a.len() > 1 && a.starts_with('-') && a[1..].chars().all(|c| c.is_ascii_digit()) {
            count = a[1..].parse().unwrap_or(10);
        } else if a.starts_with('-') && a != "-" {
            return Util::Decline;
        } else {
            files.push(a.clone());
        }
        i += 1;
    }
    let mut status = 0;
    let inputs = read_inputs_dirs(tool, &files, stdin, sink, &mut status);
    let many = inputs.len() > 1 && !quiet;
    for (k, (name, data)) in inputs.iter().enumerate() {
        if many {
            if k > 0 {
                sink.out.push(b'\n');
            }
            sink.out
                .extend_from_slice(format!("==> {name} <==\n").as_bytes());
        }
        if bytes {
            let n = data.len();
            let slice = if head {
                if count >= 0 {
                    &data[..(count as usize).min(n)]
                } else {
                    &data[..n.saturating_sub((-count) as usize)]
                }
            } else if from_start {
                &data[(count.max(1) as usize - 1).min(n)..]
            } else {
                &data[n.saturating_sub(count.unsigned_abs() as usize)..]
            };
            sink.out.extend_from_slice(slice);
            continue;
        }
        // Lines, keeping the file's own terminators.
        let mut starts = vec![0usize];
        for (j, &b) in data.iter().enumerate() {
            if b == b'\n' && j + 1 < data.len() {
                starts.push(j + 1);
            }
        }
        let nlines = if data.is_empty() { 0 } else { starts.len() };
        let (from, to) = if head {
            if count >= 0 {
                (0, (count as usize).min(nlines))
            } else {
                (0, nlines.saturating_sub((-count) as usize))
            }
        } else if from_start {
            ((count.max(1) as usize - 1).min(nlines), nlines)
        } else {
            (nlines.saturating_sub(count.unsigned_abs() as usize), nlines)
        };
        if from < to {
            let a = starts[from];
            let b = if to < nlines { starts[to] } else { data.len() };
            sink.out.extend_from_slice(&data[a..b]);
        }
    }
    Util::Done(status)
}

fn wc(args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    let Some((flags, files)) = short_flags(args, "lwcm") else {
        return Util::Decline;
    };
    let (mut l, mut w, mut c, m) = (
        flags.contains(&'l'),
        flags.contains(&'w'),
        flags.contains(&'c'),
        flags.contains(&'m'),
    );
    if !(l || w || c || m) {
        l = true;
        w = true;
        c = true;
    }
    let mut status = 0;
    let from_stdin = files.is_empty();
    let inputs = read_inputs_dirs("wc", &files, stdin, sink, &mut status);
    let count = |d: &[u8]| -> [usize; 4] {
        let text = String::from_utf8_lossy(d);
        [
            d.iter().filter(|&&b| b == b'\n').count(),
            text.split_ascii_whitespace().count(),
            text.chars().count(),
            d.len(),
        ]
    };
    let rows: Vec<(String, [usize; 4])> =
        inputs.iter().map(|(n, d)| (n.clone(), count(d))).collect();
    let mut total = [0usize; 4];
    for (_, r) in &rows {
        for (t, v) in total.iter_mut().zip(r) {
            *t += v;
        }
    }
    let selected = |r: &[usize; 4]| {
        let mut v = Vec::new();
        if l {
            v.push(r[0]);
        }
        if w {
            v.push(r[1]);
        }
        if m {
            v.push(r[2]);
        }
        if c {
            v.push(r[3]);
        }
        v
    };
    let ncols = usize::from(l) + usize::from(w) + usize::from(c) + usize::from(m);
    // GNU pads to the width of the total byte count when every input is a
    // file, to 7 when one is a pipe, and not at all for a single number.
    let width = if ncols == 1 && rows.len() <= 1 {
        0
    } else if from_stdin || files.iter().any(|f| f == "-") {
        7
    } else {
        let sizes: u64 = files
            .iter()
            .filter_map(|f| std::fs::metadata(f).ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .sum();
        sizes.to_string().len().max(1)
    };
    let mut emit = |vals: Vec<usize>, name: Option<&str>| {
        let s: Vec<String> = vals.iter().map(|v| format!("{v:>width$}")).collect();
        let mut line = s.join(" ");
        if let Some(n) = name {
            line.push(' ');
            line.push_str(n);
        }
        line.push('\n');
        sink.out.extend_from_slice(line.as_bytes());
    };
    for (name, r) in &rows {
        emit(
            selected(r),
            if from_stdin {
                None
            } else {
                Some(name.as_str())
            },
        );
    }
    if rows.len() > 1 {
        emit(selected(&total), Some("total"));
    }
    Util::Done(status)
}

/// A POSIX basic regular expression as a Rust regex: in BRE, `+ ? | ( ) { }`
/// are literal and their backslashed forms are the operators.
pub(crate) fn bre_to_regex(p: &str) -> Option<String> {
    let mut out = String::new();
    let mut it = p.chars().peekable();
    let mut first = true;
    while let Some(c) = it.next() {
        match c {
            '\\' => match it.next() {
                Some(o @ ('+' | '?' | '|' | '(' | ')' | '{' | '}')) => out.push(o),
                Some(d @ '1'..='9') => {
                    let _ = d;
                    return None; // back-references
                }
                Some(o @ ('w' | 'W' | 's' | 'S' | 'b' | 'B')) => {
                    out.push('\\');
                    out.push(o);
                }
                Some('<') | Some('>') => out.push_str("\\b"),
                Some(o) => out.push_str(&regex::escape(&o.to_string())),
                None => out.push_str("\\\\"),
            },
            '+' | '?' | '|' | '(' | ')' | '{' | '}' => out.push_str(&regex::escape(&c.to_string())),
            '*' if first => out.push_str("\\*"),
            _ => out.push(c),
        }
        first = false;
    }
    Some(out)
}

/// An extended regex: already close to Rust's, but back-references are not.
fn ere_to_regex(p: &str) -> Option<String> {
    if p.contains("\\1") || p.contains("\\2") {
        return None;
    }
    Some(p.replace("\\<", "\\b").replace("\\>", "\\b"))
}

fn grep(args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    let mut flags = HashSet::new();
    let mut patterns: Vec<String> = Vec::new();
    let mut ops = Vec::new();
    let mut i = 0;
    let mut done = false;
    while i < args.len() {
        let a = &args[i];
        if !done && a == "--" {
            done = true;
        } else if !done && a == "-e" {
            match args.get(i + 1) {
                Some(p) => patterns.push(p.clone()),
                None => return Util::Decline,
            }
            i += 1;
        } else if !done && a.starts_with("--") {
            return Util::Decline;
        } else if !done && a.len() > 1 && a.starts_with('-') {
            for c in a[1..].chars() {
                if !"rRnclLivEFwoqshHx".contains(c) {
                    return Util::Decline;
                }
                flags.insert(c);
            }
        } else {
            ops.push(a.clone());
        }
        i += 1;
    }
    if patterns.is_empty() {
        if ops.is_empty() {
            sink.eline("Usage: grep [OPTION]... PATTERNS [FILE]...");
            return Util::Done(2);
        }
        patterns.push(ops.remove(0));
    }
    let f = |c| flags.contains(&c);
    let recursive = f('r') || f('R');
    let mut alts = Vec::new();
    for p in patterns.iter().flat_map(|p| p.split('\n')) {
        let r = if f('F') {
            Some(regex::escape(p))
        } else if f('E') {
            ere_to_regex(p)
        } else {
            bre_to_regex(p)
        };
        let Some(mut r) = r else { return Util::Decline };
        if f('w') {
            r = format!("\\b(?:{r})\\b");
        }
        if f('x') {
            r = format!("^(?:{r})$");
        }
        alts.push(format!("(?:{r})"));
    }
    let src = format!("{}{}", if f('i') { "(?i)" } else { "" }, alts.join("|"));
    let Ok(re) = regex::bytes::Regex::new(&src) else {
        return Util::Decline;
    };

    // Collect (display name, path or stdin) in GNU's order.
    let mut status_err = false;
    let mut targets: Vec<(String, Option<String>)> = Vec::new();
    if ops.is_empty() {
        if recursive {
            if let Some(refusal) = jail_refusal("grep", ".") {
                sink.eline(&refusal);
                return Util::Done(2);
            }
            walk_files(".", "", &mut targets, true);
        } else {
            targets.push(("(standard input)".into(), None));
        }
    } else {
        for op in &ops {
            let p = Path::new(op);
            if op == "-" {
                targets.push(("(standard input)".into(), None));
            } else if let Some(refusal) = jail_refusal("grep", op) {
                // Refused before the operand is looked at, so an outside directory
                // is not walked and an outside file is not stat'd.
                sink.eline(&refusal);
                status_err = true;
            } else if p.is_dir() {
                if recursive {
                    walk_files(op, op, &mut targets, false);
                } else if !f('s') {
                    sink.eline(&format!("grep: {op}: Is a directory"));
                }
            } else if p.exists() {
                targets.push((op.clone(), Some(op.clone())));
            } else {
                if !f('s') {
                    sink.eline(&format!("grep: {op}: No such file or directory"));
                }
                status_err = true;
            }
        }
    }
    if targets.len() > crate::builtins::WALK_MAX_ENTRIES {
        sink.eline(&format!(
            "grep: more than {} files under the search path; nothing was searched",
            crate::builtins::WALK_MAX_ENTRIES
        ));
        return Util::Done(2);
    }
    // GNU prints file names for several operands, or when -r walks a directory
    // (an operand that is a directory, or no operand at all). One file operand
    // under -r prints no name.
    let walks_dir = recursive && (ops.is_empty() || ops.iter().any(|o| Path::new(o).is_dir()));
    let prefix = (targets.len() > 1 || walks_dir || ops.len() > 1) && !f('h') || f('H');
    let mut any = false;
    for (name, path) in &targets {
        let data = match path {
            Some(p) => match std::fs::read(p) {
                Ok(d) => d,
                Err(e) => {
                    if !f('s') {
                        sink.eline(&format!("grep: {name}: {}", io_reason(&e)));
                    }
                    status_err = true;
                    continue;
                }
            },
            None => stdin.read_all(),
        };
        let binary = data.iter().take(32 * 1024).any(|&b| b == 0);
        let mut count = 0usize;
        let mut out = Vec::new();
        for (n, line) in lines_of(&data).into_iter().enumerate() {
            let hit = re.is_match(line) != f('v');
            if !hit {
                continue;
            }
            count += 1;
            if f('q') || f('l') || f('L') || f('c') || binary {
                continue;
            }
            let lead = {
                let mut s = Vec::new();
                if prefix {
                    s.extend_from_slice(name.as_bytes());
                    s.push(b':');
                }
                if f('n') {
                    s.extend_from_slice(format!("{}:", n + 1).as_bytes());
                }
                s
            };
            if f('o') && !f('v') {
                for m in re.find_iter(line) {
                    if m.as_bytes().is_empty() {
                        continue;
                    }
                    out.extend_from_slice(&lead);
                    out.extend_from_slice(m.as_bytes());
                    out.push(b'\n');
                }
            } else {
                out.extend_from_slice(&lead);
                out.extend_from_slice(line);
                out.push(b'\n');
            }
        }
        if count > 0 {
            any = true;
        }
        if f('q') {
            if any {
                return Util::Done(0);
            }
            continue;
        }
        if f('l') {
            if count > 0 {
                sink.out.extend_from_slice(format!("{name}\n").as_bytes());
            }
        } else if f('L') {
            if count == 0 {
                sink.out.extend_from_slice(format!("{name}\n").as_bytes());
            }
        } else if f('c') {
            let line = if prefix {
                format!("{name}:{count}\n")
            } else {
                format!("{count}\n")
            };
            sink.out.extend_from_slice(line.as_bytes());
        } else if binary {
            if count > 0 {
                sink.eline(&format!("grep: {name}: binary file matches"));
            }
        } else {
            sink.out.extend_from_slice(&out);
        }
    }
    Util::Done(if status_err && !(any && f('q')) {
        2
    } else if any {
        0
    } else {
        1
    })
}

/// Files beneath `dir` in directory order, depth first, as GNU grep -r and
/// find visit them; symbolic links met during the walk are not followed.
fn walk_files(dir: &str, shown: &str, out: &mut Vec<(String, Option<String>)>, strip_dot: bool) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.filter_map(|e| e.ok()) {
        // One past the cap is enough for the caller to refuse the search.
        if out.len() > crate::builtins::WALK_MAX_ENTRIES {
            return;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let path = if dir.ends_with('/') {
            format!("{dir}{name}")
        } else {
            format!("{dir}/{name}")
        };
        let display = if strip_dot && shown.is_empty() {
            name.clone()
        } else if shown.ends_with('/') {
            format!("{shown}{name}")
        } else {
            format!("{shown}/{name}")
        };
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk_files(&path, &display, out, false);
        } else {
            out.push((display, Some(path)));
        }
    }
}

fn numeric_prefix(s: &str) -> f64 {
    let t = s.trim_start();
    let mut end = 0;
    for (i, c) in t.char_indices() {
        if c.is_ascii_digit() || c == '.' || (i == 0 && c == '-') {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    t[..end].parse().unwrap_or(0.0)
}

/// One `-k` key: fields `start` through `end` (1-based; no `end` means to the
/// end of the line), with the key's own `n` and `r` options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SortKey {
    start: usize,
    end: Option<usize>,
    numeric: bool,
    reverse: bool,
}

/// The options `sort` runs in-process. Anything else (`-h`, `-b`, `-k2.3`, long
/// options) makes [`SortSpec::parse`] decline, so the program that owns those
/// options runs instead.
#[derive(Debug, Default)]
struct SortSpec {
    numeric: bool,
    fold: bool,
    reverse: bool,
    unique: bool,
    stable: bool,
    sep: Option<char>,
    keys: Vec<SortKey>,
    files: Vec<String>,
}

/// One field of a `-k` spec: a 1-based number and its `n`/`r` options.
fn sort_field(text: &str) -> Option<(usize, bool, bool)> {
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let n: usize = text[..digits].parse().ok()?;
    if n == 0 {
        return None;
    }
    let (mut numeric, mut reverse) = (false, false);
    for c in text[digits..].chars() {
        match c {
            'n' => numeric = true,
            'r' => reverse = true,
            _ => return None,
        }
    }
    Some((n, numeric, reverse))
}

/// `F[opts][,F[opts]]`, the `-k` argument.
fn sort_key(spec: &str) -> Option<SortKey> {
    let (first, second) = match spec.split_once(',') {
        Some((a, b)) => (a, Some(b)),
        None => (spec, None),
    };
    let (start, n1, r1) = sort_field(first)?;
    let (end, n2, r2) = match second {
        Some(s) => {
            let (e, n, r) = sort_field(s)?;
            if e < start {
                return None;
            }
            (Some(e), n, r)
        }
        None => (None, false, false),
    };
    Some(SortKey {
        start,
        end,
        numeric: n1 || n2,
        reverse: r1 || r2,
    })
}

impl SortSpec {
    fn parse(args: &[String]) -> Option<SortSpec> {
        let mut spec = SortSpec::default();
        let mut ops_only = false;
        let mut i = 0;
        while i < args.len() {
            let a = args[i].as_str();
            if !ops_only && a == "--" {
                ops_only = true;
            } else if ops_only || a == "-" || !a.starts_with('-') {
                spec.files.push(a.to_string());
            } else if a.starts_with("--") {
                return None;
            } else {
                let chars: Vec<char> = a[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'r' => spec.reverse = true,
                        'n' => spec.numeric = true,
                        'f' => spec.fold = true,
                        'u' => spec.unique = true,
                        's' => spec.stable = true,
                        c @ ('k' | 't') => {
                            let rest: String = chars[j + 1..].iter().collect();
                            let value = if rest.is_empty() {
                                i += 1;
                                args.get(i)?.clone()
                            } else {
                                rest
                            };
                            if c == 'k' {
                                spec.keys.push(sort_key(&value)?);
                            } else {
                                let mut it = value.chars();
                                let sep = it.next()?;
                                if it.next().is_some() {
                                    return None;
                                }
                                spec.sep = Some(sep);
                            }
                            break;
                        }
                        _ => return None,
                    }
                    j += 1;
                }
            }
            i += 1;
        }
        Some(spec)
    }
}

/// The byte ranges of the fields of `line`. With a separator, fields are the
/// pieces between it. Without one, GNU's default: a field is a run of blanks
/// followed by a run of non-blanks, so its leading blanks belong to it.
fn sort_fields(line: &str, sep: Option<char>) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    match sep {
        Some(c) => {
            for (i, ch) in line.char_indices() {
                if ch == c {
                    out.push((start, i));
                    start = i + ch.len_utf8();
                }
            }
        }
        None => {
            let mut prev_nonblank = false;
            for (i, ch) in line.char_indices() {
                let blank = ch == ' ' || ch == '\t';
                if blank && prev_nonblank {
                    out.push((start, i));
                    start = i;
                }
                prev_nonblank = !blank;
            }
        }
    }
    out.push((start, line.len()));
    out
}

/// The text of `key` in a line whose fields are `fields`. A field that does not
/// exist is empty, and an `end` past the last field runs to the end of the line.
fn sort_key_text<'a>(line: &'a str, fields: &[(usize, usize)], key: &SortKey) -> &'a str {
    let Some(&(s, _)) = fields.get(key.start - 1) else {
        return "";
    };
    let e = match key.end {
        None => line.len(),
        Some(end) => fields.get(end - 1).map_or(line.len(), |&(_, fe)| fe),
    };
    &line[s..e]
}

fn sort(args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    let Some(spec) = SortSpec::parse(args) else {
        return Util::Decline;
    };
    // GNU sort: an unreadable input is reported as "cannot read" and makes the
    // exit status 2 (read_inputs would say 1, which is what cat and grep say).
    let mut status = 0;
    let mut lines: Vec<String> = Vec::new();
    let mut inputs: Vec<Vec<u8>> = Vec::new();
    if spec.files.is_empty() {
        inputs.push(stdin.read_all());
    }
    for f in &spec.files {
        if f == "-" {
            inputs.push(stdin.read_all());
            continue;
        }
        if let Some(refusal) = jail_refusal("sort", f) {
            sink.eline(&refusal);
            status = 2;
            continue;
        }
        match std::fs::read(f) {
            Ok(d) => inputs.push(d),
            Err(e) => {
                sink.eline(&format!("sort: cannot read: {f}: {}", io_reason(&e)));
                status = 2;
            }
        }
    }
    for d in inputs {
        lines.extend(
            lines_of(&d)
                .iter()
                .map(|l| String::from_utf8_lossy(l).into_owned()),
        );
    }
    let fields: Vec<Vec<(usize, usize)>> = if spec.keys.is_empty() {
        Vec::new()
    } else {
        lines.iter().map(|l| sort_fields(l, spec.sep)).collect()
    };
    // Compare two lines by their keys alone (`-u` dedups on this), or by the
    // whole line when there are no keys.
    let by_keys = |i: usize, j: usize| -> std::cmp::Ordering {
        if spec.keys.is_empty() {
            let (a, b) = (&lines[i], &lines[j]);
            return if spec.numeric {
                numeric_prefix(a)
                    .partial_cmp(&numeric_prefix(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            } else if spec.fold {
                collate(&a.to_uppercase(), &b.to_uppercase())
            } else {
                collate(a, b)
            };
        }
        for key in &spec.keys {
            let a = sort_key_text(&lines[i], &fields[i], key);
            let b = sort_key_text(&lines[j], &fields[j], key);
            let numeric = key.numeric || spec.numeric;
            let o = if numeric {
                numeric_prefix(a)
                    .partial_cmp(&numeric_prefix(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            } else if spec.fold {
                collate(&a.to_uppercase(), &b.to_uppercase())
            } else {
                collate(a, b)
            };
            let o = if key.reverse { o.reverse() } else { o };
            if o != std::cmp::Ordering::Equal {
                return o;
            }
        }
        std::cmp::Ordering::Equal
    };
    // With no keys, equal lines fall back to comparing the whole line, unless
    // `-s`. With keys, equal lines keep their input order.
    let last_resort = spec.keys.is_empty() && !spec.stable;
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by(|&i, &j| {
        by_keys(i, j).then_with(|| {
            if last_resort {
                collate(&lines[i], &lines[j])
            } else {
                std::cmp::Ordering::Equal
            }
        })
    });
    if spec.unique {
        order.dedup_by(|&mut j, &mut i| by_keys(i, j) == std::cmp::Ordering::Equal);
    }
    if spec.reverse {
        order.reverse();
    }
    for i in order {
        sink.out.extend_from_slice(lines[i].as_bytes());
        sink.out.push(b'\n');
    }
    Util::Done(status)
}

fn uniq(args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    let Some((flags, files)) = short_flags(args, "cdui") else {
        return Util::Decline;
    };
    if files.len() > 1 {
        return Util::Decline;
    }
    let mut status = 0;
    let data = read_inputs("uniq", &files, stdin, sink, &mut status)
        .into_iter()
        .next()
        .map(|(_, d)| d)
        .unwrap_or_default();
    let eq = |a: &[u8], b: &[u8]| {
        if flags.contains(&'i') {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    let lines = lines_of(&data);
    let mut k = 0;
    while k < lines.len() {
        let mut j = k + 1;
        while j < lines.len() && eq(lines[j], lines[k]) {
            j += 1;
        }
        let n = j - k;
        let keep = (!flags.contains(&'d') || n > 1) && (!flags.contains(&'u') || n == 1);
        if keep {
            if flags.contains(&'c') {
                sink.out.extend_from_slice(format!("{n:>7} ").as_bytes());
            }
            sink.out.extend_from_slice(lines[k]);
            sink.out.push(b'\n');
        }
        k = j;
    }
    Util::Done(status)
}

/// A `cut` list: `1,3-5,7-` as inclusive 1-based ranges.
fn cut_list(s: &str) -> Option<Vec<(usize, usize)>> {
    let mut v = Vec::new();
    for part in s.split(',') {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (
                if a.is_empty() { 1 } else { a.parse().ok()? },
                if b.is_empty() {
                    usize::MAX
                } else {
                    b.parse().ok()?
                },
            ),
            None => {
                let n = part.parse().ok()?;
                (n, n)
            }
        };
        if a == 0 {
            return None;
        }
        v.push((a, b));
    }
    Some(v)
}

fn cut(args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    let mut delim = '\t';
    let mut fields = None;
    let mut chars = None;
    let mut only_delimited = false;
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let take = |i: &mut usize, flag: &str| -> Option<String> {
            if a.len() > flag.len() {
                Some(a[flag.len()..].to_string())
            } else {
                *i += 1;
                args.get(*i).cloned()
            }
        };
        if a.starts_with("-d") {
            let d = take(&mut i, "-d");
            match d.and_then(|d| d.chars().next()) {
                Some(c) => delim = c,
                None => return Util::Decline,
            }
        } else if a.starts_with("-f") {
            fields = take(&mut i, "-f").and_then(|l| cut_list(&l));
            if fields.is_none() {
                return Util::Decline;
            }
        } else if a.starts_with("-c") || a.starts_with("-b") {
            chars = take(&mut i, "-c").and_then(|l| cut_list(&l));
            if chars.is_none() {
                return Util::Decline;
            }
        } else if a == "-s" {
            only_delimited = true;
        } else if a.starts_with('-') && a != "-" {
            return Util::Decline;
        } else {
            files.push(a.clone());
        }
        i += 1;
    }
    if fields.is_none() && chars.is_none() {
        sink.eline("cut: you must specify a list of bytes, characters, or fields");
        return Util::Done(1);
    }
    let within =
        |ranges: &[(usize, usize)], n: usize| ranges.iter().any(|&(a, b)| a <= n && n <= b);
    let mut status = 0;
    for (_, data) in read_inputs("cut", &files, stdin, sink, &mut status) {
        for line in lines_of(&data) {
            let text = String::from_utf8_lossy(line);
            if let Some(r) = &chars {
                let s: String = text
                    .chars()
                    .enumerate()
                    .filter(|(k, _)| within(r, k + 1))
                    .map(|(_, c)| c)
                    .collect();
                sink.out.extend_from_slice(s.as_bytes());
                sink.out.push(b'\n');
            } else if let Some(r) = &fields {
                if !text.contains(delim) {
                    if !only_delimited {
                        sink.out.extend_from_slice(line);
                        sink.out.push(b'\n');
                    }
                    continue;
                }
                let picked: Vec<&str> = text
                    .split(delim)
                    .enumerate()
                    .filter(|(k, _)| within(r, k + 1))
                    .map(|(_, f)| f)
                    .collect();
                sink.out
                    .extend_from_slice(picked.join(&delim.to_string()).as_bytes());
                sink.out.push(b'\n');
            }
        }
    }
    Util::Done(status)
}

#[derive(Clone)]
enum Addr {
    Line(usize),
    Last,
    Re(regex::bytes::Regex),
}

enum SedCmd {
    Print,
    Delete,
    Quit,
    Subst(regex::bytes::Regex, Vec<RepItem>, bool, bool, usize),
}

/// One piece of an `s` command's replacement, after its escapes are read.
enum RepItem {
    /// Literal text.
    Text(Vec<u8>),
    /// `&` (group 0) or `\N`.
    Group(usize),
    /// `\U`, `\L`, `\E` (`U`, `L`, `E`: the case held until `\E` or the other
    /// of `\U`/`\L`), `\u` and `\l` (`u`, `l`: the next character only).
    Case(char),
}

/// The text of one replacement: `items` with the match's groups filled in,
/// and the case escapes applied as GNU sed applies them.
fn expand_rep(items: &[RepItem], caps: &regex::bytes::Captures, out: &mut Vec<u8>) {
    let mut hold: Option<char> = None;
    let mut next: Option<char> = None;
    for item in items {
        let piece: &[u8] = match item {
            RepItem::Text(t) => t.as_slice(),
            RepItem::Group(n) => caps.get(*n).map_or(&b""[..], |m| m.as_bytes()),
            RepItem::Case(c) => {
                match *c {
                    'U' | 'L' => hold = Some(*c),
                    'E' => hold = None,
                    _ => next = Some(*c),
                }
                continue;
            }
        };
        if hold.is_none() && next.is_none() {
            out.extend_from_slice(piece);
            continue;
        }
        for ch in String::from_utf8_lossy(piece).chars() {
            // The one-shot case applies to this character alone, then the held one.
            let mapped: String = match (next.take(), hold) {
                (Some('u'), _) | (None, Some('U')) => ch.to_uppercase().collect(),
                (Some(_), _) | (None, Some(_)) => ch.to_lowercase().collect(),
                (None, None) => ch.to_string(),
            };
            out.extend_from_slice(mapped.as_bytes());
        }
    }
}

struct SedLine {
    a1: Option<Addr>,
    a2: Option<Addr>,
    cmd: SedCmd,
    active: bool,
}

/// The parts of an `s` command's body, split at its delimiter. A `\` keeps the
/// character after it in the part it is in, so an escaped delimiter does not end
/// a part; the parts keep their backslashes for [`unescape_delim`] and the
/// replacement's own escapes to read.
fn split_s_parts(body: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut it = body.chars();
    while let Some(c) = it.next() {
        match c {
            '\\' => {
                cur.push('\\');
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            c if c == sep => parts.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    parts.push(cur);
    parts
}

/// A pattern with each `\<delimiter>` read as the delimiter character itself,
/// literal in the syntax in use: `s|a\|b|c|` matches `a|b`, not an alternation,
/// and `s.a\.b.c.` matches `a.b` only. Other escapes are left for the regex
/// converters.
fn unescape_delim(pat: &str, sep: char, ere: bool) -> String {
    let mut out = String::new();
    let mut it = pat.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some(n) if n == sep => {
                // Special in this syntax, so it needs its escape to be literal.
                let special = if ere { ".*[]^$+?|(){}" } else { ".*[]^$" };
                if special.contains(n) {
                    out.push('\\');
                }
                out.push(n);
            }
            Some(n) => {
                out.push('\\');
                out.push(n);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// `sed` for the everyday scripts: `Np`, `a,bp`, `$p`, `/re/p`, `Nd`, `q`,
/// and `s/re/rep/[gp]`, with `-n`, `-e` and `-E`. Anything else declines.
/// `s/RE/REP/N`: replace the Nth match only, or from the Nth on with `g`.
fn subst_nth(
    re: &regex::bytes::Regex,
    text: &[u8],
    rep: &[RepItem],
    global: bool,
    nth: usize,
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut last = 0;
    for (k, caps) in re.captures_iter(text).enumerate() {
        let k = k + 1;
        if k < nth {
            continue;
        }
        if k > nth && !global {
            break;
        }
        let whole = caps.get(0).expect("group 0 always matches");
        out.extend_from_slice(&text[last..whole.start()]);
        expand_rep(rep, &caps, &mut out);
        last = whole.end();
    }
    out.extend_from_slice(&text[last..]);
    out
}

fn sed(args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    let mut quiet = false;
    let mut ere = false;
    let mut scripts = Vec::new();
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "-n" => quiet = true,
            "-E" | "-r" => ere = true,
            "-e" => {
                i += 1;
                match args.get(i) {
                    Some(s) => scripts.push(s.clone()),
                    None => return Util::DeclineFor("option `-e` with no script".into()),
                }
            }
            "-ne" | "-nE" | "-En" => {
                quiet = true;
                ere = a.contains('E');
                if a == "-ne" {
                    i += 1;
                    match args.get(i) {
                        Some(s) => scripts.push(s.clone()),
                        None => return Util::DeclineFor("option `-e` with no script".into()),
                    }
                }
            }
            s if s.starts_with('-') && s != "-" => {
                return Util::DeclineFor(format!("option `{s}`"));
            }
            _ => {
                if scripts.is_empty() {
                    scripts.push(a.clone());
                } else {
                    files.push(a.clone());
                }
            }
        }
        i += 1;
    }
    let to_re = |p: &str| -> Option<regex::bytes::Regex> {
        let r = if ere {
            ere_to_regex(p)?
        } else {
            bre_to_regex(p)?
        };
        regex::bytes::Regex::new(&r).ok()
    };
    let mut prog: Vec<SedLine> = Vec::new();
    for script in &scripts {
        for raw in script.split(['\n', ';']) {
            let s = raw.trim();
            if s.is_empty() {
                continue;
            }
            let c: Vec<char> = s.chars().collect();
            let mut k = 0;
            let addr = |k: &mut usize| -> Result<Option<Addr>, ()> {
                match c.get(*k) {
                    Some(d) if d.is_ascii_digit() => {
                        let mut n = 0usize;
                        while let Some(d) = c.get(*k).filter(|d| d.is_ascii_digit()) {
                            n = n * 10 + (*d as usize - '0' as usize);
                            *k += 1;
                        }
                        Ok(Some(Addr::Line(n)))
                    }
                    Some('$') => {
                        *k += 1;
                        Ok(Some(Addr::Last))
                    }
                    Some('/') => {
                        let end = c[*k + 1..].iter().position(|&x| x == '/').ok_or(())?;
                        let pat: String = c[*k + 1..*k + 1 + end].iter().collect();
                        *k += end + 2;
                        Ok(Some(Addr::Re(to_re(&pat).ok_or(())?)))
                    }
                    _ => Ok(None),
                }
            };
            let Ok(a1) = addr(&mut k) else {
                return Util::DeclineFor(format!("the address `{s}`"));
            };
            let mut a2 = None;
            if a1.is_some() && c.get(k) == Some(&',') {
                k += 1;
                match addr(&mut k) {
                    Ok(Some(a)) => a2 = Some(a),
                    _ => return Util::DeclineFor(format!("the address range `{s}`")),
                }
            }
            let rest: String = c[k..].iter().collect();
            let rest = rest.trim();
            let cmd = match rest {
                "p" => SedCmd::Print,
                "d" => SedCmd::Delete,
                "q" => SedCmd::Quit,
                r if r.starts_with('s') && r.len() > 1 => {
                    let sep = r[1..].chars().next().unwrap_or('/');
                    let body = split_s_parts(&r[1 + sep.len_utf8()..], sep);
                    let [pat, rep, fl] = body.as_slice() else {
                        return Util::DeclineFor(format!(
                            "the s command with its parts not split by `{sep}`"
                        ));
                    };
                    let pat = unescape_delim(pat, sep, ere);
                    let fl: &str = fl;
                    let digits: String = fl.chars().take_while(char::is_ascii_digit).collect();
                    let tail = &fl[digits.len()..];
                    if let Some(f) = tail.chars().find(|f| !matches!(f, 'g' | 'p')) {
                        return Util::DeclineFor(format!("the s command's flag `{f}`"));
                    }
                    let nth = if digits.is_empty() {
                        1
                    } else {
                        match digits.parse::<usize>() {
                            Ok(n) if n >= 1 => n,
                            _ => {
                                return Util::DeclineFor(format!(
                                    "the s command's occurrence number `{digits}`"
                                ))
                            }
                        }
                    };
                    let Some(re) = to_re(pat.as_str()) else {
                        return Util::DeclineFor(format!("the regular expression `{pat}`"));
                    };
                    // `&` is the match and `\N` a group; the case escapes are
                    // applied to the text they reach (see `expand_rep`).
                    let mut items: Vec<RepItem> = Vec::new();
                    let mut it = rep.chars();
                    while let Some(ch) = it.next() {
                        let text = match ch {
                            '&' => {
                                items.push(RepItem::Group(0));
                                continue;
                            }
                            '\\' => match it.next() {
                                Some(d @ '1'..='9') => {
                                    items.push(RepItem::Group(d as usize - '0' as usize));
                                    continue;
                                }
                                Some(c @ ('U' | 'L' | 'u' | 'l' | 'E')) => {
                                    items.push(RepItem::Case(c));
                                    continue;
                                }
                                Some('n') => '\n',
                                Some('t') => '\t',
                                Some(o) => o,
                                None => '\\',
                            },
                            o => o,
                        };
                        let mut b = [0; 4];
                        let bytes = text.encode_utf8(&mut b).as_bytes();
                        match items.last_mut() {
                            Some(RepItem::Text(t)) => t.extend_from_slice(bytes),
                            _ => items.push(RepItem::Text(bytes.to_vec())),
                        }
                    }
                    SedCmd::Subst(re, items, tail.contains('g'), tail.contains('p'), nth)
                }
                _ => {
                    return Util::DeclineFor(if rest.is_empty() {
                        format!("the address `{s}` with no command")
                    } else {
                        format!("sed command `{rest}`")
                    })
                }
            };
            prog.push(SedLine {
                a1,
                a2,
                cmd,
                active: false,
            });
        }
    }
    if prog.is_empty() {
        return Util::Decline;
    }
    let mut status = 0;
    let mut data = Vec::new();
    for (_, d) in read_inputs("sed", &files, stdin, sink, &mut status) {
        data.extend_from_slice(&d);
    }
    let lines = lines_of(&data);
    let n = lines.len();
    let hits = |a: &Addr, ln: usize, line: &[u8]| match a {
        Addr::Line(k) => *k == ln,
        Addr::Last => ln == n,
        Addr::Re(r) => r.is_match(line),
    };
    'lines: for (idx, line) in lines.iter().enumerate() {
        let ln = idx + 1;
        let mut pattern = line.to_vec();
        let mut deleted = false;
        for sl in prog.iter_mut() {
            let selected = match (&sl.a1, &sl.a2) {
                (None, _) => true,
                (Some(a), None) => hits(a, ln, &pattern),
                (Some(a), Some(b)) => {
                    if sl.active {
                        let end = match b {
                            Addr::Line(k) => ln >= *k,
                            _ => hits(b, ln, &pattern),
                        };
                        if end {
                            sl.active = false;
                        }
                        true
                    } else if hits(a, ln, &pattern) {
                        let ends_now = match b {
                            Addr::Line(k) => *k <= ln,
                            _ => false,
                        };
                        sl.active = !ends_now;
                        true
                    } else {
                        false
                    }
                }
            };
            if !selected {
                continue;
            }
            match &sl.cmd {
                SedCmd::Print => {
                    sink.out.extend_from_slice(&pattern);
                    sink.out.push(b'\n');
                }
                SedCmd::Delete => {
                    deleted = true;
                    break;
                }
                SedCmd::Quit => {
                    if !quiet {
                        sink.out.extend_from_slice(&pattern);
                        sink.out.push(b'\n');
                    }
                    break 'lines;
                }
                SedCmd::Subst(re, rep, global, print, nth) => {
                    let fill = |caps: &regex::bytes::Captures| {
                        let mut out = Vec::new();
                        expand_rep(rep, caps, &mut out);
                        out
                    };
                    let replaced = if *nth > 1 {
                        subst_nth(re, &pattern, rep, *global, *nth)
                    } else if *global {
                        re.replace_all(&pattern, fill).into_owned()
                    } else {
                        re.replace(&pattern, fill).into_owned()
                    };
                    let changed = replaced != pattern;
                    pattern = replaced;
                    if changed && *print {
                        sink.out.extend_from_slice(&pattern);
                        sink.out.push(b'\n');
                    }
                }
            }
        }
        if !deleted && !quiet {
            sink.out.extend_from_slice(&pattern);
            sink.out.push(b'\n');
        }
    }
    Util::Done(status)
}

fn ls(args: &[String], sink: &mut Sink) -> Util {
    let Some((flags, mut ops)) = short_flags(args, "aAl1d") else {
        return Util::Decline;
    };
    let long = flags.contains(&'l');
    if long && !cfg!(unix) {
        return Util::Decline;
    }
    let all = flags.contains(&'a');
    let almost = flags.contains(&'A');
    let dirs_as_files = flags.contains(&'d');
    if ops.is_empty() {
        ops.push(".".into());
    }
    let mut status = 0;
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for op in &ops {
        if let Some(refusal) = jail_refusal("ls", op) {
            sink.eline(&refusal);
            status = 2;
            continue;
        }
        let md = if long {
            std::fs::symlink_metadata(op)
        } else {
            std::fs::metadata(op)
        };
        match md {
            Ok(m) if m.is_dir() && !dirs_as_files => dirs.push(op.clone()),
            Ok(_) => files.push(op.clone()),
            Err(e) => {
                sink.eline(&format!("ls: cannot access '{op}': {}", io_reason(&e)));
                status = 2;
            }
        }
    }
    files.sort_by(|a, b| collate(a, b));
    dirs.sort_by(|a, b| collate(a, b));
    let headers = files.len() + dirs.len() > 1 || status != 0;
    let mut first = true;
    if !files.is_empty() {
        let entries: Vec<(String, String)> = files.iter().map(|f| (f.clone(), f.clone())).collect();
        emit_ls(&entries, long, false, sink);
        first = false;
    }
    for d in &dirs {
        if !first {
            sink.out.push(b'\n');
        }
        first = false;
        if headers {
            sink.out.extend_from_slice(format!("{d}:\n").as_bytes());
        }
        let Ok(rd) = std::fs::read_dir(d) else {
            sink.eline(&format!(
                "ls: cannot open directory '{d}': Permission denied"
            ));
            status = 2;
            continue;
        };
        let mut names: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| all || almost || !n.starts_with('.'))
            .collect();
        if all {
            names.push(".".into());
            names.push("..".into());
        }
        names.sort_by(|a, b| collate(a, b));
        let entries: Vec<(String, String)> = names
            .iter()
            .map(|n| {
                let p = if d.ends_with('/') {
                    format!("{d}{n}")
                } else {
                    format!("{d}/{n}")
                };
                (n.clone(), p)
            })
            .collect();
        emit_ls(&entries, long, true, sink);
    }
    Util::Done(status)
}

/// `(shown name, path)` pairs, in order.
fn emit_ls(entries: &[(String, String)], long: bool, total: bool, sink: &mut Sink) {
    if !long {
        for (n, _) in entries {
            sink.out.extend_from_slice(n.as_bytes());
            sink.out.push(b'\n');
        }
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let now = chrono::Local::now();
        let mut rows = Vec::new();
        let mut blocks: u64 = 0;
        for (name, path) in entries {
            let Ok(m) = std::fs::symlink_metadata(path) else {
                continue;
            };
            blocks += m.blocks();
            let ft = m.file_type();
            let mode = m.mode();
            let kind = if ft.is_dir() {
                'd'
            } else if ft.is_symlink() {
                'l'
            } else {
                use std::os::unix::fs::FileTypeExt;
                if ft.is_block_device() {
                    'b'
                } else if ft.is_char_device() {
                    'c'
                } else if ft.is_fifo() {
                    'p'
                } else if ft.is_socket() {
                    's'
                } else {
                    '-'
                }
            };
            let bit = |m: u32, c: char| if mode & m != 0 { c } else { '-' };
            let special =
                |x: u32, s: u32, lower: char, upper: char| match (mode & x != 0, mode & s != 0) {
                    (true, true) => lower,
                    (false, true) => upper,
                    (true, false) => 'x',
                    (false, false) => '-',
                };
            let perms: String = [
                kind,
                bit(0o400, 'r'),
                bit(0o200, 'w'),
                special(0o100, 0o4000, 's', 'S'),
                bit(0o040, 'r'),
                bit(0o020, 'w'),
                special(0o010, 0o2000, 's', 'S'),
                bit(0o004, 'r'),
                bit(0o002, 'w'),
                special(0o001, 0o1000, 't', 'T'),
            ]
            .iter()
            .collect();
            let mtime = chrono::DateTime::from_timestamp(m.mtime(), 0)
                .map(|t| t.with_timezone(&chrono::Local))
                .unwrap_or(now);
            let age = now.signed_duration_since(mtime);
            let recent = age.num_seconds() >= 0 && age.num_days() < 182;
            let date = if recent {
                mtime.format("%b %e %H:%M").to_string()
            } else {
                mtime.format("%b %e  %Y").to_string()
            };
            let mut shown = name.clone();
            if ft.is_symlink() {
                if let Ok(t) = std::fs::read_link(path) {
                    shown = format!("{name} -> {}", t.to_string_lossy());
                }
            }
            rows.push((
                perms,
                m.nlink().to_string(),
                user_name(m.uid()),
                group_name(m.gid()),
                m.size().to_string(),
                date,
                shown,
            ));
        }
        if total {
            sink.out
                .extend_from_slice(format!("total {}\n", blocks.div_ceil(2)).as_bytes());
        }
        let w = |f: fn(&(String, String, String, String, String, String, String)) -> &String| {
            rows.iter().map(|r| f(r).len()).max().unwrap_or(0)
        };
        let (wl, wu, wg, ws) = (w(|r| &r.1), w(|r| &r.2), w(|r| &r.3), w(|r| &r.4));
        for r in &rows {
            let line = format!(
                "{} {:>wl$} {:<wu$} {:<wg$} {:>ws$} {} {}\n",
                r.0, r.1, r.2, r.3, r.4, r.5, r.6
            );
            sink.out.extend_from_slice(line.as_bytes());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (total, sink);
    }
}

#[cfg(unix)]
fn user_name(uid: u32) -> String {
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: getpwuid_r writes into the caller's buffers and sets `result`
    // to `&pwd` or null; `pw_name` points into `buf`, which outlives the read.
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc == 0 && !result.is_null() {
        // SAFETY: as above; the name is a NUL-terminated string within `buf`.
        let name = unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) };
        return name.to_string_lossy().into_owned();
    }
    uid.to_string()
}

#[cfg(unix)]
fn group_name(gid: u32) -> String {
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut grp: libc::group = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::group = std::ptr::null_mut();
    // SAFETY: getgrgid_r writes into the caller's buffers and sets `result`
    // to `&grp` or null; `gr_name` points into `buf`, which outlives the read.
    let rc = unsafe { libc::getgrgid_r(gid, &mut grp, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc == 0 && !result.is_null() {
        // SAFETY: as above.
        let name = unsafe { std::ffi::CStr::from_ptr(grp.gr_name) };
        return name.to_string_lossy().into_owned();
    }
    gid.to_string()
}

/// `find` with `-name`, `-iname`, `-path`, `-type`, `-maxdepth`, `-mindepth`,
/// `!`/`-not` and `-print`, visiting in directory order as GNU find does.
fn find(args: &[String], sink: &mut Sink) -> Util {
    enum Test {
        Name(String, bool),
        Path(String),
        Type(char),
    }
    let mut starts = Vec::new();
    let mut i = 0;
    while i < args.len() && !args[i].starts_with('-') && args[i] != "!" {
        starts.push(args[i].clone());
        i += 1;
    }
    if starts.is_empty() {
        starts.push(".".into());
    }
    let mut tests: Vec<(bool, Test)> = Vec::new();
    let mut maxdepth = usize::MAX;
    let mut mindepth = 0usize;
    let mut negate = false;
    while i < args.len() {
        let a = args[i].as_str();
        let val = args.get(i + 1).cloned();
        match a {
            "!" | "-not" => {
                negate = !negate;
                i += 1;
                continue;
            }
            "-print" => {}
            "-name" | "-iname" | "-path" | "-wholename" | "-type" | "-maxdepth" | "-mindepth" => {
                let Some(v) = val else { return Util::Decline };
                match a {
                    "-name" => tests.push((negate, Test::Name(v, false))),
                    "-iname" => tests.push((negate, Test::Name(v.to_lowercase(), true))),
                    "-path" | "-wholename" => tests.push((negate, Test::Path(v))),
                    "-type" => match v.as_str() {
                        "f" | "d" | "l" => {
                            tests.push((negate, Test::Type(v.chars().next().unwrap_or('f'))))
                        }
                        _ => return Util::Decline,
                    },
                    "-maxdepth" => match v.parse() {
                        Ok(n) => maxdepth = n,
                        Err(_) => return Util::Decline,
                    },
                    _ => match v.parse() {
                        Ok(n) => mindepth = n,
                        Err(_) => return Util::Decline,
                    },
                }
                i += 1;
            }
            _ => return Util::Decline,
        }
        negate = false;
        i += 1;
    }
    let matches = |path: &str, ft: &std::fs::FileType| {
        tests.iter().all(|(neg, t)| {
            let base = path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(path);
            let hit = match t {
                Test::Name(p, false) => glob_match(p, base),
                Test::Name(p, true) => glob_match(p, &base.to_lowercase()),
                Test::Path(p) => glob_match(p, path),
                Test::Type('d') => ft.is_dir(),
                Test::Type('l') => ft.is_symlink(),
                Test::Type(_) => ft.is_file(),
            };
            hit != *neg
        })
    };
    let mut status = 0;
    // Matches printed so far. One past the cap marks the walk as stopped.
    let mut count = 0usize;
    fn visit(
        path: &str,
        depth: usize,
        maxdepth: usize,
        mindepth: usize,
        matches: &dyn Fn(&str, &std::fs::FileType) -> bool,
        sink: &mut Sink,
        status: &mut i32,
        count: &mut usize,
    ) {
        if *count > crate::builtins::WALK_MAX_ENTRIES {
            return;
        }
        let md = match std::fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                sink.eline(&format!("find: '{path}': {}", io_reason(&e)));
                *status = 1;
                return;
            }
        };
        let ft = md.file_type();
        if depth >= mindepth && matches(path, &ft) {
            if *count >= crate::builtins::WALK_MAX_ENTRIES {
                *count += 1;
                sink.eline(&format!(
                    "find: stopped after {} entries; the listing is partial",
                    crate::builtins::WALK_MAX_ENTRIES
                ));
                *status = 1;
                return;
            }
            *count += 1;
            sink.out.extend_from_slice(path.as_bytes());
            sink.out.push(b'\n');
        }
        if ft.is_dir() && depth < maxdepth {
            let rd = match std::fs::read_dir(path) {
                Ok(r) => r,
                Err(e) => {
                    sink.eline(&format!("find: '{path}': {}", io_reason(&e)));
                    *status = 1;
                    return;
                }
            };
            for e in rd.filter_map(|e| e.ok()) {
                let name = e.file_name().to_string_lossy().into_owned();
                let child = if path.ends_with('/') {
                    format!("{path}{name}")
                } else {
                    format!("{path}/{name}")
                };
                visit(
                    &child,
                    depth + 1,
                    maxdepth,
                    mindepth,
                    matches,
                    sink,
                    status,
                    count,
                );
            }
        }
    }
    for s in &starts {
        if let Some(refusal) = jail_refusal("find", s) {
            sink.eline(&refusal);
            status = 1;
            continue;
        }
        visit(
            s,
            0,
            maxdepth,
            mindepth,
            &matches,
            sink,
            &mut status,
            &mut count,
        );
    }
    Util::Done(status)
}

fn date(args: &[String], sink: &mut Sink) -> Util {
    match args {
        [f] if f.starts_with('+') => {
            let now = chrono::Local::now();
            let fmt = &f[1..];
            // chrono panics on a format it cannot parse; ask it first.
            use chrono::format::{Item, StrftimeItems};
            if StrftimeItems::new(fmt).any(|i| matches!(i, Item::Error)) {
                return Util::Decline;
            }
            sink.out
                .extend_from_slice(format!("{}\n", now.format(fmt)).as_bytes());
            Util::Done(0)
        }
        _ => Util::Decline,
    }
}

// ════════════════════════════════════════════════════════════════════════
// Entry point
// ════════════════════════════════════════════════════════════════════════

/// Run `src` as a shell script, writing to the real stdout and stderr.
///
/// Returns the exit status, or `None` when the script uses something this
/// subset does not run -- decided before anything executes, so the caller can
/// fall back without a half-run script behind it.
pub fn run(src: &str) -> Option<i32> {
    run_with_args(src, &[])
}

/// [`run`] with the script's arguments, which become `$1`..`$9` and `$#`.
pub fn run_with_args(src: &str, args: &[String]) -> Option<i32> {
    run_named(src, SHELL_NAME, args)
}

/// [`run_with_args`] for a script file: `$0` is `script` (its path), not `ish`.
pub fn run_script(src: &str, script: &str, args: &[String]) -> Option<i32> {
    run_named(src, script, args)
}

fn run_named(src: &str, script_name: &str, args: &[String]) -> Option<i32> {
    let list = parse(src).ok()?;
    let mut sh = Shell {
        vars: HashMap::new(),
        exported: HashSet::new(),
        last: 0,
        glob_refused: None,
        positional: args.to_vec(),
        script_name: script_name.to_string(),
        errexit: false,
        nounset: false,
        pipefail: false,
        errexit_off: 0,
        unbound: None,
        line: 1,
    };
    let mut stdin = Input::real();
    let mut sink = Sink::new(ErrMode::Real);
    let mut status = 0;
    // Output reaches the terminal as each top-level command finishes, not at
    // the end -- but only here: a loop body inside a pipeline must feed the
    // next stage, not the terminal.
    for ao in &list {
        let flow = sh.run_and_or(ao, &mut stdin, &mut sink);
        // A failed write is this command's failure. A closed pipe (`| head`)
        // is not reported: nobody is reading, as before.
        let mut write_failed = false;
        let written = std::io::stdout()
            .write_all(&redacted(&sink.out))
            .and_then(|()| std::io::stdout().flush());
        if let Err(e) = written {
            if e.kind() != std::io::ErrorKind::BrokenPipe {
                write_failed = true;
                let _ = writeln!(
                    std::io::stderr(),
                    "{SHELL_NAME}: write error: {}",
                    io_reason(&e)
                );
            }
        }
        sink.out.clear();
        match flow {
            Flow::Next(c) => status = c,
            Flow::Exit(c) => return Some(if write_failed && c == 0 { 1 } else { c }),
            Flow::Break(_) | Flow::Continue(_) => {}
        }
        if write_failed && status == 0 {
            status = 1;
        }
    }
    Some(status)
}

/// How the subset runs a command word in-process, for `--explain`: `Some(true)`
/// for a utility that reads the paths it is given, `Some(false)` for one that
/// does not, and `None` for a name the subset does not run itself (which runs
/// as a program through the effect gate).
pub fn in_process_reads(name: &str) -> Option<bool> {
    match name {
        "cat" | "head" | "tail" | "wc" | "grep" | "sort" | "uniq" | "cut" | "sed" | "ls"
        | "find" | "test" | "[" => Some(true),
        "echo" | "printf" | "date" | "basename" | "dirname" | "seq" | "tr" | "true" | ":"
        | "false" | "exit" | "break" | "continue" | "export" | "unset" | "shift" | "cd"
        | "read" | "pwd" => Some(false),
        _ => None,
    }
}

/// Whether `src` is inside the subset, without running it.
pub fn supports(src: &str) -> Result<(), String> {
    parse(src).map(|_| ()).map_err(|u| u.what)
}

/// Why the subset does not run `src`: the construct, and where it starts, as a
/// 1-based line and column. `None` when the subset runs it.
pub fn refusal(src: &str) -> Option<Refusal> {
    let u = parse(src).err()?;
    let (line, column) = line_col(src, u.at.unwrap_or(0));
    let syntax = u.syntax.is_some();
    let hint = match &u.syntax {
        Some(hint) => hint.clone(),
        None => refusal_hint(&u.what),
    };
    Some(Refusal {
        construct: u.what,
        line,
        column,
        hint,
        syntax,
    })
}

/// The hint that goes with a refusal of `construct`: what to do instead.
pub fn refusal_hint(construct: &str) -> String {
    // Agent mode refuses to set IRONSHELL_ALLOW_SH (an IRONSHELL_ name, see
    // safety::env_name_steers_children), so the suggestion to set it is not
    // offered there. The rest of each hint stands.
    let agent = crate::safety::current_mode() == crate::safety::Mode::Agent;
    if construct.starts_with("arithmetic expansion") {
        return if agent {
            "arithmetic is not in the subset; compute in ish (ish -c)".into()
        } else {
            "arithmetic is not in the subset; compute in ish (ish -c) or set IRONSHELL_ALLOW_SH=true to run the unsupported parts through bash -lc, outside the effect gate".into()
        };
    }
    // The operator of a `${name...}` form, named in its refusal as `op`.
    let op = construct.split('`').nth(1).unwrap_or("");
    let tip: String = match construct {
        "function definition" => "inline the body, or write the logic in ish".into(),
        "case ... esac" => {
            "rewrite it as if/elif with [ \"$x\" = value ] tests".into()
        }
        "subshell" => "drop the parentheses, or group the commands with { ...; } when they need not run isolated".into(),
        "array assignment" => "arrays are not in the subset: keep the words in a space-separated string and loop over it with `for w in $words; do ...; done`".into(),
        "background job" => "run it in the foreground: drop the trailing &".into(),
        "[[" | "[[ ... ]]" => "use [ ... ] (test) instead of [[ ... ]]".into(),
        "process substitution" => "write the output to a file first, then read the file".into(),
        c if c.starts_with("here-document") => {
            "pass the text with printf '%s\\n' ... | cmd, or write it to a file first".into()
        }
        c if c.starts_with("here-string") => {
            "pipe it instead: printf '%s\\n' \"abc\" | grep -c b, or write the text to a file first".into()
        }
        c if c.starts_with("${...} operator") => match op {
            "/" | "//" => "replace text with sed, e.g. sed 's/OLD/NEW/g'".into(),
            ":offset" => "take a substring with cut -c, e.g. cut -c4-, or sed".into(),
            _ => "change the text with tr or sed".into(),
        },
        _ => "run the supported parts on their own".into(),
    };
    if agent {
        return tip;
    }
    format!("{tip}. Otherwise set IRONSHELL_ALLOW_SH=true to run the unsupported parts through bash -lc, outside the effect gate")
}

/// See [`refusal`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub construct: String,
    pub line: usize,
    pub column: usize,
    /// What to do instead: for a syntax error, what was expected; otherwise
    /// the hint for the unsupported construct.
    pub hint: String,
    /// Whether this is a syntax error (`E_PARSE`), not an unsupported construct.
    pub syntax: bool,
}

/// The first word of each simple command in `src`, in source order, for the
/// evaluation recorder. Commands inside `if`, loops and groups are included. A
/// first word that is not a plain literal (`$cmd`, `$(…)`) is an empty string,
/// and an assignment with no words names no command, so it is not listed.
/// Source outside the subset has no heads.
pub fn heads(src: &str) -> Vec<String> {
    explain_commands(src)
        .into_iter()
        .filter_map(|c| c.head)
        .collect()
}

/// One command as `--explain` reads it.
pub struct ExplainCommand {
    /// The first word as literal text (empty when it is computed), or `None`
    /// for a command with no words, only redirections.
    pub head: Option<String>,
    /// The other words, as literal text (empty when computed).
    pub args: Vec<String>,
    /// The output files its redirections name, as `(operator, target)`, for
    /// `>`, `>>`, `2>`, `2>>` and `&>`. `/dev/null` is not a file.
    pub writes: Vec<(String, String)>,
    /// The input files its `<` redirections name (empty when computed).
    pub reads: Vec<String>,
}

/// Every command in `src` in source order, including those inside `if`, loops
/// and groups, with the output files each one's redirections name. Source
/// outside the subset has none.
pub fn explain_commands(src: &str) -> Vec<ExplainCommand> {
    let mut out = Vec::new();
    if let Ok(list) = parse(src) {
        walk_list(&list, &mut out);
    }
    out
}

/// The files `tee` writes when it runs in-process: its operands, when every
/// option is one the subset supports (`-a`). `None` when it declines and runs as
/// a program.
pub fn tee_operands(args: &[String]) -> Option<Vec<String>> {
    short_flags(args, "a").map(|(_, ops)| ops)
}

/// The paths an in-process utility writes, for `--explain`: `tee`'s files and
/// `mkdir`'s and `touch`'s operands. `None` for any other command, or when the
/// utility declines its options and runs as a program.
pub fn write_operands(head: &str, args: &[String]) -> Option<Vec<String>> {
    match head {
        "tee" => tee_operands(args),
        "mkdir" => short_flags(args, "p").map(|(_, ops)| ops),
        "touch" => short_flags(args, "").map(|(_, ops)| ops),
        _ => None,
    }
}

/// The file operands an in-process utility reads, for `--explain`: each one is
/// judged by the workspace jail as the run judges it when it opens the file. `-`
/// (standard input) is among them as given. `None` when the utility's operand
/// grammar is not modelled here (a flag takes a value, or a pattern comes before
/// the files), or when it declines its options and runs as a program.
pub fn read_operands(head: &str, args: &[String]) -> Option<Vec<String>> {
    match head {
        "cat" => short_flags(args, "n").map(|(_, ops)| ops),
        "wc" => short_flags(args, "lwcm").map(|(_, ops)| ops),
        "ls" => {
            // `ls -l` declines on Windows and runs as a program.
            let (flags, ops) = short_flags(args, "aAl1d")?;
            (!flags.contains(&'l') || cfg!(unix)).then_some(ops)
        }
        "sed" => sed_operands(args),
        _ => None,
    }
}

/// The files `sed` reads: the operands after its script, as `sed()` splits its
/// arguments. `None` where `sed()` declines.
fn sed_operands(args: &[String]) -> Option<Vec<String>> {
    let mut have_script = false;
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-n" | "-E" | "-r" => {}
            "-e" => {
                i += 1;
                if i >= args.len() {
                    return None;
                }
                have_script = true;
            }
            "-ne" | "-nE" | "-En" => {
                if a == "-ne" {
                    i += 1;
                    if i >= args.len() {
                        return None;
                    }
                }
                have_script = true;
            }
            s if s.starts_with('-') && s != "-" => return None,
            _ => {
                if have_script {
                    files.push(args[i].clone());
                } else {
                    have_script = true;
                }
            }
        }
        i += 1;
    }
    Some(files)
}

fn walk_list(list: &List, out: &mut Vec<ExplainCommand>) {
    for ao in list {
        walk_pipeline(&ao.first, out);
        for (_, p) in &ao.rest {
            walk_pipeline(p, out);
        }
    }
}

fn walk_pipeline(p: &Pipeline, out: &mut Vec<ExplainCommand>) {
    let no_words: &[Word] = &[];
    for cmd in &p.cmds {
        let (redirs, words): (&[Redir], &[Word]) = match cmd {
            Cmd::Simple { redirs, words, .. } => (redirs.as_slice(), words.as_slice()),
            Cmd::If { redirs, .. }
            | Cmd::For { redirs, .. }
            | Cmd::While { redirs, .. }
            | Cmd::Group { redirs, .. } => (redirs.as_slice(), no_words),
        };
        let head = words.first().map(literal_text);
        let writes: Vec<(String, String)> = redirs.iter().filter_map(write_of).collect();
        let reads: Vec<String> = redirs
            .iter()
            .filter_map(|r| match r {
                Redir::In(target) => Some(literal_text(target)),
                _ => None,
            })
            .collect();
        if head.is_some() || !writes.is_empty() || !reads.is_empty() {
            out.push(ExplainCommand {
                head,
                args: words.iter().skip(1).map(literal_text).collect(),
                writes,
                reads,
            });
        }
        match cmd {
            Cmd::Simple { .. } => {}
            Cmd::If {
                branches,
                otherwise,
                ..
            } => {
                for (cond, body) in branches {
                    walk_list(cond, out);
                    walk_list(body, out);
                }
                if let Some(list) = otherwise {
                    walk_list(list, out);
                }
            }
            Cmd::For { body, .. } | Cmd::Group { body, .. } => walk_list(body, out),
            Cmd::While { cond, body, .. } => {
                walk_list(cond, out);
                walk_list(body, out);
            }
        }
    }
}

/// The output file a redirection names, as `(operator, target)`, or `None` for
/// an input, a duplication and `/dev/null`.
fn write_of(r: &Redir) -> Option<(String, String)> {
    let (op, target) = match r {
        Redir::Out { fd, append, target } => {
            let op = match (*fd == 2, *append) {
                (true, true) => "2>>",
                (true, false) => "2>",
                (false, true) => ">>",
                (false, false) => ">",
            };
            (op, target)
        }
        Redir::Both(target) => ("&>", target),
        Redir::In(_) | Redir::Dup { .. } => return None,
    };
    let text = literal_text(target);
    if text == "/dev/null" {
        return None;
    }
    Some((op.to_string(), text))
}

/// The text of a word made only of literal parts, or empty when any part is a
/// parameter or command substitution.
fn literal_text(word: &Word) -> String {
    word.iter()
        .map(|part| match part {
            Part::Lit(s, _) => Some(s.as_str()),
            _ => None,
        })
        .collect::<Option<String>>()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run in-process and capture stdout and the status.
    fn sh(src: &str) -> (String, i32) {
        let list = parse(src).unwrap_or_else(|u| panic!("{src}: {}", u.what));
        let mut shell = Shell {
            vars: HashMap::new(),
            exported: HashSet::new(),
            last: 0,
            glob_refused: None,
            positional: Vec::new(),
            script_name: SHELL_NAME.to_string(),
            errexit: false,
            nounset: false,
            pipefail: false,
            errexit_off: 0,
            unbound: None,
            line: 1,
        };
        let mut stdin = Input::buf(Vec::new());
        let mut sink = Sink::new(ErrMode::Null);
        let c = match shell.run_list(&list, &mut stdin, &mut sink) {
            Flow::Next(c) | Flow::Exit(c) => c,
            _ => 0,
        };
        (String::from_utf8_lossy(&sink.out).into_owned(), c)
    }

    /// seq gathers its whole output and printf allocates its padding before
    /// writing, so both refuse sizes past their caps instead of growing.
    #[test]
    fn seq_and_printf_refuse_sizes_past_their_caps() {
        assert_eq!(sh("seq 3"), ("1\n2\n3\n".into(), 0));
        assert_eq!(sh("seq 5 -2 1"), ("5\n3\n1\n".into(), 0));
        assert_eq!(sh("seq 3 1"), ("".into(), 0));
        assert_eq!(sh("seq 1 99999999999"), ("".into(), 1));
        assert_eq!(
            sh("seq 9223372036854775806 9223372036854775807"),
            ("9223372036854775806\n9223372036854775807\n".into(), 0)
        );
        assert_eq!(sh("printf '%5s|' ab"), ("   ab|".into(), 0));
        assert_eq!(sh("printf '%99999999s' x"), ("".into(), 1));
        assert_eq!(sh("printf '%999999999999999999999999s' x"), ("".into(), 1));
    }

    #[test]
    fn variables_quoting_and_control_flow() {
        assert_eq!(sh("X=5; echo $X"), ("5\n".into(), 0));
        assert_eq!(sh("export FOO=bar && echo $FOO"), ("bar\n".into(), 0));
        assert_eq!(sh(r#"echo "${NOPE_AE_X:-unset}""#), ("unset\n".into(), 0));
        assert_eq!(
            sh("for i in 1 2 3; do echo $i; done"),
            ("1\n2\n3\n".into(), 0)
        );
        // A loop feeding a pipeline must feed it, not the terminal.
        assert_eq!(
            sh("for i in 1 2 3; do echo $i; done | head -1"),
            ("1\n".into(), 0)
        );
        assert_eq!(sh("false || echo recovered"), ("recovered\n".into(), 0));
        assert_eq!(sh("true && echo chained"), ("chained\n".into(), 0));
        assert_eq!(sh("false && echo no"), (String::new(), 1));
        assert_eq!(sh("! false"), (String::new(), 0));
        assert_eq!(
            sh("if [ 1 -lt 2 ]; then echo yes; else echo no; fi"),
            ("yes\n".into(), 0)
        );
        assert_eq!(
            sh("i=0; while [ $i -lt 3 ]; do echo $i; i=$(echo 3); done"),
            ("0\n".into(), 0)
        );
        assert_eq!(
            sh("echo 'a  b' \"c  d\" e\\ f"),
            ("a  b c  d e f\n".into(), 0)
        );
        assert_eq!(sh("echo $(echo one two)"), ("one two\n".into(), 0));
        assert_eq!(
            sh("X='a b'; for w in $X; do echo [$w]; done"),
            ("[a]\n[b]\n".into(), 0)
        );
        assert_eq!(
            sh("echo \"lines: $(printf 'a\\nb\\n' | wc -l)\""),
            ("lines: 2\n".into(), 0)
        );
        assert_eq!(sh("exit 3; echo no"), (String::new(), 3));
    }

    #[test]
    fn text_utilities_match_gnu() {
        let input = "b\na\nc\na\n";
        let p = |cmd: &str| sh(&format!("printf '{}' | {cmd}", input.replace('\n', "\\n")));
        assert_eq!(p("sort").0, "a\na\nb\nc\n");
        assert_eq!(p("sort -u").0, "a\nb\nc\n");
        assert_eq!(p("sort | uniq -c").0, "      2 a\n      1 b\n      1 c\n");
        assert_eq!(p("head -2").0, "b\na\n");
        assert_eq!(p("tail -n 1").0, "a\n");
        assert_eq!(p("tail -n +3").0, "c\na\n");
        assert_eq!(p("wc -l").0, "4\n");
        assert_eq!(p("grep -c a").0, "2\n");
        assert_eq!(p("grep -n c").0, "3:c\n");
        assert_eq!(p("sed -n '2,3p'").0, "a\nc\n");
        assert_eq!(p("sed 's/a/X/'").0, "b\nX\nc\nX\n");
        assert_eq!(sh("echo k=v | cut -d= -f2").0, "v\n");
        assert_eq!(sh("printf 'x y z\\n' | wc").0, "      1       3       6\n");
        assert_eq!(sh("echo abc | head -c 2").0, "ab");
        assert_eq!(sh("grep x /nonexistent/ae-x").1, 2);
        assert_eq!(p("grep zzz").1, 1);
    }

    #[test]
    fn declines_what_it_does_not_run() {
        for src in [
            "f() { echo; }",
            "case x in x) echo;; esac",
            "echo $((1+2))",
            "cat <<EOF\nx\nEOF",
            "(cd /tmp; ls)",
            "sleep 1 &",
        ] {
            assert!(supports(src).is_err(), "{src} should be declined");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_locale_that_was_never_generated_sorts_by_bytes() {
        // glibc's `ls` and `sort` fall back to bytes when LANG names a locale
        // the host lacks; ae-disposable sets en_US.UTF-8 without generating it.
        let _env = crate::safety::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("LC_ALL", "xx_NOWHERE.UTF-8");
        let natural = locale_collates();
        std::env::remove_var("LC_ALL");
        assert!(!natural);
    }

    #[test]
    fn collation_matches_glibc_en_us() {
        // Measured against glibc 2.41 `sort` under en_US.UTF-8.
        let mut v: Vec<&str> = vec![
            "b",
            "B",
            "a",
            "A",
            "_a",
            ".a",
            "a_b",
            "ab",
            "a.b",
            "a-b",
            "A1",
            "a1",
            "a10",
            "a2",
            "Cargo.lock",
            "cargo.toml",
            "README.md",
            "build.rs",
            "_z",
            "Z",
            "z",
            "1",
            "10",
            "2",
            "a b",
            "ab_",
            "aB",
            "Ab",
        ];
        // The ordering itself, independent of whether this host generated
        // en_US.UTF-8.
        v.sort_by(|a, b| collate_in(true, a, b));
        assert_eq!(
            v,
            vec![
                "1",
                "10",
                "2",
                ".a",
                "_a",
                "a",
                "A",
                "a1",
                "A1",
                "a10",
                "a2",
                "a b",
                "a-b",
                "a.b",
                "a_b",
                "ab",
                "ab_",
                "aB",
                "Ab",
                "b",
                "B",
                "build.rs",
                "Cargo.lock",
                "cargo.toml",
                "README.md",
                "_z",
                "z",
                "Z"
            ]
        );
    }

    #[test]
    fn globstar_is_recursive_for_builtins_only() {
        let root = std::env::temp_dir().join(format!("ae_globstar_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b")).unwrap();
        for f in ["top.rs", "a/mid.rs", "a/b/deep.rs", "a/b/deep.rsx"] {
            std::fs::write(root.join(f), "").unwrap();
        }
        let r = root.to_string_lossy().replace('\\', "/");
        let found = glob_paths(&format!("{r}/**/*.rs"));
        let names: Vec<&str> = found
            .iter()
            .map(|p| p.rsplit('/').next().unwrap())
            .collect();
        assert_eq!(names, vec!["deep.rs", "mid.rs", "top.rs"], "{found:?}");
        // Without globstar, `**` is `*`: one level only.
        let shell = glob_expand(&format!("{r}/**/*.rs")).expect("under the cap");
        assert_eq!(shell.len(), 1, "{shell:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn globs() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(!glob_match("*.rs", "main.rsx"));
        assert!(glob_match("a?c", "abc"));
        assert!(glob_match("[a-c]x", "bx"));
        assert!(!glob_match("[!a-c]x", "bx"));
    }
}

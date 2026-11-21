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
//! What it does not parse (functions, `case`, here-documents,
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
    /// `$((…))`: the expression as a word (its `$` expansions come first), and
    /// whether quoted.
    Arith(Vec<Part>, bool),
}

type Word = Vec<Part>;

#[derive(Debug, Clone)]
enum Redir {
    /// `[n]>file`, `[n]>>file`
    Out { fd: u8, append: bool, target: Word },
    /// `<file`
    In(Word),
    /// `<<< word`: the word, expanded as one double-quoted word, and a newline
    Str(Word),
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

/// The part that separates the offset from the length in the word of a
/// `${name:offset:length}`.
const SUBSTRING_SPLIT: &str = "\u{1}";

fn skip_blanks(c: &[char], i: &mut usize) {
    while c.get(*i).is_some_and(|x| x.is_whitespace()) {
        *i += 1;
    }
}

/// A C integer constant as bash's arithmetic reads it: decimal, `0x` hex or a
/// leading `0` for octal.
fn parse_c_integer(token: &str) -> Option<i64> {
    if let Some(h) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        i64::from_str_radix(h, 16).ok()
    } else if token.len() > 1 && token.starts_with('0') {
        i64::from_str_radix(&token[1..], 8).ok()
    } else {
        token.parse().ok()
    }
}

/// `chars` lexed as the inside of a double-quoted word, appended to `w`.
fn lex_quoted(chars: &[char], w: &mut Word) -> PResult<()> {
    let mut text = chars.to_vec();
    text.push('"');
    let mut q = Lexer { s: &text, i: 0 };
    q.double_quoted(w)
}

/// Whether `s` is a shell identifier: a letter or `_`, then letters, digits
/// and `_`.
fn valid_name(s: &str) -> bool {
    let mut it = s.chars();
    it.next().is_some_and(is_name_start) && it.all(is_name_char)
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}
fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The operand of a `${x:-...}` inside double quotes, less its own unescaped
/// `"` characters: the text between them is quoted already. A `$(...)` and a
/// backquoted command keep theirs.
fn strip_inner_quotes(inner: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        match inner[i] {
            '\\' => {
                out.push('\\');
                if let Some(&n) = inner.get(i + 1) {
                    out.push(n);
                    i += 1;
                }
            }
            '$' if inner.get(i + 1) == Some(&'(') => {
                let mut depth = 0;
                while i < inner.len() {
                    out.push(inner[i]);
                    match inner[i] {
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            '`' => {
                out.push('`');
                i += 1;
                while i < inner.len() {
                    out.push(inner[i]);
                    if inner[i] == '`' {
                        break;
                    }
                    i += 1;
                }
            }
            '"' => {}
            c => out.push(c),
        }
        i += 1;
    }
    out
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
                    if self.peek_at(1) != Some('<') {
                        return unsupported("here-document (<<)");
                    }
                    if fd.is_some_and(|f| f != 0) {
                        return unsupported("input descriptor");
                    }
                    // `<<< word`: the word is the command's standard input.
                    self.i += 2;
                    return Ok(Redir::Str(self.target()?));
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
                let mut digits = String::new();
                while let Some(c @ '0'..='9') = self.peek() {
                    digits.push(c);
                    self.i += 1;
                }
                if digits.is_empty() {
                    return unsupported(">&word");
                }
                // Only descriptors 1 and 2 are open in a script that cannot open
                // one (`3>f` and `exec 3>f` are refused). `echo hi >&3` is bash's
                // `Bad file descriptor`, and `>&0` writes to the input, so both
                // are refused here, before anything runs.
                return match (fd, digits.as_str()) {
                    (1 | 2, "1" | "2") => Ok(Redir::Dup {
                        from: fd,
                        to: digits.as_bytes()[0] - b'0',
                    }),
                    (0, _) | (_, "0") => unsupported("descriptor 0 in `>&`"),
                    _ => unsupported("descriptor above 2"),
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
            // `< <(cmd)` and `> >(cmd)`: a process substitution is the target.
            Some('<' | '>') if self.peek_at(1) == Some('(') => unsupported("process substitution"),
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
                '$' if self.peek_at(1) == Some('\'') => {
                    flush(&mut lit, &mut parts);
                    let open = self.i;
                    self.i += 2;
                    let q = self.ansi_c_quoted(open)?;
                    parts.push(Part::Lit(q, true));
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
                '{' if self.brace_expansion_ahead() => {
                    return unsupported_at("brace expansion {a,b} or {a..b}", self.i);
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

    /// The body of `$'...'`, called just past the opening `$'` (`open` is the
    /// `$`): backslash escapes are decoded, the closing `'` is consumed.
    fn ansi_c_quoted(&mut self, open: usize) -> PResult<String> {
        let mut q = String::new();
        loop {
            let Some(c) = self.peek() else {
                return unterminated("unterminated quote", open, "a `'`");
            };
            self.i += 1;
            match c {
                '\'' => return Ok(q),
                '\\' => {
                    let Some(e) = self.peek() else {
                        return unterminated("unterminated quote", open, "a `'`");
                    };
                    self.i += 1;
                    match e {
                        'n' => q.push('\n'),
                        't' => q.push('\t'),
                        'r' => q.push('\r'),
                        'a' => q.push('\x07'),
                        'b' => q.push('\x08'),
                        'e' | 'E' => q.push('\x1b'),
                        'f' => q.push('\x0c'),
                        'v' => q.push('\x0b'),
                        '\\' | '\'' | '"' | '?' => q.push(e),
                        '0'..='7' => {
                            let mut v = e.to_digit(8).unwrap_or(0);
                            for _ in 0..2 {
                                match self.peek().and_then(|d| d.to_digit(8)) {
                                    Some(d) => {
                                        v = v * 8 + d;
                                        self.i += 1;
                                    }
                                    None => break,
                                }
                            }
                            q.push(char::from_u32(v & 0xff).unwrap_or('\0'));
                        }
                        'x' => {
                            let mut v = 0u32;
                            let mut n = 0;
                            while n < 2 {
                                match self.peek().and_then(|d| d.to_digit(16)) {
                                    Some(d) => {
                                        v = v * 16 + d;
                                        self.i += 1;
                                        n += 1;
                                    }
                                    None => break,
                                }
                            }
                            if n == 0 {
                                q.push_str("\\x");
                            } else {
                                q.push(char::from_u32(v).unwrap_or('\0'));
                            }
                        }
                        // \uHHHH and \UHHHHHHHH: a Unicode code point of up to
                        // four or eight hex digits; none at all is literal.
                        'u' | 'U' => {
                            let max = if e == 'u' { 4 } else { 8 };
                            let mut v = 0u32;
                            let mut n = 0;
                            while n < max {
                                match self.peek().and_then(|d| d.to_digit(16)) {
                                    Some(d) => {
                                        v = v * 16 + d;
                                        self.i += 1;
                                        n += 1;
                                    }
                                    None => break,
                                }
                            }
                            match (n, char::from_u32(v)) {
                                (0, _) | (_, None) => {
                                    q.push('\\');
                                    q.push(e);
                                }
                                (_, Some(ch)) => q.push(ch),
                            }
                        }
                        // \cX: the control character, X with its high bits cleared
                        // (\c? is DEL).
                        'c' => match self.peek() {
                            Some(x) if x.is_ascii() => {
                                self.i += 1;
                                q.push(if x == '?' {
                                    '\x7f'
                                } else {
                                    char::from(x as u8 & 0x1f)
                                });
                            }
                            _ => q.push_str("\\c"),
                        },
                        other => {
                            q.push('\\');
                            q.push(other);
                        }
                    }
                }
                _ => q.push(c),
            }
        }
    }

    /// At an unquoted `{`: does a brace expansion (`{a,b}` or `{1..3}`) start
    /// here? `{}`, `{a}` and a lone `{` are literal, as in bash.
    fn brace_expansion_ahead(&self) -> bool {
        let mut depth = 0usize;
        let mut found = false;
        let mut j = self.i;
        while let Some(&c) = self.s.get(j) {
            match c {
                ' ' | '\t' | '\r' | '\n' | ';' | '|' | '&' | '<' | '>' | '(' | ')' => return false,
                '\\' => j += 1,
                '\'' | '"' => {
                    j += 1;
                    while self.s.get(j).is_some_and(|&d| d != c) {
                        if c == '"' && self.s[j] == '\\' {
                            j += 1;
                        }
                        j += 1;
                    }
                }
                '{' => depth += 1,
                '}' => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                    if depth == 0 {
                        return found;
                    }
                }
                ',' if depth == 1 => found = true,
                '.' if depth == 1 && self.s.get(j + 1) == Some(&'.') => found = true,
                _ => {}
            }
            j += 1;
        }
        false
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
                    if let Some(end) = self.arith_end() {
                        // The expression is read as double-quoted text, so
                        // `$n`, `${n}` and `$(cmd)` in it expand first, as in
                        // bash; its names and operators are read at run time.
                        let start = self.i + 2;
                        let inner: Vec<char> = self.s[start..end].to_vec();
                        if inner.contains(&'"') || inner.contains(&'`') {
                            return unsupported_at("arithmetic expansion $((...))", dollar_at);
                        }
                        let mut w = Vec::new();
                        lex_quoted(&inner, &mut w).map_err(|e| relocate(e, start, dollar_at))?;
                        self.i = end + 2;
                        parts.push(Part::Arith(w, quoted));
                        return Ok(());
                    }
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
                // `${#@}` and `${#*}` are `$#`.
                if self.peek() == Some('#')
                    && matches!(self.peek_at(1), Some('@' | '*'))
                    && self.peek_at(2) == Some('}')
                {
                    self.i += 3;
                    parts.push(Part::Var("#".to_string(), None, quoted));
                    return Ok(());
                }
                if self.peek() == Some('#')
                    && self
                        .peek_at(1)
                        .is_some_and(|c| is_name_start(c) || c.is_ascii_digit())
                {
                    self.i += 1;
                    // `${#1}` is the length of the first argument.
                    let name = if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                        let mut digits = String::new();
                        while let Some(d) = self.peek().filter(|d| d.is_ascii_digit()) {
                            digits.push(d);
                            self.i += 1;
                        }
                        digits
                    } else {
                        self.name()
                    };
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
                    // `${@}` and `${*}` are `$@` and `$*`; with anything
                    // before the brace (`${@:2}`) they are outside the subset.
                    Some(c @ ('@' | '*')) if self.peek_at(1) == Some('}') => {
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
                    // `${name:offset}` and `${name:offset:length}`: `op` stays a
                    // bare `:`, its word the offset and the length (see
                    // `Shell::substring`). A `-` right after the colon is the
                    // `:-` operator above, so a negative offset is written
                    // with a blank or parentheses.
                    Some(c)
                        if op == ":"
                            && (c.is_ascii_alphanumeric()
                                || matches!(c, '_' | ' ' | '\t' | '$' | '(')) => {}
                    // `${x` with the text ending, or a blank, before any `}`.
                    Some(c) if c.is_whitespace() && !self.s[self.i..].contains(&'}') => {
                        return unterminated("unterminated ${", dollar_at, "a `}`");
                    }
                    None => return unterminated("unterminated ${", dollar_at, "a `}`"),
                    Some('[') if name == "PIPESTATUS" => {
                        return unsupported_at("`PIPESTATUS` array", dollar_at)
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
                let word = if op == ":" {
                    // The offset, then (after a marker part) the length, each
                    // read as double-quoted text so `$n` and blanks work.
                    let mut depth = 0i32;
                    let mut colon = None;
                    for (k, c) in inner.iter().enumerate() {
                        match c {
                            '{' | '(' => depth += 1,
                            '}' | ')' => depth -= 1,
                            ':' if depth == 0 => {
                                colon = Some(k);
                                break;
                            }
                            _ => {}
                        }
                    }
                    let (offset, length) = match colon {
                        Some(k) => (&inner[..k], Some(&inner[k + 1..])),
                        None => (&inner[..], None),
                    };
                    let mut w = Vec::new();
                    lex_quoted(offset, &mut w).map_err(|e| relocate(e, start, dollar_at))?;
                    if let Some(length) = length {
                        w.push(Part::Lit(SUBSTRING_SPLIT.to_string(), true));
                        lex_quoted(length, &mut w).map_err(|e| relocate(e, start, dollar_at))?;
                    }
                    w
                } else if inner.is_empty() {
                    Vec::new()
                } else {
                    let mut sub = Lexer { s: &inner, i: 0 };
                    let mut w = Vec::new();
                    // Treat the operand like double-quoted text when the
                    // expansion itself is quoted, else as a word.
                    if quoted && !matches!(op.as_str(), "%" | "%%" | "#" | "##") {
                        // A `"` inside the operand opens and closes quoting
                        // that is already in force, so it only drops out.
                        let mut chars = strip_inner_quotes(&inner);
                        chars.push('"');
                        let mut q = Lexer { s: &chars, i: 0 };
                        q.double_quoted(&mut w)
                            .map_err(|e| relocate(e, start, dollar_at))?;
                    } else {
                        // The operand runs to the closing brace, blanks
                        // included (`${1:?usage: f COLUMN}`), as in bash; its
                        // words are kept apart by the blanks between them.
                        // A leading blank is part of it (`${v%% *}` trims from
                        // the first space).
                        let lead: String = inner
                            .iter()
                            .take_while(|c| matches!(c, ' ' | '\t'))
                            .collect();
                        if !lead.is_empty() {
                            sub.i += lead.chars().count();
                            w.push(Part::Lit(lead, false));
                        }
                        if sub.i < inner.len() {
                            w.extend(sub.word().map_err(|e| relocate(e, start, dollar_at))?);
                        }
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
                            if sub.i >= inner.len() {
                                break;
                            }
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

    /// With `self.i` at the `(` of `$((`: the offset of the first `)` of the
    /// closing `))`, when the text up to it holds balanced parentheses. `None`
    /// when the first unbalanced `)` is not followed by another (`$((cmd) ...)`
    /// is a command substitution holding a subshell) or the text ends.
    fn arith_end(&self) -> Option<usize> {
        let mut depth = 0usize;
        let mut j = self.i + 2;
        while let Some(&c) = self.s.get(j) {
            match c {
                '(' => depth += 1,
                ')' if depth == 0 => {
                    return (self.s.get(j + 1) == Some(&')')).then_some(j);
                }
                ')' => depth -= 1,
                _ => {}
            }
            j += 1;
        }
        None
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
                Part::Arith(..) => "$((...))".to_string(),
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

/// The `< word` of a list that is nothing but one redirection from a file
/// (the body of `$(< file)`).
fn bare_input_redirect(list: &List) -> Option<&Redir> {
    let [ao] = list.as_slice() else {
        return None;
    };
    if !ao.rest.is_empty() || ao.first.negate {
        return None;
    }
    let [Cmd::Simple {
        assigns,
        words,
        redirs,
    }] = ao.first.cmds.as_slice()
    else {
        return None;
    };
    match redirs.as_slice() {
        [r @ Redir::In(_)] if assigns.is_empty() && words.is_empty() => Some(r),
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
                if kw == "if" && self.peek_keyword() == Some("fi") {
                    format!("the `if` at line {line} needs `then` before its body")
                } else if matches!(kw, "while" | "until") && self.peek_keyword() == Some("done") {
                    format!("the `{kw}` at line {line} needs `do` before its body")
                } else if self.peek().is_none() {
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

    /// [`operator_what`] for the operator at the current token: `((` written
    /// together is bash's arithmetic command, not two subshells.
    fn operator_here(&self, t: &Tok) -> &'static str {
        let adjacent = self.at.get(self.i + 1).copied() == self.at.get(self.i).map(|a| a + 1);
        if matches!(t, Tok::LParen)
            && matches!(self.toks.get(self.i + 1), Some(Tok::LParen))
            && adjacent
        {
            return "arithmetic command ((...))";
        }
        operator_what(t)
    }

    fn peek_keyword(&self) -> Option<&str> {
        match self.peek() {
            Some(Tok::Word(w)) => plain(w).filter(|s| RESERVED.contains(s)),
            _ => None,
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Some(Tok::Nl)) {
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

    /// The list of an `if`, `elif`, `while` or `until` condition: bash needs
    /// at least one command in it, so an empty one is a syntax error at the
    /// token that stopped it (`do`, `then`).
    fn condition(&mut self, stop: &[&str]) -> PResult<List> {
        let list = self.list(stop)?;
        if list.is_empty() {
            return self.syntax("a condition needs at least one command".to_string());
        }
        Ok(list)
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
                // A separator or operator with no command before it.
                Some(Tok::Semi | Tok::DSemi | Tok::And | Tok::Or | Tok::Pipe) => {
                    return self.syntax("a command is needed before this token".to_string());
                }
                Some(t) => return unsupported(self.operator_here(t)),
            }
            out.push(self.and_or()?);
            match self.peek() {
                None | Some(Tok::Nl) => {}
                // One `;` ends a command; a second has no command before it.
                Some(Tok::Semi) => self.i += 1,
                Some(Tok::DSemi) => {
                    return self.syntax("`;;` is only valid in a `case`".to_string());
                }
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
                let cond = self.condition(&["then"])?;
                self.expect_keyword("then")?;
                let body = self.list(&["elif", "else", "fi"])?;
                branches.push((cond, body));
                let mut otherwise = None;
                loop {
                    self.skip_newlines();
                    match self.peek_keyword() {
                        Some("elif") => {
                            self.i += 1;
                            let c = self.condition(&["then"])?;
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
                if matches!(self.peek(), Some(Tok::LParen)) {
                    // `for ((i=0; i<n; i++))` is bash's, not in the subset.
                    return unsupported_at("arithmetic `for ((...))` loop", at);
                }
                let var = match self.peek() {
                    Some(Tok::Word(w)) => match plain(w) {
                        Some(n)
                            if n.chars().next().is_some_and(is_name_start)
                                && n.chars().all(is_name_char) =>
                        {
                            n.to_string()
                        }
                        _ => {
                            return self
                                .syntax("a `for` needs a variable name to loop with".to_string())
                        }
                    },
                    _ => {
                        return self
                            .syntax("a `for` needs a variable name to loop with".to_string())
                    }
                };
                self.i += 1;
                while matches!(self.peek(), Some(Tok::Nl)) {
                    self.i += 1;
                }
                if self.peek_keyword() != Some("in") {
                    return unsupported("for without in");
                }
                self.i += 1;
                // `do` is a word of the list until a `;` or newline ends it.
                let mut items = Vec::new();
                while let Some(Tok::Word(w)) = self.peek() {
                    items.push(w.clone());
                    self.i += 1;
                }
                match self.peek() {
                    Some(Tok::Semi) => {
                        self.i += 1;
                        self.skip_newlines();
                    }
                    Some(Tok::Nl) => self.skip_newlines(),
                    _ => {}
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
                let cond = self.condition(&["do"])?;
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
                    if words.len() == 1
                        && assigns.is_empty()
                        && redirs.is_empty()
                        && matches!(self.toks.get(self.i + 1), Some(Tok::RParen)) =>
                {
                    return unsupported_at("function definition", start);
                }
                // A `(` after a command's name is not a subshell (that starts a
                // command) and not `name()`: bash's syntax error.
                Some(Tok::LParen)
                    if words
                        .last()
                        .and_then(plain)
                        .is_some_and(|w| w.ends_with('=')) =>
                {
                    // `export a=(1 2)`, `declare -a a=(...)`: an array assignment.
                    return unsupported_at("array assignment", start);
                }
                Some(Tok::LParen) if !words.is_empty() => {
                    return self.syntax("a `(` can only start a subshell or follow a function name as `name()`; quote it to pass it as text".to_string());
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
                Some(Tok::DSemi) => {
                    return self.syntax("`;;` is only valid in a `case`".to_string());
                }
                Some(t @ Tok::LParen) => {
                    return unsupported(self.operator_here(t));
                }
                _ => break,
            }
        }
        if assigns.is_empty() && words.is_empty() && redirs.is_empty() {
            return self.syntax("a command is needed here".to_string());
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
    "time",
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
        "read" => read_refusal(&words[1..]),
        "sed" => sed_refusal(&words[1..]),
        // `command -v NAME...` runs (see `command_v`); any other `command` does not.
        "command" if arg == Some("-v") && words.len() > 2 => None,
        n if UNSUPPORTED_BUILTINS.contains(&n) => Some(format!("`{n}` builtin")),
        _ => None,
    }
}

/// `sed -f` with the script on standard input (`-f /dev/stdin`, `-f -`): the
/// in-process sed reads no script file, and a PATH sed would find the pipe
/// where this shell has already put the input. `None` for every other `sed`.
fn sed_refusal(words: &[Word]) -> Option<String> {
    let stdin_script = |v: &str| matches!(v, "/dev/stdin" | "-" | "/dev/fd/0" | "/proc/self/fd/0");
    let mut k = 0;
    while k < words.len() {
        let Some(a) = plain(&words[k]) else {
            k += 1;
            continue;
        };
        if a == "-f" || a == "--file" {
            if words.get(k + 1).and_then(plain).is_some_and(stdin_script) {
                return Some("`sed -f /dev/stdin`".to_string());
            }
            k += 1;
        } else if a
            .strip_prefix("-f")
            .or_else(|| a.strip_prefix("--file="))
            .is_some_and(stdin_script)
        {
            return Some("`sed -f /dev/stdin`".to_string());
        }
        k += 1;
    }
    None
}

/// The option of `read` the subset does not run, as the refusal names it, or
/// `None` when it runs every one written. `read` takes `-r`, `-s`, `-e`, `-p
/// PROMPT`, `-u 0`, and `-d DELIM` and `-n N` together with `-r`; `-N`, `-t`,
/// `-a`, `-i` and the rest are refused, as is a name that is an array element.
fn read_refusal(words: &[Word]) -> Option<String> {
    let mut k = 0;
    let mut raw = false;
    let mut needs_raw: Option<&str> = None;
    while k < words.len() {
        let Some(a) = plain(&words[k]) else { break };
        if a == "--" {
            k += 1;
            break;
        }
        if a.len() < 2 || !a.starts_with('-') {
            break;
        }
        k += 1;
        let flags: Vec<char> = a[1..].chars().collect();
        for (at, &c) in flags.iter().enumerate() {
            match c {
                'r' => raw = true,
                's' | 'e' => {}
                'p' | 'd' | 'n' | 'u' => {
                    // The value is the rest of this word, or the next word.
                    let rest: String = flags[at + 1..].iter().collect();
                    let value = if rest.is_empty() {
                        let v = words.get(k).and_then(plain).map(str::to_string);
                        k += 1;
                        v
                    } else {
                        Some(rest)
                    };
                    match c {
                        'u' if value.as_deref() != Some("0") => {
                            return Some("`read -u`".to_string())
                        }
                        'n' if !value.as_deref().is_some_and(|v| {
                            !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit())
                        }) =>
                        {
                            return Some("`read -n`".to_string())
                        }
                        'd' => needs_raw = Some("-d"),
                        'n' => needs_raw = Some("-n"),
                        _ => {}
                    }
                    break;
                }
                other => return Some(format!("`read -{other}`")),
            }
        }
    }
    if let (Some(opt), false) = (needs_raw, raw) {
        return Some(format!("`read {opt}` without -r"));
    }
    words[k.min(words.len())..]
        .iter()
        .map(literal_text)
        .find(|n| n.contains('['))
        .map(|n| format!("`read {n}` (an array element)"))
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
    // `name+=value` appends: the name keeps its `+` here, and `run_simple`
    // reads it.
    let base = name.strip_suffix('+').unwrap_or(name);
    if base.is_empty()
        || !base.chars().next().is_some_and(is_name_start)
        || !base.chars().all(is_name_char)
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
    /// Standard input is a regular file (`< file`): `wc` then pads its numbers
    /// to the width of the file's size, as GNU `wc` does, not to 7.
    file: bool,
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
            file: false,
        }
    }
    /// The contents of a regular file read as standard input (`< file`).
    fn file(data: Vec<u8>) -> Self {
        Input {
            file: true,
            ..Input::buf(data)
        }
    }
    fn real() -> Self {
        Input {
            data: Vec::new(),
            pos: 0,
            real: true,
            eof: false,
            file: false,
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
        self.read_record(b'\n')
    }

    /// Reads one more chunk of real standard input, up to the next `delim`, into
    /// the buffer. `Err` for a record longer than [`READ_LINE_MAX`].
    fn fill_chunk(&mut self, delim: u8) -> Result<(), String> {
        use std::io::BufRead;
        let mut chunk = Vec::new();
        let limit = (READ_LINE_MAX + 1) as u64;
        match std::io::stdin()
            .lock()
            .take(limit)
            .read_until(delim, &mut chunk)
        {
            Ok(0) | Err(_) => self.eof = true,
            Ok(_) if chunk.len() > READ_LINE_MAX && chunk.last() != Some(&delim) => {
                self.eof = true;
                return Err(format!("a line longer than {READ_LINE_MAX} bytes"));
            }
            Ok(_) => self.data.extend_from_slice(&chunk),
        }
        Ok(())
    }

    /// [`Input::read_line`] with `delim` ending the record instead of a newline.
    fn read_record(&mut self, delim: u8) -> Result<Option<(Vec<u8>, bool)>, String> {
        loop {
            if let Some(nl) = self.data[self.pos..].iter().position(|&b| b == delim) {
                let line = self.data[self.pos..self.pos + nl].to_vec();
                self.pos += nl + 1;
                if self.real {
                    self.data.drain(..self.pos);
                    self.pos = 0;
                }
                return Ok(Some((line, true)));
            }
            if self.real && !self.eof {
                self.fill_chunk(delim)?;
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

    /// `read -n N`: up to `n` characters, fewer when a newline comes first (it
    /// is consumed and not returned). The flag is true when the line ended by
    /// a newline or by reaching `n` characters, false at end of input.
    fn read_chars(&mut self, n: usize) -> Result<Option<(Vec<u8>, bool)>, String> {
        if n == 0 {
            return Ok(Some((Vec::new(), true)));
        }
        loop {
            let rest = &self.data[self.pos..];
            let mut chars = 0usize;
            let mut end = None;
            for (i, &b) in rest.iter().enumerate() {
                if b == b'\n' {
                    end = Some((i, i + 1));
                    break;
                }
                // A byte that starts a character, not one that continues it.
                if b & 0xC0 != 0x80 {
                    if chars == n {
                        end = Some((i, i));
                        break;
                    }
                    chars += 1;
                }
            }
            if end.is_none() && chars == n {
                end = Some((rest.len(), rest.len()));
            }
            if let Some((take, consume)) = end {
                let line = rest[..take].to_vec();
                self.pos += consume;
                if self.real {
                    self.data.drain(..self.pos);
                    self.pos = 0;
                }
                return Ok(Some((line, true)));
            }
            if self.real && !self.eof {
                self.fill_chunk(b'\n')?;
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

/// The parts of the shell's state a subshell may change without the parent
/// seeing it (see [`Shell::subshell_state`]).
struct SubshellState {
    vars: HashMap<String, String>,
    exported: Vec<String>,
    unset_env: Vec<String>,
    positional: Vec<String>,
    options: (bool, bool, bool),
    cwd: Option<std::path::PathBuf>,
}

/// Where a descriptor of a redirected command points.
#[derive(Clone, PartialEq)]
enum Dest {
    /// The enclosing command's standard output.
    Out,
    /// The enclosing command's standard error.
    Err,
    /// `/dev/null`.
    Null,
    /// A file, and whether it is appended to.
    File(String, bool),
}

impl Dest {
    /// Send `data` where this descriptor points.
    fn write(&self, data: &[u8], sink: &mut Sink) -> Result<(), String> {
        match self {
            Dest::Out => sink.out.extend_from_slice(data),
            Dest::Err => sink.ewrite(data),
            Dest::Null => {}
            Dest::File(path, append) => write_file(path, data, *append)?,
        }
        Ok(())
    }
}

/// Bash opens every file of a command's redirections, left to right, so one
/// that a later redirection of the same descriptor replaces is still created
/// (or truncated, unless it is appended to). `old` is the destination being
/// replaced; it is opened now unless a descriptor in `keep` still points at it,
/// since that descriptor's own write opens it.
fn open_replaced(old: &Dest, keep: &[&Dest]) -> Result<(), String> {
    match old {
        Dest::File(path, append) if !keep.contains(&old) => write_file(path, b"", *append),
        _ => Ok(()),
    }
}

/// A word as it was written, for an error message that names it.
fn word_source(w: &Word) -> String {
    w.iter()
        .map(|p| match p {
            Part::Lit(s, _) => s.clone(),
            Part::Var(n, None, _) => format!("${n}"),
            Part::Var(n, Some(_), _) => format!("${{{n}}}"),
            Part::Len(n) => format!("${{#{n}}}"),
            Part::Cmd(..) => "$(...)".to_string(),
            Part::Arith(..) => "$((...))".to_string(),
        })
        .collect()
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
    /// Set on the sink of a redirected simple command: its own expansion
    /// errors happen before its redirections apply, so they belong to the
    /// standard error the command had before (`parent_err`), not to `2>&1`.
    before_redirs: bool,
    parent_err: Vec<u8>,
}

impl Sink {
    fn new(err_mode: ErrMode) -> Self {
        Sink {
            out: Vec::new(),
            err_mode,
            err: Vec::new(),
            before_redirs: false,
            parent_err: Vec::new(),
        }
    }
    /// A line for the standard error the command's expansion would write to.
    fn expansion_eline(&mut self, s: &str) {
        if self.before_redirs {
            self.parent_err
                .extend_from_slice(format!("{s}\n").as_bytes());
        } else {
            self.eline(s);
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

thread_local! {
    /// The redirections (`>path`, `>>path`, `<path`, as resolved paths) around
    /// the command being run, for the approval of an external command.
    static EXEC_REDIRS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// A redirection target as an absolute path, for an approval token.
fn resolved_for_binding(path: &str) -> String {
    std::path::absolute(path)
        .map(|p| p.to_string_lossy().replace("\\", "/"))
        .unwrap_or_else(|_| path.to_string())
}

struct Shell {
    vars: HashMap<String, String>,
    /// The names exported to children, in the order they were last exported.
    exported: Vec<String>,
    /// Inherited variables the script unset: removed from every child's
    /// environment, and bound into the approval of a command that runs.
    unset_env: Vec<String>,
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
    /// Whether `unbound` came from an arithmetic expansion. Bash does not end
    /// the script for that error: it discards the command line it was on.
    arith_failed: bool,
    /// Set when a command was refused for an arithmetic error: the top level
    /// reads it, skips the rest of that line and goes on with the next.
    discarded: bool,
    /// The line of the pipeline now running, for an expansion error's message.
    line: usize,
    /// How many command substitutions are running: a fatal expansion error in
    /// one ends it with status 1, where the main shell's `-c` string gives 127.
    subst_depth: u32,
}

const SHELL_NAME: &str = "ish";

/// When this shell started, for `$SECONDS`.
static SHELL_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// The status of a command a policy refused (`E_POLICY_DENY`), as the other
/// refusals of the shell exit.
const POLICY_DENY_STATUS: i32 = 77;

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
        // `$RANDOM` is a new number each time it is read, whatever the
        // environment holds.
        if name == "RANDOM" && !self.vars.contains_key(name) {
            return Some(next_random().to_string());
        }
        // The line of the command now running, as bash numbers it.
        if name == "LINENO" {
            return Some(self.line.to_string());
        }
        // A variable the script unset stays unset, whatever the environment
        // this process started with holds.
        let folded = crate::safety::fold_env_name(name);
        let was_unset = self
            .unset_env
            .iter()
            .any(|n| crate::safety::fold_env_name(n) == folded);
        // The PWD this process inherited is trusted only while it names the
        // directory it is in, as bash checks it at start-up: a parent that
        // `cd`ed first and a stale PWD in the environment gave `$PWD/bin` the
        // wrong place.
        if name == "PWD" && !was_unset {
            return Some(logical_pwd());
        }
        match std::env::var(name).ok().filter(|_| !was_unset) {
            Some(v) => Some(v),
            // Windows has no HOME; the platform's home directory stands in for it
            // (only here, the process environment is left alone).
            None if name == "HOME" && !was_unset => {
                dirs::home_dir().map(|p| p.to_string_lossy().into_owned())
            }
            // The variables a bash always has, which a Windows environment
            // lacks or spells differently. `OLDPWD` stays unset until a `cd`.
            None if name == "PWD" => Some(logical_pwd()),
            None if name == "USER" => std::env::var("USERNAME").ok(),
            None if name == "SHELL" => std::env::current_exe()
                .ok()
                .map(|p| posix_path(&p.to_string_lossy())),
            None if name == "PPID" => parent_pid(),
            // `$UID` and `$EUID` are the user's numeric id (bash makes them
            // read-only at start-up). Windows has none: it reads as 1000, so a
            // test for root (`[ "$EUID" -eq 0 ]`) takes the ordinary-user path.
            None if name == "UID" || name == "EUID" => Some(user_id().to_string()),
            // The platform bash reports: `msys` on Windows (as Git for Windows'
            // bash does), `linux-gnu`, or `darwin`.
            None if name == "OSTYPE" => Some(
                if cfg!(windows) {
                    "msys"
                } else if cfg!(target_os = "macos") {
                    "darwin"
                } else {
                    "linux-gnu"
                }
                .to_string(),
            ),
            // The bash this engine follows, so a script that tests for bash
            // (`[ -n "$BASH_VERSION" ]`) finds one; `-ish` marks that it is not.
            None if name == "BASH_VERSION" => Some("5.2.0(1)-ish".to_string()),
            None if name == "HOSTNAME" => sysinfo::System::host_name(),
            None if name == "HOSTTYPE" => Some(std::env::consts::ARCH.to_string()),
            // The machine type bash was built for, written as its OSTYPE is.
            None if name == "MACHTYPE" => Some(if cfg!(windows) {
                format!("{}-pc-msys", std::env::consts::ARCH)
            } else if cfg!(target_os = "macos") {
                format!("{}-apple-darwin", std::env::consts::ARCH)
            } else {
                format!("{}-pc-linux-gnu", std::env::consts::ARCH)
            }),
            None if name == "BASHPID" => Some(std::process::id().to_string()),
            // Whole seconds since this shell started.
            None if name == "SECONDS" => Some(
                SHELL_START
                    .get_or_init(std::time::Instant::now)
                    .elapsed()
                    .as_secs()
                    .to_string(),
            ),
            // Set in every bash: the default field separators, and OPTIND 1.
            None if name == "IFS" => Some(" \t\n".to_string()),
            None if name == "OPTIND" => Some("1".to_string()),
            None => None,
        }
    }

    /// What `"$*"` puts between the arguments: the first character of IFS,
    /// nothing when IFS is empty, a space when it is unset.
    fn star_sep(&self) -> String {
        match self.vars.get("IFS") {
            Some(ifs) => ifs.chars().next().map(String::from).unwrap_or_default(),
            None => " ".to_string(),
        }
    }

    fn set_var(&mut self, name: &str, value: String) {
        // A variable that is not exported never reaches a child, so a steering
        // name may be a plain shell variable even in agent mode (`tmp=x`). The
        // check is made where a value reaches a child: `export`, a prefix
        // assignment on a command (see `run_simple` and `refuse_steering`).
        self.vars.insert(name.to_string(), value);
        // PATH comes from the environment, so a plain assignment changes what
        // programs inherit (bash's variables of that kind stay exported). Not
        // in agent mode, where it steers programs and is refused if exported.
        if name == "PATH"
            && crate::safety::current_mode() != crate::safety::Mode::Agent
            && std::env::var_os("PATH").is_some()
            && !self.exported.iter().any(|n| n == "PATH")
        {
            self.mark_exported("PATH".to_string());
        }
    }

    /// Under `set -u`, an unset parameter is noted for the command whose
    /// expansion met it, which then exits (see [`Shell::unbound_refusal`]).
    /// The first one wins.
    fn note_unbound(&mut self, name: &str) {
        if self.nounset && self.unbound.is_none() {
            // Bash names a positional parameter with its `$`: `$1: unbound
            // variable`, and a variable by its bare name.
            let named = if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
                format!("${name}")
            } else {
                name.to_string()
            };
            self.unbound = Some(self.expansion_message(&named, "unbound variable"));
        }
    }

    /// An expansion error in bash's shape: `script: line N: NAME: text`.
    fn expansion_message(&self, name: &str, text: &str) -> String {
        format!("{}: line {}: {name}: {text}", self.script_name, self.line)
    }

    /// The exit for a command whose expansion failed (see [`Shell::note_unbound`]):
    /// the message goes to stderr, and the script ends with status 1 (127 in a `-c` string).
    fn unbound_refusal(&mut self, sink: &mut Sink) -> Option<Flow> {
        let msg = self.unbound.take()?;
        sink.expansion_eline(&msg);
        // bash 5.2 ends a `-c` string with 127 for a failed `${x:?}` or an
        // unbound variable, and a script file (or stdin) with 1; a failed
        // arithmetic expansion is 1 in both.
        // Inside a command substitution, and under `set -e`, it is 1 in a `-c`
        // string too.
        let status = if std::mem::take(&mut self.arith_failed) {
            self.discarded = true;
            1
        } else if self.subst_depth > 0 || self.errexit {
            1
        } else if self.script_name == SHELL_NAME {
            127
        } else {
            1
        };
        self.last = status;
        Some(Flow::Exit(status))
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

        let items = self.inline_operands(w);
        for (idx, (part, sep)) in items.iter().enumerate() {
            // A blank between the words of an unquoted operand ends a field.
            if *sep {
                if have {
                    fields.push((std::mem::take(&mut cur), std::mem::take(&mut mask)));
                    have = false;
                }
                continue;
            }
            match part {
                Part::Lit(s, quoted) => {
                    let s = if idx == 0 && tilde {
                        let home = self.tilde_home(s[1..].starts_with('/'));
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
                // `"$*"` is one field, the arguments joined by the first
                // character of IFS.
                Part::Var(name, None, true) if name == "*" => {
                    let joined = self.positional.join(&self.star_sep());
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
                Part::Arith(expr, quoted) => {
                    let value = self.arith_expand(expr, stdin);
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

    /// `w` with each unquoted `${x:-w}` and `${x:+w}` whose operand
    /// applies replaced by the operand's own parts, so that quotes inside it
    /// keep their text whole (`${y:-"a  b"}` is one word with two spaces).
    /// Each part comes with whether it is a run of blanks between the words of
    /// an operand, which ends a field. An operand that does not apply is left
    /// to [`Shell::param`].
    fn inline_operands(&mut self, w: &Word) -> Vec<(Part, bool)> {
        let mut out = Vec::with_capacity(w.len());
        for part in w {
            if let Part::Var(name, Some((op, word)), false) = part {
                let kind = op.trim_start_matches(':');
                // (`${x:=w}` is left to `param`: bash splits its result.)
                if matches!(kind, "-" | "+") && op != ":" {
                    let value = self.var(name);
                    let null = match &value {
                        None => true,
                        Some(v) => op.starts_with(':') && v.is_empty(),
                    };
                    if null != (kind == "+") {
                        out.extend(self.inline_operands(word));
                        continue;
                    }
                }
            }
            let sep = matches!(part, Part::Lit(s, false)
                if !s.is_empty() && s.chars().all(|c| c == ' ' || c == '\t'));
            out.push((part.clone(), sep));
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
        let ifs = self
            .vars
            .get("IFS")
            .cloned()
            .unwrap_or_else(|| " \t\n".to_string());
        let (pieces, starts_ws, ends_ws) = ifs_split(value, &ifs);
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
        _quoted: bool,
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
        if op.as_str() == ":" {
            return self.substring(name, value.unwrap_or_default(), word, stdin);
        }
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
                let pat = self.pattern(word, false, stdin);
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

    /// `${name:offset}` and `${name:offset:length}`: the part of `value` from
    /// `offset`, `length` long, counted in characters (bash in a UTF-8 locale;
    /// in the C locale it counts bytes). A negative offset counts from the end,
    /// and a negative length stops that many characters before the end. An
    /// offset outside the value gives nothing; a negative length that ends
    /// before the offset is bash's "substring expression < 0" error. Offset and
    /// length are sums of integers and variable names, as bash reads them.
    fn substring(&mut self, name: &str, value: String, word: &Word, stdin: &mut Input) -> String {
        let split = word
            .iter()
            .position(|p| matches!(p, Part::Lit(s, _) if s == SUBSTRING_SPLIT));
        let (offset_word, length_word) = match split {
            Some(k) => (word[..k].to_vec(), Some(word[k + 1..].to_vec())),
            None => (word.clone(), None),
        };
        let offset_text = self.expand_assign(&offset_word, stdin);
        let length_text = length_word.map(|w| self.expand_assign(&w, stdin));
        let Some(offset) = self.arith_eval(&offset_text) else {
            self.arith_error(name, &offset_text);
            return String::new();
        };
        let length = match &length_text {
            None => None,
            Some(t) => match self.arith_eval(t) {
                Some(n) => Some(n),
                None => {
                    self.arith_error(name, t);
                    return String::new();
                }
            },
        };
        let chars: Vec<char> = value.chars().collect();
        let n = chars.len() as i64;
        let start = if offset < 0 {
            n.saturating_add(offset)
        } else {
            offset
        };
        if !(0..=n).contains(&start) {
            return String::new();
        }
        let end = match length {
            None => n,
            Some(l) if l >= 0 => start.saturating_add(l).min(n),
            Some(l) => n.saturating_add(l),
        };
        if end < start {
            if self.unbound.is_none() {
                let shown = length_text.unwrap_or_default();
                self.unbound =
                    Some(self.expansion_message(shown.trim(), "substring expression < 0"));
            }
            return String::new();
        }
        chars[start as usize..end as usize].iter().collect()
    }

    /// Note an offset or length that is not arithmetic this shell reads.
    fn arith_error(&mut self, name: &str, text: &str) {
        if self.unbound.is_none() {
            self.unbound = Some(self.expansion_message(
                &format!("{name}: {}", text.trim()),
                "syntax error: operand expected",
            ));
        }
    }

    /// The value of `text` as the integer expression of a substring offset: a
    /// sum or difference of integers (decimal, `0x` hex, `0` octal), variable
    /// names (an unset one is 0) and parentheses, with unary `+` and `-`.
    fn arith_eval(&self, text: &str) -> Option<i64> {
        let c: Vec<char> = text.chars().collect();
        let mut i = 0;
        let v = self.arith_sum(&c, &mut i)?;
        skip_blanks(&c, &mut i);
        (i == c.len()).then_some(v)
    }

    fn arith_sum(&self, c: &[char], i: &mut usize) -> Option<i64> {
        let mut v = self.arith_term(c, i)?;
        loop {
            skip_blanks(c, i);
            match c.get(*i) {
                Some('+') => {
                    *i += 1;
                    v = v.checked_add(self.arith_term(c, i)?)?;
                }
                Some('-') => {
                    *i += 1;
                    v = v.checked_sub(self.arith_term(c, i)?)?;
                }
                _ => return Some(v),
            }
        }
    }

    fn arith_term(&self, c: &[char], i: &mut usize) -> Option<i64> {
        skip_blanks(c, i);
        match *c.get(*i)? {
            '-' => {
                *i += 1;
                self.arith_term(c, i)?.checked_neg()
            }
            '+' => {
                *i += 1;
                self.arith_term(c, i)
            }
            '(' => {
                *i += 1;
                let v = self.arith_sum(c, i)?;
                skip_blanks(c, i);
                if c.get(*i) == Some(&')') {
                    *i += 1;
                    Some(v)
                } else {
                    None
                }
            }
            d if d.is_ascii_digit() => {
                let from = *i;
                while c.get(*i).is_some_and(|x| x.is_ascii_alphanumeric()) {
                    *i += 1;
                }
                let token: String = c[from..*i].iter().collect();
                parse_c_integer(&token)
            }
            n if is_name_start(n) => {
                let from = *i;
                while c.get(*i).is_some_and(|x| is_name_char(*x)) {
                    *i += 1;
                }
                let name: String = c[from..*i].iter().collect();
                let value = self.var(&name).unwrap_or_default();
                let value = value.trim();
                if value.is_empty() {
                    Some(0)
                } else {
                    Some(parse_c_integer(value).unwrap_or(0))
                }
            }
            _ => None,
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
                Part::Var(_, _, part_quoted)
                | Part::Cmd(_, part_quoted)
                | Part::Arith(_, part_quoted) => {
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

    /// `$(< word)`: the file the word names, less trailing newlines. A missing
    /// or refused file is bash's message on standard error, status 1 and no
    /// text.
    fn substitute_file(&mut self, w: &Word) -> String {
        let mut input = Input::buf(Vec::new());
        let fields = self.expand(w, &mut input);
        let fail = |this: &mut Self, msg: String| {
            let _ = writeln!(std::io::stderr(), "{msg}");
            this.last = 1;
            String::new()
        };
        let [path] = fields.as_slice() else {
            return fail(self, format!("{SHELL_NAME}: ambiguous redirect"));
        };
        if let Some(refusal) = jail_refusal(SHELL_NAME, path) {
            return fail(self, refusal);
        }
        match read_file(path) {
            Ok(d) => {
                self.last = 0;
                let mut s = String::from_utf8_lossy(&d).into_owned();
                while s.ends_with('\n') {
                    s.pop();
                }
                s
            }
            Err(e) => fail(self, format!("{SHELL_NAME}: {path}: {}", io_reason(&e))),
        }
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
        // `$(< file)`: a body that is only a redirection from a file is the
        // file's contents, as bash reads it without running anything.
        if let Some(Redir::In(w)) = bare_input_redirect(&list) {
            return self.substitute_file(&w.clone());
        }
        let mut sink = Sink::new(ErrMode::Real);
        let mut input = Input::buf(Vec::new());
        // Bash does not pass `set -e` into a command substitution (it clears
        // it unless `inherit_errexit` is set), so a failing command in one does
        // not end the script. `set -u` is inherited. An unbound parameter the
        // substitution's own commands meet is theirs; the caller's is kept.
        // A command substitution is a subshell: what it assigns, exports,
        // unsets, `cd`s into or `shift`s is gone when it ends.
        let saved = self.subshell_state();
        let errexit = std::mem::replace(&mut self.errexit, false);
        let unbound = self.unbound.take();
        let arith_failed = std::mem::take(&mut self.arith_failed);
        // The body's lines count from its own start, so the caller's line is
        // restored: its later messages name its own line.
        let line = self.line;
        self.subst_depth += 1;
        let outer_external = EXTERNAL_RAN.with(|f| f.replace(false));
        let flow = self.run_list(&list, &mut input, &mut sink);
        let ran_external = EXTERNAL_RAN.with(|f| f.replace(outer_external));
        EXTERNAL_RAN.with(|f| f.set(outer_external || ran_external));
        self.subst_depth -= 1;
        self.restore_subshell(saved);
        self.errexit = errexit;
        self.unbound = unbound;
        self.arith_failed = arith_failed;
        // A discard inside the substitution ends only the substitution.
        self.discarded = false;
        self.line = line;
        self.last = match flow {
            Flow::Next(c) | Flow::Exit(c) => c,
            _ => 0,
        };
        let mut s = String::from_utf8_lossy(&sink.out).into_owned();
        // Windows programs (`jq`, `tar`, `git`) end their lines with CRLF, and
        // a script written for Linux compares `$(jq -r .a)` with `b`, not with
        // `b\r`. When a program ran inside the substitution, its CRLF line
        // ends are read as LF. Bash on Windows does not do this; the in-process
        // utilities, which write LF, are not touched.
        if cfg!(windows) && ran_external && s.contains("\r\n") {
            s = s.replace("\r\n", "\n");
        }
        while s.ends_with('\n') {
            s.pop();
        }
        s
    }

    /// What `~` stands for. On Windows the home directory has backslashes, so
    /// before a `/` they become `/`, and `~/x` is one uniform path.
    fn tilde_home(&self, before_slash: bool) -> String {
        let home = self.var("HOME").unwrap_or_default();
        if cfg!(windows) && before_slash {
            home.replace('\\', "/")
        } else {
            home
        }
    }

    /// An unquoted literal of an assignment value with its tildes expanded: a
    /// leading `~` (when `lead`) and a `~` after a `:` or `=`, each ending at a
    /// `/`, a `:` or the end, as bash expands them in `x=a:~/q`.
    fn assign_tildes(&self, s: &str, lead: bool) -> String {
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let starts =
                c == '~' && ((i == 0 && lead) || (i > 0 && matches!(chars[i - 1], ':' | '=')));
            let next = chars.get(i + 1);
            if starts && next.is_none_or(|n| matches!(n, '/' | ':')) {
                out.push_str(&self.tilde_home(next == Some(&'/')));
            } else {
                out.push(c);
            }
        }
        out
    }

    /// The value of an assignment (`x=word`). Bash does no field splitting,
    /// globbing or trimming here: the expansions are joined as text. A command
    /// substitution loses only its trailing newlines (see [`Shell::substitute`]).
    fn expand_assign(&mut self, w: &Word, stdin: &mut Input) -> String {
        let tilde = matches!(w.first(), Some(Part::Lit(s, false))
            if s == "~" || s.starts_with("~/") || s.starts_with("~:"));
        let mut out = String::new();
        for (idx, part) in w.iter().enumerate() {
            match part {
                Part::Lit(s, quoted) => {
                    if *quoted {
                        out.push_str(s);
                    } else {
                        out.push_str(&self.assign_tildes(s, idx == 0 && tilde));
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
                    let sep = if name == "*" {
                        self.star_sep()
                    } else {
                        " ".to_string()
                    };
                    out.push_str(&self.positional.join(&sep));
                }
                Part::Var(name, op, quoted) => {
                    let value = self.param(name, op, *quoted, stdin);
                    out.push_str(&value);
                }
                Part::Cmd(body, _) => {
                    let value = self.substitute(body);
                    out.push_str(&value);
                }
                Part::Arith(expr, _) => {
                    let value = self.arith_expand(expr, stdin);
                    out.push_str(&value);
                }
            }
        }
        out
    }

    /// `$((expr))`: the expression's `$` expansions are made first, then the
    /// text is evaluated as 64-bit integer arithmetic (see [`ArithEval`]). An
    /// error fails the command with status 1 and, as in bash, discards the rest
    /// of its line; the script goes on with the next line (see
    /// [`Shell::unbound_refusal`]).
    fn arith_expand(&mut self, expr: &Word, stdin: &mut Input) -> String {
        let text = self.expand_assign(expr, stdin);
        match arith_eval(self, &text) {
            Ok(n) => n.to_string(),
            Err(why) => {
                if self.unbound.is_none() {
                    // The message names the expression; a huge one is cut.
                    let shown: String = text.trim().chars().take(200).collect();
                    self.unbound = Some(self.expansion_message(&shown, &why));
                    self.arith_failed = true;
                }
                String::new()
            }
        }
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

    /// What a subshell cannot change in its parent.
    fn subshell_state(&self) -> SubshellState {
        SubshellState {
            vars: self.vars.clone(),
            exported: self.exported.clone(),
            unset_env: self.unset_env.clone(),
            positional: self.positional.clone(),
            options: (self.errexit, self.nounset, self.pipefail),
            cwd: std::env::current_dir().ok(),
        }
    }

    fn restore_subshell(&mut self, s: SubshellState) {
        self.vars = s.vars;
        self.exported = s.exported;
        self.unset_env = s.unset_env;
        self.positional = s.positional;
        (self.errexit, self.nounset, self.pipefail) = s.options;
        if let Some(dir) = s.cwd {
            if std::env::current_dir().ok().as_ref() != Some(&dir) {
                let _ = std::env::set_current_dir(dir);
            }
        }
    }

    fn run_pipeline(&mut self, p: &Pipeline, stdin: &mut Input, sink: &mut Sink) -> Flow {
        self.line = p.line;
        // A `!` pipeline is exempt from errexit, and so is everything in it.
        if p.negate {
            self.errexit_off += 1;
        }
        // A source that never ends (`yes`, `/dev/zero`, `/dev/urandom`) gives
        // a prefix; see `endless_plan`. The flags are the caller's again when
        // this pipeline is done.
        let plan = endless_plan(p);
        let saved_flags = (
            ENDLESS_BYTES.with(|b| b.replace(ENDLESS_FIRST)),
            ENDLESS_USED.with(|f| f.replace(false)),
            ENDLESS_ARMED.with(|f| f.replace(plan.is_some())),
            LIMIT_HIT.with(|f| f.replace(false)),
        );
        let saved_written = ENDLESS_WRITTEN.with(|w| std::mem::take(&mut *w.borrow_mut()));
        let outer_refused = SOURCE_REFUSED.with(|f| f.replace(false));
        let (out_mark, err_mark) = (sink.out.len(), sink.err.len());
        let mut cap = ENDLESS_FIRST;
        let flow = if p.cmds.len() == 1 {
            loop {
                let f = self.run_cmd(&p.cmds[0], stdin, sink);
                if plan.is_some() && endless_retry(&mut cap) {
                    sink.out.truncate(out_mark);
                    sink.err.truncate(err_mark);
                    continue;
                }
                break f;
            }
        } else {
            let mut data: Option<Vec<u8>> = None;
            let mut statuses: Vec<i32> = Vec::new();
            let n = p.cmds.len();
            let mut k = 0;
            while k < n {
                let cmd = &p.cmds[k];
                let mut local_in = match data.take() {
                    Some(d) => Input::buf(d),
                    None => Input::buf(Vec::new()),
                };
                let input: &mut Input = if k == 0 { &mut *stdin } else { &mut local_in };
                let mut stage = Sink::new(sink.err_mode);
                // Each stage of a pipeline is a subshell in bash: `exit` and
                // `break` end the stage, not the script, and a variable it
                // sets, an option it changes or a directory it enters is gone
                // when it ends (`echo x | read v` leaves `v` as it was).
                let saved = self.subshell_state();
                // A failed `${x:?}` in the last stage, when it is a plain
                // command, ends with the status the main shell gives it
                // (127 in a `-c` string); in any other stage it is 1.
                let bump = u32::from(!(k + 1 == n && matches!(cmd, Cmd::Simple { .. })));
                self.subst_depth += bump;
                // A program that feeds `head` is stopped once `head` has what
                // it wants, as SIGPIPE stops it in bash (`yes | head -3`).
                if k + 1 < n && matches!(cmd, Cmd::Simple { .. }) {
                    STREAM_LIMIT.with(|l| l.set(head_limit(&p.cmds[k + 1])));
                }
                EXTERNAL_PROGRAMS.with(|p| p.borrow_mut().clear());
                let c = match self.run_cmd(cmd, input, &mut stage) {
                    Flow::Next(c) | Flow::Exit(c) => c,
                    _ => 0,
                };
                STREAM_LIMIT.with(|l| l.set(None));
                self.subst_depth -= bump;
                self.restore_subshell(saved);
                // A `--timeout` that killed a stage stops the later ones too.
                if RUN_TIMEOUT_STATUS.load(std::sync::atomic::Ordering::SeqCst) != 0 {
                    sink.out.extend_from_slice(&stage.out);
                    if stage.err_mode == ErrMode::Capture {
                        sink.err.extend_from_slice(&stage.err);
                    }
                    break;
                }
                statuses.push(c);
                if stage.err_mode == ErrMode::Capture {
                    sink.err.extend_from_slice(&stage.err);
                }
                // The reader that stops an endless source got too little of
                // it: go again from the start with more.
                if plan == Some(k) && endless_retry(&mut cap) {
                    sink.err.truncate(err_mark);
                    statuses.clear();
                    data = None;
                    k = 0;
                    continue;
                }
                if k + 1 == n {
                    sink.out.extend_from_slice(&stage.out);
                } else {
                    data = Some(pipe_text(stage.out));
                }
                k += 1;
            }
            // The stages before the reader that stopped an endless source
            // were still writing: SIGPIPE ends them, status 141.
            if let Some(limiter) = plan {
                if ENDLESS_USED.with(|f| f.get()) && LIMIT_HIT.with(|f| f.get()) {
                    for c in statuses.iter_mut().take(limiter) {
                        *c = 141;
                    }
                }
            }
            // Under `pipefail` the status is the last failing stage's, or 0.
            let mut status = 0;
            for c in &statuses {
                if !self.pipefail || *c != 0 {
                    status = *c;
                }
            }
            Flow::Next(status)
        };
        // An endless source nothing stopped is not run to the end: a pipeline
        // that does not end is refused, with no output.
        let mine = ENDLESS_WRITTEN.with(|w| std::mem::take(&mut *w.borrow_mut()));
        ENDLESS_WRITTEN.with(|w| *w.borrow_mut() = saved_written);
        let mut flow =
            if ENDLESS_USED.with(|f| f.get()) && (plan.is_none() || !LIMIT_HIT.with(|f| f.get())) {
                sink.out.truncate(out_mark);
                sink.err.truncate(err_mark);
                undo_endless_writes(mine);
                endless_refusal(sink);
                Flow::Next(1)
            } else {
                flow
            };
        // A source that refused (`seq` past its cap) fails the pipeline, and
        // its reader's count of nothing is not shown.
        if SOURCE_REFUSED.with(|f| f.replace(outer_refused)) && p.cmds.len() > 1 {
            sink.out.truncate(out_mark);
            flow = Flow::Next(1);
        }
        ENDLESS_BYTES.with(|b| b.set(saved_flags.0));
        ENDLESS_USED.with(|f| f.set(saved_flags.1));
        ENDLESS_ARMED.with(|f| f.set(saved_flags.2));
        LIMIT_HIT.with(|f| f.set(saved_flags.3));
        if p.negate {
            self.errexit_off -= 1;
        }
        // The run's deadline passed while a program ran: the script ends with
        // the status of that refusal (E_BUDGET_EXCEEDED, 69), whichever stage
        // was killed and whatever the last stage's own status was.
        let timed_out = RUN_TIMEOUT_STATUS.load(std::sync::atomic::Ordering::SeqCst);
        if timed_out != 0 {
            return Flow::Exit(timed_out);
        }
        match flow {
            Flow::Next(c) => {
                let c = if p.negate { i32::from(c == 0) } else { c };
                self.last = c;
                // errexit: a failing pipeline ends the script, unless exempt.
                // A compound command (loop, group, `if`) that is a whole
                // pipeline of its own does not end the script by its status:
                // a failure inside it that mattered already did, and one that
                // did not (the last `&&` list or `!` pipeline of its body)
                // is exempt, as in bash. As the last stage of a longer
                // pipeline it is a subshell, and its status counts.
                let compound_alone = p.cmds.len() == 1 && !matches!(p.cmds[0], Cmd::Simple { .. });
                if c != 0 && !p.negate && self.errexit && self.errexit_off == 0 && !compound_alone {
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
        self.arith_failed = false;
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
        // Resolve redirections into a local input and sink, left to right as
        // the shell applies them: each descriptor points at a destination, and
        // `2>&1` copies where descriptor 1 points at that moment.
        let mut local_in: Option<Input> = None;
        // The files this command's redirections read and write, as resolved
        // paths: an external command inside binds them into its approval.
        let mut bound: Vec<String> = Vec::new();
        let mut d1 = Dest::Out;
        let mut d2 = Dest::Err;
        for r in redirs {
            match r {
                Redir::In(w) => {
                    let path = match self.redirect_target(w, stdin, sink) {
                        Ok(p) => p,
                        Err(flow) => return flow,
                    };
                    if matches!(path.as_str(), "/dev/stdin" | "/dev/fd/0") {
                        // Bash's own: standard input as it is.
                        local_in = None;
                        continue;
                    }
                    if path == "/dev/tty" {
                        if !has_terminal() {
                            sink.eline(&format!(
                                "{SHELL_NAME}: /dev/tty: No such device or address"
                            ));
                            self.last = 1;
                            return Flow::Next(1);
                        }
                        local_in = Some(Input::real());
                        continue;
                    }
                    if let Some(refusal) = jail_refusal(SHELL_NAME, &path) {
                        sink.eline(&refusal);
                        self.last = 1;
                        return Flow::Next(1);
                    }
                    bound.push(format!("<{}", resolved_for_binding(&path)));
                    match read_file(&path) {
                        Ok(d) => {
                            let regular = std::fs::metadata(&path).is_ok_and(|m| m.is_file());
                            local_in = Some(if regular {
                                Input::file(d)
                            } else {
                                Input::buf(d)
                            })
                        }
                        Err(e) => {
                            sink.eline(&format!("{SHELL_NAME}: {path}: {}", io_reason(&e)));
                            self.last = 1;
                            return Flow::Next(1);
                        }
                    }
                }
                Redir::Str(w) => {
                    // As a double-quoted word: no field splitting and no globbing.
                    let mut text = self.expand_assign(w, stdin);
                    text.push('\n');
                    local_in = Some(Input::buf(text.into_bytes()));
                }
                Redir::Out { fd, append, target } => {
                    let path = match self.redirect_target(target, stdin, sink) {
                        Ok(p) => p,
                        Err(flow) => return flow,
                    };
                    let dest = if path == "/dev/null" {
                        Dest::Null
                    } else if let Some(d) = self.device_dest(&path, sink) {
                        match d {
                            Ok(d) => d,
                            Err(flow) => return flow,
                        }
                    } else {
                        match self.refuse_write(sink, &path) {
                            Ok(p) => Dest::File(p, *append),
                            Err(flow) => return flow,
                        }
                    };
                    if let Dest::File(p, _) = &dest {
                        let fd = if *fd == 2 { "2" } else { "" };
                        let op = if *append { ">>" } else { ">" };
                        bound.push(format!("{fd}{op}{}", resolved_for_binding(p)));
                    }
                    if *fd == 2 {
                        if let Err(msg) = open_replaced(&d2, &[&d1, &dest]) {
                            sink.eline(&msg);
                            self.last = 1;
                            return Flow::Next(1);
                        }
                        d2 = dest;
                    } else {
                        if let Err(msg) = open_replaced(&d1, &[&d2, &dest]) {
                            sink.eline(&msg);
                            self.last = 1;
                            return Flow::Next(1);
                        }
                        d1 = dest;
                    }
                }
                Redir::Both(target) => {
                    let path = match self.redirect_target(target, stdin, sink) {
                        Ok(p) => p,
                        Err(flow) => return flow,
                    };
                    let dest = if path == "/dev/null" {
                        Dest::Null
                    } else if let Some(d) = self.device_dest(&path, sink) {
                        match d {
                            Ok(d) => d,
                            Err(flow) => return flow,
                        }
                    } else {
                        match self.refuse_write(sink, &path) {
                            Ok(p) => Dest::File(p, false),
                            Err(flow) => return flow,
                        }
                    };
                    if let Dest::File(p, _) = &dest {
                        bound.push(format!("&>{}", resolved_for_binding(p)));
                    }
                    let opened = open_replaced(&d1, &[&d2, &dest])
                        .and_then(|()| open_replaced(&d2, &[&d1, &dest]));
                    if let Err(msg) = opened {
                        sink.eline(&msg);
                        self.last = 1;
                        return Flow::Next(1);
                    }
                    d1 = dest.clone();
                    d2 = dest;
                }
                Redir::Dup { from: 2, to: 1 } => {
                    if let Err(msg) = open_replaced(&d2, &[&d1]) {
                        sink.eline(&msg);
                        self.last = 1;
                        return Flow::Next(1);
                    }
                    d2 = d1.clone();
                }
                Redir::Dup { from: 1, to: 2 } => {
                    if let Err(msg) = open_replaced(&d1, &[&d2]) {
                        sink.eline(&msg);
                        self.last = 1;
                        return Flow::Next(1);
                    }
                    d1 = d2.clone();
                }
                Redir::Dup { .. } => {}
            }
        }
        if let Some(flow) = self.unbound_refusal(sink) {
            return flow;
        }
        if let Some(flow) = self.glob_refusal(sink) {
            return flow;
        }
        // Both descriptors going to one place keep their order (merged); else
        // standard error is captured to be sent on its own.
        let err_mode = if d1 == d2 {
            ErrMode::Merge
        } else if d2 == Dest::Null {
            ErrMode::Null
        } else if d2 == Dest::Err && sink.err_mode == ErrMode::Real {
            ErrMode::Real
        } else {
            ErrMode::Capture
        };
        let mut local = Sink::new(err_mode);
        local.before_redirs = matches!(cmd, Cmd::Simple { .. });
        let mark = EXEC_REDIRS.with(|r| {
            let mut r = r.borrow_mut();
            let mark = r.len();
            r.extend(bound);
            mark
        });
        let flow = match local_in.as_mut() {
            Some(i) => self.run_cmd_inner(cmd, i, &mut local),
            None => self.run_cmd_inner(cmd, stdin, &mut local),
        };
        EXEC_REDIRS.with(|r| r.borrow_mut().truncate(mark));
        if !local.parent_err.is_empty() {
            let early = std::mem::take(&mut local.parent_err);
            sink.ewrite(&early);
        }
        if err_mode == ErrMode::Capture {
            let err = std::mem::take(&mut local.err);
            if let Err(msg) = d2.write(&err, sink) {
                sink.eline(&msg);
            }
        }
        if let Err(msg) = d1.write(&local.out, sink) {
            sink.eline(&msg);
            self.last = 1;
            return Flow::Next(1);
        }
        flow
    }

    /// Where a write to one of the device names bash itself provides goes:
    /// `/dev/stdout` and `/dev/fd/1` are this command's standard output, `/dev/stderr`
    /// and `/dev/fd/2` its standard error, and `/dev/tty` the terminal (standard
    /// error here), which with none is bash's "No such device or address".
    /// `None` for any other name.
    fn device_dest(&mut self, path: &str, sink: &mut Sink) -> Option<Result<Dest, Flow>> {
        match path {
            "/dev/stdout" | "/dev/fd/1" => Some(Ok(Dest::Out)),
            "/dev/stderr" | "/dev/fd/2" => Some(Ok(Dest::Err)),
            "/dev/tty" => {
                if has_terminal() {
                    Some(Ok(Dest::Err))
                } else {
                    sink.eline(&format!(
                        "{SHELL_NAME}: /dev/tty: No such device or address"
                    ));
                    self.last = 1;
                    Some(Err(Flow::Next(1)))
                }
            }
            _ => None,
        }
    }

    /// The one file name a redirection's word expands to. A word that gives
    /// none or several is bash's `ambiguous redirect`: the command does not run
    /// and the status is 1.
    fn redirect_target(
        &mut self,
        w: &Word,
        stdin: &mut Input,
        sink: &mut Sink,
    ) -> Result<String, Flow> {
        let mut fields = self.expand(w, stdin);
        // A failed expansion in a redirection's word fails that command, as in
        // bash, and the script goes on (the arithmetic error aside).
        if !self.arith_failed {
            if let Some(msg) = self.unbound.take() {
                sink.eline(&msg);
                self.last = 1;
                return Err(Flow::Next(1));
            }
        }
        if let Some(flow) = self.unbound_refusal(sink) {
            return Err(flow);
        }
        if fields.len() != 1 {
            let msg = self.expansion_message(&word_source(w), "ambiguous redirect");
            sink.eline(&msg);
            self.last = 1;
            return Err(Flow::Next(1));
        }
        Ok(fields.remove(0))
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
                        // `break` and `continue` are commands that succeed, and
                        // the loop's status is that of the last command it ran.
                        Flow::Break(n) => {
                            if n > 1 {
                                return Flow::Break(n - 1);
                            }
                            status = 0;
                            break;
                        }
                        Flow::Continue(n) => {
                            if n > 1 {
                                return Flow::Continue(n - 1);
                            }
                            status = 0;
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
                        // `break` and `continue` are commands that succeed, and
                        // the loop's status is that of the last command it ran.
                        Flow::Break(n) => {
                            if n > 1 {
                                return Flow::Break(n - 1);
                            }
                            status = 0;
                            break;
                        }
                        Flow::Continue(n) => {
                            if n > 1 {
                                return Flow::Continue(n - 1);
                            }
                            status = 0;
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
            // `export NAME=$value` is an assignment: bash does not split or
            // glob what the value expands to.
            if argv.first().map(String::as_str) == Some("export") {
                if let Some(Part::Lit(s, _)) = w.first() {
                    if let Some((n, rest)) = s.split_once('=') {
                        let n = n.strip_suffix('+').unwrap_or(n);
                        if valid_name(n) {
                            let eq = s.len() - rest.len() - 1;
                            let mut tail = w.clone();
                            tail[0] = Part::Lit(s[eq + 1..].to_string(), true);
                            let v = self.expand_assign(&tail, stdin);
                            argv.push(format!("{}{v}", &s[..eq + 1]));
                            continue;
                        }
                    }
                }
            }
            argv.extend(self.expand(w, stdin));
        }
        let mut env_overrides = Vec::new();
        for (name, w) in assigns {
            let (name, append) = match name.strip_suffix('+') {
                Some(base) => (base.to_string(), true),
                None => (name.clone(), false),
            };
            let name = &name;
            // A prefix assignment on a command reaches that command's
            // environment; a bare assignment stays in the shell.
            if !argv.is_empty() && refuse_steering(sink, name) {
                return Flow::Next(POLICY_DENY_STATUS);
            }
            let mut v = self.expand_assign(w, stdin);
            if append {
                v = self.var(name).unwrap_or_default() + &v;
            }
            if argv.is_empty() {
                // A failed arithmetic expansion discards the assignment.
                if !self.arith_failed {
                    self.set_var(name, v);
                }
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
                // As bash reads it: a number (blanks and a sign allowed) taken
                // modulo 256; anything else ends the shell with status 2, and
                // a second operand is an error that does not end it.
                let Some(a) = args.first() else {
                    return Flow::Exit(self.last);
                };
                return match a.trim().parse::<i64>() {
                    Err(_) => {
                        sink.eline(&format!(
                            "{SHELL_NAME}: exit: {a}: numeric argument required"
                        ));
                        Flow::Exit(2)
                    }
                    Ok(_) if args.len() > 1 => {
                        sink.eline(&format!("{SHELL_NAME}: exit: too many arguments"));
                        Flow::Exit(1)
                    }
                    Ok(n) => Flow::Exit((n & 255) as i32),
                };
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
                let mut status = 0;
                for a in args {
                    let (n, v) = match a.split_once('=') {
                        Some((n, v)) => (n.to_string(), Some(v.to_string())),
                        None => (a.clone(), None),
                    };
                    // `export z+=2` appends to z (and sets it when it is unset).
                    let (n, appends) = match n.strip_suffix('+') {
                        Some(base) if v.is_some() && !base.is_empty() => (base.to_string(), true),
                        _ => (n, false),
                    };
                    // A name that is no identifier is bash's error, and sets nothing.
                    if !valid_name(&n) {
                        sink.eline(&format!(
                            "{SHELL_NAME}: export: `{a}': not a valid identifier"
                        ));
                        status = 1;
                        continue;
                    }
                    if refuse_steering(sink, &n) {
                        return Flow::Next(POLICY_DENY_STATUS);
                    }
                    if let Some(mut v) = v {
                        if appends {
                            v = self.var(&n).unwrap_or_default() + &v;
                        }
                        self.set_var(&n, v);
                    }
                    self.mark_exported(n);
                }
                return Flow::Next(status);
            }
            "unset" => {
                for a in args {
                    // Removing a variable the operator set (`HTTPS_PROXY`,
                    // `GIT_CONFIG_GLOBAL`) changes what children get, so an
                    // agent may no more unset a steering name than export one.
                    if refuse_steering(sink, a) {
                        return Flow::Next(POLICY_DENY_STATUS);
                    }
                    self.vars.remove(a);
                    self.exported.retain(|n| n != a);
                    // An inherited variable is removed from what children get.
                    if !a.is_empty() && !a.contains(['=', '\0']) && std::env::var_os(a).is_some() {
                        let folded = crate::safety::fold_env_name(a);
                        self.unset_env
                            .retain(|n| crate::safety::fold_env_name(n) != folded);
                        self.unset_env.push(a.clone());
                    }
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
                // Options: `-L`, `-P` (and `-e`, `-@`), then `--`. The shell
                // follows no links of its own, so -L and -P do the same here.
                let mut rest: &[String] = args;
                while let Some(a) = rest.first() {
                    if a == "--" {
                        rest = &rest[1..];
                        break;
                    }
                    if a.len() > 1 && a.starts_with('-') {
                        if let Some(bad) =
                            a[1..].chars().find(|c| !matches!(c, 'L' | 'P' | 'e' | '@'))
                        {
                            sink.eline(&format!("{SHELL_NAME}: cd: -{bad}: invalid option"));
                            sink.eline("cd: usage: cd [-L|[-P [-e]] [-@]] [dir]");
                            return Flow::Next(2);
                        }
                        rest = &rest[1..];
                        continue;
                    }
                    break;
                }
                if rest.len() > 1 {
                    sink.eline(&format!("{SHELL_NAME}: cd: too many arguments"));
                    return Flow::Next(1);
                }
                let target = match rest.first() {
                    Some(t) if t == "-" => match self.var("OLDPWD").filter(|d| !d.is_empty()) {
                        Some(d) => d,
                        None => {
                            sink.eline(&format!("{SHELL_NAME}: cd: OLDPWD not set"));
                            return Flow::Next(1);
                        }
                    },
                    Some(t) => t.clone(),
                    None => match self.var("HOME") {
                        Some(h) => h,
                        None => {
                            sink.eline(&format!("{SHELL_NAME}: cd: HOME not set"));
                            return Flow::Next(1);
                        }
                    },
                };
                // An empty directory name is the current directory, as in bash.
                let target = if target.is_empty() {
                    ".".to_string()
                } else {
                    target
                };
                // CDPATH: a relative name that does not start with `./` or
                // `../` is looked for under each directory of it, in order (an
                // empty entry is the current directory); when none holds it,
                // the name itself is tried. The new directory is printed when
                // it was found under a named entry. In agent mode CDPATH
                // steers the shell and is not read.
                let mut cdpath_print = false;
                let target = match (rest.first(), self.var("CDPATH")) {
                    (Some(t), Some(cdpath))
                        if t != "-"
                            && !t.is_empty()
                            && !cdpath.is_empty()
                            && crate::safety::current_mode() != crate::safety::Mode::Agent
                            && !matches!(t.as_str(), "." | "..")
                            && !t.starts_with(['/', '\\'])
                            && !t.starts_with("./")
                            && !t.starts_with("../")
                            && !(cfg!(windows) && t.as_bytes().get(1) == Some(&b':')) =>
                    {
                        let entries = split_path_entries(&cdpath, true);
                        let mut found = None;
                        for entry in entries {
                            let base = if entry.is_empty() {
                                "."
                            } else {
                                entry.as_str()
                            };
                            let cand = format!("{}/{}", base.trim_end_matches('/'), t);
                            if !crate::safety::names_a_remote_share(&cand)
                                && std::path::Path::new(&cand).is_dir()
                            {
                                cdpath_print = !entry.is_empty();
                                found = Some(cand);
                                break;
                            }
                        }
                        found.unwrap_or(target)
                    }
                    _ => target,
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
                            self.set_var("OLDPWD", posix_path(&o.to_string_lossy()));
                        }
                        if let Ok(d) = std::env::current_dir() {
                            self.set_var("PWD", posix_path(&d.to_string_lossy()));
                        }
                        // `cd -` says where it went, as bash does.
                        if cdpath_print || rest.first().map(|t| t == "-").unwrap_or(false) {
                            sink.out.extend_from_slice(logical_pwd().as_bytes());
                            sink.out.push(b'\n');
                        }
                        Flow::Next(0)
                    }
                    Err(e) => {
                        sink.eline(&format!("{SHELL_NAME}: cd: {target}: {}", io_reason(&e)));
                        Flow::Next(1)
                    }
                };
            }
            "test" | "[" if env.is_empty() => {
                let operands = if name == "[" {
                    match args.split_last() {
                        Some((last, rest)) if last == "]" => rest,
                        _ => {
                            sink.eline(&format!("{SHELL_NAME}: [: missing `]'"));
                            return Flow::Next(2);
                        }
                    }
                } else {
                    args
                };
                return Flow::Next(test(operands, sink, Some(&*self)));
            }
            "read" => return Flow::Next(self.read_builtin(args, env, stdin)),
            // `printf -v VAR` sets a variable, and bash reads options before the
            // format: `--` ends them and any other is an error.
            "printf"
                if args
                    .first()
                    .is_some_and(|a| a.len() > 1 && a.starts_with('-')) =>
            {
                return Flow::Next(self.printf_options(args, env, stdin, sink));
            }
            "command" if env.is_empty() && args.first().map(String::as_str) == Some("-v") => {
                let path = self.command_path(env);
                return Flow::Next(command_v(&args[1..], path.as_deref(), sink));
            }
            "timeout" => {
                if let Some(flow) = self.timeout_builtin(args, env, stdin, sink) {
                    return flow;
                }
            }
            "mktemp" if env.is_empty() => {
                if let Util::Done(c) = mktemp(args, self.var("TMPDIR"), sink) {
                    return Flow::Next(c);
                }
            }
            "pwd" => {
                // `-L` and `-P`, `--` ends the options; any other is an error.
                for a in args {
                    if a == "--" {
                        break;
                    }
                    if a.len() < 2 || !a.starts_with('-') {
                        break;
                    }
                    if let Some(bad) = a[1..].chars().find(|c| !matches!(c, 'L' | 'P')) {
                        sink.eline(&format!("{SHELL_NAME}: pwd: -{bad}: invalid option"));
                        sink.eline("pwd: usage: pwd [-LP]");
                        return Flow::Next(2);
                    }
                }
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
        // A bare name is looked up on the shell's PATH, the lookup that
        // `command -v` uses. Names the subset runs itself (and falls back to
        // GNU programs for) keep their own lookup.
        if !name.contains(['/', '\\']) && !SUBSET_NAMES.contains(&name) {
            let found = self.command_path(env).and_then(|path| find_in(name, &path));
            match found {
                Some(path) => {
                    let text = path.to_string_lossy().into_owned();
                    if cfg!(windows) && path.extension().is_none() && has_shell_shebang(&text) {
                        // A script found on PATH is a program, not the user's
                        // script. One outside the subset (Git's `gunzip` and
                        // `zcat` use `case`) runs with the real `sh` through the
                        // effect gate, as any other program does; with no `sh`
                        // on PATH it is refused, naming the script.
                        if let Some(flow) =
                            self.path_script_outside_subset(&text, argv, env, stdin, sink)
                        {
                            return flow;
                        }
                        return Flow::Next(self.run_script_child(
                            &text,
                            &argv[1..],
                            env,
                            stdin,
                            sink,
                        ));
                    }
                    // Only a PATH the script set changes which program runs; with
                    // the system's own PATH the usual resolution keeps the
                    // program's name as its argv[0] (the GNU tools of Git for
                    // Windows print it in their messages).
                    // A program that is not an `.exe` (`more.com`) is run by
                    // its path, as the system's own lookup would not find it.
                    let not_exe = cfg!(windows)
                        && path
                            .extension()
                            .is_some_and(|e| !e.eq_ignore_ascii_case("exe"));
                    if self.path_was_set(env) || not_exe {
                        RESOLVED_PROGRAM.with(|r| *r.borrow_mut() = Some(text));
                    }
                }
                None if self.path_was_set(env) => {
                    sink.eline(&format!("{SHELL_NAME}: {name}: command not found"));
                    return Flow::Next(127);
                }
                None => {}
            }
        }
        let flow = Flow::Next(self.external(argv, env, stdin, sink, why.as_deref()));
        RESOLVED_PROGRAM.with(|r| *r.borrow_mut() = None);
        flow
    }

    /// A shell script found on PATH whose text the subset does not run: `Some`
    /// with the result of running it with the real `sh` (through the effect
    /// gate, with the script named in the approval), or of refusing it when
    /// there is no `sh`. `None` for a script the subset runs, or whose text
    /// cannot be read here.
    fn path_script_outside_subset(
        &mut self,
        script: &str,
        argv: &[String],
        env: &[(String, String)],
        stdin: &mut Input,
        sink: &mut Sink,
    ) -> Option<Flow> {
        if jail_refusal("sh", script).is_some() {
            return None;
        }
        let bytes = std::fs::read(script).ok().filter(|b| b.len() <= 4 << 20)?;
        let src = String::from_utf8_lossy(&bytes);
        let why = refusal(&src).filter(|r| !r.syntax)?;
        let sh = self
            .command_path(env)
            .and_then(|path| find_in("sh", &path))
            .map(|p| p.to_string_lossy().into_owned());
        let Some(sh) = sh else {
            sink.eline(&format!(
                "{SHELL_NAME}: {script}: this shell script, found on PATH, uses {} at line {}, which is outside the in-process POSIX subset, and no `sh` was found on PATH to run it",
                why.construct, why.line
            ));
            return Some(Flow::Next(POLICY_DENY_STATUS));
        };
        let mut sh_argv = vec![sh, script.to_string()];
        sh_argv.extend(argv[1..].iter().cloned());
        let status = self.external(&sh_argv, env, stdin, sink, None);
        RESOLVED_PROGRAM.with(|r| *r.borrow_mut() = None);
        Some(Flow::Next(status))
    }

    /// `timeout [OPTION] DURATION COMMAND [ARG]...`: run the command and end it
    /// when the time is up, with status 124 (GNU's). Only a program is ended:
    /// an in-process tool runs to its end. `-k`, `-s` and `-v` are taken and
    /// have no effect; `--preserve-status` gives 143 for a command that was
    /// ended. `None` when an option is another one, which runs the program.
    fn timeout_builtin(
        &mut self,
        args: &[String],
        env: &[(String, String)],
        stdin: &mut Input,
        sink: &mut Sink,
    ) -> Option<Flow> {
        let mut i = 0;
        let mut preserve = false;
        // `-s KILL`: the command is ended by SIGKILL, and GNU timeout then
        // exits 128+9, not 124 (even with `--preserve-status`).
        let mut sigkill = false;
        let is_kill = |s: &str| matches!(s, "KILL" | "SIGKILL" | "9" | "kill" | "sigkill");
        while let Some(a) = args.get(i) {
            match a.as_str() {
                "--" => {
                    i += 1;
                    break;
                }
                "--foreground" | "-v" | "--verbose" => i += 1,
                "--preserve-status" => {
                    preserve = true;
                    i += 1;
                }
                "-s" => {
                    sigkill = args.get(i + 1).is_some_and(|s| is_kill(s));
                    i += 2;
                }
                "-k" => i += 2,
                o if o.starts_with("--signal=") => {
                    sigkill = is_kill(&o["--signal=".len()..]);
                    i += 1;
                }
                o if o.starts_with("-s") && o.len() > 2 => {
                    sigkill = is_kill(&o[2..]);
                    i += 1;
                }
                o if o.starts_with("--kill-after=") => i += 1,
                o if o.starts_with("-k") && o.len() > 2 => i += 1,
                o if o.len() > 1 && o.starts_with('-') => return None,
                _ => break,
            }
        }
        let usage = "Try 'timeout --help' for more information.";
        let Some(duration) = args.get(i) else {
            sink.eline(&format!("timeout: missing operand\n{usage}"));
            return Some(Flow::Next(125));
        };
        let (num, unit) = match duration.chars().last() {
            Some(c @ ('s' | 'm' | 'h' | 'd')) => (&duration[..duration.len() - 1], c),
            _ => (duration.as_str(), 's'),
        };
        let seconds = match num.parse::<f64>() {
            Ok(n) if n >= 0.0 && n.is_finite() && !num.starts_with('+') => {
                n * match unit {
                    'm' => 60.0,
                    'h' => 3600.0,
                    'd' => 86400.0,
                    _ => 1.0,
                }
            }
            _ => {
                sink.eline(&format!(
                    "timeout: invalid time interval '{duration}'\n{usage}"
                ));
                return Some(Flow::Next(125));
            }
        };
        let command = &args[i + 1..];
        if command.is_empty() {
            sink.eline(&format!("timeout: missing operand\n{usage}"));
            return Some(Flow::Next(125));
        }
        let outer = LOCAL_DEADLINE.with(|d| d.get());
        let mine = (seconds > 0.0).then(|| {
            std::time::Instant::now() + std::time::Duration::from_secs_f64(seconds.min(1e7))
        });
        let deadline = match (outer, mine) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        LOCAL_DEADLINE.with(|d| d.set(deadline));
        let timed_before = LOCAL_TIMED_OUT.with(|f| f.replace(false));
        let flow = self.dispatch(command, env, stdin, sink);
        let fired = LOCAL_TIMED_OUT.with(|f| f.replace(timed_before));
        LOCAL_DEADLINE.with(|d| d.set(outer));
        Some(match flow {
            Flow::Next(_) if fired && sigkill => Flow::Next(137),
            Flow::Next(_) if fired && preserve => Flow::Next(143),
            other => other,
        })
    }

    /// `printf` with options: `-v VAR` (or `-vVAR`) puts the output in VAR
    /// instead of writing it, `--` ends the options, and any other option is
    /// bash's `invalid option`, status 2, as is a missing format.
    fn printf_options(
        &mut self,
        args: &[String],
        env: &[(String, String)],
        stdin: &mut Input,
        sink: &mut Sink,
    ) -> i32 {
        let usage = "printf: usage: printf [-v var] format [arguments]";
        let mut var: Option<String> = None;
        let mut k = 0;
        while k < args.len() {
            let a = args[k].as_str();
            if a == "--" {
                k += 1;
                break;
            }
            if a.len() < 2 || !a.starts_with('-') {
                break;
            }
            if a.starts_with("-v") {
                let name = if a.len() > 2 {
                    a[2..].to_string()
                } else {
                    k += 1;
                    match args.get(k) {
                        Some(n) => n.clone(),
                        None => {
                            sink.eline(&format!(
                                "{SHELL_NAME}: printf: -v: option requires an argument"
                            ));
                            sink.eline(usage);
                            return 2;
                        }
                    }
                };
                var = Some(name);
                k += 1;
                continue;
            }
            let bad = a.chars().nth(1).unwrap_or('-');
            sink.eline(&format!("{SHELL_NAME}: printf: -{bad}: invalid option"));
            sink.eline(usage);
            return 2;
        }
        let rest = &args[k..];
        if rest.is_empty() {
            sink.eline(usage);
            return 2;
        }
        let Some(name) = var else {
            let mut why = None;
            if env.is_empty() {
                match utility("printf", rest, stdin, sink) {
                    Util::Done(c) => return c,
                    Util::Decline => {}
                    Util::DeclineFor(part) => why = Some(part),
                }
            }
            let argv: Vec<String> = std::iter::once("printf".to_string())
                .chain(rest.iter().cloned())
                .collect();
            return self.external(&argv, env, stdin, sink, why.as_deref());
        };
        if name.contains('[') {
            sink.eline(&format!(
                "{SHELL_NAME}: printf: -v {name}: an array element is not in the subset"
            ));
            return POLICY_DENY_STATUS;
        }
        if !valid_name(&name) {
            sink.eline(&format!(
                "{SHELL_NAME}: printf: `{name}': not a valid identifier"
            ));
            return 2;
        }
        if refuse_steering(sink, &name) {
            return POLICY_DENY_STATUS;
        }
        let mut cap = Sink::new(ErrMode::Capture);
        let done = utility("printf", rest, stdin, &mut cap);
        sink.ewrite(&cap.err);
        match done {
            Util::Done(c) => {
                self.set_var(&name, String::from_utf8_lossy(&cap.out).into_owned());
                c
            }
            _ => {
                sink.eline(&format!(
                    "{SHELL_NAME}: printf: this format is not in the subset with -v"
                ));
                2
            }
        }
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
        let mut prompt: Option<String> = None;
        // `-d`: the byte that ends the line (NUL for an empty DELIM); `-n`: the
        // most characters to take, a newline ending the line sooner.
        let mut delim = b'\n';
        let mut nchars: Option<usize> = None;
        let mut k = 0;
        while k < args.len() {
            let a = args[k].as_str();
            if a == "--" {
                k += 1;
                break;
            }
            if a.len() < 2 || !a.starts_with('-') {
                break;
            }
            k += 1;
            let flags: Vec<char> = a[1..].chars().collect();
            for (at, &c) in flags.iter().enumerate() {
                match c {
                    'r' => raw = true,
                    // Only a terminal changes for these two.
                    's' | 'e' => {}
                    'p' | 'd' | 'n' | 'u' => {
                        let rest: String = flags[at + 1..].iter().collect();
                        let value = if rest.is_empty() {
                            k += 1;
                            match args.get(k - 1) {
                                Some(v) => v.clone(),
                                None => {
                                    let _ = writeln!(
                                        std::io::stderr(),
                                        "{SHELL_NAME}: read: -{c}: option requires an argument"
                                    );
                                    return 2;
                                }
                            }
                        } else {
                            rest
                        };
                        match c {
                            'p' => prompt = Some(value),
                            'd' => delim = value.bytes().next().unwrap_or(0),
                            'n' => match value.parse::<usize>() {
                                Ok(n) => nchars = Some(n),
                                Err(_) => {
                                    let _ = writeln!(
                                        std::io::stderr(),
                                        "{SHELL_NAME}: read: {value}: invalid number"
                                    );
                                    return 1;
                                }
                            },
                            _ if value == "0" => {}
                            _ => {
                                let _ = writeln!(
                                    std::io::stderr(),
                                    "{SHELL_NAME}: read: -u {value}: unsupported option"
                                );
                                return 2;
                            }
                        }
                        break;
                    }
                    other => {
                        // Refused before the script ran; this is a guard only.
                        let _ = writeln!(
                            std::io::stderr(),
                            "{SHELL_NAME}: read: -{other}: unsupported option"
                        );
                        return 2;
                    }
                }
            }
        }
        names.extend(args[k.min(args.len())..].iter().cloned());
        // `-d` and `-n` are run only with `-r` (see `read_refusal`).
        if !raw && (delim != b'\n' || nchars.is_some()) {
            let _ = writeln!(
                std::io::stderr(),
                "{SHELL_NAME}: read: -d and -n need -r in this shell"
            );
            return 2;
        }
        // A name that is no identifier is an error before anything is read.
        if let Some(bad) = names.iter().find(|n| !valid_name(n)) {
            let _ = writeln!(
                std::io::stderr(),
                "{SHELL_NAME}: read: `{bad}': not a valid identifier"
            );
            return 1;
        }
        // With no name, the line goes to REPLY whole, without field splitting.
        let default_reply = names.is_empty();
        if default_reply {
            names.push("REPLY".into());
        }
        // As bash does, the prompt goes to stderr and only when the input is a
        // terminal, so a script reading a pipe prints nothing extra.
        if let Some(p) = &prompt {
            use std::io::IsTerminal;
            if !stdin.is_buffered() && std::io::stdin().is_terminal() {
                let mut err = std::io::stderr();
                let _ = err.write_all(p.as_bytes());
                let _ = err.flush();
            }
        }
        let first = match nchars {
            Some(n) => stdin.read_chars(n),
            None => stdin.read_record(delim),
        };
        let (mut line, mut terminated) = match first {
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
        // Without -r, a backslash before the newline continues the line: both
        // are removed and the next line is joined on. A doubled backslash is
        // an escaped backslash, so only an odd run continues.
        while !raw && terminated {
            let run = line.iter().rev().take_while(|&&b| b == b'\\').count();
            if run % 2 == 0 {
                break;
            }
            line.pop();
            match stdin.read_line() {
                Ok(Some((more, t))) => {
                    line.extend_from_slice(&more);
                    terminated = t;
                }
                _ => {
                    terminated = false;
                    break;
                }
            }
        }
        let line = String::from_utf8_lossy(&line).into_owned();
        // Without -r a backslash quotes the next character, and is removed:
        // a quoted character is never a separator. Each character carries
        // whether it was quoted.
        let mut chars: Vec<(char, bool)> = Vec::with_capacity(line.len());
        if raw {
            chars.extend(line.chars().map(|c| (c, false)));
        } else {
            let mut it = line.chars();
            while let Some(c) = it.next() {
                if c == '\\' {
                    if let Some(n) = it.next() {
                        chars.push((n, true));
                    }
                } else {
                    chars.push((c, false));
                }
            }
        }
        // As bash does, a last line with no newline is assigned but reads as
        // a failure, so `while read` does not run its body for it.
        let status = if terminated { 0 } else { 1 };
        if default_reply {
            self.set_var("REPLY", chars.iter().map(|&(c, _)| c).collect());
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
        for (n, v) in names.iter().zip(read_fields(&chars, &ifs, names.len())) {
            self.set_var(n, v);
        }
        status
    }

    /// Mark `name` exported, last in the order, so that where names differing
    /// only in case are one variable (Windows) the later export wins.
    fn mark_exported(&mut self, name: String) {
        let folded = crate::safety::fold_env_name(&name);
        self.unset_env
            .retain(|n| crate::safety::fold_env_name(n) != folded);
        self.exported.retain(|n| *n != name);
        self.exported.push(name);
    }

    /// The variables a child gets beyond this process's own, in the order they
    /// are applied: the exports (in export order), then the prefix assignments
    /// of the command.
    fn child_env_pairs(&self, env: &[(String, String)]) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = Vec::new();
        for n in &self.exported {
            if let Some(v) = self.vars.get(n) {
                pairs.push((n.clone(), v.clone()));
            }
        }
        pairs.extend(env.iter().cloned());
        pairs
    }

    /// The inherited variables a child must not see: those the script unset,
    /// less any a later export or prefix assignment sets again (`pairs`).
    fn removed_env(&self, pairs: &[(String, String)]) -> Vec<String> {
        let mut removed: Vec<String> = self
            .unset_env
            .iter()
            .filter(|n| {
                let folded = crate::safety::fold_env_name(n);
                !pairs
                    .iter()
                    .any(|(k, _)| crate::safety::fold_env_name(k) == folded)
            })
            .map(|n| crate::safety::fold_env_name(n))
            .collect();
        removed.sort();
        removed
    }

    /// Give `cmd` the environment of [`Shell::child_env_pairs`], without the
    /// variables the script unset.
    fn apply_child_env(&self, cmd: &mut std::process::Command, pairs: &[(String, String)]) {
        for n in &self.unset_env {
            cmd.env_remove(n);
        }
        for (n, v) in pairs {
            if cfg!(windows) && n.eq_ignore_ascii_case("PATH") {
                cmd.env(n, split_path_var(v).join(";"));
            } else {
                cmd.env(n, v);
            }
        }
    }

    /// The PATH a command is looked up on: the shell's own variable (an
    /// export or a plain assignment, or a prefix on this command), as bash
    /// does. In agent mode PATH steers the programs that run and is not the
    /// script's to change, so the process's own is used.
    fn command_path(&self, env: &[(String, String)]) -> Option<String> {
        if crate::safety::current_mode() == crate::safety::Mode::Agent {
            return std::env::var("PATH").ok();
        }
        if let Some((_, v)) = env
            .iter()
            .rev()
            .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
        {
            return Some(v.clone());
        }
        self.var("PATH")
    }

    /// Whether a script set PATH itself (so a name not found on it is not
    /// found, where otherwise the system's own lookup may still find it).
    fn path_was_set(&self, env: &[(String, String)]) -> bool {
        crate::safety::current_mode() != crate::safety::Mode::Agent
            && (self.vars.contains_key("PATH")
                || env.iter().any(|(k, _)| k.eq_ignore_ascii_case("PATH")))
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
        // What the child gets beyond this process's environment (exports and
        // prefix assignments) is part of what is approved.
        let pairs = self.child_env_pairs(env);
        // Later entries win, as they do on the command below, and names that
        // differ only in case are one variable where the system folds them.
        let mut deduped: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        for (n, v) in &pairs {
            deduped.insert(crate::safety::fold_env_name(n), v.clone());
        }
        let child_env: Vec<(String, String)> = deduped.into_iter().collect();
        let removed = self.removed_env(&child_env);
        let redirs = EXEC_REDIRS.with(|r| r.borrow().clone());
        if let Err(e) = crate::safety::guard_exec_env(
            "posix",
            argv.join(" "),
            argv,
            &child_env,
            &removed,
            &redirs,
        ) {
            sink.eline(&e.to_string());
            return e.code.exit_code();
        }
        if let Some(refusal) = self.steering_refusal(env) {
            sink.eline(&refusal);
            return POLICY_DENY_STATUS;
        }
        let found = RESOLVED_PROGRAM.with(|r| r.borrow_mut().take());
        let program = match found {
            Some(p) => p,
            None => match resolve_program(argv, env, why) {
                Ok(p) => p,
                Err((code, message)) => {
                    sink.eline(&message);
                    return code;
                }
            },
        };
        let mut cmd = crate::safety::child_command(&program);
        add_program_args(&mut cmd, &program, &argv[1..]);
        self.apply_child_env(&mut cmd, &pairs);
        crate::safety::strip_ironshell_env(&mut cmd);
        if cfg!(windows) {
            // Windows looks a bare name up in its own system directories
            // before PATH, so `expand` or `bash` would run System32's. The
            // child's PATH is searched first when the command has one of its
            // own, so it is given this process's: the program that runs is the
            // one `command -v` names.
            if !pairs.iter().any(|(k, _)| k.eq_ignore_ascii_case("PATH")) {
                if let Some(path) = std::env::var_os("PATH") {
                    cmd.env("PATH", path);
                }
            }
            // A program built on the MSYS or Cygwin runtime (the GNU tools of
            // Git for Windows) re-reads its command line and expands globs and
            // braces in it, which mangles a script such as sed's `2{p;b};p`.
            // This shell has already expanded every word, so none is left to it.
            for var in ["MSYS", "CYGWIN"] {
                let current = pairs
                    .iter()
                    .rev()
                    .find(|(k, _)| k == var)
                    .map(|(_, v)| v.clone())
                    .or_else(|| std::env::var(var).ok())
                    .unwrap_or_default();
                if !current.split_whitespace().any(|w| w == "noglob") {
                    cmd.env(var, format!("{current} noglob").trim());
                }
            }
        }
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
            return POLICY_DENY_STATUS;
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
        self.apply_child_env(&mut cmd, &self.child_env_pairs(env));
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
        // With a run deadline (`--timeout`) the child leads its own process
        // group (unix), so the whole tree can be killed when the deadline passes.
        let local_deadline = LOCAL_DEADLINE.with(|d| d.get());
        let bounded = crate::safety::remaining_deadline().is_some() || local_deadline.is_some();
        let limit = STREAM_LIMIT.with(|l| l.take());
        let program_name = cmd.get_program().to_string_lossy().into_owned();
        EXTERNAL_RAN.with(|f| f.set(true));
        #[cfg(unix)]
        if bounded {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                sink.eline(&format!("{SHELL_NAME}: {name}: command not found"));
                if matches!(name, "cp" | "mv" | "xargs" | "unzip") {
                    sink.eline(&format!(
                        "{SHELL_NAME}: {name} is not part of ish -b's in-process subset, so it runs only as a program of that name on PATH, and none was found"
                    ));
                }
                let spawned_args: Vec<String> = cmd
                    .get_args()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                if let Some(alt) = ish_alternative(name, &spawned_args) {
                    sink.eline(&alt);
                }
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
        EXTERNAL_PROGRAMS.with(|p| p.borrow_mut().push(program_name.clone()));
        // When the input is a prefix of an endless stream, the program that
        // reads it is watched: if it exits while its input is still open (or
        // stops reading and the pipe breaks), it stopped by itself, as `awk
        // 'NR==3{exit}'` does, and the prefix was enough (see `endless_plan`).
        let watch = ENDLESS_USED.with(|f| f.get());
        let reader_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = if feed {
            let data = stdin.read_all();
            let grace = std::time::Duration::from_millis(match data.len() {
                0..=0x10400 => 1000,
                0x10401..=0x100400 => 2000,
                _ => 4000,
            });
            let done = std::sync::Arc::clone(&reader_done);
            child.stdin.take().map(|mut w| {
                std::thread::spawn(move || -> bool {
                    if w.write_all(&data).is_err() {
                        return true;
                    }
                    if watch {
                        let t = std::time::Instant::now();
                        while t.elapsed() < grace {
                            if done.load(std::sync::atomic::Ordering::SeqCst) {
                                return true;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                    }
                    false
                })
            })
        } else {
            None
        };
        let out = if bounded {
            let guard = crate::builtins::TreeGuard::new(&child);
            match wait_output_bounded(child, &guard, local_deadline) {
                Ok(Some(o)) => o,
                // `timeout` ended it: its status, with nothing said.
                Ok(None)
                    if !crate::safety::deadline_passed()
                        && local_deadline.is_some_and(|t| std::time::Instant::now() >= t) =>
                {
                    LOCAL_TIMED_OUT.with(|f| f.set(true));
                    return 124;
                }
                Ok(None) => {
                    let refusal = crate::safety::run_deadline_error("posix", name);
                    sink.eline(&refusal.to_string());
                    let code = refusal
                        .downcast_ref::<crate::safety::SafetyError>()
                        .map(|e| e.code.exit_code())
                        .unwrap_or(1);
                    // The deadline ends the whole run, not this one stage:
                    // whichever stage it killed, the script stops with this
                    // status (see `run_pipeline`).
                    RUN_TIMEOUT_STATUS.store(code, std::sync::atomic::Ordering::SeqCst);
                    return code;
                }
                Err(e) => {
                    sink.eline(&format!("{SHELL_NAME}: {name}: {}", io_reason(&e)));
                    return 1;
                }
            }
        } else if let Some((lines, want)) = limit {
            // The reader of this output stops after `want` lines (or bytes):
            // read until then, and end the program, as a closed pipe would.
            use std::io::Read;
            let err_thread = child.stderr.take().map(|mut e| {
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let _ = e.read_to_end(&mut buf);
                    buf
                })
            });
            let mut stdout = Vec::new();
            let mut cut = false;
            if let Some(mut pipe) = child.stdout.take() {
                let mut chunk = [0u8; 8192];
                let mut seen = 0usize;
                while let Ok(n) = pipe.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    stdout.extend_from_slice(&chunk[..n]);
                    seen += if lines {
                        chunk[..n].iter().filter(|b| **b == b'\n').count()
                    } else {
                        n
                    };
                    if seen >= want {
                        cut = true;
                        break;
                    }
                }
            }
            let mut killed = false;
            if cut && matches!(child.try_wait(), Ok(None)) {
                killed = child.kill().is_ok();
            }
            let status = match child.wait() {
                Ok(s) => s,
                Err(e) => {
                    sink.eline(&format!("{SHELL_NAME}: {name}: {}", io_reason(&e)));
                    return 1;
                }
            };
            reader_done.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(w) = writer {
                let _ = w.join();
            }
            sink.out
                .extend_from_slice(&native_text(&program_name, stdout));
            if let Some(t) = err_thread {
                let err = t.join().unwrap_or_default();
                if !err.is_empty() {
                    sink.ewrite(&err);
                }
            }
            // Killed for a closed pipe: bash's status for SIGPIPE. (A program
            // may also die of the closed pipe itself before it is killed, as
            // `yes` does on macOS: its signal gives the same 141.)
            return if killed {
                141
            } else {
                exit_status_code(&status)
            };
        } else {
            match child.wait_with_output() {
                Ok(o) => o,
                Err(e) => {
                    sink.eline(&format!("{SHELL_NAME}: {name}: {}", io_reason(&e)));
                    return 1;
                }
            }
        };
        reader_done.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(w) = writer {
            if w.join().unwrap_or(false) && watch {
                LIMIT_HIT.with(|f| f.set(true));
            }
        }
        sink.out
            .extend_from_slice(&native_text(&program_name, out.stdout));
        if !out.stderr.is_empty() {
            sink.ewrite(&out.stderr);
        }
        exit_status_code(&out.status)
    }
}

/// The status a shell reports for a finished program: its exit code, or 128
/// plus the signal that ended it (141 for SIGPIPE, 137 for SIGKILL), as bash
/// reports it. A status with neither is 128.
fn exit_status_code(status: &std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return 128 + sig;
        }
    }
    128
}

thread_local! {
    /// Whether a program (not an in-process utility) has run since the flag was
    /// last cleared; `Shell::substitute` reads it to decide on CRLF line ends.
    static EXTERNAL_RAN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

thread_local! {
    /// How much of its output the next pipeline stage reads (`(lines, n)`:
    /// `n` lines when `lines`, else `n` bytes), set for an external program
    /// that feeds `head`. See `head_limit`.
    static STREAM_LIMIT: std::cell::Cell<Option<(bool, usize)>> = const { std::cell::Cell::new(None) };
}

thread_local! {
    /// The time by which the program `timeout` runs must end, if one does.
    static LOCAL_DEADLINE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    /// Set when `LOCAL_DEADLINE` killed a program.
    static LOCAL_TIMED_OUT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The programs run since the pipeline stage began, to know whether its
    /// output came from a native Windows program (see `pipe_text`).
    static EXTERNAL_PROGRAMS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    /// The path a command was found at on the shell's PATH, for `external`.
    static RESOLVED_PROGRAM: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Whether the program at `p` is a native Windows one (not built on MSYS).
fn is_native_program(p: &str) -> bool {
    #[cfg(windows)]
    {
        !is_msys_program(p)
    }
    #[cfg(not(windows))]
    {
        let _ = p;
        false
    }
}

/// The output of a native Windows program (a PE executable that is neither on
/// the MSYS runtime nor this shell) with its CRLF line ends read as LF, when
/// the output is text: it holds no NUL, and every LF has a CR before it. `jq
/// -r .version p.json` then prints `1.2.3\n` to the script's stdout and into a
/// file, as it does on Linux. Binary output, a mixed text, the output of a
/// program on the MSYS runtime and the output of the in-process tools are
/// returned as they are.
fn native_text(program: &str, out: Vec<u8>) -> Vec<u8> {
    if !cfg!(windows) || !out.contains(&b'\n') || out.contains(&0) {
        return out;
    }
    let lf = out.iter().filter(|b| **b == b'\n').count();
    let crlf = out.windows(2).filter(|w| w == b"\r\n").count();
    if lf != crlf || !is_native_pe(program) {
        return out;
    }
    let mut v = Vec::with_capacity(out.len());
    for (i, &b) in out.iter().enumerate() {
        if b == b'\r' && out.get(i + 1) == Some(&b'\n') {
            continue;
        }
        v.push(b);
    }
    v
}

/// Whether `program` (a path, or a name on PATH) is a native Windows PE
/// executable: not on the MSYS runtime, not a script, not this shell.
#[cfg(windows)]
fn is_native_pe(program: &str) -> bool {
    let path = if program.contains(['/', '\\']) {
        let p = std::path::PathBuf::from(program);
        if p.is_file() {
            p
        } else {
            p.with_extension("exe")
        }
    } else {
        // The system's own folders count here (`cmd`, `ipconfig`), unlike for
        // a GNU tool's lookup.
        match std::env::var("PATH")
            .ok()
            .and_then(|path| find_in(program, &path))
        {
            Some(p) => p,
            None => return false,
        }
    };
    if std::env::current_exe().is_ok_and(|me| me == path) {
        return false;
    }
    let mut head = [0u8; 2];
    let is_pe = std::fs::File::open(&path)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
        .is_ok()
        && &head == b"MZ";
    is_pe && !is_msys_program(program)
}

#[cfg(not(windows))]
fn is_native_pe(_program: &str) -> bool {
    false
}

/// The text a pipeline stage hands to the next one. A native Windows program
/// (`jq`, `cmd`, `git`) ends its lines with CRLF, which a script written for
/// Linux does not expect in `jq -r .a | while read n` or `... | grep -x x`;
/// the CR is dropped when the stage ran such a program. The output of a
/// program on the MSYS runtime and of the in-process tools is left as it is,
/// and so is output that goes to a file or the terminal.
fn pipe_text(out: Vec<u8>) -> Vec<u8> {
    let programs = EXTERNAL_PROGRAMS.with(|p| std::mem::take(&mut *p.borrow_mut()));
    if !cfg!(windows) || programs.is_empty() || !out.windows(2).any(|w| w == b"\r\n") {
        return out;
    }
    if !programs.iter().any(|p| is_native_program(p)) {
        return out;
    }
    let mut v = Vec::with_capacity(out.len());
    let mut i = 0;
    while i < out.len() {
        if out[i] == b'\r' && out.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        v.push(out[i]);
        i += 1;
    }
    v
}

thread_local! {
    /// Set when a source of a pipeline refused to run (`seq` past its cap):
    /// the pipeline fails and its reader's output is not shown.
    static SOURCE_REFUSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The files an endless run wrote, with their length before: `None` when
    /// the run created them. A refused run takes them back (see
    /// `undo_endless_writes`).
    static ENDLESS_WRITTEN: std::cell::RefCell<Vec<(String, Option<u64>, bool)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// A run of an endless source that nothing stopped is refused, and the files
/// it wrote are not left half-written (a prefix of 64 KiB to 16 MiB of `y`):
/// a file the run created is removed, and one it appended to is cut back to its
/// earlier length. A file that `>` truncated stays empty, as `>` left it.
fn undo_endless_writes(files: Vec<(String, Option<u64>, bool)>) {
    for (path, before, append) in files {
        match before {
            None => {
                let _ = std::fs::remove_file(&path);
            }
            Some(len) if append => {
                if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&path) {
                    let _ = f.set_len(len);
                }
            }
            Some(_) => {
                if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&path) {
                    let _ = f.set_len(0);
                }
            }
        }
    }
}

/// Note that `path` is about to be written while an endless source's prefix
/// is in play (see `undo_endless_writes`).
fn note_endless_write(path: &str, append: bool) {
    if !ENDLESS_USED.with(|f| f.get()) {
        return;
    }
    ENDLESS_WRITTEN.with(|w| {
        let mut w = w.borrow_mut();
        if !w.iter().any(|(p, _, _)| p == path) {
            let before = std::fs::metadata(path).ok().map(|m| m.len());
            w.push((path.to_string(), before, append));
        }
    });
}

thread_local! {
    /// How many bytes an endless source (`yes`, `/dev/zero`, `/dev/urandom`,
    /// a `seq` past its cap) gives the pipeline now running; see
    /// [`endless_plan`].
    static ENDLESS_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(ENDLESS_FIRST) };
    /// Set when a source gave only a prefix of an endless stream.
    static ENDLESS_USED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set when the pipeline running has a reader that stops (see
    /// [`endless_plan`]), so `seq` may give a prefix too.
    static ENDLESS_ARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set when `head`, `sed q` or `grep -m` has what it wanted.
    static LIMIT_HIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The first prefix of an endless stream a pipeline reads, in bytes. When the
/// reader that stops it is not satisfied the run is repeated with 16 times as
/// much, up to [`ENDLESS_LAST`].
const ENDLESS_FIRST: usize = 64 * 1024;
/// The most an endless stream is read for: past it the pipeline fails.
const ENDLESS_LAST: usize = 16 * 1024 * 1024;

fn note_limit() {
    LIMIT_HIT.with(|f| f.set(true));
}

/// A device that never ends: `Some(true)` for the random ones, `Some(false)`
/// for `/dev/zero`.
fn endless_device(path: &str) -> Option<bool> {
    match path {
        "/dev/urandom" | "/dev/random" => Some(true),
        "/dev/zero" => Some(false),
        _ => None,
    }
}

/// The prefix of an endless device the pipeline now running reads.
fn endless_bytes(random: bool) -> Vec<u8> {
    use rand::RngCore;
    ENDLESS_USED.with(|f| f.set(true));
    let mut v = vec![0u8; ENDLESS_BYTES.with(|b| b.get())];
    if random {
        rand::thread_rng().fill_bytes(&mut v);
    }
    v
}

/// After a pass over an endless source: whether to go again with a longer
/// prefix, because no reader of it has stopped yet.
fn endless_retry(cap: &mut usize) -> bool {
    if ENDLESS_USED.with(|f| f.get()) && !LIMIT_HIT.with(|f| f.get()) && *cap < ENDLESS_LAST {
        *cap = (*cap * 16).min(ENDLESS_LAST);
        ENDLESS_BYTES.with(|b| b.set(*cap));
        ENDLESS_USED.with(|f| f.set(false));
        true
    } else {
        false
    }
}

/// The literal text of a word, when it is all literal.
fn lit_text(w: &Word) -> Option<String> {
    let mut s = String::new();
    for part in w {
        match part {
            Part::Lit(t, _) => s.push_str(t),
            _ => return None,
        }
    }
    Some(s)
}

/// The words of a plain command, as literal text where they have it.
fn simple_words(cmd: &Cmd) -> Option<Vec<Option<String>>> {
    match cmd {
        Cmd::Simple { assigns, words, .. } if assigns.is_empty() => {
            Some(words.iter().map(lit_text).collect())
        }
        _ => None,
    }
}

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Whether `a` is a `sed` script that ends by `q`: `5q`, `q`, `/re/q`, `2q3`.
fn sed_quit_script(a: &str) -> bool {
    let rest = if let Some(tail) = a.strip_prefix('/') {
        tail.find('/').map(|i| &tail[i + 1..])
    } else {
        Some(a.trim_start_matches(|c: char| c.is_ascii_digit()))
    };
    rest.is_some_and(|r| {
        r.strip_prefix('q')
            .is_some_and(|d| d.is_empty() || all_digits(d))
    })
}

/// Whether `cmd` ends its reading by itself once it has its share: `head -n N`
/// or `-c N` (or `-N`), `sed Nq` and `grep -m N`.
fn is_limiter(cmd: &Cmd) -> bool {
    let Some(w) = simple_words(cmd) else {
        return false;
    };
    let args: Vec<&str> = w.iter().skip(1).filter_map(|a| a.as_deref()).collect();
    match w.first().and_then(|n| n.as_deref()) {
        Some("head") => args.iter().enumerate().any(|(i, a)| {
            ((*a == "-n" || *a == "-c") && args.get(i + 1).is_some_and(|n| all_digits(n)))
                || a.strip_prefix("-n")
                    .or_else(|| a.strip_prefix("-c"))
                    .is_some_and(all_digits)
                || a.strip_prefix('-').is_some_and(all_digits)
        }),
        Some("sed") => args
            .iter()
            .any(|a| !a.starts_with('-') && sed_quit_script(a)),
        Some("grep") => {
            !args.iter().any(|a| {
                a.starts_with('-') && !a.starts_with("--") && a[1..].contains(['c', 'l', 'L'])
            }) && args.iter().enumerate().any(|(i, a)| {
                (*a == "-m" && args.get(i + 1).is_some_and(|n| all_digits(n)))
                    || a.strip_prefix("-m").is_some_and(all_digits)
                    || a.starts_with("--max-count=")
            })
        }
        // A program run as one (`awk 'NR==3{exit}'`) may stop reading: it is
        // given a prefix, and when it exits before it has read to the end of
        // it, it stopped (see `Shell::run_piped`).
        Some(name) => is_external_name(name),
        None => false,
    }
}

/// Whether a command name is run as a program: not a builtin of the shell and
/// not a tool the subset runs itself.
fn is_external_name(name: &str) -> bool {
    !name.contains(['/', '\\'])
        && !SUBSET_NAMES.contains(&name)
        && !matches!(
            name,
            "true"
                | ":"
                | "false"
                | "exit"
                | "break"
                | "continue"
                | "export"
                | "unset"
                | "set"
                | "shift"
                | "cd"
                | "read"
                | "command"
                | "timeout"
                | "mktemp"
                | "pwd"
                | "yes"
                | "seq"
        )
}

/// Whether a stage before a limiter passes on a prefix of what it reads as a
/// prefix of what it writes, so a longer input could only add to its output.
fn is_streaming_stage(cmd: &Cmd) -> bool {
    let Some(w) = simple_words(cmd) else {
        return false;
    };
    let args: Vec<&str> = w.iter().skip(1).filter_map(|a| a.as_deref()).collect();
    match w.first().and_then(|n| n.as_deref()) {
        Some("yes" | "seq" | "cat" | "tr" | "cut") => true,
        Some("timeout") => args.iter().any(|a| matches!(*a, "yes" | "seq")),
        Some("grep") => !args.iter().any(|a| {
            a.starts_with('-') && !a.starts_with("--") && a[1..].contains(['c', 'l', 'L', 'q'])
        }),
        Some("sed") => !args
            .iter()
            .any(|a| *a == "-i" || *a == "-s" || *a == "-z" || a.contains('$')),
        _ => false,
    }
}

/// For a pipeline that starts on an endless source, the index of the first
/// stage that stops reading, when every stage before it is a streaming one.
/// Such a pipeline is run on a prefix of the source and, when that was too
/// short for the reader, again on a longer one (the stages up to that reader
/// have no effects to repeat). `None` for any other pipeline.
fn endless_plan(p: &Pipeline) -> Option<usize> {
    let Cmd::Simple { words, redirs, .. } = p.cmds.first()? else {
        return None;
    };
    simple_words(&p.cmds[0])?;
    let name = words.first().and_then(lit_text);
    let device = |w: &Word| lit_text(w).is_some_and(|t| endless_device(&t).is_some());
    // `timeout 1 yes | head -1`: the program the timeout runs is the source.
    let timed = name.as_deref() == Some("timeout")
        && words
            .iter()
            .skip(1)
            .filter_map(lit_text)
            .any(|w| matches!(w.as_str(), "yes" | "seq"));
    let mention = matches!(name.as_deref(), Some("yes" | "seq"))
        || timed
        || words.iter().skip(1).any(device)
        || redirs
            .iter()
            .any(|r| matches!(r, Redir::In(w) if device(w)));
    if !mention {
        return None;
    }
    let limiter = p.cmds.iter().position(is_limiter)?;
    p.cmds[..limiter]
        .iter()
        .all(is_streaming_stage)
        .then_some(limiter)
}

/// The refusal for an endless source with no reader that stops it.
fn endless_refusal(sink: &mut Sink) {
    sink.eline(&format!(
        "{SHELL_NAME}: an endless input (yes, /dev/zero, /dev/urandom, a huge seq) has no reader that stops it, or none that stops it within {} MiB; end it with head, sed Nq or grep -m, or put only cat, tr, cut, grep and sed in between",
        ENDLESS_LAST >> 20
    ));
}

/// What `head` reads of its standard input, if `cmd` is a plain `head -n N`,
/// `head -N`, `head -nN` or `head -c N` with no file operand and nothing
/// that needs the expansion of a word: `Some((true, N))` for lines and
/// `Some((false, N))` for bytes. Anything else is `None` (the whole output
/// is read, as before).
fn head_limit(cmd: &Cmd) -> Option<(bool, usize)> {
    let Cmd::Simple { assigns, words, .. } = cmd else {
        return None;
    };
    if !assigns.is_empty() {
        return None;
    }
    let mut text: Vec<String> = Vec::new();
    for w in words {
        let mut s = String::new();
        for part in w {
            match part {
                Part::Lit(t, _) => s.push_str(t),
                _ => return None,
            }
        }
        text.push(s);
    }
    if text.first().map(String::as_str) != Some("head") {
        return None;
    }
    let mut want: Option<(bool, usize)> = None;
    let mut i = 1;
    while i < text.len() {
        let a = text[i].as_str();
        let number = |s: &str| s.parse::<usize>().ok().filter(|_| !s.is_empty());
        if a == "-n" || a == "-c" {
            let n = number(text.get(i + 1)?)?;
            want = Some((a == "-n", n));
            i += 2;
        } else if let Some(n) = a.strip_prefix("-n").and_then(number) {
            want = Some((true, n));
            i += 1;
        } else if let Some(n) = a.strip_prefix("-c").and_then(number) {
            want = Some((false, n));
            i += 1;
        } else {
            // A file operand, `-q`, `-v`, a negative count: not handled.
            let n = a.strip_prefix('-').and_then(number)?;
            want = Some((true, n));
            i += 1;
        }
    }
    // `head` with no option reads ten lines.
    Some(want.unwrap_or((true, 10))).map(|(l, n)| (l, n.max(1)))
}

/// Wait for `child` and collect its output like `wait_with_output`, but stop at
/// the run's deadline: the child's tree is killed and `Ok(None)` returned. The
/// pipes are read on their own threads, and after a kill they are given two
/// seconds, not joined, so a descendant that kept one open cannot hold the call.
fn wait_output_bounded(
    mut child: std::process::Child,
    guard: &crate::builtins::TreeGuard,
    local: Option<std::time::Instant>,
) -> std::io::Result<Option<std::process::Output>> {
    use std::io::Read;
    fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            let _ = tx.send(buf);
        });
        rx
    }
    let out_rx = drain(child.stdout.take());
    let err_rx = drain(child.stderr.take());
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                guard.kill(&mut child);
                return Err(e);
            }
        }
        if crate::safety::deadline_passed() || local.is_some_and(|t| std::time::Instant::now() >= t)
        {
            guard.kill(&mut child);
            return Ok(None);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let grace = std::time::Duration::from_secs(2);
    let stdout = out_rx.recv_timeout(grace).unwrap_or_default();
    let stderr = err_rx.recv_timeout(grace).unwrap_or_default();
    Ok(Some(std::process::Output {
        status,
        stdout,
        stderr,
    }))
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
    "cut", "sed", "ls", "find", "date", "basename", "dirname", "seq", "mkdir", "touch", "tr", "rm",
    "rmdir",
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

/// Give `cmd` the arguments of a program, each as one argument, whatever it holds.
///
/// Elsewhere that is `Command::args`. On Windows `std` quotes an argument by
/// the rules of the MSVC runtime (`a"b` is written `"a\"b"`), which a program
/// built on the MSYS or Cygwin runtime (the GNU tools of Git for Windows) does
/// not read that way once this shell sets `MSYS=noglob` for it (see
/// `Shell::external`): it then splits only at double and single quotes and
/// knows no backslash escape, so `a"b` arrived as `a\b` and `echo "hi there"`
/// as two words. For such a program the arguments are written as single-quoted
/// pieces, with each `'` in double quotes (`it's` is `'it'"'"'s'`), which that
/// reader returns byte for byte; an empty argument is `""`.
fn add_program_args(cmd: &mut std::process::Command, program: &str, args: &[String]) {
    #[cfg(windows)]
    if args.iter().any(|a| a.contains(['"', '\'', '\\'])) && is_msys_program(program) {
        use std::os::windows::process::CommandExt;
        for a in args {
            cmd.raw_arg(msys_noglob_quote(a));
        }
        return;
    }
    let _ = program;
    cmd.args(args);
}

/// One argument as a program on the MSYS or Cygwin runtime, run with
/// `MSYS=noglob`, reads it back unchanged (see [`add_program_args`]).
#[cfg(any(windows, test))]
fn msys_noglob_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"".to_string();
    }
    let mut out = String::with_capacity(arg.len() + 4);
    for (k, piece) in arg.split('\'').enumerate() {
        if k > 0 {
            out.push_str("\"'\"");
        }
        if !piece.is_empty() {
            out.push('\'');
            out.push_str(piece);
            out.push('\'');
        }
    }
    out
}

/// Whether `program` (a path, or a name searched on PATH) is built on the MSYS
/// or Cygwin runtime: it imports `msys-2.0.dll` or `cygwin1.dll`. (The names
/// appearing in the file is not enough: this shell's own executable holds both
/// as text.) A program that cannot be found or read is not.
#[cfg(windows)]
fn is_msys_program(program: &str) -> bool {
    let path = if program.contains(['/', '\\']) {
        let p = std::path::PathBuf::from(program);
        if p.is_file() {
            p
        } else {
            p.with_extension("exe")
        }
    } else {
        match path_candidates(program).next() {
            Some(p) => p,
            None => return false,
        }
    };
    if imports_msys_runtime(&path) {
        return true;
    }
    // Git for Windows puts launchers in `Git\bin` (`sh.exe`, `bash.exe`) that
    // import no runtime themselves and start the program of the same name in
    // `Git\usr\bin`, handing it the command line as written: that program is
    // the one whose reading of the arguments counts.
    let launched = path
        .parent()
        .and_then(|bin| bin.parent())
        .zip(path.file_name())
        .map(|(git, name)| git.join("usr").join("bin").join(name));
    launched.is_some_and(|p| p != path && imports_msys_runtime(&p))
}

/// Whether the executable at `path` imports the MSYS or Cygwin runtime.
#[cfg(windows)]
fn imports_msys_runtime(path: &std::path::Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    pe_imports(&bytes)
        .iter()
        .any(|d| d == "msys-2.0.dll" || d == "cygwin1.dll")
}

/// The DLLs a Windows executable imports, lower-cased, read from the import
/// directory of its PE headers. Empty for a file that is not a well-formed PE
/// image (every offset is checked, so a damaged file gives what was read so far).
#[cfg(any(windows, test))]
fn pe_imports(bytes: &[u8]) -> Vec<String> {
    let u16_at = |o: usize| -> Option<usize> {
        bytes
            .get(o..o + 2)
            .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
    };
    let u32_at = |o: usize| -> Option<usize> {
        bytes
            .get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    let mut out = Vec::new();
    let mut read = || -> Option<()> {
        if bytes.get(..2)? != b"MZ" {
            return None;
        }
        let pe = u32_at(0x3c)?;
        if bytes.get(pe..pe + 4)? != b"PE\0\0" {
            return None;
        }
        let sections = u16_at(pe + 6)?;
        let opt_size = u16_at(pe + 20)?;
        let opt = pe + 24;
        // The import table is data directory 1: at 0x78 in a PE32+ header, 0x68 in PE32.
        let dir = match u16_at(opt)? {
            0x20b => opt + 0x78,
            0x10b => opt + 0x68,
            _ => return None,
        };
        let import_rva = u32_at(dir)?;
        if import_rva == 0 {
            return None;
        }
        let table = opt + opt_size;
        let to_file = |rva: usize| -> Option<usize> {
            (0..sections).find_map(|k| {
                let s = table + k * 40;
                let (size, va, raw) = (u32_at(s + 8)?, u32_at(s + 12)?, u32_at(s + 20)?);
                let span = size.max(u32_at(s + 16)?);
                (rva >= va && rva < va + span).then(|| raw + (rva - va))
            })
        };
        let mut at = to_file(import_rva)?;
        for _ in 0..512 {
            let name_rva = u32_at(at + 12)?;
            if name_rva == 0 {
                break;
            }
            let name_at = to_file(name_rva)?;
            let name: Vec<u8> = bytes
                .get(name_at..)?
                .iter()
                .copied()
                .take_while(|b| *b != 0)
                .take(260)
                .collect();
            out.push(String::from_utf8_lossy(&name).to_ascii_lowercase());
            at += 20;
        }
        Some(())
    };
    let _ = read();
    out
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
            None => {
                let mut msg = format!("{SHELL_NAME}: {name}: command not found");
                if let Some(alt) = ish_alternative(name, &argv[1..]) {
                    msg.push('\n');
                    msg.push_str(&alt);
                }
                Err((127, msg))
            }
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
        match unsupported_flag(name, &argv[1..]).or_else(|| {
            argv[1..]
                .iter()
                .find(|a| a.len() > 1 && a.starts_with('-'))
                .cloned()
        }) {
            Some(flag) => flag,
            None => "this form".to_string(),
        }
    };
    // The in-process utility is what refused; the GNU program that could have
    // run it comes second.
    let mut msg = format!(
        "{SHELL_NAME}: {name}: the in-process {name} does not support {what}; a GNU-compatible {name} on PATH would run it, and none was found"
    );
    if let Some(alt) = ish_alternative(name, &argv[1..]) {
        msg.push('\n');
        msg.push_str(&alt);
    }
    if let Some(hint) = windows_tools_hint() {
        msg.push('\n');
        msg.push_str(&hint);
    }
    Err((2, msg))
}

/// The hint line naming the ish builtin that does what the subset's missing
/// `name` would, for a command the subset does not run: `cp` and `mv` are
/// builtins of that name, and `find`'s `-name` with `-o` is `glob` with a
/// `{a,b}` alternation. `None` for any other name.
fn ish_alternative(name: &str, args: &[String]) -> Option<String> {
    let alt = match name {
        "cp" => r#"cp("a", "b")"#.to_string(),
        "mv" => r#"mv("a", "b")"#.to_string(),
        // `glob` matches names, not times, so `-newer` is a filter on `ls`.
        "find" if args.iter().any(|a| a.starts_with("-newer")) => {
            return Some(
                r#"  hint: in ish, ls(dir) | where(fn(f) => f.modified > fs_stat("ref").modified) (ish -c '...')"#
                    .to_string(),
            );
        }
        "find" if args.iter().any(|a| a == "-o") => {
            return Some(
                r#"  hint: run one find per alternative, or in ish use glob("**/*.{a,b}") (ish -c '...')"#
                    .to_string(),
            );
        }
        "find" => r#"glob("**/*.{rs,py}")"#.to_string(),
        _ => return None,
    };
    Some(format!("  hint: in ish, use {alt} (ish -c '...')"))
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
    // Named explicitly, not guessed from an argument: the guess skips a bare
    // file name (`e1.txt`, which has no directory part), which is what a human
    // session passes, so only agent mode's absolute paths were recorded.
    crate::journal::record_paths(
        "posix",
        crate::safety::Effect::WriteLocal,
        vec![path.to_string()],
    );
    note_endless_write(path, append);
    let res = write_file_unrecorded(path, data, append);
    crate::journal::settle(mark, res.is_ok());
    res
}

/// [`write_file`] without the journal: for a caller that has recorded the paths
/// itself, as one step (`sed -i.bak` records the edit and its backup together).
fn write_file_unrecorded(path: &str, data: &[u8], append: bool) -> Result<(), String> {
    // The handle is judged for hard links and truncated only after that check.
    crate::safety::open_in_place("posix", Path::new(path), append)
        .and_then(|mut f| f.write_all(data).map_err(anyhow::Error::from))
        .map_err(|e| match e.downcast_ref::<std::io::Error>() {
            Some(io) => format!("{SHELL_NAME}: {path}: {}", io_reason(io)),
            None => e.to_string(),
        })
}

/// The message GNU tools print for an I/O error.
/// A file name as GNU tools print it in a diagnostic: bare, unless it holds a
/// character the shell would treat specially (a glob that matched nothing, a
/// space), when it is in single quotes (`wc: '*.rs': No such file ...`).
fn gnu_quote(name: &str) -> String {
    const SPECIAL: &str = "*?[]{}$&;|<>()\"\\!#~^`";
    let special = name
        .chars()
        .any(|c| c.is_whitespace() || SPECIAL.contains(c));
    if special && !name.contains('\'') {
        format!("'{name}'")
    } else {
        name.to_string()
    }
}

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
            // Windows refuses a name such as an unmatched `*.rs` as malformed
            // (ERROR_INVALID_NAME, ERROR_BAD_PATHNAME). No such file can exist,
            // and GNU says so in these words.
            if cfg!(windows) && matches!(e.raw_os_error(), Some(123) | Some(161)) {
                return "No such file or directory".into();
            }
            let s = e.to_string();
            s.split(" (os error").next().unwrap_or(&s).to_string()
        }
    }
}

/// A path as `pwd -W` of Git bash writes it on Windows: with `/`, not `\`, so
/// `${d##*/}` and `basename` find the last component. Unchanged elsewhere.
fn posix_path(p: &str) -> String {
    if cfg!(windows) {
        p.replace('\\', "/")
    } else {
        p.to_string()
    }
}

/// The numeric user id for `$UID` and `$EUID`: the effective id on Unix, and
/// 1000 on Windows, which has no such number.
fn user_id() -> u32 {
    #[cfg(unix)]
    {
        // SAFETY: geteuid cannot fail and takes no arguments.
        unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        1000
    }
}

/// `$RANDOM`: 0..=32767, from a generator seeded once from the clock and pid.
fn next_random() -> u32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut x = STATE.load(Ordering::Relaxed);
    if x == 0 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        x = nanos ^ (u64::from(std::process::id()) << 32) ^ 0x9E37_79B9_7F4A_7C15;
        if x == 0 {
            x = 1;
        }
    }
    // xorshift64: never 0 from a non-zero state.
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    STATE.store(x, Ordering::Relaxed);
    ((x >> 33) & 0x7FFF) as u32
}

/// `$PPID`: the parent process's id, or `None` when the system will not say.
fn parent_pid() -> Option<String> {
    let me = sysinfo::get_current_pid().ok()?;
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[me]), true);
    sys.process(me)?.parent().map(|p| p.as_u32().to_string())
}

fn logical_pwd() -> String {
    let phys = std::env::current_dir().ok();
    if let (Ok(pwd), Some(phys)) = (std::env::var("PWD"), phys.as_ref()) {
        if std::fs::canonicalize(&pwd).ok().as_deref()
            == std::fs::canonicalize(phys).ok().as_deref()
        {
            return posix_path(&pwd);
        }
    }
    phys.map(|p| posix_path(&p.to_string_lossy()))
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

/// Whether `c` is in the POSIX character class `name`, or `None` for a name
/// that is no class.
fn posix_class(name: &str, c: char) -> Option<bool> {
    Some(match name {
        "alpha" => c.is_alphabetic(),
        "digit" => c.is_ascii_digit(),
        "alnum" => c.is_alphanumeric(),
        "upper" => c.is_uppercase(),
        "lower" => c.is_lowercase(),
        "space" => matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r'),
        "blank" => matches!(c, ' ' | '\t'),
        "punct" => c.is_ascii_punctuation(),
        "print" => !c.is_control(),
        "graph" => !c.is_control() && !c.is_whitespace(),
        "cntrl" => c.is_control(),
        "xdigit" => c.is_ascii_hexdigit(),
        _ => return None,
    })
}

/// `fnmatch` for `*`, `?` and `[…]` (with `!`/`^` negation, ranges and
/// `[:class:]` names).
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
                    // `[:alpha:]` and the other POSIX classes.
                    if p[k] == '[' && p.get(k + 1) == Some(&':') {
                        let close = (k + 2..p.len().saturating_sub(1))
                            .find(|&j| p[j] == ':' && p[j + 1] == ']');
                        if let Some(end) = close {
                            let name: String = p[k + 2..end].iter().collect();
                            if let Some(m) = posix_class(&name, c) {
                                matched |= m;
                                k = end + 2;
                                continue;
                            }
                        }
                    }
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

thread_local! {
    /// Set while `glob_paths_with_hidden_dirs` runs: `**` then enters dot-directories.
    static GLOB_HIDDEN_DIRS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Matches a glob dropped because a link led outside the workspace.
    static GLOB_JAIL_SKIPPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `glob_paths_capped`, but `**` also enters dot-directories. Only for the error
/// path of a glob that matched nothing, to tell the caller that naming the
/// dot-directory would have matched.
pub fn glob_paths_with_hidden_dirs(pattern: &str, max: usize) -> (Vec<String>, bool) {
    GLOB_HIDDEN_DIRS.with(|h| h.set(true));
    let r = glob_paths_capped(pattern, max);
    GLOB_HIDDEN_DIRS.with(|h| h.set(false));
    r
}

/// Count `n` glob matches that were dropped for leading outside the workspace.
pub fn note_glob_jail_skipped(n: usize) {
    GLOB_JAIL_SKIPPED.with(|c| c.set(c.get().saturating_add(n)));
}

/// The number of glob matches dropped for leading outside the workspace since
/// the last call, and reset it.
pub fn take_glob_jail_skipped() -> usize {
    GLOB_JAIL_SKIPPED.with(|c| c.replace(0))
}

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
        .filter(|n| !n.starts_with('.') || GLOB_HIDDEN_DIRS.with(|h| h.get()))
        .collect();
    names.sort();
    for n in names {
        if out.over {
            return;
        }
        // A walk over a large tree answers to the evaluation deadline.
        if crate::safety::deadline_passed() {
            out.over = true;
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
                // list the whole disk). Only the starting base is judged: every
                // later base came out of a stage that judged it, or is a
                // directory (never a link) listed inside one that was judged.
                if i == 0 && crate::security::outside_read_jail(base) {
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
                            let kind = e.file_type();
                            if !n.starts_with('.')
                                && kind.as_ref().map(|t| !t.is_dir()).unwrap_or(false)
                            {
                                let p = if d.is_empty() { n } else { format!("{d}/{n}") };
                                // A file listed inside a judged directory is inside
                                // it; only a link can lead out.
                                let is_link = kind.as_ref().map(|t| t.is_symlink()).unwrap_or(true);
                                if !is_link || !crate::security::outside_read_jail(&p) {
                                    next.push(p);
                                } else {
                                    note_glob_jail_skipped(1);
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
            if crate::safety::deadline_passed() {
                return (Vec::new(), true);
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
            // Only the starting base is judged here: a later base came out of a
            // stage that judged it, or is a directory (never a link) listed
            // inside one that was. Judging every name was a link-walk per entry.
            if i == 0 && crate::security::outside_read_jail(dir) {
                continue;
            }
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue;
            };
            // Each name with its kind, as the listing gave it (no further call):
            // (name, is a link, is a directory that is not a link).
            let mut names: Vec<(String, bool, bool)> = rd
                .filter_map(|e| e.ok())
                .map(|e| {
                    let kind = e.file_type();
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        kind.as_ref().map(|t| t.is_symlink()).unwrap_or(true),
                        kind.as_ref()
                            .map(|t| t.is_dir() && !t.is_symlink())
                            .unwrap_or(false),
                    )
                })
                .filter(|(n, _, _)| !n.starts_with('.') || comp.starts_with('.'))
                .filter(|(n, _, _)| glob_match(comp, n))
                .collect();
            names.sort_by(|a, b| order(&a.0, &b.0));
            for (n, is_link, is_real_dir) in names {
                let p = join(&n);
                // A link inside the workspace can name a target outside it; a
                // name that is not a link sits inside the directory it was
                // listed in.
                if is_link && crate::security::outside_read_jail(&p) {
                    note_glob_jail_skipped(1);
                    continue;
                }
                if last || is_real_dir || (is_link && Path::new(&p).is_dir()) {
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
        // A trailing slash keeps only the directories (a link to one counts).
        bases = bases
            .into_iter()
            .filter(|b| Path::new(b).is_dir())
            .map(|b| format!("{b}/"))
            .collect();
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
    let _ = crate::safety::take_redaction_count();
    let (out, _) = crate::safety::redact_str(text);
    // The utility's output is printed as it runs, so the note is too.
    crate::safety::note_redactions_on_stderr();
    if out == text {
        std::borrow::Cow::Borrowed(bytes)
    } else {
        std::borrow::Cow::Owned(out.into_bytes())
    }
}

/// The bytes of `path`. `/dev/null` reads as an empty file on every platform
/// (Windows has no such path), as the write side already treats it as a sink.
fn read_file(path: &str) -> std::io::Result<Vec<u8>> {
    if path == "/dev/null" {
        return Ok(Vec::new());
    }
    // `/dev/zero` and `/dev/urandom` have no end (and no such path on
    // Windows): a prefix is read, as long as the pipeline now running asks for.
    if let Some(random) = endless_device(path) {
        return Ok(endless_bytes(random));
    }
    std::fs::read(path)
}

fn jail_refusal(tool: &str, path: &str) -> Option<String> {
    // `/dev/null` is no file of the workspace's or anyone's: it holds nothing.
    // Nor is `/dev/stdin`: it is this command's own input, which `read_inputs`
    // takes from the stream and never opens.
    if matches!(
        path,
        "/dev/null" | "/dev/stdin" | "/dev/fd/0" | "/dev/zero" | "/dev/urandom" | "/dev/random"
    ) {
        return None;
    }
    // Under the jail, a name Windows would not keep as written is refused as
    // the builtins refuse it (E_BAD_ARG), before the jail: `cat 'a.txt.'` read
    // `a.txt`, and so did `a.txt\` and `a.txt\.`, while `file.read` refused the
    // first. A device name, a forbidden character, a trailing dot or space, or a
    // separator after a file name.
    if cfg!(windows) && crate::safety::jail_enforced(crate::safety::current_mode()) {
        if let Some(e) = crate::safety::windows_read_name_refusal(tool, path)
            .or_else(|| crate::safety::windows_trailing_separator_refusal(tool, path))
        {
            return Some(e.to_string());
        }
    }
    if crate::security::outside_read_jail(path) {
        crate::security::audit_read_refusal(tool, path);
        return Some(crate::safety::outside_workspace(tool, path).to_string());
    }
    // Off unless the operator set IRONSHELL_REFUSE_LINKED_READS=1.
    if let Some(e) = crate::safety::refuse_hard_linked_read(tool, path)
        .err()
        .map(|e| e.to_string())
    {
        return Some(e);
    }
    // In agent mode, a file that holds a private key is not read.
    crate::security::private_key_refusal_for_tool(tool, path)
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
        if f == "/dev/stdin" || f == "/dev/fd/0" {
            out.push((f.clone(), stdin.read_all()));
            continue;
        }
        if let Some(refusal) = jail_refusal(tool, f) {
            sink.eline(&refusal);
            *status = 1;
            continue;
        }
        match read_file(f) {
            Ok(d) => out.push((f.clone(), d)),
            Err(e) => {
                // `head` and `tail` word it as `cannot open 'f' for reading`.
                if matches!(tool, "head" | "tail") {
                    sink.eline(&format!(
                        "{tool}: cannot open '{f}' for reading: {}",
                        io_reason(&e)
                    ));
                } else {
                    sink.eline(&format!("{tool}: {}: {}", gnu_quote(f), io_reason(&e)));
                }
                *status = 1;
            }
        }
    }
    out
}

/// Whether this process has a terminal on standard input, output or error, so
/// that `/dev/tty` means something.
fn has_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
        || std::io::stdout().is_terminal()
        || std::io::stderr().is_terminal()
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

/// `basename` and `dirname` as GNU coreutils run them. Options may follow the
/// operands and `--` ends them; `basename` takes `-a`/`--multiple`,
/// `-s SUFFIX`/`--suffix=SUFFIX` (which implies `-a`) and `-z`/`--zero`,
/// `dirname` takes `-z`/`--zero` and any number of operands. `--help` and
/// `--version` are left to the real program.
fn name_tool(is_base: bool, args: &[String], sink: &mut Sink) -> Util {
    let tool = if is_base { "basename" } else { "dirname" };
    let try_help = format!("Try '{tool} --help' for more information.");
    let mut multiple = false;
    let mut zero = false;
    let mut suffix: Option<String> = None;
    let mut operands: Vec<&String> = Vec::new();
    let longs: &[&str] = if is_base {
        &["multiple", "suffix", "zero", "help", "version"]
    } else {
        &["zero", "help", "version"]
    };
    let mut k = 0;
    let mut options_done = false;
    while k < args.len() {
        let a = args[k].as_str();
        k += 1;
        if options_done || !a.starts_with('-') || a == "-" {
            operands.push(&args[k - 1]);
            continue;
        }
        if a == "--" {
            options_done = true;
            continue;
        }
        if let Some(long) = a.strip_prefix("--") {
            let (name, value) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            let hits: Vec<&&str> = longs.iter().filter(|l| l.starts_with(name)).collect();
            let exact = longs.iter().find(|l| **l == name);
            let chosen = match (exact, hits.as_slice()) {
                (Some(e), _) => *e,
                (None, [one]) => **one,
                (None, []) => {
                    sink.eline(&format!("{tool}: unrecognized option '{a}'\n{try_help}"));
                    return Util::Done(1);
                }
                (None, _) => {
                    sink.eline(&format!("{tool}: option '{a}' is ambiguous\n{try_help}"));
                    return Util::Done(1);
                }
            };
            match chosen {
                "help" | "version" => return Util::Decline,
                "multiple" | "zero" if value.is_some() => {
                    sink.eline(&format!(
                        "{tool}: option '--{chosen}' doesn't allow an argument\n{try_help}"
                    ));
                    return Util::Done(1);
                }
                "multiple" => multiple = true,
                "zero" => zero = true,
                _ => {
                    let v = match value {
                        Some(v) => v,
                        None if k < args.len() => {
                            k += 1;
                            args[k - 1].clone()
                        }
                        None => {
                            sink.eline(&format!(
                                "{tool}: option '--suffix' requires an argument\n{try_help}"
                            ));
                            return Util::Done(1);
                        }
                    };
                    suffix = Some(v);
                    multiple = true;
                }
            }
            continue;
        }
        let flags: Vec<char> = a[1..].chars().collect();
        for (at, &c) in flags.iter().enumerate() {
            match c {
                'z' => zero = true,
                'a' if is_base => multiple = true,
                's' if is_base => {
                    let rest: String = flags[at + 1..].iter().collect();
                    let v = if !rest.is_empty() {
                        rest
                    } else if k < args.len() {
                        k += 1;
                        args[k - 1].clone()
                    } else {
                        sink.eline(&format!(
                            "{tool}: option requires an argument -- 's'\n{try_help}"
                        ));
                        return Util::Done(1);
                    };
                    suffix = Some(v);
                    multiple = true;
                    break;
                }
                other => {
                    sink.eline(&format!("{tool}: invalid option -- '{other}'\n{try_help}"));
                    return Util::Done(1);
                }
            }
        }
    }
    if operands.is_empty() {
        sink.eline(&format!("{tool}: missing operand\n{try_help}"));
        return Util::Done(1);
    }
    if is_base && !multiple && operands.len() > 2 {
        sink.eline(&format!(
            "{tool}: extra operand '{}'\n{try_help}",
            operands[2]
        ));
        return Util::Done(1);
    }
    let end = if zero { b'\0' } else { b'\n' };
    let base_of = |name: &str, suffix: Option<&str>| -> String {
        // On Windows a `\` separates too.
        let p = posix_path(name);
        let trimmed = p.trim_end_matches('/');
        let mut b = trimmed.rsplit('/').next().unwrap_or("").to_string();
        if trimmed.is_empty() && p.starts_with('/') {
            b = "/".into();
        }
        if let Some(suffix) = suffix {
            if b.len() > suffix.len() && b.ends_with(suffix) {
                b.truncate(b.len() - suffix.len());
            }
        }
        b
    };
    let dir_of = |name: &str| -> String {
        // coreutils `dir_len`: drop the last component and the slashes
        // before it, but keep a leading slash.
        let p = posix_path(name);
        let b = p.as_bytes();
        let skip = b.iter().take_while(|&&c| c == b'/').count();
        let mut last = skip;
        let mut was_slash = false;
        for (i, &c) in b.iter().enumerate().skip(skip) {
            if c == b'/' {
                was_slash = true;
            } else if was_slash {
                last = i;
                was_slash = false;
            }
        }
        let prefix = usize::from(b.first() == Some(&b'/'));
        let mut length = last;
        while prefix < length {
            if b[length - 1] != b'/' {
                return p[..length].to_string();
            }
            length -= 1;
        }
        if prefix == 0 {
            ".".to_string()
        } else {
            p[..prefix].to_string()
        }
    };
    let mut emit = |text: String| {
        sink.out.extend_from_slice(text.as_bytes());
        sink.out.push(end);
    };
    if is_base {
        if multiple {
            for o in &operands {
                emit(base_of(o, suffix.as_deref()));
            }
        } else {
            let sfx = operands.get(1).map(|s| s.as_str());
            emit(base_of(operands[0], sfx));
        }
    } else {
        for o in &operands {
            emit(dir_of(o));
        }
    }
    Util::Done(0)
}

fn utility(name: &str, args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    match name {
        "echo" => echo(args, sink),
        "printf" => printf(args, sink),
        "test" => Util::Done(test(args, sink, None)),
        "[" => {
            if args.last().map(String::as_str) != Some("]") {
                sink.eline(&format!("{SHELL_NAME}: [: missing `]'"));
                return Util::Done(2);
            }
            Util::Done(test(&args[..args.len() - 1], sink, None))
        }
        "cat" => cat(args, stdin, sink),
        "tee" => tee(args, stdin, sink),
        "mkdir" => mkdir(args, sink),
        "touch" => touch(args, sink),
        "rm" | "rmdir" => rm_utility(name, args, sink),
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
        "basename" => name_tool(true, args, sink),
        "dirname" => name_tool(false, args, sink),
        "yes" => {
            // The line to repeat; an option is `yes`'s own (`--help`).
            if args
                .first()
                .is_some_and(|a| a.len() > 1 && a.starts_with('-'))
            {
                return Util::Decline;
            }
            let mut line = if args.is_empty() {
                "y".to_string()
            } else {
                args.join(" ")
            };
            line.push('\n');
            let want = ENDLESS_BYTES.with(|b| b.get());
            sink.out
                .extend_from_slice(line.repeat((want / line.len()).max(1)).as_bytes());
            ENDLESS_USED.with(|f| f.set(true));
            Util::Done(0)
        }
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
            // A pipeline whose reader stops (`seq 1 1000000000 | head -1`) gets
            // a prefix of the numbers, as many as its first try holds.
            if count > SEQ_MAX as i128 && ENDLESS_ARMED.with(|f| f.get()) {
                let want = ENDLESS_BYTES.with(|b| b.get());
                ENDLESS_USED.with(|f| f.set(true));
                let mut i = first;
                while sink.out.len() < want {
                    sink.out.extend_from_slice(format!("{i}\n").as_bytes());
                    match i.checked_add(step) {
                        Some(n) => i = n,
                        None => break,
                    }
                }
                return Util::Done(0);
            }
            if count > SEQ_MAX as i128 {
                sink.eline(&format!(
                    "seq: {count} numbers is more than {SEQ_MAX}; nothing was printed"
                ));
                SOURCE_REFUSED.with(|f| f.set(true));
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
    unescape(s, false)
}

/// The escapes of `echo -e` and of a `printf` `%b` argument. A `%b` argument
/// (`percent_b`) also reads `\NNN`, one to three octal digits, as the byte;
/// `echo -e` takes only `\0NNN`. Both read `\uHHHH` and `\UHHHHHHHH` (one to
/// four or eight hex digits) as that character, and the bare `\u` and `\U`
/// as written.
fn unescape(s: &str, percent_b: bool) -> (Vec<u8>, bool) {
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
            Some('e' | 'E') => out.push(0x1b),
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
            Some(d @ ('u' | 'U')) => {
                let max = if d == 'u' { 4 } else { 8 };
                let mut v = 0u32;
                let mut digits = 0;
                while digits < max {
                    match it.peek().and_then(|h| h.to_digit(16)) {
                        Some(h) => {
                            v = v * 16 + h;
                            it.next();
                            digits += 1;
                        }
                        None => break,
                    }
                }
                match (digits, char::from_u32(v)) {
                    (0, _) | (_, None) => {
                        out.push(b'\\');
                        out.push(d as u8);
                    }
                    (_, Some(ch)) => {
                        let mut utf8 = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut utf8).as_bytes());
                    }
                }
            }
            // `%b`: `\NNN`, the first digit not a 0, takes up to two more.
            Some(d @ '1'..='7') if percent_b => {
                let mut v = d as u32 - '0' as u32;
                for _ in 0..2 {
                    match it.peek() {
                        Some(o @ '0'..='7') => {
                            v = v * 8 + (*o as u32 - '0' as u32);
                            it.next();
                        }
                        _ => break,
                    }
                }
                out.push(v as u8);
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

/// Field splitting of an unquoted expansion by `ifs`, as POSIX gives it: IFS
/// white space collapses and trims, each other IFS character delimits on its
/// own (`a,,b` is a, "", b), and an empty IFS splits nothing. Returns the
/// fields, whether the value began with a delimiter and whether it ended with
/// one.
fn ifs_split(value: &str, ifs: &str) -> (Vec<String>, bool, bool) {
    if ifs.is_empty() {
        let v = if value.is_empty() {
            vec![]
        } else {
            vec![value.to_string()]
        };
        return (v, false, false);
    }
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n') && ifs.contains(c);
    let is_sep = |c: char| ifs.contains(c);
    let mut out = Vec::new();
    let mut rest = value.trim_start_matches(|c: char| is_ws(c));
    let mut starts = rest.len() != value.len();
    if rest.starts_with(|c: char| is_sep(c)) {
        starts = true;
    }
    let mut trailing = false;
    while !rest.is_empty() {
        let end = rest.find(|c: char| is_sep(c)).unwrap_or(rest.len());
        out.push(rest[..end].to_string());
        rest = &rest[end..];
        trailing = false;
        if rest.is_empty() {
            break;
        }
        let t = rest.trim_start_matches(|c: char| is_ws(c));
        trailing = true;
        rest = t;
        if let Some(c) = rest.chars().next() {
            if is_sep(c) {
                rest = rest[c.len_utf8()..].trim_start_matches(|c: char| is_ws(c));
            }
        }
    }
    (out, starts, trailing)
}

/// `read`'s field splitting, as POSIX gives it: the line as `count` values.
/// Leading and trailing IFS white space is dropped; each run of IFS white space
/// delimits, and each non-white IFS character delimits on its own, so two in a
/// row make an empty field. The last value takes the rest of the line with its
/// trailing IFS white space dropped. An empty IFS splits nothing.
fn read_fields(line: &[(char, bool)], ifs: &str, count: usize) -> Vec<String> {
    // A character that a backslash quoted is never a separator.
    let is_ws =
        |&(c, quoted): &(char, bool)| !quoted && matches!(c, ' ' | '\t' | '\n') && ifs.contains(c);
    let is_sep = |&(c, quoted): &(char, bool)| !quoted && ifs.contains(c);
    let text = |part: &[(char, bool)]| part.iter().map(|&(c, _)| c).collect::<String>();
    let n = line.len();
    let mut out = Vec::with_capacity(count);
    let mut i = 0;
    while i < n && is_ws(&line[i]) {
        i += 1;
    }
    for _ in 1..count {
        let start = i;
        while i < n && !is_sep(&line[i]) {
            i += 1;
        }
        out.push(text(&line[start..i]));
        while i < n && is_ws(&line[i]) {
            i += 1;
        }
        if i < n && is_sep(&line[i]) && !is_ws(&line[i]) {
            i += 1;
            while i < n && is_ws(&line[i]) {
                i += 1;
            }
        }
    }
    let mut end = n;
    while end > i && is_ws(&line[end - 1]) {
        end -= 1;
    }
    out.push(text(&line[i..end]));
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
    let mut out: Vec<u8> = Vec::new();
    // bash's complaints about integer arguments; they go to stderr only once the
    // whole format is done, so a declined format is not reported twice.
    let mut complaints: Vec<(String, bool)> = Vec::new();
    'format: loop {
        let mut consumed = false;
        let mut i = 0;
        while i < fmt.len() {
            let c = fmt[i];
            if c == '\\' && matches!(fmt.get(i + 1), Some('0'..='7')) {
                // `\NNN` (and `\0NNN`) is the byte of up to three octal digits.
                let mut end = i + 1;
                let mut v = 0u32;
                while end < fmt.len() && end < i + 4 && matches!(fmt[end], '0'..='7') {
                    v = v * 8 + (fmt[end] as u32 - '0' as u32);
                    end += 1;
                }
                out.push(v as u8);
                i = end;
                continue;
            }
            if c == '\\' && matches!(fmt.get(i + 1), Some('"' | '\'' | '?')) {
                // In the format (not in a `%b` argument or `echo -e`) `\"`,
                // `\'` and `\?` are the plain character.
                out.push(fmt[i + 1] as u8);
                i += 2;
                continue;
            }
            if c == '\\' {
                // `\xHH` takes up to two hex digits after the `x`.
                let mut end = (i + 2).min(fmt.len());
                // `\xHH`, `\uHHHH` and `\UHHHHHHHH` take their hex digits.
                let digits = match fmt.get(i + 1) {
                    Some('x') => 2,
                    Some('u') => 4,
                    Some('U') => 8,
                    _ => 0,
                };
                while end < fmt.len() && end < i + 2 + digits && fmt[end].is_ascii_hexdigit() {
                    end += 1;
                }
                let (e, stop) = unescape_echo(&fmt[i..end].iter().collect::<String>());
                // Only a `%b` argument takes `\c` as the end of the output; in
                // the format bash prints it as written.
                out.extend_from_slice(if stop { b"\\c" } else { &e });
                i = end;
                continue;
            }
            if c != '%' {
                let mut utf8 = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut utf8).as_bytes());
                i += 1;
                continue;
            }
            i += 1;
            if fmt.get(i) == Some(&'%') {
                out.push(b'%');
                i += 1;
                continue;
            }
            // The C flags, in any order and any number: `-` left-justifies, `+`
            // and a space put a sign on a signed number, `#` the alternate form
            // (`0x`, a leading `0`, a decimal point), `0` pads with zeros.
            let (mut left, mut plus, mut space, mut alt, mut zero) =
                (false, false, false, false, false);
            while let Some(&f) = fmt.get(i) {
                match f {
                    '-' => left = true,
                    '+' => plus = true,
                    ' ' => space = true,
                    '#' => alt = true,
                    '0' => zero = true,
                    _ => break,
                }
                i += 1;
            }
            // What follows the flags: the width, then `.` and the precision.
            // A `*` takes the width, or after the `.` the precision, from the
            // next argument; a negative width means `-` and a width of its
            // absolute value, a negative precision is none at all.
            let mut spec = String::new();
            while let Some(&f) = fmt.get(i) {
                if f.is_ascii_digit() || f == '.' {
                    spec.push(f);
                    i += 1;
                } else if f == '*' {
                    let star = rest.next();
                    consumed |= star.is_some();
                    let n = star.map_or(0, |a| {
                        let (n, complaint) = printf_int_arg(a);
                        note_int_complaint(&mut complaints, a, complaint);
                        n
                    });
                    if spec.contains('.') {
                        if n >= 0 {
                            spec.push_str(&n.to_string());
                        } else {
                            spec.pop();
                        }
                    } else if n < 0 {
                        left = true;
                        spec.push_str(&n.unsigned_abs().to_string());
                    } else {
                        spec.push_str(&n.to_string());
                    }
                    i += 1;
                } else {
                    break;
                }
            }
            // C length modifiers (`%ld`, `%lld`, `%Lf`) mean nothing here.
            while matches!(fmt.get(i), Some('h' | 'l' | 'L' | 'j' | 'z' | 't')) {
                i += 1;
            }
            let conv = fmt.get(i).copied().unwrap_or('s');
            i += 1;
            let arg = rest.next();
            consumed |= arg.is_some();
            let arg = arg.cloned().unwrap_or_default();
            // A width is padding allocated up front: `%999999999999s` asked
            // for a terabyte, and a width too long to parse was read as 0.
            let digits = spec.split('.').next().unwrap_or("");
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
            // A number's sign and `0x` prefix are kept apart from its digits,
            // so the zero padding goes between them.
            let mut sign = "";
            let mut prefix = "";
            let mut numeric = false;
            // The bytes of a `%b` argument, which need not be UTF-8.
            let mut raw: Option<Vec<u8>> = None;
            let mut stop_after = false;
            let body = match conv {
                's' | 'b' => {
                    let text = if conv == 'b' {
                        let (bytes, stop) = unescape(&arg, true);
                        stop_after = stop;
                        let shown = String::from_utf8_lossy(&bytes).into_owned();
                        if precision.is_none() {
                            raw = Some(bytes);
                        }
                        shown
                    } else {
                        arg
                    };
                    match precision {
                        Some(p) => text.chars().take(p).collect(),
                        None => text,
                    }
                }
                'd' | 'i' | 'u' | 'x' | 'X' | 'o' => {
                    // Signed conversions read the argument as `intmax_t`
                    // (`strtoimax`), the others as `uintmax_t` (`strtoumax`):
                    // a value past the type's range is clamped with a warning,
                    // and a negative one for an unsigned conversion wraps.
                    let signed = matches!(conv, 'd' | 'i');
                    let (neg, mag) = if arg.is_empty() {
                        (false, 0)
                    } else if signed {
                        let (n, complaint) = printf_int_arg(&arg);
                        note_int_complaint(&mut complaints, &arg, complaint);
                        (n < 0, n.unsigned_abs())
                    } else {
                        let (u, complaint) = printf_uint_arg(&arg);
                        note_int_complaint(&mut complaints, &arg, complaint);
                        (false, u)
                    };
                    numeric = true;
                    let mut text = match conv {
                        'd' | 'i' | 'u' => mag.to_string(),
                        'x' => format!("{mag:x}"),
                        'X' => format!("{mag:X}"),
                        _ => format!("{mag:o}"),
                    };
                    // `+` and the space flag mean something only for a signed
                    // conversion; `%+x` is `%x`.
                    if signed {
                        sign = if neg {
                            "-"
                        } else if plus {
                            "+"
                        } else if space {
                            " "
                        } else {
                            ""
                        };
                    }
                    // A precision is the least number of digits; `%.0d` of 0 is
                    // no digits at all.
                    if let Some(p) = precision {
                        if p == 0 && mag == 0 {
                            text.clear();
                        } else if text.len() < p {
                            text = format!("{}{text}", "0".repeat(p - text.len()));
                        }
                    }
                    if alt && conv == 'o' && !text.starts_with('0') {
                        text.insert(0, '0');
                    } else if alt && mag != 0 && matches!(conv, 'x' | 'X') {
                        prefix = if conv == 'x' { "0x" } else { "0X" };
                    }
                    text
                }
                'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
                    numeric = true;
                    let upper = conv.is_ascii_uppercase();
                    let p = precision.unwrap_or(6);
                    // bash reads the argument as a long double and glibc
                    // writes that exact value, so a tie in the decimal text
                    // (0.005) is judged on the binary number it became.
                    let lowered = arg.trim().to_ascii_lowercase();
                    let nonfinite = matches!(
                        lowered.trim_start_matches(['+', '-']),
                        "inf" | "infinity" | "nan"
                    );
                    let (negative, mut text) = if nonfinite {
                        // No zero padding: glibc pads these with spaces.
                        zero = false;
                        let word = if lowered.contains("nan") {
                            "nan"
                        } else {
                            "inf"
                        };
                        let word = if upper {
                            word.to_ascii_uppercase()
                        } else {
                            word.to_string()
                        };
                        (lowered.starts_with('-'), word)
                    } else {
                        let ld = if arg.is_empty() {
                            LongDouble {
                                neg: false,
                                m: 0,
                                e: 0,
                            }
                        } else {
                            match LongDouble::parse(&arg) {
                                Some(ld) => ld,
                                None => return Util::Decline,
                            }
                        };
                        let text = match conv {
                            'f' | 'F' => c_fixed(&ld, p),
                            'e' | 'E' => c_exp(&ld, p, upper),
                            _ => c_general(&ld, p, upper, alt),
                        };
                        (ld.neg, text)
                    };
                    // `#` keeps the decimal point of `%.0f` and `%.0e`.
                    if !nonfinite && alt && p == 0 && conv != 'g' && conv != 'G' {
                        let at = text.find(['e', 'E']).unwrap_or(text.len());
                        text.insert(at, '.');
                    }
                    sign = if negative {
                        "-"
                    } else if plus {
                        "+"
                    } else if space {
                        " "
                    } else {
                        ""
                    };
                    text
                }
                // An empty argument is a NUL byte, as bash writes it.
                'c' => arg
                    .chars()
                    .next()
                    .map_or_else(|| "\0".to_string(), String::from),
                'q' => shell_quote_q(&arg),
                _ => return Util::Decline,
            };
            let pad = width.saturating_sub(sign.len() + prefix.len() + body.chars().count());
            // With a precision, the `0` flag is ignored for an integer, as in C.
            let zero_pad = zero
                && numeric
                && !(precision.is_some() && matches!(conv, 'd' | 'i' | 'u' | 'x' | 'X' | 'o'));
            let body_bytes: &[u8] = raw.as_deref().unwrap_or(body.as_bytes());
            if left {
                out.extend_from_slice(sign.as_bytes());
                out.extend_from_slice(prefix.as_bytes());
                out.extend_from_slice(body_bytes);
                out.extend_from_slice(" ".repeat(pad).as_bytes());
            } else if zero_pad {
                out.extend_from_slice(sign.as_bytes());
                out.extend_from_slice(prefix.as_bytes());
                out.extend_from_slice("0".repeat(pad).as_bytes());
                out.extend_from_slice(body_bytes);
            } else {
                out.extend_from_slice(" ".repeat(pad).as_bytes());
                out.extend_from_slice(sign.as_bytes());
                out.extend_from_slice(prefix.as_bytes());
                out.extend_from_slice(body_bytes);
            }
            if stop_after {
                // `\c` in a `%b` argument ends all output, the rest of the
                // format and the later arguments included.
                break 'format;
            }
        }
        if !consumed || rest.len() == 0 {
            break;
        }
    }
    // bash writes a complaint as it reads the argument, before the output.
    for (c, _) in &complaints {
        sink.eline(c);
    }
    sink.out.extend_from_slice(&out);
    Util::Done(if complaints.iter().any(|(_, fatal)| *fatal) {
        1
    } else {
        0
    })
}

/// `printf %q`: the argument quoted so the shell reads it back as one word,
/// as bash writes it. Text with a control character is `$'...'` with C escapes
/// (`\n`, `\t`, `\E`, `\001`); anything else has a backslash before each
/// character the shell treats specially (a `#` only at the start, a `~` at the
/// start or after `:` or `=`), and nothing is `''`.
fn shell_quote_q(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars().any(|c| c.is_ascii_control()) {
        let mut out = String::from("$'");
        for c in s.chars() {
            match c {
                '\x07' => out.push_str("\\a"),
                '\x08' => out.push_str("\\b"),
                '\t' => out.push_str("\\t"),
                '\n' => out.push_str("\\n"),
                '\x0b' => out.push_str("\\v"),
                '\x0c' => out.push_str("\\f"),
                '\r' => out.push_str("\\r"),
                '\x1b' => out.push_str("\\E"),
                '\\' => out.push_str("\\\\"),
                '\'' => out.push_str("\\'"),
                c if c.is_ascii_control() => out.push_str(&format!("\\{:03o}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('\'');
        return out;
    }
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in s.chars() {
        let special = match c {
            ' ' | '!' | '"' | '$' | '&' | '\'' | '(' | ')' | '*' | ',' | ';' | '<' | '>' | '?'
            | '[' | '\\' | ']' | '^' | '`' | '{' | '|' | '}' => true,
            '#' => prev.is_none(),
            '~' => matches!(prev, None | Some(':') | Some('=')),
            _ => false,
        };
        if special {
            out.push('\\');
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// The message of an integer argument's complaint, and whether it makes
/// `printf` exit 1: a value out of range is only a warning.
const PRINTF_RANGE: &str = "Numerical result out of range";

fn note_int_complaint(complaints: &mut Vec<(String, bool)>, arg: &str, c: Option<&'static str>) {
    match c {
        Some(PRINTF_RANGE) => complaints.push((
            format!("printf: warning: {arg}: {c}", c = PRINTF_RANGE),
            false,
        )),
        Some(c) => complaints.push((format!("printf: {arg}: {c}"), true)),
        None => {}
    }
}

/// How bash reads an integer argument of `printf`. A leading quote gives the
/// code of the character after it. Otherwise the text is a C integer constant
/// (`strtoimax`/`strtoumax` in base 0): leading blanks, an optional sign, `0x`
/// for hex, a leading `0` for octal, decimal otherwise. The result is the sign
/// and the magnitude read before any text that is not part of the number
/// (capped at 2^64), and the complaint is bash's words for that text.
fn printf_int_parts(arg: &str) -> (bool, u128, Option<&'static str>) {
    if let Some(rest) = arg.strip_prefix(['\'', '"']) {
        return (
            false,
            rest.chars().next().map_or(0, |c| c as u32 as u128),
            None,
        );
    }
    let body = arg.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
    let (neg, body) = match body.as_bytes().first().copied() {
        Some(b'-') => (true, &body[1..]),
        Some(b'+') => (false, &body[1..]),
        _ => (false, body),
    };
    let hex = body.starts_with("0x") || body.starts_with("0X");
    let (radix, prefix) = if hex {
        (16, 2)
    } else if body.starts_with('0') {
        (8, 1)
    } else {
        (10, 0)
    };
    let digits = &body[prefix..];
    let n = digits
        .bytes()
        .take_while(|b| (*b as char).is_digit(radix))
        .count();
    // In hex with no digit after `0x`, only the `0` is read.
    let (value_digits, rest) = if radix == 16 && n == 0 {
        ("", &body[1..])
    } else if radix == 10 && n == 0 {
        ("", body)
    } else {
        (&digits[..n], &digits[n..])
    };
    let complaint = if radix == 10 && n == 0 {
        Some("invalid number")
    } else if rest.is_empty() {
        None
    } else if hex {
        Some("invalid hex number")
    } else if radix == 8 && body.as_bytes().get(1).is_some_and(u8::is_ascii_digit) {
        Some("invalid octal number")
    } else {
        Some("invalid number")
    };
    // Accumulated saturating and capped at 2^64, so it cannot wrap.
    let mut mag: u128 = 0;
    for c in value_digits.chars() {
        let digit = c.to_digit(radix).unwrap_or(0);
        mag = mag
            .saturating_mul(radix as u128)
            .saturating_add(u128::from(digit));
    }
    (neg, mag.min(1u128 << 64), complaint)
}

/// A signed conversion's argument (bash's `getintmax`): past 64 bits the value
/// is clamped to `i64::MAX` or `i64::MIN` with a range warning.
fn printf_int_arg(arg: &str) -> (i64, Option<&'static str>) {
    let (neg, mag, complaint) = printf_int_parts(arg);
    let value = if neg { -(mag as i128) } else { mag as i128 };
    if value > i64::MAX as i128 || value < i64::MIN as i128 {
        let clamped = if neg { i64::MIN } else { i64::MAX };
        return (clamped, complaint.or(Some(PRINTF_RANGE)));
    }
    (value as i64, complaint)
}

/// An unsigned conversion's argument (`getuintmax`): a negative value wraps
/// to 64 bits, and a magnitude past 64 bits gives `u64::MAX` and a warning.
fn printf_uint_arg(arg: &str) -> (u64, Option<&'static str>) {
    let (neg, mag, complaint) = printf_int_parts(arg);
    if mag > u128::from(u64::MAX) {
        return (u64::MAX, complaint.or(Some(PRINTF_RANGE)));
    }
    let mag = mag as u64;
    (if neg { mag.wrapping_neg() } else { mag }, complaint)
}

/// A little-endian big unsigned integer, for `printf`'s exact floating-point
/// conversions: a decimal argument is rounded to the 64-bit mantissa of the
/// x87 `long double` that bash reads it as, and that value's exact binary
/// expansion is then written out to the requested number of digits.
#[derive(Clone)]
struct Big(Vec<u32>);

impl Big {
    fn from_u128(mut v: u128) -> Big {
        let mut limbs = Vec::new();
        while v != 0 {
            limbs.push(v as u32);
            v >>= 32;
        }
        Big(limbs)
    }
    fn is_zero(&self) -> bool {
        self.0.is_empty()
    }
    fn trim(&mut self) {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }
    fn bitlen(&self) -> usize {
        match self.0.last() {
            None => 0,
            Some(&top) => self.0.len() * 32 - top.leading_zeros() as usize,
        }
    }
    fn bit(&self, i: usize) -> bool {
        self.0.get(i / 32).is_some_and(|l| l >> (i % 32) & 1 == 1)
    }
    fn set_bit(&mut self, i: usize) {
        if self.0.len() <= i / 32 {
            self.0.resize(i / 32 + 1, 0);
        }
        self.0[i / 32] |= 1 << (i % 32);
    }
    fn clear_bit(&mut self, i: usize) {
        if let Some(l) = self.0.get_mut(i / 32) {
            *l &= !(1 << (i % 32));
        }
        self.trim();
    }
    /// Whether any bit below `n` is set.
    fn any_below(&self, n: usize) -> bool {
        let (whole, part) = (n / 32, n % 32);
        self.0.iter().take(whole).any(|&l| l != 0)
            || (part != 0
                && self
                    .0
                    .get(whole)
                    .is_some_and(|l| l & ((1u32 << part) - 1) != 0))
    }
    fn shl(&self, n: usize) -> Big {
        if self.is_zero() {
            return Big(Vec::new());
        }
        let (whole, part) = (n / 32, n % 32);
        let mut out = vec![0u32; whole];
        let mut carry = 0u32;
        for &l in &self.0 {
            if part == 0 {
                out.push(l);
            } else {
                out.push(l << part | carry);
                carry = l >> (32 - part);
            }
        }
        if carry != 0 {
            out.push(carry);
        }
        Big(out)
    }
    fn shr(&self, n: usize) -> Big {
        let (whole, part) = (n / 32, n % 32);
        if whole >= self.0.len() {
            return Big(Vec::new());
        }
        let src = &self.0[whole..];
        let mut out = Vec::with_capacity(src.len());
        for (i, &l) in src.iter().enumerate() {
            let hi = src.get(i + 1).copied().unwrap_or(0);
            out.push(if part == 0 {
                l
            } else {
                l >> part | hi << (32 - part)
            });
        }
        let mut b = Big(out);
        b.trim();
        b
    }
    fn mul_small(&mut self, k: u32) {
        let mut carry = 0u64;
        for l in &mut self.0 {
            let v = u64::from(*l) * u64::from(k) + carry;
            *l = v as u32;
            carry = v >> 32;
        }
        if carry != 0 {
            self.0.push(carry as u32);
        }
    }
    fn add_small(&mut self, k: u32) {
        let mut carry = u64::from(k);
        for l in &mut self.0 {
            if carry == 0 {
                return;
            }
            let v = u64::from(*l) + carry;
            *l = v as u32;
            carry = v >> 32;
        }
        if carry != 0 {
            self.0.push(carry as u32);
        }
    }
    /// Divide by `k` in place; the remainder.
    fn divmod_small(&mut self, k: u32) -> u32 {
        let mut rem = 0u64;
        for l in self.0.iter_mut().rev() {
            let cur = rem << 32 | u64::from(*l);
            *l = (cur / u64::from(k)) as u32;
            rem = cur % u64::from(k);
        }
        self.trim();
        rem as u32
    }
    fn cmp(&self, o: &Big) -> std::cmp::Ordering {
        self.0
            .len()
            .cmp(&o.0.len())
            .then_with(|| self.0.iter().rev().cmp(o.0.iter().rev()))
    }
    /// `self - o`, which must not be negative.
    fn sub_assign(&mut self, o: &Big) {
        let mut borrow = 0i64;
        for (i, l) in self.0.iter_mut().enumerate() {
            let v = i64::from(*l) - i64::from(o.0.get(i).copied().unwrap_or(0)) - borrow;
            *l = v as u32;
            borrow = i64::from(v < 0);
        }
        self.trim();
    }
    fn low_u64(&self) -> u64 {
        let at = |i: usize| u64::from(self.0.get(i).copied().unwrap_or(0));
        at(0) | at(1) << 32
    }
    /// Quotient and remainder, by shifting and subtracting.
    fn divmod(&self, d: &Big) -> (Big, Big) {
        if self.cmp(d) == std::cmp::Ordering::Less {
            return (Big(Vec::new()), self.clone());
        }
        let shift = self.bitlen() - d.bitlen();
        let mut divisor = d.shl(shift);
        let mut rem = self.clone();
        let mut quot = Big(Vec::new());
        for i in (0..=shift).rev() {
            if rem.cmp(&divisor) != std::cmp::Ordering::Less {
                rem.sub_assign(&divisor);
                quot.set_bit(i);
            }
            divisor = divisor.shr(1);
        }
        (quot, rem)
    }
    /// The decimal digits, most significant first (`[0]` for zero).
    fn decimal(&self) -> Vec<u8> {
        if self.is_zero() {
            return vec![0];
        }
        let mut n = self.clone();
        let mut out = Vec::new();
        while !n.is_zero() {
            let mut chunk = n.divmod_small(1_000_000_000);
            for _ in 0..9 {
                out.push((chunk % 10) as u8);
                chunk /= 10;
                if n.is_zero() && chunk == 0 {
                    break;
                }
            }
        }
        while out.len() > 1 && out.last() == Some(&0) {
            out.pop();
        }
        out.reverse();
        out
    }
}

/// A number as x87 `long double` holds it: `m * 2^e` with `m` a 64-bit
/// mantissa (zero when `m` is 0), and a sign that a zero keeps.
struct LongDouble {
    neg: bool,
    m: u64,
    e: i32,
}

/// The lowest exponent of a long double's mantissa (a denormal's last bit).
const LD_MIN_EXP: i32 = -16445;
/// The largest exponent of its top bit.
const LD_MAX_TOP: i32 = 16383;

/// `n` rounded to 64 significant bits, ties to even; `sticky` says bits below
/// `n` were dropped already. The mantissa and the exponent of its lowest bit
/// (`e0` is the exponent of `n`'s own lowest bit).
fn ld_round(n: &Big, sticky: bool, e0: i32) -> (u64, i32) {
    let bl = n.bitlen();
    if bl <= 64 {
        let up = 64 - bl;
        return (n.low_u64() << up, e0 - up as i32);
    }
    let sh = bl - 64;
    let mut m = u128::from(n.shr(sh).low_u64());
    let guard = n.bit(sh - 1);
    let rest = sticky || n.any_below(sh - 1);
    if guard && (rest || m & 1 == 1) {
        m += 1;
    }
    let mut e = e0 + sh as i32;
    if m >> 64 != 0 {
        m >>= 1;
        e += 1;
    }
    (m as u64, e)
}

impl LongDouble {
    /// What bash's `strtold` reads from a decimal argument (blanks, a sign,
    /// digits with an optional point and an `e` exponent). `None` for other
    /// text, and for a value past a long double's range.
    fn parse(text: &str) -> Option<LongDouble> {
        let t = text.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
        let (neg, t) = match t.as_bytes().first() {
            Some(b'-') => (true, &t[1..]),
            Some(b'+') => (false, &t[1..]),
            _ => (false, t),
        };
        let b = t.as_bytes();
        let mut i = 0;
        let mut digits: Vec<u8> = Vec::new();
        let mut frac = 0i64;
        let mut seen_point = false;
        while i < b.len() {
            match b[i] {
                d @ b'0'..=b'9' => {
                    digits.push(d - b'0');
                    if seen_point {
                        frac += 1;
                    }
                }
                b'.' if !seen_point => seen_point = true,
                _ => break,
            }
            i += 1;
        }
        if digits.is_empty() {
            return None;
        }
        let mut exp10 = 0i64;
        if i < b.len() && matches!(b[i], b'e' | b'E') {
            i += 1;
            let eneg = match b.get(i) {
                Some(b'-') => {
                    i += 1;
                    true
                }
                Some(b'+') => {
                    i += 1;
                    false
                }
                _ => false,
            };
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                exp10 = (exp10 * 10 + i64::from(b[i] - b'0')).min(1_000_000);
                i += 1;
            }
            if i == start {
                return None;
            }
            if eneg {
                exp10 = -exp10;
            }
        }
        if i != b.len() {
            return None;
        }
        let lead = digits.iter().take_while(|&&d| d == 0).count();
        let digits = &digits[lead..];
        if digits.is_empty() {
            return Some(LongDouble { neg, m: 0, e: 0 });
        }
        let dec_exp = exp10 - frac;
        // The value is below the smallest denormal, or above the largest finite.
        let magnitude = digits.len() as i64 + dec_exp;
        if magnitude < -4960 {
            return Some(LongDouble { neg, m: 0, e: 0 });
        }
        if magnitude > 4934 {
            return None;
        }
        let mut d = Big(Vec::new());
        for chunk in digits.chunks(9) {
            let mut scale = 1u32;
            let mut v = 0u32;
            for &x in chunk {
                scale *= 10;
                v = v * 10 + u32::from(x);
            }
            d.mul_small(scale);
            d.add_small(v);
        }
        let (mut m, mut e);
        if dec_exp >= 0 {
            let mut n = d;
            let mut left = dec_exp;
            while left > 0 {
                let step = left.min(9);
                n.mul_small(10u32.pow(step as u32));
                left -= step;
            }
            (m, e) = ld_round(&n, false, 0);
        } else {
            let mut den = Big::from_u128(1);
            let mut left = -dec_exp;
            while left > 0 {
                let step = left.min(9);
                den.mul_small(10u32.pow(step as u32));
                left -= step;
            }
            // Scale so the quotient has 66 or 67 bits, then round it.
            let s = 66 + den.bitlen() as i64 - d.bitlen() as i64;
            let (q, r, e0) = if s >= 0 {
                let (q, r) = d.shl(s as usize).divmod(&den);
                (q, r, -(s as i32))
            } else {
                let t = (-s) as usize;
                let (q, r) = d.divmod(&den.shl(t));
                (q, r, t as i32)
            };
            (m, e) = ld_round(&q, !r.is_zero(), e0);
        }
        if e + 63 > LD_MAX_TOP {
            return None;
        }
        if e < LD_MIN_EXP {
            // A denormal has fewer bits: round at the lowest one it has.
            let drop = (LD_MIN_EXP - e) as u32;
            if drop > 64 {
                m = 0;
            } else if drop == 64 {
                m = u64::from(m > 1 << 63);
            } else {
                let kept = m >> drop;
                let rem = m & ((1u64 << drop) - 1);
                let half = 1u64 << (drop - 1);
                m = kept + u64::from(rem > half || (rem == half && kept & 1 == 1));
            }
            e = LD_MIN_EXP;
        }
        Some(LongDouble { neg, m, e })
    }

    /// The exact decimal expansion: digits before the point and after it.
    fn exact_digits(&self) -> (Vec<u8>, Vec<u8>) {
        if self.m == 0 {
            return (vec![0], Vec::new());
        }
        let (mut m, mut e) = (self.m, self.e);
        let tz = m.trailing_zeros() as i32;
        let cut = tz.min(-e).max(0);
        m >>= cut;
        e += cut;
        if e >= 0 {
            return (
                Big::from_u128(u128::from(m)).shl(e as usize).decimal(),
                Vec::new(),
            );
        }
        let k = (-e) as usize;
        let whole = Big::from_u128(u128::from(m)).shr(k);
        let mut int_digits = whole.decimal();
        if whole.is_zero() {
            int_digits = vec![0];
        }
        // The fraction: the low `k` bits of `m`, as a fraction of 2^k.
        let mut fraction = Big::from_u128(u128::from(m));
        for i in k..64.max(k) {
            fraction.clear_bit(i);
        }
        let mut frac_digits = Vec::new();
        while !fraction.is_zero() {
            fraction.mul_small(10);
            let mut digit = 0u8;
            for j in 0..4 {
                if fraction.bit(k + j) {
                    digit |= 1 << j;
                    fraction.clear_bit(k + j);
                }
            }
            frac_digits.push(digit);
        }
        (int_digits, frac_digits)
    }
}

/// Round the digit string `digits` to `keep` digits, half to even on the exact
/// remainder. `true` when a carry added a digit (`99` to `100`).
fn round_digits(digits: &mut Vec<u8>, keep: usize) -> bool {
    if digits.len() <= keep {
        digits.resize(keep, 0);
        return false;
    }
    let tail = digits.split_off(keep);
    let up = match tail[0].cmp(&5) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => {
            tail[1..].iter().any(|&d| d != 0) || digits.last().is_some_and(|d| d % 2 == 1)
        }
    };
    if up {
        for d in digits.iter_mut().rev() {
            if *d == 9 {
                *d = 0;
            } else {
                *d += 1;
                return false;
            }
        }
        digits.insert(0, 1);
        return true;
    }
    false
}

fn digit_text(d: &[u8]) -> String {
    d.iter().map(|&x| char::from(b'0' + x)).collect()
}

/// C's `%f` of a long double, without the sign.
fn c_fixed(v: &LongDouble, precision: usize) -> String {
    let (int_digits, frac_digits) = v.exact_digits();
    let mut all = int_digits.clone();
    all.extend_from_slice(&frac_digits);
    round_digits(&mut all, int_digits.len() + precision);
    // A carry that made a new leading digit leaves one more before the point.
    let int_len = all.len() - precision;
    let mut out = digit_text(&all[..int_len]);
    if precision > 0 {
        out.push('.');
        out.push_str(&digit_text(&all[int_len..]));
    }
    out
}

/// The `%e` digits of `v` at `precision` decimals, and the decimal exponent.
fn exp_digits(v: &LongDouble, precision: usize) -> (Vec<u8>, i32) {
    let (int_digits, frac_digits) = v.exact_digits();
    let mut all = int_digits.clone();
    all.extend_from_slice(&frac_digits);
    let Some(first) = all.iter().position(|&d| d != 0) else {
        return (vec![0; precision + 1], 0);
    };
    let mut x = int_digits.len() as i32 - 1 - first as i32;
    let mut sig = all.split_off(first);
    if round_digits(&mut sig, precision + 1) {
        // 9.99 became 10.0: one digit too many.
        sig.pop();
        x += 1;
    }
    (sig, x)
}

/// C's `%e` of a long double, without the sign: one digit, the decimals, `e`,
/// a sign and at least two exponent digits (`1.234568e+04`).
fn c_exp(v: &LongDouble, precision: usize, upper: bool) -> String {
    let (digits, x) = exp_digits(v, precision);
    let mut out = digit_text(&digits[..1]);
    if precision > 0 {
        out.push('.');
        out.push_str(&digit_text(&digits[1..]));
    }
    out.push(if upper { 'E' } else { 'e' });
    out.push(if x < 0 { '-' } else { '+' });
    out.push_str(&format!("{:02}", x.abs()));
    out
}

/// C's `%g`: `precision` significant digits (6 by default, 1 for 0). The `%e`
/// form is used when the exponent is below -4 or at least the precision, else
/// `%f`; trailing zeros of the fraction are removed, unless `alt` (the `#`
/// flag) keeps them and the decimal point.
fn c_general(v: &LongDouble, precision: usize, upper: bool, alt: bool) -> String {
    let p = precision.max(1);
    // The exponent `%e` would show with this precision decides the form.
    let (_, x) = exp_digits(v, p - 1);
    fn strip_zeros(s: &str) -> String {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s.to_string()
        }
    }
    let strip = |s: &str| {
        if alt {
            // The point stays even with no digit after it (`%#.1g` of 1e10).
            if s.contains('.') {
                s.to_string()
            } else {
                format!("{s}.")
            }
        } else {
            strip_zeros(s)
        }
    };
    if x < -4 || x >= p as i32 {
        // glibc picks the `%e` form after rounding but keeps the decimals
        // the `%f` form would have had (`%#g` of 999999.5 is `1.e+06`).
        let mut decimals = p - 1;
        if alt {
            let (int_digits, frac_digits) = v.exact_digits();
            let mut all = int_digits.clone();
            all.extend_from_slice(&frac_digits);
            if let Some(first) = all.iter().position(|&d| d != 0) {
                let before = int_digits.len() as i32 - 1 - first as i32;
                if before < p as i32 && before >= -4 {
                    decimals = (p as i32 - 1 - before) as usize;
                }
            }
        }
        let s = c_exp(v, decimals, upper);
        let at = s.find(['e', 'E']).unwrap_or(s.len());
        format!("{}{}", strip(&s[..at]), &s[at..])
    } else {
        let decimals = (p as i32 - 1 - x) as usize;
        strip(&c_fixed(v, decimals))
    }
}

/// `test -b`, `-c`, `-p` and `-S`: a block device, character device, FIFO or
/// socket. Windows has none of them, so the answer there is no.
fn special_file(op: &str, md: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let ft = md.file_type();
        match op {
            "-b" => ft.is_block_device(),
            "-c" => ft.is_char_device(),
            "-p" => ft.is_fifo(),
            _ => ft.is_socket(),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (op, md);
        false
    }
}

/// `test -O`, `-G`, `-u`, `-g`, `-k` and `-N`: owned by the effective user or
/// group, set-user-ID, set-group-ID, sticky, or modified since last read. They
/// are mode and ownership facts Windows files do not have, so the answer there
/// is no.
fn file_property(op: &str, md: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match op {
            // SAFETY: geteuid and getegid take no arguments and cannot fail.
            "-O" => md.uid() == unsafe { libc::geteuid() },
            "-G" => md.gid() == unsafe { libc::getegid() },
            "-u" => md.mode() & 0o4000 != 0,
            "-g" => md.mode() & 0o2000 != 0,
            "-k" => md.mode() & 0o1000 != 0,
            _ => (md.mtime(), md.mtime_nsec()) > (md.atime(), md.atime_nsec()),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (op, md);
        false
    }
}

/// The operators `test` reads between two operands.
fn test_binary_op(op: &str) -> bool {
    matches!(
        op,
        "=" | "=="
            | "!="
            | "<"
            | ">"
            | "-nt"
            | "-ot"
            | "-ef"
            | "-eq"
            | "-ne"
            | "-lt"
            | "-le"
            | "-gt"
            | "-ge"
    )
}

/// `test` and `[`, as bash's `test.c` reads them: with up to four arguments
/// the POSIX rules by argument count (where `!` and a parenthesis bind to what
/// follows them), with more a recursive parse (`!` over `-a` over `-o`, and
/// parentheses). Both sides of `-a` and `-o` are always evaluated, so an
/// error on the right still shows when the left already decides.
fn test(args: &[String], sink: &mut Sink, shell: Option<&Shell>) -> i32 {
    fn unary(op: &str, a: &str, shell: Option<&Shell>) -> Option<bool> {
        let md = std::fs::metadata(a);
        // The devices bash always has, which Windows lacks: they exist, can
        // be read and written, and `/dev/null` and the others are character
        // devices; none holds data or is a regular file.
        if md.is_err()
            && matches!(
                a,
                "/dev/null" | "/dev/zero" | "/dev/urandom" | "/dev/random"
            )
        {
            match op {
                "-e" | "-a" | "-c" | "-r" | "-w" => return Some(true),
                "-f" | "-d" | "-s" | "-b" | "-p" | "-S" | "-x" => return Some(false),
                _ => {}
            }
        }
        Some(match op {
            "-e" | "-a" => md.is_ok(),
            "-f" => md.map(|m| m.is_file()).unwrap_or(false),
            "-d" => md.map(|m| m.is_dir()).unwrap_or(false),
            "-s" => md.map(|m| m.len() > 0).unwrap_or(false),
            "-r" | "-w" | "-x" => md.is_ok(),
            "-b" | "-c" | "-p" | "-S" => md.as_ref().map(|m| special_file(op, m)).unwrap_or(false),
            "-O" | "-G" | "-u" | "-g" | "-k" | "-N" => {
                md.as_ref().map(|m| file_property(op, m)).unwrap_or(false)
            }
            // A terminal on that descriptor; any other number or text is not one.
            "-t" => {
                use std::io::IsTerminal;
                match a.trim().parse::<i32>() {
                    Ok(0) => std::io::stdin().is_terminal(),
                    Ok(1) => std::io::stdout().is_terminal(),
                    Ok(2) => std::io::stderr().is_terminal(),
                    _ => false,
                }
            }
            "-L" | "-h" => std::fs::symlink_metadata(a)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false),
            "-z" => a.is_empty(),
            "-n" => !a.is_empty(),
            // A variable that is set (an array element is not in the subset).
            "-v" => {
                valid_name(a) && shell.is_some_and(|s| s.var(a).is_some())
                    || (!a.is_empty()
                        && a.bytes().all(|b| b.is_ascii_digit())
                        && shell.is_some_and(|s| s.var(a).is_some()))
            }
            // A shell option that is on: the three `set` runs, and the ones
            // bash has on in a script.
            "-o" => match a {
                "errexit" => shell.is_some_and(|s| s.errexit),
                "nounset" => shell.is_some_and(|s| s.nounset),
                "pipefail" => shell.is_some_and(|s| s.pipefail),
                "braceexpand" | "hashall" | "interactive-comments" => true,
                _ => false,
            },
            // A name reference: the subset has none.
            "-R" => false,
            _ => return None,
        })
    }
    fn is_unop(op: &str) -> bool {
        let b = op.as_bytes();
        b.len() == 2 && b[0] == b'-' && b"abcdefghknoprstuvwxzGLNOSR".contains(&b[1])
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
            // `\<` and `\>`: byte order, as in the C locale.
            "<" => Ok(a.as_bytes() < b.as_bytes()),
            ">" => Ok(a.as_bytes() > b.as_bytes()),
            // File times: a file that exists is newer than one that does not.
            "-nt" | "-ot" => {
                let time = |p: &str| std::fs::metadata(p).and_then(|m| m.modified()).ok();
                let (x, y) = (time(a), time(b));
                Ok(match (op, x, y) {
                    ("-nt", Some(x), Some(y)) => x > y,
                    ("-nt", Some(_), None) => true,
                    ("-ot", Some(x), Some(y)) => x < y,
                    ("-ot", None, Some(_)) => true,
                    _ => false,
                })
            }
            // The same file: both exist and resolve to one path.
            "-ef" => Ok(match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
                (Ok(x), Ok(y)) => x == y,
                _ => false,
            }),
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
    struct P<'a> {
        a: &'a [String],
        pos: usize,
        shell: Option<&'a Shell>,
        sink: &'a mut Sink,
        expr_used: bool,
    }
    type R = Result<bool, i32>;
    impl P<'_> {
        fn syntax<T>(&mut self, msg: &str) -> Result<T, i32> {
            self.sink.eline(&format!("{SHELL_NAME}: test: {msg}"));
            Err(2)
        }
        fn at(&self, k: usize) -> &str {
            self.a.get(k).map(String::as_str).unwrap_or("")
        }
        /// `op ARG`, with the jail's refusal for a path outside it.
        fn unary_test(&mut self, op: &str, arg: &str) -> R {
            let file_test = matches!(
                op,
                "-e" | "-a"
                    | "-f"
                    | "-d"
                    | "-s"
                    | "-r"
                    | "-w"
                    | "-x"
                    | "-L"
                    | "-h"
                    | "-b"
                    | "-c"
                    | "-p"
                    | "-S"
                    | "-O"
                    | "-G"
                    | "-u"
                    | "-g"
                    | "-k"
                    | "-N"
            );
            if let Some(refusal) = file_test.then(|| jail_refusal("test", arg)).flatten() {
                self.sink.eline(&refusal);
                return Err(2);
            }
            match unary(op, arg, self.shell) {
                Some(v) => Ok(v),
                None => self.syntax(&format!("{op}: unary operator expected")),
            }
        }
        /// `A OP B` at `pos`; advances past all three.
        fn binary_operator(&mut self) -> R {
            let (x, op, y) = (
                self.at(self.pos).to_string(),
                self.at(self.pos + 1).to_string(),
                self.at(self.pos + 2).to_string(),
            );
            self.pos += 3;
            if matches!(op.as_str(), "-nt" | "-ot" | "-ef") {
                for p in [&x, &y] {
                    if let Some(refusal) = jail_refusal("test", p) {
                        self.sink.eline(&refusal);
                        return Err(2);
                    }
                }
            }
            match binary(&x, &op, &y) {
                Some(Ok(v)) => Ok(v),
                Some(Err(m)) => self.syntax(&m),
                None => self.syntax(&format!("{op}: binary operator expected")),
            }
        }
        /// `-X ARG` at `pos`; advances past both.
        fn unary_operator(&mut self) -> R {
            let op = self.at(self.pos).to_string();
            if !is_unop(&op) {
                return Ok(false);
            }
            // `-t` alone tests descriptor 1.
            if op == "-t" {
                self.pos += 1;
                if self.pos < self.a.len() {
                    let v = self.at(self.pos).trim().parse::<i64>().is_ok();
                    let arg = self.at(self.pos).to_string();
                    self.pos += 1;
                    return if v {
                        self.unary_test(&op, &arg)
                    } else {
                        Ok(false)
                    };
                }
                return self.unary_test(&op, "1");
            }
            if self.pos + 1 >= self.a.len() {
                return self.syntax("argument expected");
            }
            let arg = self.at(self.pos + 1).to_string();
            self.pos += 2;
            self.unary_test(&op, &arg)
        }
        fn expr(&mut self) -> R {
            self.expr_used = true;
            if self.pos >= self.a.len() {
                return self.syntax("argument expected");
            }
            self.or()
        }
        fn or(&mut self) -> R {
            let value = self.and()?;
            if self.pos < self.a.len() && self.at(self.pos) == "-o" {
                self.pos += 1;
                let rest = self.or()?;
                return Ok(rest || value);
            }
            Ok(value)
        }
        fn and(&mut self) -> R {
            let value = self.term()?;
            if self.pos < self.a.len() && self.at(self.pos) == "-a" {
                self.pos += 1;
                let rest = self.and()?;
                return Ok(rest && value);
            }
            Ok(value)
        }
        fn term(&mut self) -> R {
            if self.pos >= self.a.len() {
                return self.syntax("argument expected");
            }
            // Leading `!`s.
            if self.at(self.pos) == "!" {
                let mut not = false;
                while self.pos < self.a.len() && self.at(self.pos) == "!" {
                    self.pos += 1;
                    not = !not;
                }
                let v = self.term()?;
                return Ok(if not { !v } else { v });
            }
            if self.at(self.pos) == "(" {
                self.pos += 1;
                let v = self.expr()?;
                if self.pos >= self.a.len() {
                    return self.syntax("`)' expected");
                }
                if self.at(self.pos) != ")" {
                    let found = self.at(self.pos).to_string();
                    return self.syntax(&format!("`)' expected, found {found}"));
                }
                self.pos += 1;
                return Ok(v);
            }
            if self.pos + 3 <= self.a.len() && test_binary_op(self.at(self.pos + 1)) {
                return self.binary_operator();
            }
            let cur = self.at(self.pos).to_string();
            if cur.len() == 2 && cur.starts_with('-') {
                if !is_unop(&cur) {
                    return self.syntax(&format!("{cur}: unary operator expected"));
                }
                // An operator with nothing left to test is a plain word.
                if self.pos + 1 < self.a.len() {
                    return self.unary_operator();
                }
            }
            self.pos += 1;
            Ok(!cur.is_empty())
        }
        fn two(&mut self) -> R {
            let first = self.at(self.pos).to_string();
            if first == "!" {
                return Ok(self.at(self.pos + 1).is_empty());
            }
            if is_unop(&first) {
                return self.unary_operator();
            }
            self.syntax(&format!("{first}: unary operator expected"))
        }
        fn three(&mut self) -> R {
            let (first, second) = (
                self.at(self.pos).to_string(),
                self.at(self.pos + 1).to_string(),
            );
            if test_binary_op(&second) {
                self.binary_operator()
            } else if second == "-a" || second == "-o" {
                // Three words with `-a` or `-o` between: the outer two are
                // strings, whatever they look like (`[ -n -a x ]`).
                let (x, y) = (!first.is_empty(), !self.at(self.pos + 2).is_empty());
                Ok(if second == "-a" { x && y } else { x || y })
            } else if first == "!" {
                self.pos += 1;
                Ok(!self.two()?)
            } else if first == "(" && self.at(self.pos + 2) == ")" {
                Ok(!second.is_empty())
            } else {
                self.syntax(&format!("{second}: binary operator expected"))
            }
        }
        fn four(&mut self) -> R {
            if self.at(self.pos) == "!" {
                self.pos += 1;
                return Ok(!self.three()?);
            }
            if self.at(self.pos) == "(" && self.at(self.pos + 3) == ")" {
                self.pos += 1;
                return self.two();
            }
            self.expr()
        }
        fn posix(&mut self) -> R {
            match self.a.len() - self.pos {
                0 => Ok(false),
                1 => Ok(!self.at(self.pos).is_empty()),
                2 => self.two(),
                3 => self.three(),
                4 => self.four(),
                _ => self.expr(),
            }
        }
    }
    let mut p = P {
        a: args,
        pos: 0,
        shell,
        sink,
        expr_used: false,
    };
    let r = p.posix();
    // The count rules take every word; only the parse leaves words over.
    if r.is_ok() && !p.expr_used {
        p.pos = args.len();
    }
    let r = match r {
        Ok(v) if p.pos < args.len() => p.syntax("too many arguments").map(|()| v),
        other => other,
    };
    match r {
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
    // Each file is read when its turn comes, so an error for one is written
    // between the output of its neighbours, in the order GNU cat writes them
    // (it matters when standard error goes where standard output goes).
    let mut inputs: Vec<Vec<String>> = files.iter().map(|f| vec![f.clone()]).collect();
    if inputs.is_empty() {
        inputs.push(Vec::new());
    }
    for one in &inputs {
        for (_, data) in read_inputs_dirs("cat", one, stdin, sink, &mut status) {
            cat_emit(&data, number, &mut line_no, sink);
        }
    }
    Util::Done(status)
}

/// What `cat` writes for one input: the bytes, or with `-n` each line numbered.
fn cat_emit(data: &[u8], number: bool, line_no: &mut usize, sink: &mut Sink) {
    if !number {
        sink.out.extend_from_slice(data);
        return;
    }
    // GNU `cat -n`: a number six wide and a tab before each line, counted
    // across every file. A last line with no newline gets none added.
    for line in data.split_inclusive(|&b| b == b'\n') {
        *line_no += 1;
        sink.out
            .extend_from_slice(format!("{:>6}\t", *line_no).as_bytes());
        sink.out.extend_from_slice(line);
    }
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
        // The names bash and Linux give the standard streams.
        if f == "/dev/stdout" || f == "/dev/fd/1" {
            sink.out.extend_from_slice(&data);
            continue;
        }
        if f == "/dev/stderr" || f == "/dev/fd/2" {
            sink.ewrite(&data);
            continue;
        }
        let resolved = match check_write_in_place(f) {
            Ok(p) => p,
            Err(msg) => {
                sink.eline(&gnu_refusal(&format!("tee: {f}"), &msg));
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

/// Record, before `mkdir` runs, the directories it will create under `path`:
/// the missing ones outermost first (only `path` itself without `-p`). Nothing
/// is recorded when the journal is off or the directory exists.
fn record_created_dirs(path: &str, parents: bool) {
    let target = crate::journal::normalized_path(path);
    let mut created: Vec<String> = Vec::new();
    let mut cursor = Some(Path::new(&target));
    while let Some(dir) = cursor {
        if dir.as_os_str().is_empty() || dir.exists() {
            break;
        }
        created.push(dir.display().to_string());
        if !parents {
            break;
        }
        cursor = dir.parent();
    }
    created.reverse();
    if !created.is_empty() {
        crate::journal::record_group("mkdir", crate::safety::Effect::WriteLocal, created);
    }
}

/// `mkdir [-p] DIR...`: each directory is judged by the jail as a `>` target is,
/// so a directory outside the workspace is refused and nothing is made.
fn mkdir(args: &[String], sink: &mut Sink) -> Util {
    let Some((flags, dirs)) = short_flags(args, "p") else {
        return Util::Decline;
    };
    if dirs.is_empty() {
        sink.eline("mkdir: missing operand\nTry 'mkdir --help' for more information.");
        return Util::Done(1);
    }
    let parents = flags.contains(&'p');
    let mut status = 0;
    for d in &dirs {
        let resolved = match check_write(d) {
            Ok(p) => p,
            Err(msg) => {
                sink.eline(&gnu_refusal(
                    &format!("mkdir: cannot create directory '{d}'"),
                    &msg,
                ));
                status = 1;
                continue;
            }
        };
        // Every directory this creates is one journal step, as the mkdir
        // builtin records it: undo removes them innermost first, while empty.
        let mark = crate::journal::mark();
        record_created_dirs(&resolved, parents);
        let res = if parents {
            std::fs::create_dir_all(&resolved)
        } else {
            std::fs::create_dir(&resolved)
        };
        crate::journal::settle(mark, res.is_ok());
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

/// The directory `mktemp` makes its files in when it is given none: `$TMPDIR`
/// when that names a directory, else the system's temporary directory. Under the
/// workspace jail (agent mode, or a workspace set) the system's directory is out
/// of bounds, so a `$TMPDIR` outside the workspace is not used and the shell's
/// own `<workspace>/.ironshell/tmp` is, as `fs_tempdir` uses it. The path is
/// written with `/`, which the in-process tools and the programs a script runs
/// resolve alike (on Windows a bare `/tmp` is `C:\tmp` to the former and Git's
/// temp directory to the latter).
fn mktemp_root(tmpdir: Option<String>) -> String {
    let candidate = tmpdir.filter(|d| !d.is_empty() && (!cfg!(windows) || Path::new(d).is_dir()));
    if crate::safety::jail_enforced(crate::safety::current_mode()) {
        if let Some(d) = candidate {
            if crate::safety::within_workspace(&crate::safety::resolve_path_str(&d)) {
                return d;
            }
        }
        let dir = crate::safety::workspace_root()
            .join(".ironshell")
            .join("tmp");
        let _ = std::fs::create_dir_all(&dir);
        return posix_path(&dir.to_string_lossy());
    }
    candidate.unwrap_or_else(|| posix_path(&std::env::temp_dir().to_string_lossy()))
}

/// `mktemp [-d] [-u] [-q] [-t] [-p DIR | --tmpdir[=DIR]] [--suffix=S] [TEMPLATE]`
/// as GNU coreutils run it: a file (or with `-d` a directory) is created from
/// TEMPLATE, whose last run of at least three `X`s becomes random letters and
/// digits, and its name is printed. With no TEMPLATE it is `tmp.XXXXXXXXXX` in
/// the temporary directory (see [`mktemp_root`]); with `-t`, `-p` or
/// `--tmpdir` the template is a name inside that directory. Each name is judged
/// by the jail as a `>` target is. Any other option is declined (it runs as a
/// program).
fn mktemp(args: &[String], tmpdir: Option<String>, sink: &mut Sink) -> Util {
    let (mut make_dir, mut dry, mut quiet, mut in_tmp) = (false, false, false, false);
    let mut dir_arg: Option<String> = None;
    let mut suffix: Option<String> = None;
    let mut operands: Vec<String> = Vec::new();
    let mut done = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if done || !a.starts_with('-') || a == "-" {
            operands.push(a.clone());
            continue;
        }
        if a == "--" {
            done = true;
            continue;
        }
        if let Some(long) = a.strip_prefix("--") {
            let (name, value) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            match name {
                "directory" => make_dir = true,
                "dry-run" => dry = true,
                "quiet" => quiet = true,
                "tmpdir" => {
                    in_tmp = true;
                    if value.is_some() {
                        dir_arg = value;
                    }
                }
                "suffix" => match value.or_else(|| {
                    i += 1;
                    args.get(i - 1).cloned()
                }) {
                    Some(v) => suffix = Some(v),
                    None => return Util::Decline,
                },
                _ => return Util::Decline,
            }
            continue;
        }
        let chars: Vec<char> = a[1..].chars().collect();
        for (k, c) in chars.iter().enumerate() {
            match c {
                'd' => make_dir = true,
                'u' => dry = true,
                'q' => quiet = true,
                't' => in_tmp = true,
                'p' => {
                    in_tmp = true;
                    let rest: String = chars[k + 1..].iter().collect();
                    if !rest.is_empty() {
                        dir_arg = Some(rest);
                    } else if let Some(next) = args.get(i) {
                        dir_arg = Some(next.clone());
                        i += 1;
                    } else {
                        return Util::Decline;
                    }
                    break;
                }
                _ => return Util::Decline,
            }
        }
    }
    let say = |sink: &mut Sink, msg: &str| {
        if !quiet {
            sink.eline(msg);
        }
    };
    if operands.len() > 1 {
        say(
            sink,
            "mktemp: too many templates\nTry 'mktemp --help' for more information.",
        );
        return Util::Done(1);
    }
    let template = match operands.pop() {
        Some(t) => t,
        None => {
            in_tmp = true;
            "tmp.XXXXXXXXXX".to_string()
        }
    };
    if in_tmp && template.contains('/') {
        say(
            sink,
            &format!("mktemp: invalid template, '{template}', contains directory separator"),
        );
        return Util::Done(1);
    }
    // The text after the last X is the suffix when none was given with --suffix.
    let (stem, tail) = match &suffix {
        Some(s) => {
            if s.contains('/') {
                say(
                    sink,
                    &format!("mktemp: invalid suffix '{s}', contains directory separator"),
                );
                return Util::Done(1);
            }
            if !template.ends_with('X') {
                say(
                    sink,
                    &format!("mktemp: with --suffix, template '{template}' must end in X"),
                );
                return Util::Done(1);
            }
            (template.clone(), s.clone())
        }
        None => match template.rfind('X') {
            Some(p) => (template[..=p].to_string(), template[p + 1..].to_string()),
            None => (template.clone(), String::new()),
        },
    };
    let xs = stem.chars().rev().take_while(|c| *c == 'X').count();
    if xs < 3 {
        say(
            sink,
            &format!("mktemp: too few X's in template '{template}'"),
        );
        return Util::Done(1);
    }
    let prefix = &stem[..stem.len() - xs];
    let dir = if in_tmp {
        let d = dir_arg.unwrap_or_else(|| mktemp_root(tmpdir));
        if d.ends_with('/') || d.ends_with('\\') {
            d
        } else {
            format!("{d}/")
        }
    } else {
        String::new()
    };
    let kind = if make_dir { "directory" } else { "file" };
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    for _ in 0..100 {
        let mut random = String::with_capacity(xs);
        while random.len() < xs {
            for b in uuid::Uuid::new_v4().as_bytes() {
                if random.len() < xs {
                    random.push(CHARS[*b as usize % CHARS.len()] as char);
                }
            }
        }
        let name = format!("{dir}{prefix}{random}{tail}");
        if dry {
            sink.out.extend_from_slice(format!("{name}\n").as_bytes());
            return Util::Done(0);
        }
        let resolved = match check_write(&name) {
            Ok(p) => p,
            Err(msg) => {
                say(
                    sink,
                    &gnu_refusal(
                        &format!(
                            "mktemp: failed to create {kind} via template '{dir}{stem}{tail}'"
                        ),
                        &msg,
                    ),
                );
                return Util::Done(1);
            }
        };
        let mark = crate::journal::mark();
        if make_dir {
            record_created_dirs(&resolved, false);
        } else {
            crate::journal::record_paths(
                "posix",
                crate::safety::Effect::WriteLocal,
                vec![resolved.clone()],
            );
        }
        let res = if make_dir {
            std::fs::create_dir(&resolved).map(|()| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ =
                        std::fs::set_permissions(&resolved, std::fs::Permissions::from_mode(0o700));
                }
            })
        } else {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            opts.open(&resolved).map(|_| ())
        };
        crate::journal::settle(mark, res.is_ok());
        match res {
            Ok(()) => {
                sink.out.extend_from_slice(format!("{name}\n").as_bytes());
                return Util::Done(0);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                say(
                    sink,
                    &format!(
                        "mktemp: failed to create {kind} via template '{dir}{stem}{tail}': {}",
                        io_reason(&e)
                    ),
                );
                return Util::Done(1);
            }
        }
    }
    say(
        sink,
        &format!("mktemp: failed to create {kind} via template '{dir}{stem}{tail}': File exists"),
    );
    Util::Done(1)
}

/// `command -v NAME...`: for each name, what the shell would run: the name
/// itself for a shell builtin or keyword and for a utility this shell runs
/// in-process (bash prints a path for those it does not have built in; the name
/// runs the same here), else the path of the program on PATH, written with `/`.
/// A name that is none of these prints nothing. The status is 0 when any name
/// was found (or none was given) and 1 otherwise, as in bash.
fn command_v(args: &[String], path: Option<&str>, sink: &mut Sink) -> i32 {
    let mut names = args;
    if names.first().map(String::as_str) == Some("--") {
        names = &names[1..];
    } else if let Some(bad) = names.first().filter(|a| a.len() > 1 && a.starts_with('-')) {
        sink.eline(&format!(
            "{SHELL_NAME}: command: {bad}: invalid option\ncommand: usage: command [-pVv] command [arg ...]"
        ));
        return 2;
    }
    if names.is_empty() {
        return 0;
    }
    const BUILTINS: &[&str] = &[
        "echo", "printf", "test", "[", "cd", "pwd", "read", "export", "unset", "shift", "exit",
        "break", "continue", "true", "false", ":", "set", "if", "then", "else", "elif", "fi",
        "for", "while", "until", "do", "done", "in", "{", "}", "!", "mktemp", "timeout", "yes",
    ];
    let mut found = false;
    for name in names {
        let shown = if BUILTINS.contains(&name.as_str())
            || SUBSET_NAMES.contains(&name.as_str())
            || UNSUPPORTED_BUILTINS.contains(&name.as_str())
        {
            Some(name.clone())
        } else {
            find_on_path(name, path)
        };
        if let Some(s) = shown {
            sink.out.extend_from_slice(s.as_bytes());
            sink.out.push(b'\n');
            found = true;
        }
    }
    i32::from(!found)
}

/// The program `name` names, as `command -v` shows it: a name with a `/` must
/// itself be an executable file; otherwise each `PATH` directory is searched
/// (on Windows also for the `.exe`, `.cmd`, `.bat` and `.com` of the name).
fn find_on_path(name: &str, path: Option<&str>) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    if name.contains('/') || (cfg!(windows) && name.contains('\\')) {
        let exts: &[&str] = if cfg!(windows) {
            &["", ".exe", ".cmd", ".bat", ".com"]
        } else {
            &[""]
        };
        return exts.iter().find_map(|e| {
            let candidate = format!("{name}{e}");
            runs_file(Path::new(&candidate)).then(|| posix_path(&candidate))
        });
    }
    let path = match path {
        Some(p) => p.to_string(),
        None => std::env::var("PATH").ok()?,
    };
    find_in(name, &path).map(|p| posix_path(&p.to_string_lossy()))
}

/// Whether `p` is a file a shell may run: on Unix one with an execute bit.
fn runs_file(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.is_file()
            && p.metadata()
                .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// The directories of a PATH value. On Unix it is split at `:`. On Windows an
/// entry is split at `;`, and a script that builds one with the colons of a
/// bash (`export PATH="$PWD/bin:$PATH"`) is read too: a `:` right after a
/// single letter that starts an entry and before a slash is a drive, any other
/// `:` separates, and an MSYS `/c/dir` entry is `C:/dir`.
fn split_path_var(path: &str) -> Vec<String> {
    split_path_entries(path, false)
}

/// [`split_path_var`], keeping the empty entries when `keep_empty` (CDPATH:
/// an empty entry is the current directory).
fn split_path_entries(path: &str, keep_empty: bool) -> Vec<String> {
    if !cfg!(windows) {
        return path.split(':').map(String::from).collect();
    }
    let mut out = Vec::new();
    for piece in path.split(';') {
        let chars: Vec<char> = piece.chars().collect();
        let mut cur = String::new();
        for (i, &c) in chars.iter().enumerate() {
            if c == ':' {
                let drive = cur.len() == 1
                    && cur.chars().all(|x| x.is_ascii_alphabetic())
                    && matches!(chars.get(i + 1), Some('/') | Some('\\'));
                if !drive {
                    out.push(std::mem::take(&mut cur));
                    continue;
                }
            }
            cur.push(c);
        }
        out.push(cur);
    }
    out.into_iter()
        .filter(|d| keep_empty || !d.is_empty())
        .map(|d| {
            let b = d.as_bytes();
            if b.len() >= 2
                && b[0] == b'/'
                && b[1].is_ascii_alphabetic()
                && (b.len() == 2 || b[2] == b'/')
            {
                format!(
                    "{}:/{}",
                    (b[1] as char).to_ascii_uppercase(),
                    &d[(3).min(d.len())..]
                )
            } else {
                d
            }
        })
        .collect()
}

/// The names a program may be given on Windows after its name, in PATHEXT
/// order (lower case, with the dot).
fn path_exts() -> Vec<String> {
    let text = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    text.split(';')
        .filter(|e| e.starts_with('.') && e.len() > 1)
        .map(|e| e.to_ascii_lowercase())
        .collect()
}

/// The program `name` runs, searching the directories of `path` in order: one
/// lookup for `command -v` and for running a command, so they cannot disagree.
/// Only absolute entries count, so a relative one such as `.` never runs a
/// program from the working folder. On Windows a name takes the PATHEXT
/// extensions (`.exe`, `.cmd`, `.bat`) in order, and a file with none is
/// taken last (a script with a shebang).
fn find_in(name: &str, path: &str) -> Option<std::path::PathBuf> {
    if name.is_empty() {
        return None;
    }
    let exts = if cfg!(windows) {
        path_exts()
    } else {
        Vec::new()
    };
    let lower = name.to_ascii_lowercase();
    let has_ext = exts.iter().any(|e| lower.ends_with(e.as_str()));
    for dir in split_path_var(path) {
        let dir = Path::new(&dir);
        if !dir.is_absolute() {
            continue;
        }
        let mut names: Vec<String> = Vec::new();
        if cfg!(windows) {
            if has_ext {
                names.push(name.to_string());
            } else {
                names.extend(exts.iter().map(|e| format!("{name}{e}")));
                names.push(name.to_string());
            }
        } else {
            names.push(name.to_string());
        }
        for n in names {
            let candidate = dir.join(n);
            if runs_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// `touch FILE...`: a missing file is created empty, an existing one gets the
/// current modification time. Each file is judged by the jail as a `>` target.
fn touch(args: &[String], sink: &mut Sink) -> Util {
    let Some((_, files)) = short_flags(args, "") else {
        return Util::Decline;
    };
    if files.is_empty() {
        sink.eline("touch: missing file operand\nTry 'touch --help' for more information.");
        return Util::Done(1);
    }
    let mut status = 0;
    for f in &files {
        let resolved = match check_write_in_place(f) {
            Ok(p) => p,
            Err(msg) => {
                sink.eline(&gnu_refusal(&format!("touch: cannot touch '{f}'"), &msg));
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

/// Run a builtin by name, as the jailed `rm` and `rmdir` of the language run, so
/// the workspace jail, the approval gate and the journal judge it as they judge
/// a call from a script.
fn jailed_builtin(name: &str, args: Vec<crate::value::Value>) -> anyhow::Result<()> {
    let mut env = crate::env::Env::new();
    crate::builtins::call(name, args, &mut env).map(|_| ())
}

/// A guarded builtin's refusal, as the text a GNU tool prints after `lead`
/// (`mkdir: cannot create directory '../x'`). `msg` is the refusal as
/// `check_write` returns it, the JSON a builtin error renders as. A jail refusal
/// is `outside the workspace`, with the operand as written (the JSON names the
/// resolved path and a hint for the builtin API, which a script cannot use).
/// An approval request keeps its JSON: it carries the token that approves it.
/// Anything that is not that JSON is returned as it is.
fn gnu_refusal(lead: &str, msg: &str) -> String {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(msg) else {
        return msg.to_string();
    };
    let err = &json["error"];
    match err["code"].as_str() {
        None | Some("E_NEEDS_APPROVAL") => msg.to_string(),
        Some("E_OUTSIDE_WORKSPACE") => format!("{lead}: outside the workspace"),
        Some(_) => match err["message"].as_str() {
            Some(m) => {
                let builtin = err["builtin"].as_str().unwrap_or("");
                let m = m.strip_prefix(&format!("{builtin}: ")).unwrap_or(m);
                format!("{lead}: {m}")
            }
            None => msg.to_string(),
        },
    }
}

/// What a failed `rm` or `rmdir` of `operand` prints, and the status it leaves:
/// GNU text, as the other in-process tools print theirs, with no JSON. A request
/// for approval is the exception, and is the same refusal `cp` gets from the
/// exec gate (status 75, the JSON with the token), naming `name`, not the
/// `rmdir` builtin that an `rm -r` of a directory runs.
fn rm_failure(name: &str, operand: &str, e: &anyhow::Error) -> (i32, String) {
    use crate::safety::ErrorCode;
    let verb = if name == "rmdir" {
        "failed to remove"
    } else {
        "cannot remove"
    };
    let lead = format!("{name}: {verb} '{operand}'");
    let Some(se) = e.downcast_ref::<crate::safety::SafetyError>() else {
        return (1, format!("{lead}: {e}"));
    };
    let prose = se
        .message
        .strip_prefix(&format!("{}: ", se.builtin))
        .unwrap_or(&se.message)
        .to_string();
    match se.code {
        ErrorCode::NeedsApproval => {
            let mut shown = se.clone();
            if shown.builtin != name {
                shown.message =
                    shown
                        .message
                        .replacen(&format!("{}:", shown.builtin), &format!("{name}:"), 1);
                shown.builtin = name.to_string();
            }
            (ErrorCode::NeedsApproval.exit_code(), shown.to_string())
        }
        ErrorCode::PolicyDeny => (
            ErrorCode::PolicyDeny.exit_code(),
            format!("{lead}: {prose}"),
        ),
        ErrorCode::OutsideWorkspace => (1, format!("{lead}: outside the workspace")),
        ErrorCode::BadState if se.message.contains("is not empty") => {
            (1, format!("{lead}: Directory not empty"))
        }
        ErrorCode::NotFound => (1, format!("{lead}: No such file or directory")),
        ErrorCode::OsDenied => (1, format!("{lead}: Permission denied")),
        _ => (1, format!("{lead}: {prose}")),
    }
}

/// The first flag of `args` that in-process `rm` (letters `rRf`) or `rmdir` (no
/// letters) does not run, as written: a long option whole, and for a cluster of
/// short ones the one bad letter (`-rI` names `-I`, not `-rI`). `None` when
/// every flag is supported, or for another command.
fn unsupported_flag(name: &str, args: &[String]) -> Option<String> {
    let allowed = match name {
        "rm" => "rRf",
        "rmdir" => "",
        _ => return None,
    };
    for a in args {
        if a == "--" {
            break;
        }
        if a.starts_with("--") {
            return Some(a.split('=').next().unwrap_or(a).to_string());
        }
        if a.len() > 1 && a.starts_with('-') {
            if let Some(c) = a[1..].chars().find(|c| !allowed.contains(*c)) {
                return Some(format!("-{c}"));
            }
        }
    }
    None
}

/// `rm [-r|-R] [-f] [--] FILE...` and `rmdir DIR...`, in process.
///
/// They run on the code of the `rm` and `rmdir` builtins, so a path outside the
/// workspace is refused, a removal is journalled when the journal is on, and a
/// link (a symlink, or a junction on Windows) is removed itself and never
/// followed. A name with a trailing separator, or ending in `.` or `..`, is
/// refused as `rm()` refuses it, since it names what a link points at. `-r` of a
/// real directory removes it as `rmdir(path, true)` does. Any other flag
/// declines, so a GNU program on PATH, if one is found, runs it.
fn rm_utility(name: &str, args: &[String], sink: &mut Sink) -> Util {
    let remove_dirs = name == "rmdir";
    let Some((flags, operands)) = short_flags(args, if remove_dirs { "" } else { "rRf" }) else {
        // Under the workspace jail an rm the subset cannot run is refused,
        // not handed to a PATH program the jail cannot confine (a GNU rm
        // follows `jx/` through a link out of the workspace).
        if crate::safety::jail_enforced(crate::safety::current_mode()) {
            let flag = unsupported_flag(name, args).unwrap_or_else(|| "this form".to_string());
            sink.eline(&format!(
                "{SHELL_NAME}: {name}: the in-process {name} does not support {flag}, and under the workspace jail no external {name} is run in its place; use -r, -R or -f"
            ));
            return Util::Done(2);
        }
        return Util::Decline;
    };
    let recursive = flags.contains(&'r') || flags.contains(&'R');
    let force = flags.contains(&'f');
    if operands.is_empty() {
        if force && !remove_dirs {
            return Util::Done(0);
        }
        sink.eline(&format!(
            "{name}: missing operand\nTry '{name} --help' for more information."
        ));
        return Util::Done(1);
    }
    let mut status = 0;
    for operand in &operands {
        if let Err((code, msg)) = rm_operand(name, operand, recursive, force) {
            sink.eline(&msg);
            status = code;
            // A request for approval or a policy refusal ends the command, as
            // the exec gate's refusal of `cp` does; it is not one operand's.
            if code != 1 {
                break;
            }
        }
    }
    Util::Done(status)
}

/// Remove one operand of `rm` or `rmdir`; the error is the status and the line
/// to print (see [`rm_failure`]).
fn rm_operand(
    name: &str,
    operand: &str,
    recursive: bool,
    force: bool,
) -> Result<(), (i32, String)> {
    use crate::value::Value;
    let verb = if name == "rmdir" {
        "failed to remove"
    } else {
        "cannot remove"
    };
    if operand.ends_with(['/', '\\']) || crate::safety::path_ends_in_dot_component(operand) {
        return Err((1, format!(
            "{name}: {verb} '{operand}': a trailing separator, `.` or `..` is refused, because it names what a link points at; name it without"
        )));
    }
    let remote = crate::safety::names_a_remote_share(operand);
    let meta = if remote {
        None
    } else {
        Some(std::fs::symlink_metadata(operand))
    };
    if let Some(Err(e)) = &meta {
        if e.kind() == std::io::ErrorKind::NotFound {
            if force && name == "rm" {
                return Ok(());
            }
            return Err((
                1,
                format!("{name}: {verb} '{operand}': No such file or directory"),
            ));
        }
    }
    let is_link = !remote && crate::safety::is_link(operand);
    let is_dir = matches!(&meta, Some(Ok(m)) if m.is_dir());
    let fail = |why: &str| (1, format!("{name}: {verb} '{operand}': {why}"));
    let failed = |e: anyhow::Error| rm_failure(name, operand, &e);
    if name == "rmdir" {
        if is_link || !is_dir {
            return Err(fail("Not a directory"));
        }
        return jailed_builtin(
            "rmdir",
            vec![Value::Str(operand.to_string()), Value::Bool(false)],
        )
        .map_err(failed);
    }
    if is_dir && !is_link {
        if !recursive {
            return Err(fail("Is a directory"));
        }
        return jailed_builtin(
            "rmdir",
            vec![Value::Str(operand.to_string()), Value::Bool(true)],
        )
        .map_err(failed);
    }
    jailed_builtin(
        "rm",
        vec![Value::Str(operand.to_string()), Value::Bool(force)],
    )
    .map_err(failed)
}

/// The characters of a `tr` set: ranges (`a-z`), the classes `[:upper:]`,
/// `[:lower:]`, `[:digit:]`, `[:space:]`, `[:alpha:]`, `[:alnum:]`, and the
/// escapes `\n`, `\t`, `\r`, `\\`. `None` for anything else (`[=c=]`, `[c*n]`,
/// octal escapes), so the caller declines.
fn tr_set(spec: &str) -> Option<Vec<char>> {
    Some(tr_items(spec)?.into_iter().flat_map(|(_, c)| c).collect())
}

/// A set as its items: a character class (with its name) or a run of plain
/// characters, each with the characters it stands for.
fn tr_items(spec: &str) -> Option<Vec<(Option<String>, Vec<char>)>> {
    let chars: Vec<char> = spec.chars().collect();
    let mut out: Vec<(Option<String>, Vec<char>)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' && chars.get(i + 1) == Some(&':') {
            let rest: String = chars[i..].iter().collect();
            let end = rest.find(":]")?;
            let class = &rest[2..end];
            // Each class in byte order, as GNU tr walks it in the C locale, so
            // a class in SET1 pairs with SET2 the way GNU tr pairs it.
            let test: fn(&u8) -> bool = match class {
                "upper" => |b| b.is_ascii_uppercase(),
                "lower" => |b| b.is_ascii_lowercase(),
                "digit" => |b| b.is_ascii_digit(),
                "xdigit" => |b| b.is_ascii_hexdigit(),
                "space" => |b| matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c),
                "blank" => |b| matches!(b, b' ' | b'\t'),
                "alpha" => |b| b.is_ascii_alphabetic(),
                "alnum" => |b| b.is_ascii_alphanumeric(),
                "punct" => |b| b.is_ascii_punctuation(),
                "cntrl" => |b| b.is_ascii_control(),
                "graph" => |b| b.is_ascii_graphic(),
                "print" => |b| b.is_ascii_graphic() || *b == b' ',
                _ => return None,
            };
            let members: Vec<char> = (0u8..=127).filter(test).map(char::from).collect();
            out.push((Some(class.to_string()), members));
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
            out.push((None, (c..=hi).collect()));
            i += width + 2;
        } else {
            out.push((None, vec![c]));
            i += width;
        }
    }
    Some(out)
}

/// GNU tr's refusals when translating with classes: SET2 may hold only
/// `[:upper:]` and `[:lower:]`, and each must stand where SET1 has the other.
fn tr_class_error(sets: &[String]) -> Option<&'static str> {
    let [a, b] = sets else { return None };
    let (items1, items2) = (tr_items(a)?, tr_items(b)?);
    if items2.iter().any(|(class, _)| {
        class
            .as_deref()
            .is_some_and(|c| c != "upper" && c != "lower")
    }) {
        return Some(
            "tr: when translating, the only character classes that may appear in\nstring2 are 'upper' and 'lower'",
        );
    }
    let mut at = 0;
    for (class, members) in &items2 {
        if let Some(class) = class {
            // Where SET1 has an item beginning at this position, it must be
            // the opposite class.
            let mut pos = 0;
            let opposite = items1.iter().find_map(|(c1, m1)| {
                let hit = (pos == at).then(|| c1.clone());
                pos += m1.len();
                hit
            });
            let want = if class == "upper" { "lower" } else { "upper" };
            if opposite.flatten().as_deref() != Some(want) {
                return Some("tr: misaligned [:upper:] and/or [:lower:] construct");
            }
        }
        at += members.len();
    }
    None
}

/// `tr SET1 SET2`, `tr -d SET1`, `tr -s SET1` and `tr -s SET1 SET2`, over
/// characters. SET2 shorter than SET1 is padded with its last character, as
/// GNU tr does. Other options (`-c`, `-t`) and set forms decline to the real
/// program.
fn tr(args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    if let Some(msg) = invalid_option(
        "tr",
        args,
        "cCdst",
        &[
            "complement",
            "delete",
            "squeeze-repeats",
            "truncate-set1",
            "help",
            "version",
        ],
    ) {
        sink.eline(&msg);
        return Util::Done(1);
    }
    let Some((flags, sets)) = short_flags(args, "dsc") else {
        return Util::Decline;
    };
    let delete = flags.contains(&'d');
    let squeeze = flags.contains(&'s');
    // `tr -dc SET` deletes every character that is not in SET (the filter of
    // `tr -dc a-z </dev/urandom`); the other uses of -c go to a real tr.
    let complement = flags.contains(&'c');
    if complement && !(delete && !squeeze) {
        return Util::Decline;
    }
    if sets.is_empty() {
        sink.eline("tr: missing operand\nTry 'tr --help' for more information.");
        return Util::Done(1);
    }
    let parsed: Option<Vec<Vec<char>>> = sets.iter().map(|s| tr_set(s)).collect();
    let Some(parsed) = parsed else {
        return Util::Decline;
    };
    if !delete {
        if let Some(msg) = tr_class_error(&sets) {
            sink.eline(msg);
            return Util::Done(1);
        }
    }
    let (set1, set2) = match (delete, squeeze, parsed.as_slice()) {
        (true, false, [a]) => (a.clone(), Vec::new()),
        (false, true, [a]) => (a.clone(), Vec::new()),
        (false, _, [a, b]) if !b.is_empty() => (a.clone(), b.clone()),
        (false, false, [_]) => {
            sink.eline(&format!(
                "tr: missing operand after '{}'\nTwo strings must be given when translating.\nTry 'tr --help' for more information.",
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
        if delete && set1.contains(&c) != complement {
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
    // Headers go before each file when more than one was named, even when one
    // of them could not be read; a blank line separates a header from the
    // output of an earlier one.
    let many = files.len() > 1 && !quiet;
    let mut headed = false;
    for (name, data) in inputs.iter() {
        if many {
            if headed {
                sink.out.push(b'\n');
            }
            headed = true;
            let shown = if name == "(standard input)" {
                "standard input"
            } else {
                name.as_str()
            };
            sink.out
                .extend_from_slice(format!("==> {shown} <==\n").as_bytes());
        }
        if head && count >= 0 {
            // `head` has what it wanted: a producer that never ends is done.
            let have = if bytes {
                data.len()
            } else {
                lines_of(data).len()
            };
            if have >= count as usize {
                note_limit();
            }
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
    // Standard input is a regular file (`< f`), also when `-` names it.
    let stdin_file = stdin.file && (from_stdin || files == ["-"]);
    let count = |d: &[u8]| -> [usize; 4] {
        let text = String::from_utf8_lossy(d);
        [
            d.iter().filter(|&&b| b == b'\n').count(),
            text.split_ascii_whitespace().count(),
            text.chars().count(),
            d.len(),
        ]
    };
    // What is printed, in operand order: a row of counts, or a message. A
    // directory has a row of zeros and then the message, as GNU prints them.
    enum Item {
        Row(Option<String>, [usize; 4]),
        Msg(String),
    }
    let mut items: Vec<Item> = Vec::new();
    if from_stdin {
        items.push(Item::Row(None, count(&stdin.read_all())));
    } else {
        for f in &files {
            if f == "-" {
                items.push(Item::Row(Some("-".into()), count(&stdin.read_all())));
                continue;
            }
            if jail_refusal("wc", f).is_none() && Path::new(f).is_dir() {
                items.push(Item::Row(Some(f.clone()), [0; 4]));
                items.push(Item::Msg(format!("wc: {f}: Is a directory")));
                status = 1;
                continue;
            }
            let mut tmp = Sink::new(ErrMode::Capture);
            let got = read_inputs("wc", std::slice::from_ref(f), stdin, &mut tmp, &mut status);
            for (_, d) in &got {
                items.push(Item::Row(Some(f.clone()), count(d)));
            }
            if !tmp.err.is_empty() {
                let text = String::from_utf8_lossy(&tmp.err).into_owned();
                items.push(Item::Msg(text.trim_end_matches('\n').to_string()));
            }
        }
    }
    let rows: Vec<[usize; 4]> = items
        .iter()
        .filter_map(|i| match i {
            Item::Row(_, r) => Some(*r),
            Item::Msg(_) => None,
        })
        .collect();
    let mut total = [0usize; 4];
    for r in &rows {
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
    // regular file, to 7 when one is not (a pipe, /dev/null, a directory),
    // and not at all for a single number.
    let width = if ncols == 1 && files.len() <= 1 {
        0
    } else if stdin_file {
        rows.first().map_or(1, |r| r[3].to_string().len())
    } else if from_stdin
        || files.iter().any(|f| {
            f == "-" || f == "/dev/null" || std::fs::metadata(f).is_ok_and(|m| !m.is_file())
        })
    {
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
    let line = |vals: Vec<usize>, name: Option<&str>| -> Vec<u8> {
        let s: Vec<String> = vals.iter().map(|v| format!("{v:>width$}")).collect();
        let mut line = s.join(" ");
        if let Some(n) = name {
            line.push(' ');
            line.push_str(n);
        }
        line.push('\n');
        line.into_bytes()
    };
    for item in &items {
        match item {
            Item::Row(name, r) => {
                let text = line(selected(r), name.as_deref());
                sink.out.extend_from_slice(&text);
            }
            Item::Msg(m) => sink.eline(m),
        }
    }
    // GNU prints the total whenever there is more than one operand, readable or not.
    if files.len() > 1 {
        let text = line(selected(&total), Some("total"));
        sink.out.extend_from_slice(&text);
    }
    Util::Done(status)
}

/// A POSIX basic regular expression as a Rust regex: in BRE, `+ ? | ( ) { }`
/// are literal and their backslashed forms are the operators. A `*` at the
/// start of the regex, after `\(` or `\|`, or after a leading `^`, is an
/// ordinary character; `` \` `` and `\'` are the start and end of the text.
/// A `^` is an anchor only at the start of the regex or after `\(` or `\|`, and
/// a `$` only at its end or before `\)` or `\|`; anywhere else (`a$b`, `$HOME`,
/// `a^b`) each is an ordinary character, as in GNU sed and grep.
pub(crate) fn bre_to_regex(p: &str) -> Option<String> {
    bre_translate(p, false)
}

/// `bre_to_regex`, optionally keeping a back-reference (`\1` to `\9`) in the
/// output, for an engine that supports it (`fancy-regex`).
pub(crate) fn bre_translate(p: &str, keep_backrefs: bool) -> Option<String> {
    let mut out = String::new();
    let mut it = p.chars().peekable();
    let mut first = true;
    // The piece just before was the `^` anchor.
    let mut prev_caret = false;
    // Inside a bracket expression `^` and `$` are the bracket's own, and the
    // position that matters for `]` and a leading `^` is tracked here.
    let mut in_bracket = false;
    let mut bracket_start = false;
    while let Some(c) = it.next() {
        if in_bracket {
            let at_start = std::mem::replace(&mut bracket_start, false);
            match c {
                '^' if at_start => {
                    out.push('^');
                    bracket_start = true;
                    continue;
                }
                ']' if !at_start => in_bracket = false,
                '[' if it.peek() == Some(&':') => {
                    // A class such as `[:alpha:]` ends at its own `:]`.
                    out.push('[');
                    let mut prev = ' ';
                    for d in it.by_ref() {
                        out.push(d);
                        if d == ']' && prev == ':' {
                            break;
                        }
                        prev = d;
                    }
                    continue;
                }
                _ => {}
            }
            match c {
                '+' | '?' | '[' | '|' | '(' | ')' | '{' | '}' => {
                    out.push_str(&regex::escape(&c.to_string()));
                }
                '\\' => match it.next() {
                    Some(o) => out.push_str(&regex::escape(&o.to_string())),
                    None => out.push_str("\\\\"),
                },
                _ => out.push(c),
            }
            continue;
        }
        // Whether a `*` right after this piece is an ordinary character.
        let mut star_literal = false;
        let was_caret = std::mem::replace(&mut prev_caret, false);
        match c {
            '[' => {
                in_bracket = true;
                bracket_start = true;
                out.push('[');
            }
            '^' if !first || was_caret => out.push_str("\\^"),
            '$' => {
                let mut ahead = it.clone();
                let anchor = match ahead.next() {
                    None => true,
                    Some('\\') => matches!(ahead.next(), Some(')' | '|')),
                    Some(_) => false,
                };
                out.push_str(if anchor { "$" } else { "\\$" });
            }
            '\\' => match it.next() {
                Some(o @ ('+' | '?' | '|' | '(' | ')' | '{' | '}')) => {
                    out.push(o);
                    star_literal = matches!(o, '|' | '(');
                }
                Some(d @ '1'..='9') => {
                    if !keep_backrefs {
                        return None; // back-references
                    }
                    out.push('\\');
                    out.push(d);
                }
                Some(o @ ('w' | 'W' | 's' | 'S' | 'b' | 'B')) => {
                    out.push('\\');
                    out.push(o);
                }
                Some('<') | Some('>') => out.push_str("\\b"),
                Some('`') => out.push_str("\\A"),
                Some('\'') => out.push_str("\\z"),
                Some(o) => out.push_str(&regex::escape(&o.to_string())),
                None => out.push_str("\\\\"),
            },
            '+' | '?' | '|' | '(' | ')' | '{' | '}' => out.push_str(&regex::escape(&c.to_string())),
            '*' if first => out.push_str("\\*"),
            '^' if first => {
                out.push('^');
                star_literal = true;
                prev_caret = true;
            }
            _ => out.push(c),
        }
        first = star_literal;
    }
    Some(out)
}

/// An extended regex: already close to Rust's, but back-references are not,
/// and `` \` `` and `\'` are the start and end of the text.
fn ere_to_regex(p: &str) -> Option<String> {
    if p.contains("\\1") || p.contains("\\2") {
        return None;
    }
    let mut out = String::new();
    let mut it = p.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('<') | Some('>') => out.push_str("\\b"),
            Some('`') => out.push_str("\\A"),
            Some('\'') => out.push_str("\\z"),
            Some(o @ ('w' | 'W' | 's' | 'S' | 'b' | 'B')) => {
                out.push('\\');
                out.push(o);
            }
            // GNU grep reads `\d` as a plain `d` (newer versions warn about the
            // stray backslash); Rust's regex would match a digit.
            Some(o) if o.is_ascii_alphanumeric() => out.push(o),
            Some(o) => {
                out.push('\\');
                out.push(o);
            }
            None => out.push('\\'),
        }
    }
    Some(out)
}

/// The `--include`, `--exclude` and `--exclude-dir` globs of a `grep`, each
/// matched against a file's or directory's base name, as GNU grep does.
#[derive(Default)]
struct GrepFilter {
    include: Vec<String>,
    exclude: Vec<String>,
    exclude_dir: Vec<String>,
}

impl GrepFilter {
    /// Whether a file is left out. `names` are the strings its name is tried as:
    /// the base name for a file the walk finds, and for an operand written on the
    /// command line the whole operand and each tail after a `/`.
    fn skips_file(&self, names: &[&str]) -> bool {
        let hit = |globs: &[String]| globs.iter().any(|g| names.iter().any(|n| glob_match(g, n)));
        hit(&self.exclude) || (!self.include.is_empty() && !hit(&self.include))
    }

    /// Whether a directory is not entered (`--exclude-dir`).
    fn skips_dir(&self, names: &[&str]) -> bool {
        self.exclude_dir.iter().any(|g| {
            let g = g.trim_end_matches('/');
            names.iter().any(|n| glob_match(g, n))
        })
    }
}

/// The strings a command-line operand is matched as: the operand, less a
/// trailing `/`, and every tail of it that starts after a `/`.
fn operand_names(op: &str) -> Vec<&str> {
    let t = match op.trim_end_matches('/') {
        "" => op,
        t => t,
    };
    let mut names = vec![t];
    for (k, c) in t.char_indices() {
        if c == '/' {
            names.push(&t[k + 1..]);
        }
    }
    names
}

/// Which of `--include`, `--exclude` and `--exclude-dir` the argument is, and its
/// glob when written `--include=GLOB`; `None` when written `--include GLOB`.
fn grep_glob_opt(a: &str) -> Option<(&'static str, Option<String>)> {
    for name in ["include", "exclude-dir", "exclude"] {
        let long = format!("--{name}");
        if a == long {
            return Some((name, None));
        }
        if let Some(v) = a.strip_prefix(&format!("{long}=")) {
            return Some((name, Some(v.to_string())));
        }
    }
    None
}

/// The plain flags of a cluster of short grep options (`-rn`, `-nA2`, `-3`),
/// and the context options in it: `A`, `B` or `C` with the value written in the
/// same word (`-A2`), or `None` when it is the next word; a run of digits is
/// `-C` of that number. `Err(c)` is the first flag the in-process grep does
/// not run.
type GrepCluster = (Vec<char>, Vec<(char, Option<String>)>);

fn grep_cluster(a: &str) -> Result<GrepCluster, char> {
    let mut plain = Vec::new();
    let mut context = Vec::new();
    let chars: Vec<char> = a[1..].chars().collect();
    let mut k = 0;
    while k < chars.len() {
        let c = chars[k];
        if matches!(c, 'A' | 'B' | 'C' | 'm') {
            let rest: String = chars[k + 1..].iter().collect();
            context.push((c, if rest.is_empty() { None } else { Some(rest) }));
            break;
        } else if c.is_ascii_digit() {
            let mut digits = String::new();
            while k < chars.len() && chars[k].is_ascii_digit() {
                digits.push(chars[k]);
                k += 1;
            }
            context.push(('C', Some(digits)));
            continue;
        } else if "rRnclLivEFwoqshHxb".contains(c) {
            plain.push(c);
        } else {
            return Err(c);
        }
        k += 1;
    }
    Ok((plain, context))
}

/// A long context option: `--after-context`, `--before-context`, `--context`
/// or `--group-separator`, with its value when written `--name=VALUE` (`None`
/// when it is the next word). The name may be abbreviated, as getopt allows.
fn grep_long_context(a: &str) -> Option<(&'static str, Option<String>)> {
    let body = a.strip_prefix("--")?;
    let (name, value) = match body.split_once('=') {
        Some((n, v)) => (n, Some(v.to_string())),
        None => (body, None),
    };
    if name.is_empty() {
        return None;
    }
    for full in [
        "after-context",
        "before-context",
        "context",
        "group-separator",
        "max-count",
    ] {
        if full.starts_with(name) && name.len() >= 3 {
            return Some((full, value));
        }
    }
    None
}

/// `--byte-offset`, or an abbreviation of it (`--byte`): the flag `-b`.
fn grep_long_flag(a: &str) -> Option<char> {
    let name = a.strip_prefix("--")?;
    (name.len() >= 2 && "byte-offset".starts_with(name)).then_some('b')
}

/// The lines of context `grep -A/-B/-C/-NUM` ask for.
#[derive(Default)]
struct GrepContext {
    after: Option<usize>,
    before: Option<usize>,
    both: Option<usize>,
    separator: Option<String>,
    no_separator: bool,
    /// `-m N` / `--max-count=N`: stop after this many selected lines per file.
    /// `None` is no limit (also a negative or an overflowing N, as in GNU).
    max: Option<usize>,
}

/// The digits of a count as GNU grep reads one: an optional `+`, then digits.
/// `None` when it is anything else; `Some(None)` when the digits overflow a
/// `usize`, which GNU clamps to its maximum.
fn grep_count(value: &str) -> Option<Option<usize>> {
    let digits = value.strip_prefix('+').unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(digits.parse::<usize>().ok())
}

impl GrepContext {
    /// Whether any of `-A`, `-B`, `-C` was given, even as 0: GNU then prints
    /// `--` between groups of lines that do not touch.
    fn requested(&self) -> bool {
        self.after.is_some() || self.before.is_some() || self.both.is_some()
    }

    /// Apply one option; `Err` is GNU's message, without the `grep: `.
    fn set(&mut self, kind: &str, value: &str) -> Result<(), String> {
        if kind == "group-separator" {
            self.separator = Some(value.to_string());
            self.no_separator = false;
            return Ok(());
        }
        if kind == "m" || kind == "max-count" {
            // A negative count is no limit; text that is no number is an error.
            self.max = match value.strip_prefix('-') {
                Some(rest) if grep_count(rest).is_some() => None,
                _ => match grep_count(value) {
                    Some(n) => n,
                    None => return Err("invalid max count".to_string()),
                },
            };
            return Ok(());
        }
        // A context longer than any file is clamped, not refused.
        let n: usize = match grep_count(value) {
            Some(n) => n.unwrap_or(usize::MAX),
            None => return Err(format!("{value}: invalid context length argument")),
        };
        match kind {
            "A" | "after-context" => self.after = Some(n),
            "B" | "before-context" => self.before = Some(n),
            _ => self.both = Some(n),
        }
        Ok(())
    }

    /// Lines before and after a match: `-A` and `-B` win over `-C`, in any order.
    fn lines(&self) -> (usize, usize) {
        (
            self.before.or(self.both).unwrap_or(0),
            self.after.or(self.both).unwrap_or(0),
        )
    }
}

/// What GNU getopt says about an argument that starts with `-` and is no option
/// of the utility: `invalid option -- 'c'` for a short one, `unrecognized option
/// '--name'` for a long one, then the usage line(s). `short` lists the short
/// options, each followed by `:` when it takes a value; `long` the long names,
/// of which a prefix is as good as the whole (getopt's abbreviations). `None`
/// when every option is one of them. The caller prints it and exits with the
/// utility's own status (grep and sort 2, cut and tr 1).
fn invalid_option(tool: &str, args: &[String], short: &str, long: &[&str]) -> Option<String> {
    let takes_value = |c: char| {
        short
            .find(c)
            .is_some_and(|p| short[p + c.len_utf8()..].starts_with(':'))
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        i += 1;
        if a == "--" {
            break;
        }
        let what = if let Some(name) = a.strip_prefix("--") {
            let name = name.split('=').next().unwrap_or("");
            if long.iter().any(|l| l.starts_with(name)) {
                // A long option that takes a value may take it as the next word;
                // which ones do is the utility's own business, and the worst
                // outcome of guessing wrong is a missed refusal.
                continue;
            }
            format!("unrecognized option '--{name}'")
        } else if a.len() > 1 && a.starts_with('-') {
            let mut bad = None;
            for (k, c) in a.char_indices().skip(1) {
                if !short.contains(c) || c == ':' {
                    bad = Some(c);
                    break;
                }
                if takes_value(c) {
                    // The rest of the word is the value, or the next word is.
                    if k + c.len_utf8() >= a.len() {
                        i += 1;
                    }
                    break;
                }
            }
            match bad {
                Some(c) => format!("invalid option -- '{c}'"),
                None => continue,
            }
        } else {
            continue;
        };
        let usage = if tool == "grep" {
            "\nUsage: grep [OPTION]... PATTERNS [FILE]..."
        } else {
            ""
        };
        return Some(format!(
            "{tool}: {what}{usage}\nTry '{tool} --help' for more information."
        ));
    }
    None
}

const GREP_SHORT: &str = "0123456789A:B:C:D:EFGHILPRTUVX:abcd:e:f:hilm:noqrsuvwxyZz";
const GREP_LONG: &[&str] = &[
    "extended-regexp",
    "fixed-strings",
    "basic-regexp",
    "perl-regexp",
    "regexp",
    "file",
    "ignore-case",
    "no-ignore-case",
    "word-regexp",
    "line-regexp",
    "null-data",
    "no-messages",
    "invert-match",
    "version",
    "help",
    "max-count",
    "byte-offset",
    "line-number",
    "line-buffered",
    "with-filename",
    "no-filename",
    "label",
    "only-matching",
    "quiet",
    "silent",
    "binary-files",
    "text",
    "binary",
    "unix-byte-offsets",
    "devices",
    "directories",
    "exclude",
    "exclude-from",
    "exclude-dir",
    "include",
    "recursive",
    "dereference-recursive",
    "files-with-matches",
    "files-without-match",
    "count",
    "color",
    "colour",
    "initial-tab",
    "null",
    "before-context",
    "after-context",
    "context",
    "group-separator",
    "no-group-separator",
];

fn grep(args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    if let Some(msg) = invalid_option("grep", args, GREP_SHORT, GREP_LONG) {
        sink.eline(&msg);
        return Util::Done(2);
    }
    let mut flags = HashSet::new();
    let mut patterns: Vec<String> = Vec::new();
    let mut ops = Vec::new();
    let mut filter = GrepFilter::default();
    let mut context = GrepContext::default();
    let mut i = 0;
    let mut done = false;
    while i < args.len() {
        let a = &args[i];
        if !done && a == "--" {
            done = true;
        } else if !done && a == "--no-group-separator" {
            context.no_separator = true;
        } else if let Some((kind, inline)) = grep_long_context(a).filter(|_| !done) {
            let value = match inline {
                Some(v) => v,
                None => {
                    i += 1;
                    match args.get(i) {
                        Some(v) => v.clone(),
                        None => {
                            sink.eline(&format!(
                                "grep: option '--{kind}' requires an argument\nUsage: grep [OPTION]... PATTERNS [FILE]...\nTry 'grep --help' for more information."
                            ));
                            return Util::Done(2);
                        }
                    }
                }
            };
            if let Err(msg) = context.set(kind, &value) {
                sink.eline(&format!("grep: {msg}"));
                return Util::Done(2);
            }
        } else if let Some(flag) = grep_long_flag(a).filter(|_| !done) {
            flags.insert(flag);
        } else if !done && a == "-e" {
            match args.get(i + 1) {
                Some(p) => patterns.push(p.clone()),
                None => return Util::Decline,
            }
            i += 1;
        } else if let Some((kind, inline)) = grep_glob_opt(a).filter(|_| !done) {
            let glob = match inline {
                Some(g) => g,
                None => {
                    i += 1;
                    match args.get(i) {
                        Some(g) => g.clone(),
                        None => {
                            sink.eline(&format!(
                                "grep: option '--{kind}' requires an argument\nUsage: grep [OPTION]... PATTERNS [FILE]...\nTry 'grep --help' for more information."
                            ));
                            return Util::Done(2);
                        }
                    }
                }
            };
            match kind {
                "include" => filter.include.push(glob),
                "exclude" => filter.exclude.push(glob),
                _ => filter.exclude_dir.push(glob),
            }
        } else if !done && a.starts_with("--") {
            return Util::DeclineFor(a.clone());
        } else if !done && a.len() > 1 && a.starts_with('-') {
            let (plain, with_value) = match grep_cluster(a) {
                Ok(parsed) => parsed,
                Err(c) => return Util::DeclineFor(format!("-{c}")),
            };
            flags.extend(plain);
            for (kind, inline) in with_value {
                let value = match inline {
                    Some(v) => v,
                    None => {
                        i += 1;
                        match args.get(i) {
                            Some(v) => v.clone(),
                            None => {
                                sink.eline(&format!(
                                    "grep: option requires an argument -- '{kind}'\nUsage: grep [OPTION]... PATTERNS [FILE]...\nTry 'grep --help' for more information."
                                ));
                                return Util::Done(2);
                            }
                        }
                    }
                };
                if let Err(msg) = context.set(&kind.to_string(), &value) {
                    sink.eline(&format!("grep: {msg}"));
                    return Util::Done(2);
                }
            }
        } else {
            ops.push(a.clone());
        }
        i += 1;
    }
    if patterns.is_empty() {
        if ops.is_empty() {
            sink.eline(
                "Usage: grep [OPTION]... PATTERNS [FILE]...\nTry 'grep --help' for more information.",
            );
            return Util::Done(2);
        }
        patterns.push(ops.remove(0));
    }
    let f = |c| flags.contains(&c);
    // GNU stops before it reads anything for `-m 0` (except under `-L`).
    if context.max == Some(0) && !f('L') {
        return Util::Done(1);
    }
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
            walk_files(".", "", &mut targets, true, &filter);
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
                if filter.skips_dir(&operand_names(op)) {
                    // `--exclude-dir` leaves out a directory named on the
                    // command line too.
                } else if recursive {
                    walk_files(op, op, &mut targets, false, &filter);
                } else {
                    // GNU grep says so (unless `-s`) and exits 2, as for a
                    // file it cannot read.
                    if !f('s') {
                        sink.eline(&format!("grep: {op}: Is a directory"));
                    }
                    status_err = true;
                }
            } else if (p.exists() || op == "/dev/null") && filter.skips_file(&operand_names(op)) {
                // `--include` and `--exclude` judge a file named on the command
                // line as well, by its name, before it is opened.
            } else if p.exists() || op == "/dev/null" {
                targets.push((op.clone(), Some(op.clone())));
            } else {
                if !f('s') {
                    sink.eline(&format!(
                        "grep: {}: No such file or directory",
                        gnu_quote(op)
                    ));
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
    // Whether an earlier file printed a group of lines with context, which a
    // later group is then separated from by `--`.
    let mut printed_group = false;
    for (name, path) in &targets {
        // A file found by walking a directory is not an operand the jail judged:
        // in agent mode one that holds a private key is skipped, as it is when
        // named (see `jail_refusal`).
        if path
            .as_ref()
            .is_some_and(|p| crate::security::private_key_refusal_for_tool("grep", p).is_some())
        {
            continue;
        }
        let data = match path {
            Some(p) => match read_file(p) {
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
        let all_lines = lines_of(&data);
        // Where each line starts in the file, for `-b`.
        let mut starts: Vec<usize> = Vec::new();
        if f('b') {
            let mut at = 0usize;
            for l in &all_lines {
                starts.push(at);
                at += l.len() + 1;
            }
        }
        // The prefix of a line: the file name (when several), then the line
        // number, then (`-b`) the byte offset, each ended by `:` for a selected
        // line and `-` for context. `skip` is how far into the line a `-o`
        // match starts.
        let lead_of = |n: usize, sep: u8, skip: usize| {
            let mut s = Vec::new();
            if prefix {
                s.extend_from_slice(name.as_bytes());
                s.push(sep);
            }
            if f('n') {
                s.extend_from_slice(format!("{}", n + 1).as_bytes());
                s.push(sep);
            }
            if f('b') {
                s.extend_from_slice(format!("{}", starts[n] + skip).as_bytes());
                s.push(sep);
            }
            s
        };
        let (before, after) = context.lines();
        // Under `-o` each line grep would print, selected or context, prints
        // its matches instead, one per line, as GNU grep does. A selected
        // line under `-o -v` and a context line without `-v` hold no match,
        // so they print nothing, but the `--` between groups still does and
        // the lines still count for `-c` and the exit status.
        let only_matching = f('o');
        let with_context = context.requested();
        let emit = |out: &mut Vec<u8>, n: usize, line: &[u8], sep: u8| {
            if only_matching {
                // `-b` gives each match's own offset, as GNU does under `-o`.
                for m in re.find_iter(line) {
                    if m.as_bytes().is_empty() {
                        continue;
                    }
                    out.extend_from_slice(&lead_of(n, sep, m.start()));
                    out.extend_from_slice(m.as_bytes());
                    out.push(b'\n');
                }
            } else {
                out.extend_from_slice(&lead_of(n, sep, 0));
                out.extend_from_slice(line);
                out.push(b'\n');
            }
        };
        let silent = f('q') || f('l') || f('L') || f('c') || binary;
        let mut last_printed: Option<usize> = None;
        let mut after_left = 0usize;
        for (n, line) in all_lines.iter().copied().enumerate() {
            // `-m N` is reached: only the trailing context of the last
            // selected line is left to print, whatever those lines are.
            // GNU 3.11 prints those lines as context even when they match.
            let stopped = context.max.is_some_and(|m| count >= m);
            if stopped {
                note_limit();
            }
            if stopped && (silent || !with_context || after_left == 0) {
                break;
            }
            let hit = !stopped && re.is_match(line) != f('v');
            if !hit {
                if with_context && after_left > 0 && !silent {
                    after_left -= 1;
                    emit(&mut out, n, line, b'-');
                    last_printed = Some(n);
                }
                continue;
            }
            count += 1;
            if silent {
                continue;
            }
            if with_context {
                let first = n
                    .saturating_sub(before)
                    .max(last_printed.map_or(0, |p| p + 1));
                let gap = match last_printed {
                    Some(p) => first > p + 1,
                    None => printed_group,
                };
                if gap && !context.no_separator {
                    out.extend_from_slice(context.separator.as_deref().unwrap_or("--").as_bytes());
                    out.push(b'\n');
                }
                for (k, before_line) in all_lines.iter().enumerate().take(n).skip(first) {
                    emit(&mut out, k, before_line, b'-');
                }
            }
            emit(&mut out, n, line, b':');
            if with_context {
                last_printed = Some(n);
                after_left = after;
            }
        }
        if count > 0 {
            any = true;
        }
        if with_context && !out.is_empty() {
            printed_group = true;
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
fn walk_files(
    dir: &str,
    shown: &str,
    out: &mut Vec<(String, Option<String>)>,
    strip_dot: bool,
    filter: &GrepFilter,
) {
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
            if !filter.skips_dir(&[name.as_str()]) {
                walk_files(&path, &display, out, false, filter);
            }
        } else if !filter.skips_file(&[name.as_str()]) {
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
    /// The key has an ordering option of its own (`n` or `r`), so the global
    /// `-n`, `-f` and `-r` do not apply to it, as in GNU sort.
    own: bool,
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
        own: n1 || n2 || r1 || r2,
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
    if let Some(msg) = invalid_option(
        "sort",
        args,
        "bcCdfghiMmnrRsuVyzk:o:S:t:T:",
        &[
            "batch-size",
            "buffer-size",
            "check",
            "compress-program",
            "debug",
            "dictionary-order",
            "field-separator",
            "files0-from",
            "general-numeric-sort",
            "human-numeric-sort",
            "ignore-case",
            "ignore-leading-blanks",
            "ignore-nonprinting",
            "key",
            "merge",
            "month-sort",
            "numeric-sort",
            "output",
            "parallel",
            "random-sort",
            "random-source",
            "reverse",
            "sort",
            "stable",
            "temporary-directory",
            "unique",
            "version-sort",
            "zero-terminated",
            "help",
            "version",
        ],
    ) {
        sink.eline(&msg);
        return Util::Done(2);
    }
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
        match read_file(f) {
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
            // A key with an option of its own takes none of the global ones.
            let numeric = key.numeric || (!key.own && spec.numeric);
            let o = if numeric {
                numeric_prefix(a)
                    .partial_cmp(&numeric_prefix(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            } else if spec.fold && !key.own {
                collate(&a.to_uppercase(), &b.to_uppercase())
            } else {
                collate(a, b)
            };
            let reverse = if key.own { key.reverse } else { spec.reverse };
            let o = if reverse { o.reverse() } else { o };
            if o != std::cmp::Ordering::Equal {
                return o;
            }
        }
        std::cmp::Ordering::Equal
    };
    // Lines that compare equal fall back to comparing the whole line (GNU's
    // last resort), unless `-s` or `-u`, which keep input order. A global `-r`
    // reverses the whole-line comparison and, with no keys, the line itself; with
    // keys it reaches only the keys that have no option of their own (see
    // `by_keys`). A stable tie keeps its input order.
    let last_resort = !spec.stable && !spec.unique;
    let global = |o: std::cmp::Ordering| if spec.reverse { o.reverse() } else { o };
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by(|&i, &j| {
        let k = by_keys(i, j);
        let k = if spec.keys.is_empty() { global(k) } else { k };
        k.then_with(|| {
            if last_resort {
                global(collate(&lines[i], &lines[j]))
            } else {
                std::cmp::Ordering::Equal
            }
        })
    });
    if spec.unique {
        order.dedup_by(|&mut j, &mut i| by_keys(i, j) == std::cmp::Ordering::Equal);
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
    if let Some(msg) = invalid_option(
        "cut",
        args,
        "b:c:d:f:nsz",
        &[
            "bytes",
            "characters",
            "complement",
            "delimiter",
            "fields",
            "only-delimited",
            "output-delimiter",
            "zero-terminated",
            "help",
            "version",
        ],
    ) {
        sink.eline(&msg);
        return Util::Done(1);
    }
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
            // GNU cut takes one byte, so a multi-byte character is an error.
            if d.as_ref().is_some_and(|d| d.len() > 1) {
                sink.eline("cut: the delimiter must be a single character\nTry 'cut --help' for more information.");
                return Util::Done(1);
            }
            match d.and_then(|d| d.chars().next()) {
                Some(c) => delim = c,
                None => return Util::Decline,
            }
        } else if a.starts_with("-f") {
            fields = take(&mut i, "-f").and_then(|l| cut_list(&l));
            if fields.is_none() {
                return Util::Decline;
            }
            if fields.iter().flatten().any(|&(a, b)| a > b) {
                sink.eline("cut: invalid decreasing range\nTry 'cut --help' for more information.");
                return Util::Done(1);
            }
        } else if a.starts_with("-c") || a.starts_with("-b") {
            chars = take(&mut i, "-c").and_then(|l| cut_list(&l));
            if chars.is_none() {
                return Util::Decline;
            }
            if chars.iter().flatten().any(|&(a, b)| a > b) {
                sink.eline("cut: invalid decreasing range\nTry 'cut --help' for more information.");
                return Util::Done(1);
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
        sink.eline("cut: you must specify a list of bytes, characters, or fields\nTry 'cut --help' for more information.");
        return Util::Done(1);
    }
    let within =
        |ranges: &[(usize, usize)], n: usize| ranges.iter().any(|&(a, b)| a <= n && n <= b);
    let mut status = 0;
    for (_, data) in read_inputs("cut", &files, stdin, sink, &mut status) {
        for line in lines_of(&data) {
            let text = String::from_utf8_lossy(line);
            if let Some(r) = &chars {
                // GNU cut counts bytes for `-c` as for `-b`, even in a UTF-8
                // locale, and passes the chosen bytes through as they are.
                sink.out.extend(
                    line.iter()
                        .enumerate()
                        .filter(|(k, _)| within(r, k + 1))
                        .map(|(_, b)| *b),
                );
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
    /// `/re/`; `None` is the empty regex `//`, which stands for the last regex
    /// used when the program runs.
    Re(Option<regex::bytes::Regex>),
}

enum SedCmd {
    Print,
    /// `P`: print up to the first newline of the pattern space.
    PrintFirst,
    Delete,
    /// `D`: delete up to the first newline and restart the cycle on what is
    /// left, without reading a line; with no newline it is `d`.
    DeleteFirst,
    Quit,
    /// `Q`: quit without printing.
    QuitSilent,
    /// `=`: print the line number.
    LineNo,
    /// `l`: print the pattern space unambiguously.
    List,
    /// `n`: print the pattern space (unless `-n`) and read the next line.
    Next,
    /// `N`: append a newline and the next line to the pattern space.
    NextAppend,
    /// `h`, `H`, `g`, `G`, `x`: copy, append, get, get-append, exchange the
    /// pattern and hold spaces.
    Hold,
    HoldAppend,
    Get,
    GetAppend,
    Exchange,
    /// `y/abc/xyz/`: the characters and what each becomes.
    Trans(Vec<(char, char)>),
    /// `s/re/rep/`: the regex (`None` for the empty regex, see [`Addr::Re`]),
    /// the replacement, and the `g`, `p` and occurrence-number flags.
    Subst(Option<regex::bytes::Regex>, Vec<RepItem>, bool, bool, usize),
    /// `{`: the index of its matching `}` in the program; an unselected block
    /// jumps there.
    Block(usize),
    EndBlock,
    /// `a`, `i` and `c`: the text, with its closing newline.
    Append(Vec<u8>),
    Insert(Vec<u8>),
    Change(Vec<u8>),
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
                // `\L`, `\U` and `\E` also drop a `\u` or `\l` not yet applied.
                match *c {
                    'U' | 'L' => {
                        hold = Some(*c);
                        next = None;
                    }
                    'E' => {
                        hold = None;
                        next = None;
                    }
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
    /// `!` after the address: the command runs on the lines it does not select.
    neg: bool,
    cmd: SedCmd,
    active: bool,
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

/// The SUFFIX of a `sed` in-place option, or `None` when `s` is not one. GNU
/// takes the suffix only attached (`-i.bak`), and `-i` alone has none. The long
/// form is the same option: `--in-place` or `--in-place=SUFFIX`. A suffix with
/// `/` or `*` (GNU's backup-name template) declines.
fn sed_inplace_suffix(s: &str) -> Option<String> {
    let rest = match s.strip_prefix("--in-place") {
        Some("") => "",
        Some(tail) => tail.strip_prefix('=')?,
        None => s.strip_prefix("-i")?,
    };
    (!rest.contains(['/', '*'])).then(|| rest.to_string())
}

/// `sed -i` runs `sed` on each FILE in place. Each one is judged as a `>` target
/// is ([`check_write_in_place`]) and read through the read jail, so an outside
/// FILE is refused and left alone, as the other FILEs still run. Each is
/// edited on its own: line numbers and `$` restart, and a range starts closed
/// (GNU's `-s`). `q` stops after its FILE. With a SUFFIX the original bytes go
/// to FILE+SUFFIX first, as a new file. The output keeps a missing final
/// newline missing, as GNU does.
fn sed_in_place(
    prog: &mut [SedLine],
    quiet: bool,
    suffix: &str,
    files: &[String],
    sink: &mut Sink,
) -> Util {
    if files.is_empty() {
        sink.eline("sed: no input files");
        return Util::Done(4);
    }
    let mut status = 0;
    let mut quit = false;
    for f in files {
        if quit {
            break;
        }
        if f == "-" {
            sink.eline("sed: couldn't edit -: not a regular file");
            status = 4;
            continue;
        }
        let resolved = match check_write_in_place(f) {
            Ok(p) => p,
            Err(msg) => {
                sink.eline(&gnu_refusal(&format!("sed: {f}"), &msg));
                status = 1;
                continue;
            }
        };
        if let Some(refusal) = jail_refusal("sed", f) {
            sink.eline(&refusal);
            status = 1;
            continue;
        }
        let data = match std::fs::read(&resolved) {
            Ok(d) => d,
            Err(e) => {
                sink.eline(&format!("sed: can't read {f}: {}", io_reason(&e)));
                status = 2;
                continue;
            }
        };
        for sl in prog.iter_mut() {
            sl.active = matches!(sl.a1, Some(Addr::Line(0)));
        }
        let mut out = Vec::new();
        let end = sed_run(prog, quiet, &data, true, &mut out);
        quit |= end.quit;
        if let Some(m) = end.failed {
            // A run-time error leaves the file as it was.
            sink.eline(&format!("sed: -e expression #1, char 0: {m}"));
            status = 1;
            continue;
        }
        if !suffix.is_empty() {
            let backup = match check_write_in_place(&format!("{f}{suffix}")) {
                Ok(p) => p,
                Err(msg) => {
                    sink.eline(&gnu_refusal(&format!("sed: {f}{suffix}"), &msg));
                    status = 1;
                    continue;
                }
            };
            // The edit and its backup are one journal step, so `undo(1)` puts
            // the file back and removes the backup together. The step is kept
            // once the backup is written, even if the edit then fails: the
            // backup exists and undo is what removes it.
            let mark = crate::journal::mark();
            crate::journal::record_group(
                "posix",
                crate::safety::Effect::WriteLocal,
                vec![backup.clone(), resolved.clone()],
            );
            let made = write_file_unrecorded(&backup, &data, false);
            let edited = match &made {
                Ok(()) => write_file_unrecorded(&resolved, &out, false),
                Err(_) => Ok(()),
            };
            crate::journal::settle(mark, made.is_ok());
            if let Err(msg) = made.and(edited) {
                sink.eline(&msg);
                status = 1;
            }
            continue;
        }
        if let Err(msg) = write_file(&resolved, &out, false) {
            sink.eline(&msg);
            status = 1;
        }
    }
    Util::Done(status)
}

/// One address of a sed command: a line number, `$` or `/re/`.
fn sed_addr(
    c: &[char],
    k: &mut usize,
    to_re: &dyn Fn(&str) -> Option<regex::bytes::Regex>,
) -> Result<Option<Addr>, ()> {
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
            if pat.is_empty() {
                return Ok(Some(Addr::Re(None)));
            }
            Ok(Some(Addr::Re(Some(to_re(&pat).ok_or(())?))))
        }
        _ => Ok(None),
    }
}

/// The byte of a GNU sed numeric escape written after `\c`, `\d`, `\o` or `\x`
/// (`kind`), reading from `c[*i]`: `\cX` is control-X, `\dNNN` up to three
/// decimal digits, `\oNNN` up to three octal digits and `\xHH` up to two hex
/// digits. `None`, with `*i` unmoved, when no character or digit follows.
fn sed_num_escape(c: &[char], i: &mut usize, kind: char) -> Option<u8> {
    if kind == 'c' {
        let x = *c.get(*i)?;
        if !x.is_ascii() {
            return None;
        }
        *i += 1;
        return Some(x.to_ascii_uppercase() as u8 ^ 0x40);
    }
    let (radix, max) = match kind {
        'd' => (10, 3),
        'o' => (8, 3),
        _ => (16, 2),
    };
    let mut v = 0u32;
    let mut n = 0;
    while n < max {
        match c.get(*i + n).and_then(|d| d.to_digit(radix)) {
            Some(d) => v = v * radix + d,
            None => break,
        }
        n += 1;
    }
    if n == 0 {
        return None;
    }
    *i += n;
    Some(v as u8)
}

/// A sed pattern with the GNU escapes read: `\n`, `\t`, `\a`, `\f`, `\r` and
/// `\v` as the characters, and `\cX`, `\dNNN`, `\oNNN` and `\xHH` as the
/// character they name (which then keeps any meaning it has in a regex, as in
/// GNU: `\x2e` is a `.` that matches any character). Also inside `[...]`;
/// `\\n` stays a backslash and `n`. `None` for a character that cannot be put
/// in a pattern string (a byte above 127, a backslash).
fn sed_pattern_escapes(p: &str) -> Option<String> {
    let c: Vec<char> = p.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] != '\\' {
            out.push(c[i]);
            i += 1;
            continue;
        }
        let Some(&n) = c.get(i + 1) else {
            out.push('\\');
            i += 1;
            continue;
        };
        i += 2;
        match n {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'a' => out.push('\x07'),
            'f' => out.push('\x0c'),
            'r' => out.push('\r'),
            'v' => out.push('\x0b'),
            'c' | 'd' | 'o' | 'x' => match sed_num_escape(&c, &mut i, n) {
                Some(b) if b < 0x80 && b != b'\\' => out.push(b as char),
                Some(_) => return None,
                // With no digits the escape is the letter itself.
                None => out.push(n),
            },
            o => {
                out.push('\\');
                out.push(o);
            }
        }
    }
    Some(out)
}

/// The bytes of the text of an `a`, `i` or `c` command, from its raw characters
/// (a backslash and the character after it stay together, so `\` and a newline
/// continue the text): `\a`, `\f`, `\n`, `\r`, `\t`, `\v`, `\cX`, `\dNNN`,
/// `\oNNN` and `\xHH` are the characters GNU sed reads them as, and any other
/// backslash is dropped and keeps the character after it (`\\` is one
/// backslash, `\q` is `q`).
fn sed_text_bytes(c: &[char]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    let push = |out: &mut Vec<u8>, ch: char| {
        let mut b = [0; 4];
        out.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
    };
    while i < c.len() {
        let ch = c[i];
        i += 1;
        if ch != '\\' {
            push(&mut out, ch);
            continue;
        }
        let Some(&n) = c.get(i) else { break };
        i += 1;
        match n {
            'a' => out.push(0x07),
            'f' => out.push(0x0c),
            'n' => out.push(b'\n'),
            'r' => out.push(b'\r'),
            't' => out.push(b'\t'),
            'v' => out.push(0x0b),
            'c' | 'd' | 'o' | 'x' => match sed_num_escape(c, &mut i, n) {
                Some(b) => out.push(b),
                None => push(&mut out, n),
            },
            o => push(&mut out, o),
        }
    }
    out
}

/// Whether a regex has a repetition with nothing to repeat, which GNU sed's
/// regex compiler refuses: in a BRE a `*` right after another `*`, in an ERE a
/// `*`, `+`, `?` or `{` at the start, after `(` or `|`, or after a leading `^`.
fn sed_dangling_repeat(p: &str, ere: bool) -> bool {
    let c: Vec<char> = p.chars().collect();
    let mut in_bracket = false;
    // Nothing to repeat yet: the start of the regex, after `(` or `|` (in a BRE
    // `\(` or `\|`), and after a leading `^`.
    let mut start = true;
    // The last piece was a BRE `*` that repeats something.
    let mut star = false;
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        i += 1;
        if in_bracket {
            if ch == ']' {
                in_bracket = false;
            }
            continue;
        }
        match ch {
            '[' => {
                in_bracket = true;
                if c.get(i) == Some(&'^') {
                    i += 1;
                }
                if c.get(i) == Some(&']') {
                    i += 1;
                }
                start = false;
                star = false;
            }
            '\\' => {
                let n = c.get(i).copied();
                i += 1;
                start = !ere && matches!(n, Some('(' | '|'));
                star = false;
            }
            '(' | '|' if ere => {
                start = true;
                star = false;
            }
            '^' if start => star = false,
            '^' if ere => {
                start = true;
                star = false;
            }
            '*' | '+' | '?' | '{' if ere => {
                if start {
                    return true;
                }
                start = false;
                star = false;
            }
            '*' if !ere => {
                if star {
                    return true;
                }
                if start {
                    start = false;
                } else {
                    star = true;
                }
            }
            _ => {
                start = false;
                star = false;
            }
        }
    }
    false
}

/// A sed script error GNU reports itself (`sed: -e expression #1, char N:
/// ...`, status 1), as opposed to a construct this sed declines. The marker
/// byte tells [`sed`] which one an `Err` from [`sed_parse`] is.
fn sed_bad(msg: &str, at: usize) -> String {
    format!("\u{1}sed: -e expression #1, char {at}: {msg}")
}

/// Reads a sed script (the `-e` pieces joined by newlines) into a flat program:
/// commands are separated by `;` or a newline, `{` and `}` bracket a block
/// (kept as the `Block` and `EndBlock` entries), and `a`, `i`, `c` take their
/// text from the rest of the line, or from the next line after `a\`, a line
/// ending in `\` continuing it. `Err` says what makes it decline.
fn sed_parse(script: &str, ere: bool) -> Result<Vec<SedLine>, String> {
    let build = |p: &str, multiline: bool| -> Option<regex::bytes::Regex> {
        let p = sed_pattern_escapes(p)?;
        // GNU sed rejects a repetition with nothing to repeat (`a**`, ERE
        // `^*`); the regex engine here would accept it. Decline, so the error
        // is GNU's.
        if sed_dangling_repeat(&p, ere) {
            return None;
        }
        let r = if ere {
            ere_to_regex(&p)?
        } else {
            bre_to_regex(&p)?
        };
        // The pattern space can hold several lines (`N`, `G`), and in sed `.`
        // matches a newline there.
        // With `M`, `^` and `$` also match at the line breaks inside it.
        let flags = if multiline { "(?sm)" } else { "(?s)" };
        regex::bytes::Regex::new(&format!("{flags}{r}")).ok()
    };
    let to_re = |p: &str| build(p, false);
    let c: Vec<char> = script.chars().collect();
    // The text from `at` to the end of its command, for a refusal message.
    let segment = |at: usize| -> String {
        let end = c[at..]
            .iter()
            .position(|&x| x == ';' || x == '\n')
            .map_or(c.len(), |e| at + e);
        c[at..end].iter().collect::<String>().trim().to_string()
    };
    let mut prog: Vec<SedLine> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut k = 0;
    loop {
        while k < c.len() && (c[k].is_whitespace() || c[k] == ';') {
            k += 1;
        }
        if k >= c.len() {
            break;
        }
        let start = k;
        let Ok(a1) = sed_addr(&c, &mut k, &to_re) else {
            return Err(format!("the address `{}`", segment(start)));
        };
        let mut a2 = None;
        if a1.is_some() && c.get(k) == Some(&',') {
            k += 1;
            match sed_addr(&c, &mut k, &to_re) {
                Ok(Some(a)) => a2 = Some(a),
                _ => return Err(format!("the address range `{}`", segment(start))),
            }
        }
        while matches!(c.get(k), Some(' ' | '\t')) {
            k += 1;
        }
        // `!` (and any spaces after it) negates the address.
        let mut neg = false;
        while c.get(k) == Some(&'!') {
            if neg {
                return Err(format!(
                    "sed command `{}` with multiple `!`",
                    segment(start)
                ));
            }
            neg = true;
            k += 1;
            while matches!(c.get(k), Some(' ' | '\t')) {
                k += 1;
            }
        }
        let Some(&cc) = c.get(k) else {
            return Err(format!("the address `{}` with no command", segment(start)));
        };
        if cc == '\n' || cc == ';' {
            return Err(format!("the address `{}` with no command", segment(start)));
        }
        let at = k;
        k += 1;
        // After `p`, `d`, `q`, `{` or `}` the command ends at a separator or a `}`.
        let ends_here = |k: &mut usize| -> bool {
            while matches!(c.get(*k), Some(' ' | '\t')) {
                *k += 1;
            }
            matches!(c.get(*k), None | Some(';' | '\n' | '}'))
        };
        let cmd = match cc {
            '{' => {
                open.push(prog.len());
                SedCmd::Block(0)
            }
            '}' => {
                if a1.is_some() {
                    return Err(format!("sed command `{}`", segment(start)));
                }
                let Some(b) = open.pop() else {
                    return Err(sed_bad("unexpected `}'", k));
                };
                prog[b].cmd = SedCmd::Block(prog.len());
                // GNU wants the end of the command after a `}`: `2{p}3{p}`
                // is an error, not two commands.
                if !ends_here(&mut k) {
                    return Err(sed_bad("extra characters after command", k + 1));
                }
                SedCmd::EndBlock
            }
            'p' | 'd' | 'q' | 'P' | 'D' | 'Q' | '=' | 'l' | 'n' | 'N' | 'h' | 'H' | 'g' | 'G'
            | 'x' => {
                if !ends_here(&mut k) {
                    return Err(format!("sed command `{}`", segment(at)));
                }
                match cc {
                    'p' => SedCmd::Print,
                    'd' => SedCmd::Delete,
                    'q' => SedCmd::Quit,
                    'P' => SedCmd::PrintFirst,
                    'D' => SedCmd::DeleteFirst,
                    'Q' => SedCmd::QuitSilent,
                    '=' => SedCmd::LineNo,
                    'l' => SedCmd::List,
                    'n' => SedCmd::Next,
                    'N' => SedCmd::NextAppend,
                    'h' => SedCmd::Hold,
                    'H' => SedCmd::HoldAppend,
                    'g' => SedCmd::Get,
                    'G' => SedCmd::GetAppend,
                    _ => SedCmd::Exchange,
                }
            }
            'y' => {
                let Some(&sep) = c.get(k) else {
                    return Err(format!("sed command `{}`", segment(at)));
                };
                if sep == '\n' || sep == '\\' {
                    return Err(format!("sed command `{}`", segment(at)));
                }
                k += 1;
                // Two parts, each closed by an unescaped delimiter; `\\`, `\n`
                // and an escaped delimiter are one character each.
                let mut parts: [Vec<char>; 2] = [Vec::new(), Vec::new()];
                let mut closed = 0;
                while closed < 2 && k < c.len() {
                    let ch = c[k];
                    k += 1;
                    if ch == '\\' {
                        match c.get(k) {
                            Some('\\') => parts[closed].push('\\'),
                            Some('n') => parts[closed].push('\n'),
                            Some(&d) if d == sep => parts[closed].push(d),
                            _ => return Err(format!("sed command `{}`", segment(at))),
                        }
                        k += 1;
                    } else if ch == sep {
                        closed += 1;
                    } else {
                        parts[closed].push(ch);
                    }
                }
                let [from, to] = parts;
                if closed < 2 || from.len() != to.len() || !ends_here(&mut k) {
                    return Err(format!("sed command `{}`", segment(at)));
                }
                SedCmd::Trans(from.into_iter().zip(to).collect())
            }
            'a' | 'i' | 'c' => {
                while matches!(c.get(k), Some(' ' | '\t')) {
                    k += 1;
                }
                if c.get(k) == Some(&'\\') {
                    k += 1;
                    // `a\` and a newline: the text starts on the next line.
                    let mut j = k;
                    while matches!(c.get(j), Some(' ' | '\t')) {
                        j += 1;
                    }
                    if c.get(j) == Some(&'\n') {
                        k = j + 1;
                    }
                }
                if k >= c.len() {
                    return Err(format!("sed command `{}` with no text", segment(at)));
                }
                // The raw text, a backslash and the character after it kept
                // together; its escapes are read by `sed_text_bytes`.
                let mut raw: Vec<char> = Vec::new();
                while k < c.len() && c[k] != '\n' {
                    if c[k] == '\\' {
                        raw.push('\\');
                        k += 1;
                    }
                    if let Some(&ch) = c.get(k) {
                        raw.push(ch);
                        k += 1;
                    }
                }
                let mut bytes = sed_text_bytes(&raw);
                bytes.push(b'\n');
                match cc {
                    'a' => SedCmd::Append(bytes),
                    'i' => SedCmd::Insert(bytes),
                    _ => SedCmd::Change(bytes),
                }
            }
            's' => {
                let Some(&sep) = c.get(k) else {
                    return Err(format!("sed command `{}`", segment(at)));
                };
                if sep == '\n' || sep == '\\' || sep == ';' {
                    return Err(format!("sed command `{}`", segment(at)));
                }
                k += 1;
                // The pattern and the replacement, each closed by an unescaped
                // delimiter; a backslash keeps the character after it.
                let mut parts = [String::new(), String::new()];
                let mut closed = 0;
                while closed < 2 && k < c.len() {
                    let ch = c[k];
                    k += 1;
                    if ch == '\\' {
                        parts[closed].push('\\');
                        if let Some(&n) = c.get(k) {
                            parts[closed].push(n);
                            k += 1;
                        }
                    } else if ch == sep {
                        closed += 1;
                    } else {
                        parts[closed].push(ch);
                    }
                }
                if closed < 2 {
                    return Err(format!("the s command with its parts not split by `{sep}`"));
                }
                let [pat, rep] = parts;
                let mut fl = String::new();
                while let Some(&f) = c.get(k).filter(|f| f.is_ascii_alphanumeric()) {
                    fl.push(f);
                    k += 1;
                }
                let pat = unescape_delim(&pat, sep, ere);
                let digits: String = fl.chars().take_while(char::is_ascii_digit).collect();
                let tail = &fl[digits.len()..];
                let multiline = tail.contains(['M', 'm']);
                if let Some(f) = tail.chars().find(|f| !matches!(f, 'g' | 'p' | 'M' | 'm')) {
                    return Err(format!("the s command's flag `{f}`"));
                }
                if !ends_here(&mut k) {
                    return Err(format!("sed command `{}`", segment(at)));
                }
                let nth = if digits.is_empty() {
                    1
                } else {
                    match digits.parse::<usize>() {
                        Ok(n) if n >= 1 => n,
                        _ => return Err(format!("the s command's occurrence number `{digits}`")),
                    }
                };
                // An empty regex is the last one used when the program runs;
                // it takes no flags of its own to read here.
                let re = if pat.is_empty() {
                    if multiline {
                        return Err("the s command's flag `M` on an empty regex".to_string());
                    }
                    None
                } else {
                    match build(pat.as_str(), multiline) {
                        Some(re) => Some(re),
                        None => return Err(format!("the regular expression `{pat}`")),
                    }
                };
                let groups = re.as_ref().map(|r| r.captures_len() - 1);
                // `&` is the match and `\N` a group; the case escapes are
                // applied to the text they reach (see `expand_rep`).
                let mut items: Vec<RepItem> = Vec::new();
                let rc: Vec<char> = rep.chars().collect();
                let mut r = 0;
                while r < rc.len() {
                    let ch = rc[r];
                    r += 1;
                    let mut b = [0; 4];
                    let bytes: Vec<u8> = match ch {
                        '&' => {
                            items.push(RepItem::Group(0));
                            continue;
                        }
                        '\\' => match rc.get(r).copied() {
                            None => b"\\".to_vec(),
                            Some(n) => {
                                r += 1;
                                match n {
                                    // `\0` is the whole match, as `&` is.
                                    '0'..='9' => {
                                        let g = n as usize - '0' as usize;
                                        if groups.is_some_and(|have| g > have) {
                                            return Err(sed_bad(
                                                &format!(
                                                    "invalid reference \\{g} on `s' command's RHS"
                                                ),
                                                k,
                                            ));
                                        }
                                        items.push(RepItem::Group(g));
                                        continue;
                                    }
                                    'U' | 'L' | 'u' | 'l' | 'E' => {
                                        items.push(RepItem::Case(n));
                                        continue;
                                    }
                                    'n' => b"\n".to_vec(),
                                    't' => b"\t".to_vec(),
                                    'a' => vec![0x07],
                                    'f' => vec![0x0c],
                                    'r' => b"\r".to_vec(),
                                    'v' => vec![0x0b],
                                    'c' | 'd' | 'o' | 'x' => match sed_num_escape(&rc, &mut r, n) {
                                        Some(v) => vec![v],
                                        // `\c` at the end is a backslash; the
                                        // others, with no digits, their letter.
                                        None if n == 'c' => b"\\".to_vec(),
                                        None => vec![n as u8],
                                    },
                                    o => o.encode_utf8(&mut b).as_bytes().to_vec(),
                                }
                            }
                        },
                        o => o.encode_utf8(&mut b).as_bytes().to_vec(),
                    };
                    match items.last_mut() {
                        Some(RepItem::Text(t)) => t.extend_from_slice(&bytes),
                        _ => items.push(RepItem::Text(bytes)),
                    }
                }
                SedCmd::Subst(re, items, tail.contains('g'), tail.contains('p'), nth)
            }
            _ => return Err(format!("sed command `{}`", segment(at))),
        };
        // Line 0 is only the start of a `0,/re/` range.
        let a1_zero = matches!(a1, Some(Addr::Line(0)));
        if a1_zero && !matches!(a2, Some(Addr::Re(_))) {
            return Err(sed_bad("invalid usage of line address 0", k));
        }
        prog.push(SedLine {
            a1,
            a2,
            neg,
            cmd,
            // `0,/re/` starts open: its regex is tried on the first line too.
            active: a1_zero,
        });
    }
    if !open.is_empty() {
        return Err("a `{` with no `}`".to_string());
    }
    Ok(prog)
}

fn sed(args: &[String], stdin: &mut Input, sink: &mut Sink) -> Util {
    let mut quiet = false;
    let mut ere = false;
    let mut scripts = Vec::new();
    let mut files = Vec::new();
    let mut positional: Vec<String> = Vec::new();
    let mut inplace: Option<String> = None;
    let mut separate = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "-n" => quiet = true,
            "-E" | "-r" => ere = true,
            "-s" | "--separate" => separate = true,
            s if sed_inplace_suffix(s).is_some() => inplace = sed_inplace_suffix(s),
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
            _ => positional.push(a.clone()),
        }
        i += 1;
    }
    // Without -e the first word is the script; with one, every word is a file.
    if scripts.is_empty() && !positional.is_empty() {
        scripts.push(positional.remove(0));
    }
    files.extend(positional);
    // Only the text of `a`, `i` and `c` goes on in the next -e: a script
    // that ends in a backslash anywhere else is an error GNU sed reports.
    // The line has an `a`, `i` or `c` at the start of a command, after its
    // address: its text is what the backslash continues.
    let text_cmd_re = regex::Regex::new(
        r"(?:^|[;{}])\s*(?:(?:\d+|\$|/(?:[^/\\]|\\.)*/)(?:\s*,\s*(?:\d+|\$|/(?:[^/\\]|\\.)*/))?\s*!?\s*)?[aic](?:\s|\\|$)",
    )
    .ok();
    for sc in scripts.iter().take(scripts.len().saturating_sub(1)) {
        let slashes = sc.chars().rev().take_while(|&c| c == '\\').count();
        let last_line = sc.rsplit('\n').next().unwrap_or("");
        let text_cmd = text_cmd_re
            .as_ref()
            .is_some_and(|re| re.is_match(last_line));
        if slashes % 2 == 1 && !text_cmd {
            return Util::DeclineFor("an -e script ending in a backslash".into());
        }
    }
    let mut prog = match sed_parse(&scripts.join("\n"), ere) {
        Ok(prog) => prog,
        Err(why) => {
            // An error GNU sed reports itself: print it, as sed does.
            if let Some(msg) = why.strip_prefix('\u{1}') {
                sink.eline(msg);
                return Util::Done(1);
            }
            return Util::DeclineFor(why);
        }
    };
    if prog.is_empty() {
        return Util::Decline;
    }
    if let Some(suffix) = inplace {
        return sed_in_place(&mut prog, quiet, &suffix, &files, sink);
    }
    // `-s` treats each file as its own input (line numbers and `$` restart).
    // With one input that is what sed does anyway; with several, the system
    // sed runs it.
    if separate && files.len() > 1 {
        return Util::DeclineFor("option `-s` with more than one file".into());
    }
    let mut status = 0;
    let mut data = Vec::new();
    for (_, d) in read_inputs("sed", &files, stdin, sink, &mut status) {
        // A file without a final newline still ends its last line: GNU sed
        // does not join it to the next file's first line.
        if !data.is_empty() && !data.ends_with(b"\n") {
            data.push(b'\n');
        }
        data.extend_from_slice(&d);
    }
    let mut out = Vec::new();
    // As GNU sed, a last line with no final newline is printed without one.
    let end = sed_run(&mut prog, quiet, &data, true, &mut out);
    sink.out.extend_from_slice(&out);
    if let Some(m) = end.failed {
        sink.eline(&format!("sed: -e expression #1, char 0: {m}"));
        return Util::Done(1);
    }
    // An input file that cannot be read is status 2 in sed.
    Util::Done(if status != 0 { 2 } else { 0 })
}

/// `l`: the pattern space with each non-printing byte as an escape (`\n`,
/// `\t`, `\\`, `\ooo`), wrapped with a `\` at the end of a line of 69
/// columns, and a `$` at the end, as GNU sed lists it.
fn sed_list(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut width = 0;
    for &b in text {
        let piece: Vec<u8> = match b {
            b'\\' => b"\\\\".to_vec(),
            0x07 => b"\\a".to_vec(),
            0x08 => b"\\b".to_vec(),
            0x0c => b"\\f".to_vec(),
            b'\n' => b"\\n".to_vec(),
            b'\r' => b"\\r".to_vec(),
            b'\t' => b"\\t".to_vec(),
            0x0b => b"\\v".to_vec(),
            0x20..=0x7e => vec![b],
            _ => format!("\\{b:03o}").into_bytes(),
        };
        if width + piece.len() > 69 {
            out.extend_from_slice(b"\\\n");
            width = 0;
        }
        width += piece.len();
        out.extend_from_slice(&piece);
    }
    out.extend_from_slice(b"$\n");
    out
}

/// How [`sed_run`] ended: whether `q` ran, and the message of a run-time error
/// (`no previous regular expression`), which stops the program with status 1.
struct SedEnd {
    quit: bool,
    failed: Option<String>,
}

/// The regex a `/re/` or `s/re/` stands for: itself, which then becomes the
/// last regex used, or for the empty regex that last one.
fn sed_resolve<'a>(
    r: &'a Option<regex::bytes::Regex>,
    last: &'a mut Option<regex::bytes::Regex>,
) -> Result<&'a regex::bytes::Regex, String> {
    match r {
        Some(x) => {
            *last = Some(x.clone());
            Ok(x)
        }
        None => last
            .as_ref()
            .ok_or_else(|| "no previous regular expression".to_string()),
    }
}

/// Runs the sed program `prog` over the lines of `data`, appending what it prints
/// to `out`. With `keep_missing_newline`, a last line with no final newline
/// is printed without one, as GNU sed does; otherwise each printed line ends in a
/// newline. Returns true when `q` ran, so no later input is read.
///
/// The cycle is GNU sed's: a line is read into the pattern space, the program
/// runs, the pattern space is printed unless `-n` or the cycle was ended by
/// `d`/`D`, and the text queued by `a` follows. `n` and `N` read the next line
/// inside the cycle (at the end of the input they end it, printing as usual),
/// `D` restarts the cycle without reading, and `h`/`H`/`g`/`G`/`x` use the hold
/// space.
fn sed_run(
    prog: &mut [SedLine],
    quiet: bool,
    data: &[u8],
    keep_missing_newline: bool,
    out: &mut Vec<u8>,
) -> SedEnd {
    // A run-time error ends the program at once, with what it printed so far.
    macro_rules! tri {
        ($e:expr) => {
            match $e {
                Ok(v) => v,
                Err(m) => {
                    return SedEnd {
                        quit: false,
                        failed: Some(m),
                    }
                }
            }
        };
    }
    // The last regex used, which an empty regex stands for.
    let mut last: Option<regex::bytes::Regex> = None;
    let unterminated = keep_missing_newline && !data.is_empty() && !data.ends_with(b"\n");
    // Set when the last line went out without its newline: the next output
    // starts with that newline.
    let mut missing = false;
    // `terminated` is whether the line being printed had its newline (see
    // `chomped` below).
    let emit = |dst: &mut Vec<u8>, missing: &mut bool, text: &[u8], terminated: bool| {
        if *missing {
            dst.push(b'\n');
            *missing = false;
        }
        dst.extend_from_slice(text);
        if !terminated {
            *missing = true;
        } else {
            dst.push(b'\n');
        }
    };
    // `a`, `i` and `c` text goes out as it is, after any newline the last line
    // went without.
    let put = |dst: &mut Vec<u8>, missing: &mut bool, text: &[u8]| {
        if *missing {
            dst.push(b'\n');
            *missing = false;
        }
        dst.extend_from_slice(text);
    };
    let lines = lines_of(data);
    let n = lines.len();
    // Index of the next line to read; the current line number is `next`.
    let mut next = 0usize;
    let mut hold: Vec<u8> = Vec::new();
    let mut pattern: Vec<u8> = Vec::new();
    // Whether the last line read had its newline, and whether the hold space
    // counts as having one (it does until `h`/`x` copy a line that had none):
    // GNU sed prints the newline of a pattern space only if this is set, and
    // `g`, `G`, `h`, `H` and `x` carry it along with the text.
    let mut chomped = true;
    let mut hold_chomped = true;
    // Set by `D`: the next cycle runs on the pattern space left, without a read.
    let mut restart = false;
    while restart || next < n {
        if restart {
            restart = false;
        } else {
            pattern = lines[next].to_vec();
            next += 1;
            chomped = !(next == n && unterminated);
        }
        // `a` text waits for the end of the cycle (or the next read).
        let mut appended: Vec<Vec<u8>> = Vec::new();
        // The cycle ended by `d`/`D`/`c`: no autoprint.
        let mut deleted = false;
        let mut pc = 0;
        while pc < prog.len() {
            let ln = next;
            let hits = |a: &Addr,
                        line: &[u8],
                        last: &mut Option<regex::bytes::Regex>|
             -> Result<bool, String> {
                Ok(match a {
                    Addr::Line(k) => *k == ln,
                    Addr::Last => ln == n,
                    Addr::Re(r) => sed_resolve(r, last)?.is_match(line),
                })
            };
            let sl = &mut prog[pc];
            let matched = match (&sl.a1, &sl.a2) {
                (None, _) => true,
                (Some(a), None) => tri!(hits(a, &pattern, &mut last)),
                (Some(a), Some(b)) => {
                    if sl.active {
                        match b {
                            // A line number already passed (`n` or `N` read
                            // it inside a cycle) closes the range without
                            // selecting this line.
                            Addr::Line(k) if ln > *k => {
                                sl.active = false;
                                false
                            }
                            _ => {
                                let end = match b {
                                    Addr::Line(k) => ln >= *k,
                                    _ => tri!(hits(b, &pattern, &mut last)),
                                };
                                if end {
                                    sl.active = false;
                                }
                                true
                            }
                        }
                    } else if tri!(hits(a, &pattern, &mut last)) {
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
            if matched == sl.neg {
                // An unselected block is skipped to its `}`.
                pc = match sl.cmd {
                    SedCmd::Block(end) => end,
                    _ => pc + 1,
                };
                continue;
            }
            pc += 1;
            match &sl.cmd {
                SedCmd::Block(_) | SedCmd::EndBlock => {}
                SedCmd::Print => emit(out, &mut missing, &pattern, chomped),
                SedCmd::PrintFirst => match pattern.iter().position(|&b| b == b'\n') {
                    Some(at) => {
                        let head = pattern[..at].to_vec();
                        emit(out, &mut missing, &head, true);
                    }
                    None => emit(out, &mut missing, &pattern, chomped),
                },
                SedCmd::LineNo => {
                    let text = format!("{ln}\n");
                    put(out, &mut missing, text.as_bytes());
                }
                SedCmd::List => {
                    let text = sed_list(&pattern);
                    put(out, &mut missing, &text);
                }
                SedCmd::Hold => {
                    hold = pattern.clone();
                    hold_chomped = chomped;
                }
                SedCmd::HoldAppend => {
                    hold.push(b'\n');
                    hold.extend_from_slice(&pattern);
                    hold_chomped = chomped;
                }
                SedCmd::Get => {
                    pattern = hold.clone();
                    chomped = hold_chomped;
                }
                SedCmd::GetAppend => {
                    pattern.push(b'\n');
                    pattern.extend_from_slice(&hold);
                    chomped = hold_chomped;
                }
                SedCmd::Exchange => {
                    std::mem::swap(&mut pattern, &mut hold);
                    std::mem::swap(&mut chomped, &mut hold_chomped);
                }
                SedCmd::Trans(map) => {
                    let text = String::from_utf8_lossy(&pattern).into_owned();
                    let mapped: String = text
                        .chars()
                        .map(|ch| map.iter().find(|(f, _)| *f == ch).map_or(ch, |(_, t)| *t))
                        .collect();
                    if mapped != text {
                        pattern = mapped.into_bytes();
                    }
                }
                SedCmd::Next | SedCmd::NextAppend => {
                    // With no next line the commands after this one do not run,
                    // and the cycle ends as usual (GNU prints the pattern space).
                    if next >= n {
                        break;
                    }
                    if matches!(sl.cmd, SedCmd::Next) {
                        if !quiet {
                            emit(out, &mut missing, &pattern, chomped);
                        }
                        pattern.clear();
                    } else {
                        pattern.push(b'\n');
                    }
                    for t in appended.drain(..) {
                        put(out, &mut missing, &t);
                    }
                    pattern.extend_from_slice(lines[next]);
                    next += 1;
                    chomped = !(next == n && unterminated);
                }
                SedCmd::Append(t) => appended.push(t.clone()),
                SedCmd::Insert(t) => put(out, &mut missing, t),
                SedCmd::Change(t) => {
                    // Over a range the text goes out once, at its end.
                    if sl.a2.is_none() || !sl.active {
                        put(out, &mut missing, t);
                    }
                    deleted = true;
                    break;
                }
                SedCmd::Delete => {
                    deleted = true;
                    break;
                }
                SedCmd::DeleteFirst => {
                    deleted = true;
                    if let Some(at) = pattern.iter().position(|&b| b == b'\n') {
                        pattern.drain(..=at);
                        restart = true;
                    }
                    break;
                }
                SedCmd::Quit | SedCmd::QuitSilent => {
                    // `q` prints the pattern space, ends a line left without its
                    // newline, and writes the queued `a` text; `Q` does none.
                    if matches!(sl.cmd, SedCmd::Quit) {
                        if !quiet {
                            emit(out, &mut missing, &pattern, chomped);
                        }
                        for t in &appended {
                            put(out, &mut missing, t);
                        }
                        put(out, &mut missing, b"");
                    }
                    note_limit();
                    return SedEnd {
                        quit: true,
                        failed: None,
                    };
                }
                SedCmd::Subst(re, rep, global, print, nth) => {
                    let re = tri!(sed_resolve(re, &mut last));
                    // An empty regex has no group count to check at parse time.
                    let top = rep
                        .iter()
                        .filter_map(|i| match i {
                            RepItem::Group(g) => Some(*g),
                            _ => None,
                        })
                        .max()
                        .unwrap_or(0);
                    if top > re.captures_len() - 1 {
                        tri!(Err(format!(
                            "invalid reference \\{top} on `s' command's RHS"
                        )));
                    }
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
                        emit(out, &mut missing, &pattern, chomped);
                    }
                }
            }
        }
        if !deleted && !quiet {
            emit(out, &mut missing, &pattern, chomped);
        }
        for t in &appended {
            put(out, &mut missing, t);
        }
    }
    SedEnd {
        quit: false,
        failed: None,
    }
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
                // A glob that matched nothing stays literal; on Windows the OS
                // rejects `*` in a name with its own text, where bash says the
                // file is not there.
                let reason =
                    if has_glob_meta(op) && e.kind() != std::io::ErrorKind::PermissionDenied {
                        "No such file or directory".to_string()
                    } else {
                        io_reason(&e)
                    };
                sink.eline(&format!("ls: cannot access '{op}': {reason}"));
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
        let now = crate::clock::LocalTime::now();
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
            let mtime = crate::clock::LocalTime::from_timestamp(m.mtime()).unwrap_or(now);
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
                let Some(v) = val else {
                    return Util::DeclineFor(a.to_string());
                };
                match a {
                    "-name" => tests.push((negate, Test::Name(v, false))),
                    "-iname" => tests.push((negate, Test::Name(v.to_lowercase(), true))),
                    "-path" | "-wholename" => tests.push((negate, Test::Path(v))),
                    "-type" => match v.as_str() {
                        "f" | "d" | "l" => {
                            tests.push((negate, Test::Type(v.chars().next().unwrap_or('f'))))
                        }
                        _ => return Util::DeclineFor(format!("-type {v}")),
                    },
                    "-maxdepth" => match v.parse() {
                        Ok(n) => maxdepth = n,
                        Err(_) => return Util::DeclineFor(format!("-maxdepth {v}")),
                    },
                    _ => match v.parse() {
                        Ok(n) => mindepth = n,
                        Err(_) => return Util::DeclineFor(format!("-mindepth {v}")),
                    },
                }
                i += 1;
            }
            // The operand the subset does not run (`-o`, `-exec`, `(`), not the
            // first option in the command, which it may well support.
            _ => return Util::DeclineFor(a.to_string()),
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
            let now = crate::clock::LocalTime::now();
            let fmt = &f[1..];
            // A layout the formatter cannot read or write is declined, not a panic.
            let Ok(text) = now.format(fmt).write() else {
                return Util::Decline;
            };
            sink.out.extend_from_slice(format!("{text}\n").as_bytes());
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

/// The exit status of a refusal that ended the run, set when `--timeout`
/// killed a program (0 until then); read by the pipeline that ran it.
static RUN_TIMEOUT_STATUS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

fn run_named(src: &str, script_name: &str, args: &[String]) -> Option<i32> {
    let list = parse(src).ok()?;
    SHELL_START.get_or_init(std::time::Instant::now);
    RUN_TIMEOUT_STATUS.store(0, std::sync::atomic::Ordering::SeqCst);
    let mut sh = Shell {
        vars: HashMap::new(),
        exported: Vec::new(),
        unset_env: Vec::new(),
        last: 0,
        glob_refused: None,
        positional: args.to_vec(),
        script_name: script_name.to_string(),
        errexit: false,
        nounset: false,
        pipefail: false,
        errexit_off: 0,
        unbound: None,
        arith_failed: false,
        discarded: false,
        line: 1,
        subst_depth: 0,
    };
    let mut stdin = Input::real();
    let mut sink = Sink::new(ErrMode::Real);
    let mut status = 0;
    // The line whose rest bash discarded after an arithmetic error.
    let mut discarded_line: Option<usize> = None;
    // Output reaches the terminal as each top-level command finishes, not at
    // the end -- but only here: a loop body inside a pipeline must feed the
    // next stage, not the terminal.
    for ao in &list {
        if discarded_line == Some(ao.first.line) {
            continue;
        }
        let flow = sh.run_and_or(ao, &mut stdin, &mut sink);
        let flow = if std::mem::take(&mut sh.discarded) {
            discarded_line = Some(ao.first.line);
            match flow {
                Flow::Exit(_) => Flow::Next(1),
                other => other,
            }
        } else {
            flow
        };
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
        "echo" | "printf" | "date" | "basename" | "dirname" | "seq" | "tr" | "mktemp" | "yes"
        | "true" | ":" | "false" | "exit" | "break" | "continue" | "export" | "unset" | "shift"
        | "cd" | "read" | "pwd" => Some(false),
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
        "function definition" => "functions are outside the subset; repeat the body, or put it in a .sh script run with ish -b, or write the logic in ish".into(),
        "`time` builtin" => "time is a bash keyword that is outside the subset; drop it and run the command on its own".into(),
        "`trap` builtin" => "trap is outside the subset; put cleanup after the commands and use `|| cleanup` / set -e".into(),
        "`source` builtin" | "`.` builtin" => "source and . are outside the subset; run the script as a command (ish -b ./x.sh) or write its commands here".into(),
        c if c.starts_with("brace expansion") => "write the words out (d/a d/b) or use a for loop".into(),
        "case ... esac" => {
            "case is outside the subset: rewrite it as if/elif with [ \"$x\" = a ] tests, one elif per pattern".into()
        }
        "for without in" => "a loop over the arguments needs its list spelled out: write for x in \"$@\"; do ...; done".into(),
        "subshell" => "a subshell ( ... ) is outside the subset: write (cd d && cmd) as `cd d && cmd; cd -`, or as `{ cd d && cmd; }`, which keeps the directory change afterwards; drop the parentheses when the commands need not run isolated".into(),
        "select loop" => "select is an interactive menu and is outside the subset: loop over the choices with `for x in a b; do ...; done`, or read one with `read -r x`".into(),
        "`umask` builtin" => "umask is outside the subset: drop it, and set a mode on what a script creates with chmod afterwards".into(),
        "`read -t`" => "read -t (a time limit) is outside the subset: run the producer under `timeout N cmd | { read -r x; ...; }`, or read with `read -r x` and no limit".into(),
        c if c.starts_with("`read -n`") || c.starts_with("`read -d`") => "read -n N and read -d DELIM run in the subset only together with -r, which also stops a backslash from being special: write `read -r -n 1 x`".into(),
        "`export -n`" => "export -n NAME (stop exporting) is outside the subset: use `unset NAME` to drop it, or pass a value to one command with `NAME=value cmd`".into(),
        c if c.starts_with("`export -") => "export takes only NAME and NAME=value here: `export NAME=value`".into(),
        ">&word" => "a redirection to a file name is `> FILE 2>&1` (both outputs) or `2> FILE`; `>&N` with a descriptor number 1 or 2 is supported".into(),
        "`PIPESTATUS` array" => "PIPESTATUS is an array, which the subset lacks: `set -o pipefail` makes a pipeline fail when any stage fails (its status is the last failing stage's); to learn which stage failed, run the stages one at a time, each into a file".into(),
        "arithmetic command ((...))" => "arithmetic commands are outside the subset: write a counter as i=$((i+1)) and a comparison as [ \"$a\" -lt \"$b\" ]".into(),
        "`command` builtin" => "only `command -v NAME` is in the subset (it prints what NAME runs and fails if there is none); to run a program just name it, `command` adds nothing here".into(),
        "`type` builtin" | "`hash` builtin" => "use `command -v NAME` to test whether a command exists and where it is".into(),
        "`local` builtin" | "`return` builtin" => "functions are outside the subset, and local and return only exist in them; use ordinary variables with distinct names, and exit".into(),
        "`declare` builtin" | "`typeset` builtin" | "`readonly` builtin" => "declare, typeset and readonly are outside the subset; assign with name=value and pass it on with `export name`".into(),
        "`let` builtin" => "let is outside the subset: write arithmetic as i=$((i+1))".into(),
        "`getopts` builtin" => "getopts is outside the subset: loop over the arguments with `while [ $# -gt 0 ]; do ...; shift; done`, testing [ \"$1\" = -v ] in if/elif".into(),
        "`eval` builtin" => "eval is outside the subset: write the command out, or write it to a script file and run it with ish -b".into(),
        "`exec` builtin" => "exec is outside the subset: `exec cmd` is `cmd` followed by exit; `exec >f` and `exec 2>&1` redirect the rest of the script, so put the redirection on each command or on a group: `{ cmd1; cmd2; } >f 2>&1`".into(),
        "`shopt` builtin" => "shopt is outside the subset, and so are nullglob and the other options it sets: test a glob with ls or [ -e ]".into(),
        "`alias` builtin" | "`unalias` builtin" => "aliases are outside the subset: write the command out".into(),
        "`wait` builtin" | "`jobs` builtin" | "`fg` builtin" | "`bg` builtin" | "`disown` builtin" => "background jobs are outside the subset, so there is nothing to wait for: run each command in the foreground".into(),
        "`pushd` builtin" | "`popd` builtin" | "`dirs` builtin" => "the directory stack is outside the subset: use cd, and cd - to go back".into(),
        "`mapfile` builtin" | "`readarray` builtin" => "read lines one at a time with `while read -r line; do ...; done < file`".into(),
        c if c.starts_with("`set -x") || c.starts_with("`set -o xtrace") => "set -x (tracing) is not supported; drop the x: `set -euo pipefail` is supported".into(),
        c if c.starts_with("`set ") => "set runs only -e, -u and -o pipefail (also as +e, +u, +o and combined, such as -euo pipefail); drop the other options".into(),
        "${...} form" => "`${@:2}`, `${@: -1}` and `${!name}` are outside the subset: take the arguments after the first with `shift` and then \"$@\"".into(),
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
            "[" => "arrays are not in the subset: keep the words in a space-separated string and loop over it; for ${BASH_SOURCE[0]}, the script's own path, use $0".into(),
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
        "sed" => {
            // `sed -i` writes each FILE, and a backup FILE+SUFFIX when it has one.
            let suffix = args.iter().find_map(|a| sed_inplace_suffix(a))?;
            let mut out = Vec::new();
            for f in sed_operands(args)?.into_iter().filter(|f| f != "-") {
                if !suffix.is_empty() {
                    out.push(format!("{f}{suffix}"));
                }
                out.push(f);
            }
            Some(out)
        }
        _ => None,
    }
}

/// The paths in-process `rm` and `rmdir` remove, for `--explain`: each operand is
/// a `delete`, judged by the jail as the run judges it. `None` for any other
/// command, or when the utility declines its flags (which the run refuses under
/// the jail).
pub fn remove_operands(head: &str, args: &[String]) -> Option<Vec<String>> {
    match head {
        "rm" => short_flags(args, "rRf").map(|(_, ops)| ops),
        "rmdir" => short_flags(args, "").map(|(_, ops)| ops),
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
        "grep" => grep_operands(args),
        "head" | "tail" => head_tail_operands(args),
        "sort" => SortSpec::parse(args).map(|spec| spec.files),
        "uniq" => short_flags(args, "cdui").and_then(|(_, ops)| (ops.len() <= 1).then_some(ops)),
        "cut" => cut_operands(args),
        "find" => Some(
            args.iter()
                .take_while(|a| !a.starts_with('-') && a.as_str() != "!")
                .cloned()
                .collect(),
        ),
        _ => None,
    }
}

/// The files `grep` reads: its operands after the pattern, as `grep()` splits
/// its arguments. `None` where `grep()` declines or refuses the options. With
/// `-r` and no file it walks `.`, which is the workspace, so it names none.
fn grep_operands(args: &[String]) -> Option<Vec<String>> {
    if invalid_option("grep", args, GREP_SHORT, GREP_LONG).is_some() {
        return None;
    }
    let mut have_pattern = false;
    let mut ops = Vec::new();
    let mut i = 0;
    let mut done = false;
    while i < args.len() {
        let a = &args[i];
        if !done && a == "--" {
            done = true;
        } else if !done && a == "-e" {
            args.get(i + 1)?;
            have_pattern = true;
            i += 1;
        } else if let Some((_, inline)) = grep_glob_opt(a).filter(|_| !done) {
            if inline.is_none() {
                i += 1;
                args.get(i)?;
            }
        } else if !done && a == "--no-group-separator" {
        } else if let Some((_, inline)) = grep_long_context(a).filter(|_| !done) {
            if inline.is_none() {
                i += 1;
                args.get(i)?;
            }
        } else if !done && a.starts_with("--") {
            return None;
        } else if !done && a.len() > 1 && a.starts_with('-') {
            let (_, with_value) = grep_cluster(a).ok()?;
            for (_, inline) in with_value {
                if inline.is_none() {
                    i += 1;
                    args.get(i)?;
                }
            }
        } else {
            ops.push(a.clone());
        }
        i += 1;
    }
    if !have_pattern {
        if ops.is_empty() {
            return None;
        }
        ops.remove(0);
    }
    Some(ops)
}

/// The files `head` and `tail` read, as `head_tail()` splits its arguments.
/// `None` where it declines.
fn head_tail_operands(args: &[String]) -> Option<Vec<String>> {
    let count_ok = |s: &str| s.strip_prefix('+').unwrap_or(s).parse::<i64>().is_ok();
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "-n" || a == "-c" {
            if !args.get(i + 1).is_some_and(|v| count_ok(v)) {
                return None;
            }
            i += 2;
            continue;
        }
        if let Some(v) = a.strip_prefix("-n").or_else(|| a.strip_prefix("-c")) {
            if !v.is_empty() {
                if !count_ok(v) {
                    return None;
                }
                i += 1;
                continue;
            }
        }
        if a == "-q"
            || (a.len() > 1 && a.starts_with('-') && a[1..].chars().all(|c| c.is_ascii_digit()))
        {
        } else if a.starts_with('-') && a != "-" {
            return None;
        } else {
            files.push(a.clone());
        }
        i += 1;
    }
    Some(files)
}

/// The files `cut` reads, as `cut()` splits its arguments. `None` where it
/// declines.
fn cut_operands(args: &[String]) -> Option<Vec<String>> {
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if ["-d", "-f", "-c", "-b"].iter().any(|f| a.starts_with(f)) {
            if a.len() <= 2 {
                i += 1;
                args.get(i)?;
            }
        } else if a == "-s" {
        } else if a.starts_with('-') && a != "-" {
            return None;
        } else {
            files.push(a.clone());
        }
        i += 1;
    }
    Some(files)
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
            "-n" | "-E" | "-r" | "-s" | "--separate" => {}
            s if sed_inplace_suffix(s).is_some() => {}
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

/// The command `timeout [OPTION]... DURATION COMMAND [ARG]...` runs (its words,
/// the command first), or `None` when there is none or an option is not one the
/// in-process `timeout` takes.
fn timeout_inner(args: &[String]) -> Option<&[String]> {
    let mut i = 0;
    while let Some(a) = args.get(i) {
        match a.as_str() {
            "--" => {
                i += 1;
                break;
            }
            "--foreground" | "-v" | "--verbose" | "--preserve-status" => i += 1,
            "-k" | "-s" => i += 2,
            o if o.starts_with("--kill-after=") || o.starts_with("--signal=") => i += 1,
            o if (o.starts_with("-k") || o.starts_with("-s")) && o.len() > 2 => i += 1,
            o if o.len() > 1 && o.starts_with('-') => return None,
            _ => break,
        }
    }
    args.get(i)?;
    let rest = &args[i + 1..];
    (!rest.is_empty()).then_some(rest)
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
        let mut args: Vec<String> = words.iter().skip(1).map(literal_text).collect();
        let mut head = head;
        // `timeout N cmd ...` runs `cmd`: it is that command that is judged.
        if head.as_deref() == Some("timeout") {
            if let Some(inner) = timeout_inner(&args) {
                head = Some(inner[0].clone());
                args = inner[1..].to_vec();
            }
        }
        if head.is_some() || !writes.is_empty() || !reads.is_empty() {
            out.push(ExplainCommand {
                head,
                args,
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
        Redir::In(_) | Redir::Str(_) | Redir::Dup { .. } => return None,
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

// ── arithmetic expansion ─────────────────────────────────────────────────

/// `text` evaluated as `$((...))` does in bash: 64-bit signed integers that
/// wrap on overflow; decimal, `0x` hex and leading-0 octal literals; names (an
/// unset or empty one is 0, a value is itself evaluated as an expression);
/// `+ - * / % **`, unary `+ - ! ~`, `<< >> < <= > >= == != & ^ | && ||`, `?:`,
/// `,`, the assignments `= += -= *= /= %= <<= >>= &= ^= |=` and `++`/`--`.
/// `Err` holds bash's message, without the place.
fn arith_eval(sh: &mut Shell, text: &str) -> Result<i64, String> {
    ArithEval {
        s: text.chars().collect(),
        i: 0,
        dead: 0,
        depth: 0,
        nest: 0,
        sh,
    }
    .run()
}

/// How deep an arithmetic expression may nest (parentheses, prefix operators,
/// right-hand sides, and the variables whose values are expressions) before
/// bash's `expression recursion level exceeded`.
const ARITH_MAX_NEST: u32 = 1024;

struct ArithEval<'a> {
    s: Vec<char>,
    i: usize,
    /// Above 0 while an operand bash would not evaluate is read (`0 && x=1`,
    /// the unchosen side of `?:`): nothing is assigned and `1/0` is no error.
    dead: u32,
    /// How deep a variable's value is being evaluated as an expression.
    depth: u32,
    /// How many recursive constructs are open now (see [`ARITH_MAX_NEST`]).
    nest: u32,
    sh: &'a mut Shell,
}

type ArithResult = Result<i64, String>;

/// A C integer constant as the arithmetic reads it, wrapping into 64 bits.
fn arith_number(tok: &str) -> Option<i64> {
    let u = if let Some(h) = tok.strip_prefix("0x").or_else(|| tok.strip_prefix("0X")) {
        // Bash reads a bare `0x` as 0.
        if h.is_empty() {
            0
        } else {
            u64::from_str_radix(h, 16).ok()?
        }
    } else if tok.len() > 1 && tok.starts_with('0') {
        u64::from_str_radix(&tok[1..], 8).ok()?
    } else {
        tok.parse::<u64>().ok()?
    };
    Some(u as i64)
}

impl ArithEval<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_whitespace()) {
            self.i += 1;
        }
    }

    fn rest_from(&self, at: usize) -> String {
        self.s[at.min(self.s.len())..].iter().collect()
    }

    /// Run `f` one level deeper: past [`ARITH_MAX_NEST`] open levels it is
    /// bash's `expression recursion level exceeded`, not a native stack
    /// overflow.
    fn nested(&mut self, f: impl FnOnce(&mut Self) -> ArithResult) -> ArithResult {
        if self.nest >= ARITH_MAX_NEST {
            // The text after the cursor can be megabytes of parentheses.
            let token: String = self.rest_from(self.i).chars().take(40).collect();
            return Err(format!(
                "expression recursion level exceeded (error token is \"{token}\")"
            ));
        }
        self.nest += 1;
        let r = f(self);
        self.nest -= 1;
        r
    }

    /// Whether the text at the cursor (no blanks skipped) starts with `op`.
    fn starts(&self, op: &str) -> bool {
        let o: Vec<char> = op.chars().collect();
        self.s[self.i..].starts_with(&o)
    }

    /// Skips blanks, then whether the text starts with `op`.
    fn at(&mut self, op: &str) -> bool {
        self.ws();
        self.starts(op)
    }

    fn eat(&mut self, op: &str) -> bool {
        if self.at(op) {
            self.i += op.chars().count();
            true
        } else {
            false
        }
    }

    fn syntax(&self) -> String {
        format!(
            "syntax error in expression (error token is \"{}\")",
            self.rest_from(self.i)
        )
    }

    fn run(&mut self) -> ArithResult {
        self.ws();
        if self.i >= self.s.len() {
            return Ok(0);
        }
        let v = self.comma()?;
        self.ws();
        if self.i < self.s.len() {
            return Err(self.syntax());
        }
        Ok(v)
    }

    fn comma(&mut self) -> ArithResult {
        let mut v = self.assign()?;
        while self.eat(",") {
            v = self.assign()?;
        }
        Ok(v)
    }

    fn ident(&mut self) -> Option<String> {
        self.ws();
        if !self.s.get(self.i).is_some_and(|c| is_name_start(*c)) {
            return None;
        }
        let start = self.i;
        while self.s.get(self.i).is_some_and(|c| is_name_char(*c)) {
            self.i += 1;
        }
        Some(self.s[start..self.i].iter().collect())
    }

    /// The value of a name: 0 when unset or empty, else its text read as a
    /// number or, failing that, evaluated as an expression.
    fn ident_value(&mut self, name: &str) -> ArithResult {
        let value = self.sh.var(name);
        if value.is_none() && self.dead == 0 {
            self.sh.note_unbound(name);
        }
        let text = value.unwrap_or_default();
        let t = text.trim();
        if t.is_empty() {
            return Ok(0);
        }
        if let Some(n) = arith_number(t) {
            return Ok(n);
        }
        if self.depth >= ARITH_MAX_NEST || self.nest >= ARITH_MAX_NEST {
            return Err(format!(
                "{t}: expression recursion level exceeded (error token is \"{name}\")"
            ));
        }
        let mut inner = ArithEval {
            s: t.chars().collect(),
            i: 0,
            dead: self.dead,
            depth: self.depth + 1,
            nest: self.nest + 1,
            sh: &mut *self.sh,
        };
        inner.run()
    }

    fn store(&mut self, name: &str, value: i64) {
        if self.dead == 0 {
            self.sh.set_var(name, value.to_string());
        }
    }

    /// `a op b` for the arithmetic operators; `tok` is where `b` starts, for the
    /// error token of a division by zero.
    fn binary(&self, op: &str, a: i64, b: i64, tok: usize) -> ArithResult {
        Ok(match op {
            "+" => a.wrapping_add(b),
            "-" => a.wrapping_sub(b),
            "*" => a.wrapping_mul(b),
            "/" | "%" => {
                if b == 0 {
                    if self.dead > 0 {
                        return Ok(0);
                    }
                    return Err(format!(
                        "division by 0 (error token is \"{}\")",
                        self.rest_from(tok)
                    ));
                }
                if op == "/" {
                    a.wrapping_div(b)
                } else {
                    a.wrapping_rem(b)
                }
            }
            "<<" => a.wrapping_shl(b as u32),
            ">>" => a.wrapping_shr(b as u32),
            "&" => a & b,
            "^" => a ^ b,
            "|" => a | b,
            _ => return Err(self.syntax()),
        })
    }

    fn assign(&mut self) -> ArithResult {
        const OPS: [&str; 11] = [
            "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=", "=",
        ];
        self.ws();
        let save = self.i;
        if let Some(name) = self.ident() {
            self.ws();
            for op in OPS {
                if self.starts(op) && !(op == "=" && self.starts("==")) {
                    self.i += op.chars().count();
                    self.ws();
                    let tok = self.i;
                    let rhs = self.nested(|s| s.assign())?;
                    let new = if op == "=" {
                        rhs
                    } else {
                        let cur = self.ident_value(&name)?;
                        self.binary(&op[..op.len() - 1], cur, rhs, tok)?
                    };
                    self.store(&name, new);
                    return Ok(new);
                }
            }
            self.i = save;
        }
        self.ternary()
    }

    fn ternary(&mut self) -> ArithResult {
        let c = self.lor()?;
        if !self.eat("?") {
            return Ok(c);
        }
        let first = c != 0;
        if !first {
            self.dead += 1;
        }
        let a = self.nested(|s| s.assign());
        if !first {
            self.dead -= 1;
        }
        let a = a?;
        if !self.eat(":") {
            return Err(format!(
                "`:' expected for conditional expression (error token is \"{}\")",
                self.rest_from(self.i)
            ));
        }
        if first {
            self.dead += 1;
        }
        let b = self.nested(|s| s.ternary());
        if first {
            self.dead -= 1;
        }
        let b = b?;
        Ok(if first { a } else { b })
    }

    fn lor(&mut self) -> ArithResult {
        let mut v = self.land()?;
        while self.eat("||") {
            let skip = v != 0;
            if skip {
                self.dead += 1;
            }
            let r = self.land();
            if skip {
                self.dead -= 1;
            }
            let r = r?;
            v = i64::from(v != 0 || r != 0);
        }
        Ok(v)
    }

    fn land(&mut self) -> ArithResult {
        let mut v = self.bor()?;
        while self.at("&&") {
            self.i += 2;
            let skip = v == 0;
            if skip {
                self.dead += 1;
            }
            let r = self.bor();
            if skip {
                self.dead -= 1;
            }
            let r = r?;
            v = i64::from(v != 0 && r != 0);
        }
        Ok(v)
    }

    fn bor(&mut self) -> ArithResult {
        let mut v = self.bxor()?;
        while self.at("|") && !self.starts("||") && !self.starts("|=") {
            self.i += 1;
            v |= self.bxor()?;
        }
        Ok(v)
    }

    fn bxor(&mut self) -> ArithResult {
        let mut v = self.band()?;
        while self.at("^") && !self.starts("^=") {
            self.i += 1;
            v ^= self.band()?;
        }
        Ok(v)
    }

    fn band(&mut self) -> ArithResult {
        let mut v = self.equality()?;
        while self.at("&") && !self.starts("&&") && !self.starts("&=") {
            self.i += 1;
            v &= self.equality()?;
        }
        Ok(v)
    }

    fn equality(&mut self) -> ArithResult {
        let mut v = self.relational()?;
        loop {
            if self.at("==") {
                self.i += 2;
                v = i64::from(v == self.relational()?);
            } else if self.at("!=") {
                self.i += 2;
                v = i64::from(v != self.relational()?);
            } else {
                return Ok(v);
            }
        }
    }

    fn relational(&mut self) -> ArithResult {
        let mut v = self.shift()?;
        loop {
            if self.at("<=") {
                self.i += 2;
                v = i64::from(v <= self.shift()?);
            } else if self.at(">=") {
                self.i += 2;
                v = i64::from(v >= self.shift()?);
            } else if self.at("<") && !self.starts("<<") {
                self.i += 1;
                v = i64::from(v < self.shift()?);
            } else if self.at(">") && !self.starts(">>") {
                self.i += 1;
                v = i64::from(v > self.shift()?);
            } else {
                return Ok(v);
            }
        }
    }

    fn shift(&mut self) -> ArithResult {
        let mut v = self.additive()?;
        loop {
            let op = if self.at("<<") && !self.starts("<<=") {
                "<<"
            } else if self.at(">>") && !self.starts(">>=") {
                ">>"
            } else {
                return Ok(v);
            };
            self.i += 2;
            self.ws();
            let tok = self.i;
            let r = self.additive()?;
            v = self.binary(op, v, r, tok)?;
        }
    }

    fn additive(&mut self) -> ArithResult {
        let mut v = self.multiplicative()?;
        loop {
            let op = if self.at("+") && !self.starts("+=") {
                "+"
            } else if self.at("-") && !self.starts("-=") {
                "-"
            } else {
                return Ok(v);
            };
            self.i += 1;
            self.ws();
            let tok = self.i;
            let r = self.multiplicative()?;
            v = self.binary(op, v, r, tok)?;
        }
    }

    fn multiplicative(&mut self) -> ArithResult {
        let mut v = self.power()?;
        loop {
            let op = if self.at("*") && !self.starts("**") && !self.starts("*=") {
                "*"
            } else if self.at("/") && !self.starts("/=") {
                "/"
            } else if self.at("%") && !self.starts("%=") {
                "%"
            } else {
                return Ok(v);
            };
            self.i += 1;
            self.ws();
            let tok = self.i;
            let r = self.power()?;
            v = self.binary(op, v, r, tok)?;
        }
    }

    /// `**` binds tighter than `*` and looser than the unary operators, and
    /// groups to the right.
    fn power(&mut self) -> ArithResult {
        let base = self.unary()?;
        if !self.at("**") {
            return Ok(base);
        }
        self.i += 2;
        self.ws();
        let tok = self.i;
        let exp = self.nested(|s| s.power())?;
        if exp < 0 {
            if self.dead > 0 {
                return Ok(0);
            }
            return Err(format!(
                "exponent less than 0 (error token is \"{}\")",
                self.rest_from(tok)
            ));
        }
        let (mut result, mut b, mut n) = (1i64, base, exp as u64);
        while n > 0 {
            if n & 1 == 1 {
                result = result.wrapping_mul(b);
            }
            b = b.wrapping_mul(b);
            n >>= 1;
        }
        Ok(result)
    }

    fn unary(&mut self) -> ArithResult {
        self.ws();
        // `++` and `--` before a name step it; before anything else (`--3`)
        // they are two signs.
        let names_next = self
            .s
            .iter()
            .skip(self.i + 2)
            .find(|c| !c.is_whitespace())
            .is_some_and(|c| c.is_alphabetic() || *c == '_');
        if (self.starts("++") || self.starts("--")) && names_next {
            let delta = if self.starts("++") { 1 } else { -1 };
            self.i += 2;
            let Some(name) = self.ident() else {
                return Err(format!(
                    "syntax error: operand expected (error token is \"{}\")",
                    self.rest_from(self.i)
                ));
            };
            let v = self.ident_value(&name)?.wrapping_add(delta);
            self.store(&name, v);
            return Ok(v);
        }
        match self.s.get(self.i).copied() {
            Some('+') => {
                self.i += 1;
                self.nested(|s| s.unary())
            }
            Some('-') => {
                self.i += 1;
                Ok(self.nested(|s| s.unary())?.wrapping_neg())
            }
            Some('!') => {
                self.i += 1;
                Ok(i64::from(self.nested(|s| s.unary())? == 0))
            }
            Some('~') => {
                self.i += 1;
                Ok(!self.nested(|s| s.unary())?)
            }
            _ => self.primary(),
        }
    }

    /// `base#digits`: the cursor is at the `#`, the base starts at `start`.
    /// The base is 2..=64. The digits are `0-9 a-z A-Z @ _`: up to base 36 a
    /// letter's case does not matter, and past it `a-z` are 10..=35, `A-Z`
    /// 36..=61, `@` 62 and `_` 63. A digit at or past the base is bash's
    /// `value too great for base`.
    fn based(&mut self, start: usize) -> ArithResult {
        let base_text: String = self.s[start..self.i].iter().collect();
        self.i += 1;
        let digits_at = self.i;
        while self
            .s
            .get(self.i)
            .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(*c, '_' | '@'))
        {
            self.i += 1;
        }
        let digits: Vec<char> = self.s[digits_at..self.i].to_vec();
        let token = format!("{base_text}#{}", digits.iter().collect::<String>());
        let base = match base_text.parse::<u32>() {
            Ok(b) if (2..=64).contains(&b) => b,
            _ => {
                return Err(format!(
                    "invalid arithmetic base (error token is \"{token}\")"
                ))
            }
        };
        if digits.is_empty() {
            return Err(format!(
                "invalid integer constant (error token is \"{base_text}#\")"
            ));
        }
        let mut value: i64 = 0;
        for c in digits {
            let d = match c {
                '0'..='9' => c as u32 - '0' as u32,
                'a'..='z' => c as u32 - 'a' as u32 + 10,
                'A'..='Z' if base <= 36 => c as u32 - 'A' as u32 + 10,
                'A'..='Z' => c as u32 - 'A' as u32 + 36,
                '@' => 62,
                _ => 63,
            };
            if d >= base {
                return Err(format!(
                    "value too great for base (error token is \"{token}\")"
                ));
            }
            value = value
                .wrapping_mul(i64::from(base))
                .wrapping_add(i64::from(d));
        }
        Ok(value)
    }

    fn primary(&mut self) -> ArithResult {
        self.ws();
        match self.s.get(self.i).copied() {
            Some('(') => {
                self.i += 1;
                let v = self.nested(|s| s.comma())?;
                if !self.eat(")") {
                    return Err(format!(
                        "missing `)' (error token is \"{}\")",
                        self.rest_from(self.i)
                    ));
                }
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() => {
                let start = self.i;
                while self
                    .s
                    .get(self.i)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
                {
                    self.i += 1;
                }
                if self.s.get(self.i) == Some(&'#') {
                    return self.based(start);
                }
                let tok: String = self.s[start..self.i].iter().collect();
                arith_number(&tok)
                    .ok_or_else(|| format!("value too great for base (error token is \"{tok}\")"))
            }
            Some(c) if is_name_start(c) => {
                let name = self.ident().unwrap_or_default();
                let v = self.ident_value(&name)?;
                if self.starts("++") || self.starts("--") {
                    let delta = if self.starts("++") { 1 } else { -1 };
                    self.i += 2;
                    self.store(&name, v.wrapping_add(delta));
                }
                Ok(v)
            }
            _ => Err(format!(
                "syntax error: operand expected (error token is \"{}\")",
                self.rest_from(self.i)
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run in-process and capture stdout and the status.
    fn sh(src: &str) -> (String, i32) {
        let list = parse(src).unwrap_or_else(|u| panic!("{src}: {}", u.what));
        let mut shell = Shell {
            vars: HashMap::new(),
            exported: Vec::new(),
            unset_env: Vec::new(),
            last: 0,
            glob_refused: None,
            positional: Vec::new(),
            script_name: SHELL_NAME.to_string(),
            errexit: false,
            nounset: false,
            pipefail: false,
            errexit_off: 0,
            unbound: None,
            arith_failed: false,
            discarded: false,
            line: 1,
            subst_depth: 0,
        };
        let mut stdin = Input::buf(Vec::new());
        let mut sink = Sink::new(ErrMode::Null);
        // As the entry point runs a script: an arithmetic error discards the
        // rest of its line and the next line goes on.
        let mut c = 0;
        let mut discarded_line = None;
        for ao in &list {
            if discarded_line == Some(ao.first.line) {
                continue;
            }
            let flow = shell.run_and_or(ao, &mut stdin, &mut sink);
            let discarded = std::mem::take(&mut shell.discarded);
            if discarded {
                discarded_line = Some(ao.first.line);
            }
            match flow {
                Flow::Next(n) => c = n,
                Flow::Exit(n) => {
                    c = n;
                    if !discarded {
                        break;
                    }
                }
                _ => {}
            }
        }
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

    /// `$((...))` as bash computes it (the expected values are bash's).
    #[test]
    fn arithmetic_expansion_matches_bash() {
        let cases: &[(&str, &str)] = &[
            ("echo $((1+2))", "3"),
            ("echo $((7/2))", "3"),
            ("echo $((-7/2))", "-3"),
            ("echo $((-7%2))", "-1"),
            ("echo $((7%-2))", "1"),
            ("echo $((2**3))", "8"),
            ("echo $((2**3**2))", "512"),
            ("echo $((-2**2))", "4"),
            ("echo $((2*3+4))", "10"),
            ("echo $((2+3*4))", "14"),
            ("echo $(( (2+3)*4 ))", "20"),
            ("echo $((10-3-2))", "5"),
            ("echo $((0x1F))", "31"),
            ("echo $((010))", "8"),
            ("echo $((1<<4))", "16"),
            ("echo $((256>>2))", "64"),
            ("echo $((~5))", "-6"),
            ("echo $((!0)) $((!5))", "1 0"),
            (
                "echo $((3<4)) $((4<3)) $((3<=3)) $((3>=4)) $((3==3)) $((3!=3))",
                "1 0 1 0 1 0",
            ),
            ("echo $((6&3)) $((6|3)) $((6^3))", "2 7 5"),
            ("echo $((1&&0)) $((1||0)) $((0||0))", "0 1 0"),
            ("echo $((5>3?10:20)) $((5<3?10:20))", "10 20"),
            ("echo $((9223372036854775807+1))", "-9223372036854775808"),
            ("echo $((9223372036854775807*2))", "-2"),
            ("echo $(( ))", "0"),
            ("n=4; echo $((n+1)) $(($n+1))", "5 5"),
            ("echo $((unset_ae_x+1))", "1"),
            ("n=0; n=$((n+1)); n=$((n+1)); echo $n", "2"),
            ("a=6; b=7; echo $((a*b))", "42"),
            ("i=2; echo $(( i < 3 ))", "1"),
            ("x=5; echo $((x+=3)) $x", "8 8"),
            ("x=5; echo $((y=x*2)) $y", "10 10"),
            (
                "x=7; echo $((x%=4)) $((x*=5)) $((x-=1)) $((x/=2))",
                "3 15 14 7",
            ),
            (
                "x=1; echo $((x<<=3)) $((x>>=1)) $((x|=1)) $((x&=6)) $((x^=2))",
                "8 4 5 4 6",
            ),
            ("echo $((0&&(1/0)))", "0"),
            ("echo $((1||(1/0)))", "1"),
            ("echo $((1?2:(1/0)))", "2"),
            (
                "i=1; echo $((i++)) $((++i)) $((i--)) $((--i)) $i",
                "1 3 3 1 1",
            ),
            ("echo $((1,2))", "2"),
            ("x=1+2; echo $((x*2))", "6"),
            ("echo \"$((1+1))\"", "2"),
            (
                "for i in $(seq 1 $((2+1))); do echo -n $i; done; echo",
                "123",
            ),
        ];
        for (src, want) in cases {
            assert_eq!(sh(src), (format!("{want}\n"), 0), "{src}");
        }
    }

    #[test]
    fn arithmetic_errors_end_the_command_with_status_1() {
        for src in [
            "echo $((1/0))",
            "echo $((1%0))",
            "echo $((08))",
            "echo $((1+))",
            "echo $((2**-1))",
            "echo $((1 2))",
            "x='1 2'; echo $((x+1))",
            "echo $((1/0)); echo after",
        ] {
            assert_eq!(sh(src), (String::new(), 1), "{src}");
        }
    }

    /// Bash (5.2.37) discards the line an arithmetic error is on and goes on
    /// with the next one; the failing command's status is 1 and `set -e` does
    /// not end the script.
    #[test]
    fn an_arithmetic_error_discards_its_line_and_the_script_goes_on() {
        let cases: &[(&str, &str, i32)] = &[
            ("echo $((1/0))\necho after", "after\n", 0),
            ("echo $((1/0)); echo after", "", 1),
            ("echo ok\necho $((1/0))", "ok\n", 1),
            ("x=$((1/0))\necho st=$?\necho after", "st=1\nafter\n", 0),
            ("x=5\nx=$((1/0))\necho $x", "5\n", 0),
            (
                "if echo $((1/0)); then echo T; else echo F; fi\necho after",
                "after\n",
                0,
            ),
            (
                "while echo $((1/0)); do echo T; break; done\necho after:$?",
                "after:1\n",
                0,
            ),
            ("set -e\necho $((1/0))\necho after", "after\n", 0),
            ("echo $((1/0)) && echo yes\necho after", "after\n", 0),
            ("for i in 1 2; do echo $((1/0)); done\necho end", "end\n", 0),
            ("echo a $((1/0)) b\necho st=$?", "st=1\n", 0),
            ("echo $(echo $((1/0)))\necho after$?", "\nafter0\n", 0),
            ("echo $((1/0))\nexit 3\necho no", "", 3),
        ];
        for (src, out, status) in cases {
            assert_eq!(sh(src), ((*out).to_string(), *status), "{src}");
        }
    }

    /// `base#digits` literals (the expected values are bash 5.2's).
    #[test]
    fn base_hash_literals_match_bash() {
        assert_eq!(
            sh("echo $((16#ff)) $((8#17)) $((2#101)) $((36#zz)) $((64#@)) $((64#_)) $((37#1)) $((64#z)) $((64#Z)) $((36#Z)) $((0x))"),
            ("255 15 5 1295 62 63 1 35 61 35 0\n".into(), 0)
        );
        assert_eq!(sh("echo $((16#FF+1)) $((10#09))"), ("256 9\n".into(), 0));
        assert_eq!(sh("x=16#ff; echo $((x+1))"), ("256\n".into(), 0));
        for src in [
            "echo $((1#0))",
            "echo $((65#1))",
            "echo $((2#2))",
            "echo $((16#))",
            "echo $((16#g))",
            "echo $((37#Z))",
            "echo $((0xg))",
        ] {
            assert_eq!(sh(src), (String::new(), 1), "{src}");
        }
    }

    /// Run `f` on a thread with the stack the shell's own thread has, since a
    /// test thread's default is too small for 1024 nested levels in a debug
    /// build.
    fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(128 * 1024 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn deep_arithmetic_is_an_error_not_a_stack_overflow() {
        let parens = format!("echo $(({}1{}))", "(".repeat(200_000), ")".repeat(200_000));
        let plus = format!(
            "echo $(({}1{}))",
            "(1+".repeat(200_000),
            ")".repeat(200_000)
        );
        let signs = format!("echo $(({}1))", "- ".repeat(200_000));
        let ok = format!("echo $(({}1{}))", "(".repeat(500), ")".repeat(500));
        let (a, b, c, d) = on_big_stack(move || (sh(&parens), sh(&plus), sh(&signs), sh(&ok)));
        assert_eq!(a, (String::new(), 1), "nested parentheses");
        assert_eq!(b, (String::new(), 1), "nested sums");
        assert_eq!(c, (String::new(), 1), "prefix signs");
        assert_eq!(d, ("1\n".into(), 0), "500 levels are fine");
    }

    #[test]
    fn a_variable_chain_runs_to_1000_levels_and_a_cycle_errors() {
        let mut chain = String::from("a0=1\n");
        for i in 1..=1000 {
            chain.push_str(&format!("a{i}=a{}\n", i - 1));
        }
        chain.push_str("echo $((a1000))");
        let (got, cycle) = on_big_stack(move || (sh(&chain), sh("a=b; b=a; echo $((a))")));
        assert_eq!(got, ("1\n".into(), 0));
        assert_eq!(cycle, (String::new(), 1));
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
    fn find_hints_name_what_does_the_job() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let newer = ish_alternative("find", &args(&[".", "-newer", "ref"])).unwrap();
        assert!(
            newer.contains("f.modified > fs_stat(\"ref\").modified"),
            "{newer}"
        );
        assert!(!newer.contains("glob"), "{newer}");
        let either = ish_alternative("find", &args(&[".", "-name", "a", "-o", "-name", "b"]));
        let either = either.unwrap();
        assert!(either.contains("one find per alternative"), "{either}");
        let plain = ish_alternative("find", &args(&[".", "-type", "f"])).unwrap();
        assert!(plain.contains("glob("), "{plain}");
        assert_eq!(ish_alternative("ls", &args(&[])), None);
    }

    #[test]
    fn declines_what_it_does_not_run() {
        for src in [
            "f() { echo; }",
            "case x in x) echo;; esac",
            "echo $!",
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

    #[test]
    fn msys_arguments_are_single_quoted_pieces() {
        // The reader of a program on the MSYS runtime (run with MSYS=noglob)
        // splits at `'` and `"` and has no escape: these are the encodings that
        // give each argument back byte for byte.
        let cases = [
            ("", "\"\""),
            ("plain", "'plain'"),
            ("a b", "'a b'"),
            ("a\"b", "'a\"b'"),
            ("it's", "'it'\"'\"'s'"),
            ("'", "\"'\""),
            ("''", "\"'\"\"'\""),
            ("x y\\", "'x y\\'"),
            ("\\\"", "'\\\"'"),
        ];
        for (arg, want) in cases {
            assert_eq!(msys_noglob_quote(arg), want, "{arg:?}");
        }
    }

    #[test]
    fn pe_imports_reads_the_import_table_not_the_text() {
        // Not a PE image, and a truncated one: nothing, and no panic.
        assert!(pe_imports(b"").is_empty());
        assert!(pe_imports(b"msys-2.0.dll cygwin1.dll").is_empty());
        assert!(pe_imports(b"MZ\x90\x00").is_empty());
        // This test program is a PE image on Windows: it imports kernel32 and
        // not the MSYS runtime, though this very file names that runtime.
        #[cfg(windows)]
        {
            let me = std::fs::read(std::env::current_exe().unwrap()).unwrap();
            let imports = pe_imports(&me);
            assert!(imports.iter().any(|d| d == "kernel32.dll"), "{imports:?}");
            assert!(!imports.iter().any(|d| d == "msys-2.0.dll"));
            assert!(!is_msys_program(
                &std::env::current_exe().unwrap().to_string_lossy()
            ));
        }
    }

    #[test]
    fn head_limit_reads_plain_head_only() {
        let limit = |src: &str| {
            let list = parse(src).unwrap();
            let p = &list[0].first;
            head_limit(&p.cmds[p.cmds.len() - 1])
        };
        assert_eq!(limit("yes | head -3"), Some((true, 3)));
        assert_eq!(limit("yes | head -n 5"), Some((true, 5)));
        assert_eq!(limit("yes | head -n5"), Some((true, 5)));
        assert_eq!(limit("yes | head -c 7"), Some((false, 7)));
        assert_eq!(limit("yes | head"), Some((true, 10)));
        assert_eq!(limit("yes | head f"), None);
        assert_eq!(limit("yes | head -n -2"), None);
        assert_eq!(limit("yes | head -q"), None);
        assert_eq!(limit("yes | head -n $n"), None);
        assert_eq!(limit("yes | tail -3"), None);
    }
}

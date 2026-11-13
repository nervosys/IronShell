use anyhow::{anyhow, Result};

use crate::ast::{BinOp, CfgCondition, ExportItem, Expr, ImportItem, Stmt, UnOp, Visibility};

/// A program that did not parse, carried as a structured failure.
///
/// Every other failure the shell produces reaches an agent as
/// `{"error":{"code":…,"message":…,"hint":…,"retryable":…}}` — that is the
/// contract `safety::ErrorCode` documents when it says "every uncoded failure
/// lands here rather than escaping as bare prose, so *every* failure is
/// branchable on `.code`." Parse errors were the exception, arriving as
/// `error: found 1 error(s): unexpected token Eof at line 1, column 27`, with
/// nothing to switch on. Measured against three other shells, it was the one
/// failure in ten where IronShell carried no machine-readable code.
///
/// This type lives in the parser rather than in `safety` on purpose:
/// `safety` is `#[cfg(feature = "native")]` and the parser is not, so reaching
/// for `SafetyError` here would make the shell's error contract differ between
/// the native and wasm builds. `Display` is the same JSON shape `SafetyError`
/// emits, so an agent cannot tell the two apart, and `repl::print_eval_error`
/// unpacks it into prose for a human exactly as it does for a `SafetyError`.
#[derive(Debug, Clone)]
pub struct ParseError {
    /// The diagnostics, already joined, with their line and column.
    pub message: String,
    /// What to do about it.
    pub hint: String,
    /// The line of the first `at line N, column M` in the message, when it has
    /// one. The JSON form carries it as `line`, so an agent can locate the fault.
    pub line: Option<usize>,
    /// The column that goes with `line`.
    pub column: Option<usize>,
}

impl ParseError {
    /// The code an agent branches on.
    pub const CODE: &'static str = "E_PARSE";

    pub fn new(message: impl Into<String>) -> Self {
        Self::with_hint(
            message,
            "fix the syntax at the reported line and column; the source \
             never reached the evaluator, so nothing ran",
        )
    }

    /// A parse error with its own hint. Its position is read from the message.
    pub fn with_hint(message: impl Into<String>, hint: impl Into<String>) -> Self {
        let message = message.into();
        let (line, column) = match position_in(&message) {
            Some((line, column)) => (Some(line), Some(column)),
            None => (None, None),
        };
        Self {
            message,
            hint: hint.into(),
            line,
            column,
        }
    }
}

/// The first `at line N, column M` in a diagnostic, as its line and column.
fn position_in(message: &str) -> Option<(usize, usize)> {
    const AT: &str = "at line ";
    let mut rest = message;
    while let Some(i) = rest.find(AT) {
        let after = &rest[i + AT.len()..];
        let line: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Some(tail) = after[line.len()..].strip_prefix(", column ") {
            let column: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let (Ok(line), Ok(column)) = (line.parse(), column.parse()) {
                return Some((line, column));
            }
        }
        rest = after;
    }
    None
}

/// A syntax error with a specific hint, carried so `parse_program` can give it
/// in place of the generic one. Its display is the ordinary message.
#[derive(Debug)]
struct HintedSyntax {
    msg: String,
    hint: String,
}

impl std::fmt::Display for HintedSyntax {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for HintedSyntax {}

/// The hint for `a[start:end]`, which IronShell does not spell as a slice.
const SLICE_HINT: &str = "IronShell has no slice syntax, so a[start:end] is not written. \
     Use the slice builtin on the subject: `a | slice(start, end)` (for example \
     `[1, 2, 3, 4] | slice(0, 2)`), or `slice(start)` to run to the end. For one \
     element, write a[i].";

/// The hint for a `{ ... }` that holds statements. Records are `{name: value}`,
/// and a braced block holds one expression: `name = value` and `let` are
/// statements, so they cannot sit between the braces.
const BLOCK_HINT: &str = "a braced `{ ... }` holds one expression, not statements: \
     `name = value` and `let` are statements, and a record field is written \
     `{name: value}`. Put one expression in the braces, or move the statements out.";

/// The hint for a `;` or a second statement inside an `if`, `else`, `try`,
/// `catch`, match-arm or lambda block. A block holds one expression, so the
/// values it needs are bound before it.
const ONE_EXPR_HINT: &str =
    "a block holds one expression; bind values before it (x = 1, then if c { x + 1 })";

/// The hint for `=` after an indexed element, `rows[0].n = 99`: elements are
/// values, so the array is rebuilt rather than assigned in place.
const INDEX_ASSIGN_HINT: &str =
    "an indexed element cannot be assigned; rebuild the array, e.g. rows = rows | map(fn(r) => ...)";

/// The hint for `c ? a : b`, which IronShell does not have.
const TERNARY_HINT: &str = "IronShell has no ?: operator; write if cond { a } else { b }";

/// The hint for `a?.b`, which IronShell does not have.
const OPTIONAL_CHAIN_HINT: &str =
    "IronShell has no ?. operator; test for null first: if a == null { null } else { a.b }";

/// The hint for `while`, `for` and `loop`, which IronShell does not have.
const LOOP_HINT: &str =
    "IronShell has no loop statements; iterate with map / where / reduce / each, \
     or arr.range(n) | each(...)";

/// The hint for a `|` inside a call's argument that the call cannot take.
const PIPE_ARG_HINT: &str =
    "`|` binds loosest: parenthesize the pipeline, e.g. (\"Hello\" | upper) == \"HELLO\"";

/// The hint for `def f(...)`, which IronShell does not have.
const FUNCTION_HINT: &str = "functions are values: f = fn(x) => x + 1";

/// A `[start:end]` index, carried as a typed error so `parse_program` can give
/// it the slice hint in place of the generic one.
#[derive(Debug)]
struct SliceSpelling(String);

impl std::fmt::Display for SliceSpelling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SliceSpelling {}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hand-rolled rather than via serde_json: the parser has no serde
        // dependency and should not acquire one for an error path.
        let esc = |s: &str| {
            s.chars()
                .fold(String::with_capacity(s.len()), |mut acc, c| {
                    match c {
                        '"' => acc.push_str("\\\""),
                        '\\' => acc.push_str("\\\\"),
                        '\n' => acc.push_str("\\n"),
                        '\r' => acc.push_str("\\r"),
                        '\t' => acc.push_str("\\t"),
                        c if (c as u32) < 0x20 => acc.push_str(&format!("\\u{:04x}", c as u32)),
                        c => acc.push(c),
                    }
                    acc
                })
        };
        let position = match (self.line, self.column) {
            (Some(line), Some(column)) => format!(r#","line":{line},"column":{column}"#),
            (Some(line), None) => format!(r#","line":{line}"#),
            _ => String::new(),
        };
        write!(
            f,
            r#"{{"error":{{"code":"{}","message":"{}","hint":"{}"{},"retryable":true}}}}"#,
            Self::CODE,
            esc(&self.message),
            esc(&self.hint),
            position
        )
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Colon,
    ColonEqual, // := walrus operator
    Dot,
    Pipe,
    Equal,
    FatArrow,
    Plus,
    Minus,
    Star,
    Caret,
    Slash,
    Percent,
    Bang,
    Tilde,    // ~ : terse lambda prefix (~x: body)
    Question, // ? : terse match prefix (?scrutinee { arms })
    Lt,
    Lte,
    Gt,
    Gte,
    EqEq,
    Ne,
    AndAnd,
    OrOr,
    Fn,
    Let,
    Mut,
    Pub,
    Export,
    True,
    False,
    Null,
    Match,
    If,
    Import,
    From,
    As,
    Async, // async keyword
    Await, // await keyword
    Try,   // try keyword
    Catch, // catch keyword
    Throw, // throw keyword
    Ident,
    String,
    Int,
    Float,
    Attribute, // #[...] attribute
    Semicolon, // ; statement separator
    Eof,
}

#[derive(Debug, Clone)]
struct Spanned {
    kind: Tok,
    text: String, // literal/ident text where relevant
    line: usize,  // 1-based line number
    col: usize,   // 1-based column number
}

/// A native source-analysis view of an identifier token. Columns count Unicode
/// scalar values, as the parser does; literals and comments are excluded.
#[cfg(feature = "native")]
pub(crate) struct SourceIdentifier {
    pub text: String,
    pub qualified: String,
    pub terminal: bool,
    pub line: usize,
    pub column: usize,
}

#[cfg(feature = "native")]
pub(crate) fn source_identifiers(builtin: &str, source: &str) -> Result<Vec<SourceIdentifier>> {
    let tokens = lex(source).map_err(|e| anyhow::Error::new(ParseError::new(e.to_string())))?;
    if tokens.len() > 8192 {
        return Err(crate::safety::budget_exceeded(
            builtin,
            "source exceeds 8192 tokens".into(),
            "analyze a smaller source unit",
        ));
    }
    let field_word = |token: &Spanned| {
        !matches!(token.kind, Tok::String | Tok::Eof)
            && token
                .text
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && token
                .text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    let identifier_at = |index: usize| {
        let token = &tokens[index];
        token.kind == Tok::Ident
            || (field_word(token)
                && ((index > 0 && tokens[index - 1].kind == Tok::Dot)
                    || tokens.get(index + 1).is_some_and(|t| t.kind == Tok::Colon)))
    };
    let mut identifiers = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if !identifier_at(index) {
            continue;
        }
        let mut start = index;
        let mut end = index;
        while start >= 2 && tokens[start - 1].kind == Tok::Dot && identifier_at(start - 2) {
            start -= 2;
        }
        while end + 2 < tokens.len() && tokens[end + 1].kind == Tok::Dot && identifier_at(end + 2) {
            end += 2;
        }
        let qualified = (start..=end)
            .step_by(2)
            .map(|i| tokens[i].text.as_str())
            .collect::<Vec<_>>()
            .join(".");
        identifiers.push(SourceIdentifier {
            text: token.text.clone(),
            qualified,
            terminal: index == end,
            line: token.line,
            column: token.col,
        });
    }
    Ok(identifiers)
}

pub struct Parser {
    toks: Vec<Spanned>,
    i: usize,
    // When true, allow the space-separated word-call sugar in parse_postfix.
    // This should only be enabled for top-level expressions (or explicitly
    // when parsing the RHS of a pipeline). It must be disabled when parsing
    // nested expression bodies like lambda bodies to avoid greedy consumption
    // of trailing atoms that belong to an outer call.
    allow_word_call: bool,
    // Stack of lambda parameter lists for implicit match scrutinee.
    // When `match { ... }` appears inside a lambda body, the first parameter
    // of the innermost enclosing lambda is used as the implicit scrutinee.
    lambda_param_stack: Vec<Vec<String>>,
    /// Whether a leading `.field` may stand for a field of the pipeline row:
    /// true while parsing an argument of a call on the right of `|`, so that
    /// `xs | where(.state == "open")` means `where(fn(__) => __.state == "open")`
    /// -- jq's spelling of "this row". Off inside an explicit `fn`, whose
    /// parameters name the row themselves.
    implicit_row: bool,
    /// Set when the argument being parsed used an implicit `.field`.
    implicit_used: bool,
    /// How deep the expression recursion currently is.
    ///
    /// The parser is recursive descent, so nesting in the *input* becomes
    /// nesting on the *native stack*. Without a limit, roughly 15,000 nested
    /// parentheses — or arrays, or records, or unary operators — overflowed the
    /// stack and aborted the process:
    ///
    /// ```text
    /// thread 'ironshell-eval' has overflowed its stack
    /// ```
    ///
    /// That is a crash rather than an error, reachable by anyone who can hand
    /// the shell a script, which for an agentic shell is the ordinary case.
    /// The evaluator has guarded call depth since long before this; the parser
    /// guarded nothing.
    depth: usize,
    /// Token indices of `[` written directly after a closing quote or brace.
    glued_brackets: std::collections::HashSet<usize>,
}

/// Parse a program, returning statements and any errors encountered.
/// Uses error recovery to report multiple errors when possible.
pub fn parse_program(src: &str) -> Result<Vec<Stmt>> {
    parse_program_with_lines(src).map(|(stmts, _)| stmts)
}

/// [`parse_program`], also returning the line each top-level statement starts
/// on, in the same order. A runtime error from a statement is reported at it.
pub fn parse_program_with_lines(src: &str) -> Result<(Vec<Stmt>, Vec<usize>)> {
    let (toks, glued_brackets) = lex_marked(src).map_err(lex_error)?;
    let mut p = Parser {
        toks,
        i: 0,
        allow_word_call: false,
        lambda_param_stack: Vec::new(),
        implicit_row: false,
        implicit_used: false,
        depth: 0,
        glued_brackets,
    };

    let mut stmts = Vec::new();
    let mut lines = Vec::new();
    let mut errors = Vec::new();

    while !p.check(Tok::Eof) {
        // Skip any semicolons between statements
        while p.match_tok(Tok::Semicolon) {}
        if p.check(Tok::Eof) {
            break;
        }
        let line = p.peek().line;
        match p.parse_stmt() {
            Ok(s) => {
                stmts.push(s);
                lines.push(line);
            }
            Err(e) => {
                errors.push(e);
                // Try to recover by synchronizing to a safe point
                p.synchronize();
            }
        }
    }

    // A second statement in a block that holds one expression is one fault. Its
    // knock-on errors (an unclosed '{', then each statement after it) only
    // bury the hint that names the fault, so that error is reported alone. The
    // hint is the first hinted error's, as it always was, so this only picks
    // which error is reported, never the hint.
    if let Some(first) = errors
        .iter()
        .find(|e| e.downcast_ref::<HintedSyntax>().is_some())
    {
        if first
            .downcast_ref::<HintedSyntax>()
            .is_some_and(|h| h.hint == ONE_EXPR_HINT)
        {
            return Err(anyhow::Error::new(ParseError::with_hint(
                first.to_string(),
                ONE_EXPR_HINT,
            )));
        }
    }

    // If we had errors, return them combined
    if !errors.is_empty() {
        let error_messages: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        let mut parse_error = ParseError::new(format!(
            "found {} error(s):\n  {}",
            errors.len(),
            error_messages.join("\n  ")
        ));
        if errors
            .iter()
            .any(|e| e.downcast_ref::<SliceSpelling>().is_some())
        {
            parse_error.hint = SLICE_HINT.to_string();
        }
        if let Some(hinted) = errors.iter().find_map(|e| e.downcast_ref::<HintedSyntax>()) {
            parse_error.hint = hinted.hint.to_string();
        }
        return Err(anyhow::Error::new(parse_error));
    }

    Ok((stmts, lines))
}

/// Parse a program without error recovery (stops at first error).
/// Useful for cases where partial parsing is not desired.
pub fn parse_program_strict(src: &str) -> Result<Vec<Stmt>> {
    let (toks, glued_brackets) = lex_marked(src).map_err(lex_error)?;
    let mut p = Parser {
        toks,
        i: 0,
        allow_word_call: false,
        lambda_param_stack: Vec::new(),
        implicit_row: false,
        implicit_used: false,
        depth: 0,
        glued_brackets,
    };
    let mut stmts = Vec::new();
    while !p.check(Tok::Eof) {
        let s = p.parse_stmt()?;
        stmts.push(s);
    }
    Ok(stmts)
}

// ============ Lexer ============

fn lex(src: &str) -> Result<Vec<Spanned>> {
    lex_marked(src).map(|(toks, _)| toks)
}

/// A string literal with no closing quote. Reported at the opening quote,
/// because that is where the literal starts and where the fix goes.
fn unterminated_string(line: usize, col: usize) -> anyhow::Error {
    anyhow::Error::new(ParseError::with_hint(
        format!("unterminated string at line {}, column {}", line, col),
        "close the string with the same quote character it opened with, \
         or escape the quote inside it",
    ))
}

/// A `${` interpolation hole with no `}` to close it. Reported at the `$`, where
/// the hole starts and where the fix goes.
fn unclosed_interpolation(line: usize, col: usize) -> anyhow::Error {
    anyhow::Error::new(ParseError::with_hint(
        format!(
            "unclosed interpolation `${{` at line {}, column {}",
            line, col
        ),
        "close it with }, or write \\${ for a literal ${",
    ))
}

/// A `\u` escape that is not `\uHHHH` or `\u{H..H}` naming a scalar value (or a
/// `\U` with four to seven hex digits, which is not the eight it needs). `at`
/// is the index just after the `u`; the message quotes the escape as written,
/// up to the closing quote, a newline, or its own end.
fn invalid_unicode_escape(
    line: usize,
    col: usize,
    chars: &[char],
    at: usize,
    kind: char,
) -> anyhow::Error {
    let mut escape = format!("\\{kind}");
    let braced = chars.get(at) == Some(&'{');
    let mut k = at;
    while k < chars.len() && escape.chars().count() < 24 {
        let c = chars[k];
        if c == '"' || c == '\n' {
            break;
        }
        escape.push(c);
        k += 1;
        if braced {
            if c == '}' {
                break;
            }
        } else if kind == 'U' && k - at == 8 {
            break;
        } else if kind == 'u' && k - at == 4 {
            break;
        }
    }
    let hint = if kind == 'U' {
        "write \\UHHHHHHHH with eight hex digits for a code point up to 10FFFF (not a surrogate)"
    } else {
        "write \\uHHHH with four hex digits or \\u{H..H} for a code point up to 10FFFF (not a surrogate)"
    };
    anyhow::Error::new(ParseError::with_hint(
        format!("invalid unicode escape `{escape}` at line {line}, column {col}"),
        hint,
    ))
}

/// Whether the text after a `//` that follows an operand is an integer-division
/// spelling rather than a comment: the rest of the line is one arithmetic
/// expression over numbers, names and parentheses (`2`, `b`, `(k+1)`,
/// `len(x)`). Prose (two words separated by a space), or any other
/// punctuation, is a comment, as SPEC section 2.0 documents `//`.
fn division_spelling(rest: &[char]) -> bool {
    let line: String = rest.iter().take_while(|c| **c != '\n').collect();
    let body = line.trim();
    let Some(first) = body.chars().next() else {
        return false;
    };
    if !(first.is_ascii_digit() || is_ident_start(first) || first == '(') {
        return false;
    }
    let allowed = |c: char| {
        c.is_ascii_alphanumeric()
            || matches!(c, '_' | '.' | '+' | '-' | '*' | '/' | '%' | '^' | '(' | ')')
            || c.is_whitespace()
    };
    if !body.chars().all(allowed) {
        return false;
    }
    // Two words with only spaces between them are prose ("note words").
    let mut prev_word = false;
    let mut gap = false;
    for c in body.chars() {
        if c.is_whitespace() {
            gap = true;
            continue;
        }
        let word = c.is_ascii_alphanumeric() || c == '_';
        if word && prev_word && gap {
            return false;
        }
        prev_word = word;
        gap = false;
    }
    true
}

/// A lexer failure as a `ParseError`. The lexer hands back a finished
/// `ParseError` when it has a specific hint; anything else is wrapped.
fn lex_error(e: anyhow::Error) -> ParseError {
    match e.downcast::<ParseError>() {
        Ok(parse_error) => parse_error,
        Err(other) => ParseError::new(other.to_string()),
    }
}

/// The tokens, and the indices of the `[` tokens written directly after a
/// closing quote or brace. A string token's text is its decoded value, so its
/// source length is not recoverable from the token; the lexer keeps this fact.
fn lex_marked(src: &str) -> Result<(Vec<Spanned>, std::collections::HashSet<usize>)> {
    let mut out = Vec::<Spanned>::new();
    let mut glued_brackets = std::collections::HashSet::<usize>::new();
    let chars: Vec<char> = src.chars().collect();
    let mut pos = 0;
    let mut line = 1usize;
    let mut col = 1usize;

    // Helper to create and push a token. Every caller is a one-character token
    // with no newline, advanced before this runs, so its start column is one
    // before the current one. Using the current column put `(`, `)`, `[` and
    // `]` one column to the right of where they are, which the index parser
    // relies on to tell `a[0]` from `a [0]`.
    macro_rules! tok {
        ($kind:expr, $text:expr) => {{
            out.push(Spanned {
                kind: $kind,
                text: $text.to_string(),
                line,
                col: col - 1,
            });
        }};
    }

    // Helper: advance position and update line/col
    fn advance(chars: &[char], pos: &mut usize, line: &mut usize, col: &mut usize) -> Option<char> {
        if *pos < chars.len() {
            let c = chars[*pos];
            *pos += 1;
            if c == '\n' {
                *line += 1;
                *col = 1;
            } else {
                *col += 1;
            }
            Some(c)
        } else {
            None
        }
    }

    // Helper: peek at current char
    fn peek(chars: &[char], pos: usize) -> Option<char> {
        if pos < chars.len() {
            Some(chars[pos])
        } else {
            None
        }
    }

    // Helper: peek at next char
    fn peek_next(chars: &[char], pos: usize) -> Option<char> {
        if pos + 1 < chars.len() {
            Some(chars[pos + 1])
        } else {
            None
        }
    }

    // Helper: read number (optionally with a leading '-' already consumed).
    //
    // Grammar: digits, optional `.digits` fraction, optional exponent
    // `[eE][+-]?digits` (an exponent makes the literal a Float). A number may
    // not run straight into an identifier character: `12abc`, `1e`, `1ex` and
    // `1key` are E_PARSE rather than a number followed by an identifier.
    fn read_number(
        chars: &[char],
        pos: &mut usize,
        line: &mut usize,
        col: &mut usize,
        mut acc: String,
    ) -> Result<(Tok, String)> {
        let mut kind = Tok::Int;
        // digits before decimal
        while let Some(ch) = peek(chars, *pos) {
            if ch.is_ascii_digit() {
                acc.push(ch);
                advance(chars, pos, line, col);
            } else {
                break;
            }
        }
        // optional fraction
        if peek(chars, *pos) == Some('.') {
            // Check it's not followed by another dot (like range operator ..)
            if peek_next(chars, *pos) != Some('.') {
                acc.push('.');
                advance(chars, pos, line, col);
                while let Some(ch) = peek(chars, *pos) {
                    if ch.is_ascii_digit() {
                        acc.push(ch);
                        advance(chars, pos, line, col);
                    } else {
                        break;
                    }
                }
                kind = Tok::Float;
            }
        }
        // optional exponent: only when digits follow `e`, optionally signed.
        // Otherwise the `e` is left in place and the identifier check below
        // rejects it (`1e`, `1ex`).
        if matches!(peek(chars, *pos), Some('e') | Some('E')) {
            let sign_len = match peek_next(chars, *pos) {
                Some('+') | Some('-') => 1,
                _ => 0,
            };
            let digit_at = *pos + 1 + sign_len;
            if peek(chars, digit_at).is_some_and(|c| c.is_ascii_digit()) {
                acc.push('e');
                advance(chars, pos, line, col);
                if sign_len == 1 {
                    if let Some(sign) = peek(chars, *pos) {
                        acc.push(sign);
                    }
                    advance(chars, pos, line, col);
                }
                while let Some(ch) = peek(chars, *pos) {
                    if ch.is_ascii_digit() {
                        acc.push(ch);
                        advance(chars, pos, line, col);
                    } else {
                        break;
                    }
                }
                kind = Tok::Float;
            }
        }
        // SI suffix on integer literals: 1k / 1M / 1G → scaled Int, in the lexer
        // (Phase 5: terse numeric form lives in the grammar, not a text pre-pass).
        // The suffix must end the literal; `1key` is rejected by the check below.
        if kind == Tok::Int {
            if let Some(suffix) = peek(chars, *pos) {
                let mult = match suffix {
                    'k' | 'K' => Some(1_000i64),
                    'M' => Some(1_000_000i64),
                    'G' => Some(1_000_000_000i64),
                    _ => None,
                };
                if let Some(m) = mult {
                    let after_ok = match peek_next(chars, *pos) {
                        Some(c) => !is_ident_part(c),
                        None => true,
                    };
                    if after_ok {
                        if let Ok(n) = acc.parse::<i64>() {
                            advance(chars, pos, line, col); // consume the suffix
                            return Ok((Tok::Int, n.saturating_mul(m).to_string()));
                        }
                    }
                }
            }
        }
        if let Some(next) = peek(chars, *pos) {
            if is_ident_part(next) {
                return Err(anyhow!(
                    "malformed number: '{}' is followed by '{}' at line {}, column {}; \
                     separate them with a space, or write a valid exponent such as 1e10",
                    acc,
                    next,
                    line,
                    col
                ));
            }
        }
        Ok((kind, acc))
    }

    while let Some(c) = peek(&chars, pos) {
        let start_line = line;
        let start_col = col;

        match c {
            ' ' | '\t' | '\r' | '\n' => {
                advance(&chars, &mut pos, &mut line, &mut col);
            }
            '(' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                tok!(Tok::LParen, "(");
            }
            ')' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                tok!(Tok::RParen, ")");
            }
            '[' => {
                // A `[` written directly after a closing quote or brace may index
                // a string or record literal. The parser decides by the atom it
                // just parsed (a block `}` never produces a record), so the lexer
                // only records the adjacency.
                let glued = pos > 0 && matches!(chars[pos - 1], '"' | '\'' | '}');
                advance(&chars, &mut pos, &mut line, &mut col);
                if glued {
                    glued_brackets.insert(out.len());
                }
                tok!(Tok::LBracket, "[");
            }
            ']' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                tok!(Tok::RBracket, "]");
            }
            '{' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                tok!(Tok::LBrace, "{");
            }
            '}' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                tok!(Tok::RBrace, "}");
            }
            ',' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                tok!(Tok::Comma, ",");
            }
            ':' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                if peek(&chars, pos) == Some('=') {
                    advance(&chars, &mut pos, &mut line, &mut col);
                    out.push(Spanned {
                        kind: Tok::ColonEqual,
                        text: ":=".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                } else {
                    out.push(Spanned {
                        kind: Tok::Colon,
                        text: ":".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                }
            }
            '.' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                out.push(Spanned {
                    kind: Tok::Dot,
                    text: ".".to_string(),
                    line: start_line,
                    col: start_col,
                });
            }
            '~' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                out.push(Spanned {
                    kind: Tok::Tilde,
                    text: "~".to_string(),
                    line: start_line,
                    col: start_col,
                });
            }
            '?' => {
                // `??` is the null-coalescing spelling of other languages. It is
                // not an operator here, and read as two terse-match prefixes it
                // failed far from its cause, so it is named where it is written.
                if peek(&chars, pos + 1) == Some('?') {
                    return Err(anyhow::Error::new(ParseError::with_hint(
                        format!(
                            "`??` at line {}, column {} is not an operator",
                            start_line, start_col
                        ),
                        "there is no ?? operator; write match x { null => 1, v => v } or if x == null { 1 } else { x }",
                    )));
                }
                advance(&chars, &mut pos, &mut line, &mut col);
                out.push(Spanned {
                    kind: Tok::Question,
                    text: "?".to_string(),
                    line: start_line,
                    col: start_col,
                });
            }
            '|' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                if peek(&chars, pos) == Some('|') {
                    advance(&chars, &mut pos, &mut line, &mut col);
                    out.push(Spanned {
                        kind: Tok::OrOr,
                        text: "||".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                } else {
                    out.push(Spanned {
                        kind: Tok::Pipe,
                        text: "|".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                }
            }
            '=' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                if peek(&chars, pos) == Some('>') {
                    advance(&chars, &mut pos, &mut line, &mut col);
                    out.push(Spanned {
                        kind: Tok::FatArrow,
                        text: "=>".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                } else if peek(&chars, pos) == Some('=') {
                    advance(&chars, &mut pos, &mut line, &mut col);
                    out.push(Spanned {
                        kind: Tok::EqEq,
                        text: "==".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                } else {
                    out.push(Spanned {
                        kind: Tok::Equal,
                        text: "=".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                }
            }
            '!' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                if peek(&chars, pos) == Some('=') {
                    advance(&chars, &mut pos, &mut line, &mut col);
                    out.push(Spanned {
                        kind: Tok::Ne,
                        text: "!=".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                } else {
                    out.push(Spanned {
                        kind: Tok::Bang,
                        text: "!".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                }
            }
            '<' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                if peek(&chars, pos) == Some('=') {
                    advance(&chars, &mut pos, &mut line, &mut col);
                    out.push(Spanned {
                        kind: Tok::Lte,
                        text: "<=".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                } else {
                    out.push(Spanned {
                        kind: Tok::Lt,
                        text: "<".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                }
            }
            '>' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                if peek(&chars, pos) == Some('=') {
                    advance(&chars, &mut pos, &mut line, &mut col);
                    out.push(Spanned {
                        kind: Tok::Gte,
                        text: ">=".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                } else {
                    out.push(Spanned {
                        kind: Tok::Gt,
                        text: ">".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                }
            }
            '&' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                if peek(&chars, pos) == Some('&') {
                    advance(&chars, &mut pos, &mut line, &mut col);
                    out.push(Spanned {
                        kind: Tok::AndAnd,
                        text: "&&".to_string(),
                        line: start_line,
                        col: start_col,
                    });
                } else {
                    return Err(anyhow!(
                        "unknown character '&' at line {}, column {}",
                        start_line,
                        start_col
                    ));
                }
            }
            '+' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                out.push(Spanned {
                    kind: Tok::Plus,
                    text: "+".to_string(),
                    line: start_line,
                    col: start_col,
                });
            }
            '-' => {
                // A `-` directly after an operand is binary minus, with or
                // without a space: `5-1`, `x-1`, `n -1`, `"s" - 1`, `a[i]-1`,
                // `f(x)-1`. So `n -1` is n - 1 and `r = n -3 * 2` is n - (3 * 2).
                // Only when no operand precedes it on the same line (no token,
                // an operator, `(`, `[`, `{`, `,`, `:`, `=`, `=>`, `|`, or a
                // token on an earlier line) is `-1` a negative literal, so
                // `[1, -1]`, `f(-1)`, `x = -1`, `a - -1` and a statement-start
                // `-1` keep their sign. A word call therefore cannot take a
                // negative literal argument: write `f(-1)`.
                let glued_to_operand = pos > 0
                    && (is_ident_part(chars[pos - 1])
                        || matches!(chars[pos - 1], '"' | ')' | ']' | '}'));
                let after_operand =
                    glued_to_operand || out.last().is_some_and(|t| ends_operand(t, start_line));
                advance(&chars, &mut pos, &mut line, &mut col);
                // negative number if followed by digit
                if let Some(d) = peek(&chars, pos) {
                    if d.is_ascii_digit() && !after_operand {
                        let (kind, text) =
                            read_number(&chars, &mut pos, &mut line, &mut col, "-".to_string())?;
                        out.push(Spanned {
                            kind,
                            text,
                            line: start_line,
                            col: start_col,
                        });
                        continue;
                    }
                }
                out.push(Spanned {
                    kind: Tok::Minus,
                    text: "-".to_string(),
                    line: start_line,
                    col: start_col,
                });
            }
            '*' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                out.push(Spanned {
                    kind: Tok::Star,
                    text: "*".to_string(),
                    line: start_line,
                    col: start_col,
                });
            }
            '^' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                out.push(Spanned {
                    kind: Tok::Caret,
                    text: "^".to_string(),
                    line: start_line,
                    col: start_col,
                });
            }
            '/' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                // Check for line comment //
                if peek(&chars, pos) == Some('/') {
                    // `//` straight after an operand, then an operand (`7 // 2`,
                    // `(7 // 2) * 10`), is the integer-division spelling from
                    // another language. Reading it as a comment silently dropped
                    // the rest of the line (`5 // 999` printed 5), so it is an
                    // error. `//` at the start of a line, or after an operator
                    // or a complete statement, is still a comment.
                    let after_operand = out.last().map_or(false, |t| {
                        t.line == start_line
                            && matches!(
                                t.kind,
                                Tok::Ident
                                    | Tok::String
                                    | Tok::Int
                                    | Tok::Float
                                    | Tok::RParen
                                    | Tok::RBracket
                                    | Tok::RBrace
                                    | Tok::True
                                    | Tok::False
                                    | Tok::Null
                            )
                    });
                    let operand_next = division_spelling(&chars[pos + 1..]);
                    if after_operand && operand_next {
                        return Err(anyhow::Error::new(ParseError::with_hint(
                            format!(
                                "`//` at line {}, column {} follows an operand",
                                start_line, start_col
                            ),
                            "`//` starts a comment in IronShell; there is no integer-division operator: use math.floor(a / b)",
                        )));
                    }
                    advance(&chars, &mut pos, &mut line, &mut col); // consume second /
                                                                    // Skip until end of line
                    while let Some(ch) = peek(&chars, pos) {
                        if ch == '\n' {
                            break;
                        }
                        advance(&chars, &mut pos, &mut line, &mut col);
                    }
                    continue; // Don't push a token, just skip the comment
                }
                out.push(Spanned {
                    kind: Tok::Slash,
                    text: "/".to_string(),
                    line: start_line,
                    col: start_col,
                });
            }
            '%' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                out.push(Spanned {
                    kind: Tok::Percent,
                    text: "%".to_string(),
                    line: start_line,
                    col: start_col,
                });
            }
            '"' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                let mut s = String::new();
                let mut closed = false;
                while let Some(ch) = peek(&chars, pos) {
                    advance(&chars, &mut pos, &mut line, &mut col);
                    if ch == '"' {
                        closed = true;
                        break;
                    }
                    if ch == '$' && peek(&chars, pos) == Some('{') {
                        // An interpolation hole is kept verbatim, quotes and
                        // all: the expression inside it is parsed when the
                        // string is evaluated, so its own `"` must not end
                        // this string.
                        if let Some(end) = interp_hole_end(&chars, pos + 1) {
                            s.push('$');
                            while pos < end {
                                let hole = chars[pos];
                                advance(&chars, &mut pos, &mut line, &mut col);
                                s.push(hole);
                            }
                            continue;
                        }
                        // A `${` that no `}` closes used to be kept as text, and
                        // the string then evaluated the rest of itself as a hole:
                        // `"a ${x"` printed `a 1`. It is refused where it starts.
                        return Err(unclosed_interpolation(line, col.saturating_sub(1)));
                    }
                    if ch == '\\' {
                        // Where the backslash is, for a malformed `\u` report.
                        let (escape_line, escape_col) = (line, col.saturating_sub(1));
                        if let Some(esc) = peek(&chars, pos) {
                            advance(&chars, &mut pos, &mut line, &mut col);
                            match esc {
                                'n' => s.push('\n'),
                                'r' => s.push('\r'),
                                't' => s.push('\t'),
                                '\\' => s.push('\\'),
                                '"' => s.push('"'),
                                '\'' => s.push('\''),
                                '0' => s.push('\0'),
                                '$' => s.push(ESCAPED_DOLLAR),
                                'x' | 'u' => match hex_escape(&chars, pos, esc) {
                                    Some((scalar, width)) => {
                                        for _ in 0..width {
                                            advance(&chars, &mut pos, &mut line, &mut col);
                                        }
                                        s.push(scalar);
                                    }
                                    // A malformed `\u` is refused: it used to print
                                    // literally and nobody noticed.
                                    None if esc == 'u' => {
                                        return Err(invalid_unicode_escape(
                                            escape_line,
                                            escape_col,
                                            &chars,
                                            pos,
                                            'u',
                                        ));
                                    }
                                    // A malformed `\x` keeps its backslash, as
                                    // any unknown escape does.
                                    None => {
                                        s.push('\\');
                                        s.push(esc);
                                    }
                                },
                                // `\UHHHHHHHH` (Python's long form): exactly eight
                                // hex digits. Without them it is not an escape and
                                // keeps its backslash, so `C:\Users` is unchanged.
                                'U' => match long_unicode_escape(&chars, pos) {
                                    Some(value) => match char::from_u32(value) {
                                        Some(scalar) => {
                                            for _ in 0..8 {
                                                advance(&chars, &mut pos, &mut line, &mut col);
                                            }
                                            s.push(scalar);
                                        }
                                        None => {
                                            return Err(invalid_unicode_escape(
                                                escape_line,
                                                escape_col,
                                                &chars,
                                                pos,
                                                'U',
                                            ));
                                        }
                                    },
                                    None => {
                                        // Four to seven hex digits after `\U` are a
                                        // malformed escape, not text: `\U0001F60 ` is
                                        // refused. Fewer keep the backslash, so
                                        // `C:\Users` and `C:\UEFI` read as written.
                                        let run = chars[pos..]
                                            .iter()
                                            .take_while(|c| c.is_ascii_hexdigit())
                                            .count();
                                        if (4..=7).contains(&run) {
                                            return Err(invalid_unicode_escape(
                                                escape_line,
                                                escape_col,
                                                &chars,
                                                pos,
                                                'U',
                                            ));
                                        }
                                        s.push('\\');
                                        s.push('U');
                                    }
                                },
                                // Not an escape the lexer knows (`C:\Users`,
                                // `\d`, `\e`). The backslash was dropped here
                                // before, so a Windows path lost its
                                // separators without any sign. Keep both.
                                other => {
                                    s.push('\\');
                                    s.push(other);
                                }
                            }
                        } else {
                            s.push('\\');
                        }
                    } else {
                        s.push(ch);
                    }
                }
                if !closed {
                    return Err(unterminated_string(start_line, start_col));
                }
                out.push(Spanned {
                    kind: Tok::String,
                    text: s,
                    line: start_line,
                    col: start_col,
                });
            }
            '\'' => {
                advance(&chars, &mut pos, &mut line, &mut col);
                let mut s = String::new();
                let mut closed = false;
                while let Some(ch) = peek(&chars, pos) {
                    advance(&chars, &mut pos, &mut line, &mut col);
                    if ch == '\'' {
                        closed = true;
                        break;
                    }
                    s.push(ch);
                }
                if !closed {
                    return Err(unterminated_string(start_line, start_col));
                }
                out.push(Spanned {
                    kind: Tok::String,
                    text: s,
                    line: start_line,
                    col: start_col,
                });
            }
            d if d.is_ascii_digit() => {
                let (kind, text) =
                    read_number(&chars, &mut pos, &mut line, &mut col, String::new())?;
                out.push(Spanned {
                    kind,
                    text,
                    line: start_line,
                    col: start_col,
                });
            }
            c if is_ident_start(c) => {
                let mut s = String::new();
                while let Some(ch) = peek(&chars, pos) {
                    if is_ident_part(ch) {
                        s.push(ch);
                        advance(&chars, &mut pos, &mut line, &mut col);
                    } else {
                        break;
                    }
                }
                let kw = match s.as_str() {
                    "fn" => Tok::Fn,
                    "let" => Tok::Let,
                    "mut" => Tok::Mut,
                    "pub" => Tok::Pub,
                    "export" => Tok::Export,
                    "true" => Tok::True,
                    "false" => Tok::False,
                    "null" => Tok::Null,
                    "match" => Tok::Match,
                    "if" => Tok::If,
                    "import" => Tok::Import,
                    "from" => Tok::From,
                    "as" => Tok::As,
                    "async" => Tok::Async,
                    "await" => Tok::Await,
                    "try" => Tok::Try,
                    "catch" => Tok::Catch,
                    "throw" => Tok::Throw,
                    // Python's and SQL's spellings of the boolean operators.
                    // A 3B model asked for IronShell wrote them in most of
                    // its attempts (benches/agentic/results/e7-local-*.md);
                    // nothing binds `and` or `or`, and a field of that name is
                    // still reachable (see `need_field`). `not` is *not* here:
                    // the prelude defines a `not` function and cfg conditions
                    // say `not(windows)`, so it is read as negation only where
                    // an operand follows (see `parse_unary`).
                    "and" => Tok::AndAnd,
                    "or" => Tok::OrOr,
                    _ => Tok::Ident,
                };
                out.push(Spanned {
                    kind: kw,
                    text: s,
                    line: start_line,
                    col: start_col,
                });
            }
            ';' => {
                // Emit semicolon token as statement separator
                advance(&chars, &mut pos, &mut line, &mut col);
                out.push(Spanned {
                    kind: Tok::Semicolon,
                    text: ";".to_string(),
                    line: start_line,
                    col: start_col,
                });
            }
            '#' => {
                advance(&chars, &mut pos, &mut line, &mut col); // consume #
                                                                // Check if this is an attribute #[...]
                if peek(&chars, pos) == Some('[') {
                    advance(&chars, &mut pos, &mut line, &mut col); // consume [
                                                                    // Read the attribute content until ]
                    let mut attr = String::new();
                    let mut depth = 1;
                    while let Some(ch) = peek(&chars, pos) {
                        advance(&chars, &mut pos, &mut line, &mut col);
                        if ch == '[' {
                            depth += 1;
                            attr.push(ch);
                        } else if ch == ']' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            attr.push(ch);
                        } else {
                            attr.push(ch);
                        }
                    }
                    // Push special attribute token
                    out.push(Spanned {
                        kind: Tok::Attribute,
                        text: attr,
                        line: start_line,
                        col: start_col,
                    });
                    continue;
                }
                // Otherwise it's a shell-style line comment
                // Skip until end of line
                while let Some(ch) = peek(&chars, pos) {
                    if ch == '\n' {
                        break;
                    }
                    advance(&chars, &mut pos, &mut line, &mut col);
                }
                continue; // Don't push a token, just skip the comment
            }
            other => {
                return Err(anyhow!(
                    "unknown character '{}' at line {}, column {}",
                    other,
                    start_line,
                    start_col
                ));
            }
        }
    }

    let (eof_line, eof_col) = eof_position(src, line, col);
    out.push(Spanned {
        kind: Tok::Eof,
        text: String::new(),
        line: eof_line,
        col: eof_col,
    });
    Ok((out, glued_brackets))
}

/// Where end of input is reported. After a trailing newline the lexer stands at
/// column 1 of the line after the last one, so an error at end of input named a
/// line that does not exist: a one-line script reported line 2. End of input is
/// placed just after the last line that has content instead.
fn eof_position(src: &str, line: usize, col: usize) -> (usize, usize) {
    if col != 1 || line == 1 {
        return (line, col);
    }
    match src
        .lines()
        .enumerate()
        .filter(|(_, text)| !text.trim().is_empty())
        .last()
    {
        Some((index, text)) => (index + 1, text.chars().count() + 1),
        None => (line, col),
    }
}

/// Whether `prev`, the last token emitted, ends an operand on line `line`, so
/// a `-` after it is binary minus. A token on an earlier line ends a statement
/// (or a list item already closed), so it does not count. `else` is a word
/// token that opens a branch, not an operand.
fn ends_operand(prev: &Spanned, line: usize) -> bool {
    if prev.line != line {
        return false;
    }
    match prev.kind {
        // `else -1` and `not -1` take a negative operand.
        Tok::Ident => !matches!(prev.text.as_str(), "else" | "not"),
        Tok::String
        | Tok::Int
        | Tok::Float
        | Tok::RParen
        | Tok::RBracket
        | Tok::RBrace
        | Tok::True
        | Tok::False
        | Tok::Null => true,
        _ => false,
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}
fn is_ident_part(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// What a `\$` escape leaves in a string literal's text. The evaluator turns it
/// back into `$` after interpolation, so `"\${x}"` is the text `${x}` and does
/// not interpolate. A private-use character, so it does not occur in real text.
pub const ESCAPED_DOLLAR: char = '\u{E000}';

/// The index just past the `}` that closes an interpolation hole whose body
/// starts at `start` (the index just after its `{`). String literals inside the
/// hole are skipped, and may hold holes of their own. `None` when the hole never
/// closes.
///
/// The lexer and the evaluator both use this, so they agree on where a hole
/// ends: `"x ${"b"} c"` is one string, and `"${"in ${n} q"}"` nests.
pub fn interp_hole_end(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            '"' => {
                i = interp_string_end(chars, i + 1)?;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The index just past the closing `"` of a string literal whose body starts at
/// `start`, skipping escapes and the holes of nested interpolation.
fn interp_string_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => return Some(i + 1),
            '$' if chars.get(i + 1) == Some(&'{') => {
                i = interp_hole_end(chars, i + 2)?;
            }
            _ => i += 1,
        }
    }
    None
}

/// The value of a `\U` escape's eight hex digits starting at `at`, or None when
/// fewer than eight hex digits follow. The value is not checked to be a scalar.
fn long_unicode_escape(chars: &[char], at: usize) -> Option<u32> {
    let hex: String = chars.get(at..at + 8)?.iter().collect();
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(&hex, 16).ok()
}

/// The scalar named by a `\xHH`, `\uHHHH` or `\u{H..H}` escape whose `x` or `u` is at
/// `at`, and how many source characters after `at` the escape spans. `None` when
/// the escape is malformed, or names no Unicode scalar value.
fn hex_escape(chars: &[char], at: usize, kind: char) -> Option<(char, usize)> {
    if kind == 'x' {
        let hex: String = chars.get(at..at + 2)?.iter().collect();
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        // `\xHH` is ASCII only: above 0x7F it is refused, and keeps its backslash.
        let value = u32::from_str_radix(&hex, 16).ok()?;
        if value > 0x7F {
            return None;
        }
        return Some((char::from_u32(value)?, 2));
    }
    if chars.get(at) != Some(&'{') {
        // `\uHHHH`: exactly four hex digits, with no braces.
        let hex: String = chars.get(at..at + 4)?.iter().collect();
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(&hex, 16).ok()?;
        return Some((char::from_u32(value)?, 4));
    }
    let close = (at + 1..chars.len().min(at + 9)).find(|&k| chars[k] == '}')?;
    let hex: String = chars[at + 1..close].iter().collect();
    if hex.is_empty() || hex.len() > 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(&hex, 16).ok()?;
    Some((char::from_u32(value)?, close - at + 1))
}

// ============ Parser ============

impl Parser {
    fn parse_stmt(&mut self) -> Result<Stmt> {
        self.reject_shell_flag()?;
        // Check for #[cfg(...)] attribute
        if self.check(Tok::Attribute) {
            let attr_text = self.peek().text.clone();
            self.i += 1; // consume the attribute token

            // Parse the cfg condition from the attribute text
            let condition = self.parse_cfg_condition(&attr_text)?;

            // Parse the following statement
            let body = self.parse_stmt()?;

            return Ok(Stmt::Cfg {
                condition,
                body: Box::new(body),
            });
        }

        // Check for `pub` visibility modifier
        let visibility = if self.match_tok(Tok::Pub) {
            Visibility::Pub
        } else {
            Visibility::Private
        };

        // Check for `mut name = value` (mutable without let)
        if self.check(Tok::Mut) && self.peek_ahead(1) == Some(Tok::Ident) {
            let peek2 = self.peek_ahead(2);
            if peek2 == Some(Tok::Equal) || peek2 == Some(Tok::ColonEqual) {
                self.match_tok(Tok::Mut); // consume 'mut'
                let name = self.need_ident("expected identifier after mut")?;
                // Accept either = or :=
                if !self.match_tok(Tok::Equal) {
                    self.need(Tok::ColonEqual, "expected '=' or ':='")?;
                }
                let value = self.parse_expr()?;
                return Ok(Stmt::Let {
                    name,
                    value,
                    is_mut: true,
                    visibility,
                    ty: None,
                    bare: false,
                });
            }
        }

        // Check for `name = value` or `name := value` (simple assignment with type inference)
        if self.check(Tok::Ident) {
            let peek1 = self.peek_ahead(1);
            if peek1 == Some(Tok::Equal) || peek1 == Some(Tok::ColonEqual) {
                let name = self.need_ident("expected identifier")?;
                // Accept either = or :=
                if !self.match_tok(Tok::Equal) {
                    self.need(Tok::ColonEqual, "expected '=' or ':='")?;
                }
                let value = self.parse_expr()?;
                return Ok(Stmt::Let {
                    name,
                    value,
                    is_mut: false,
                    visibility,
                    ty: None,
                    bare: true,
                });
            }
        }

        // `name.field(.field)* = value`: sets a field of the Record bound to `name`.
        if visibility == Visibility::Private && self.at_field_assignment() {
            let name = self.need_ident("expected identifier")?;
            let mut path = Vec::new();
            while self.match_tok(Tok::Dot) {
                path.push(self.need_field("expected field name after '.'")?);
            }
            if !self.match_tok(Tok::Equal) {
                self.need(Tok::ColonEqual, "expected '=' or ':=' after field")?;
            }
            let value = self.parse_expr()?;
            return Ok(Stmt::SetField { name, path, value });
        }

        if self.match_tok(Tok::Let) {
            // Keep let/let mut for explicit declarations
            let is_mut = self.match_tok(Tok::Mut);
            let name = self.need_ident("expected identifier after let")?;
            let ty = if self.match_tok(Tok::Colon) {
                Some(self.need_ident("expected type after ':'")?)
            } else {
                None
            };
            // Accept either = or :=
            if !self.match_tok(Tok::Equal) {
                self.need(Tok::ColonEqual, "expected '=' or ':=' in let")?;
            }
            let value = self.parse_expr()?;
            Ok(Stmt::Let {
                name,
                value,
                is_mut,
                visibility,
                ty,
                bare: false,
            })
        } else if self.match_tok(Tok::Import) {
            // Parse import statement
            if visibility == Visibility::Pub {
                return Err(self.error_at_prev("'pub' cannot be used with import statements"));
            }
            self.parse_import()
        } else if self.match_tok(Tok::Export) {
            // Parse export statement
            if visibility == Visibility::Pub {
                return Err(self.error_at_prev("'pub' cannot be used with export statements"));
            }
            self.parse_export()
        } else {
            // Top-level statement: allow word-call sugar (e.g. `print "hi"").
            if visibility == Visibility::Pub {
                return Err(
                    self.error_at_prev("'pub' can only be used with let/assignment statements")
                );
            }
            let prev = self.allow_word_call;
            self.allow_word_call = true;
            let e = self.parse_expr()?;
            self.allow_word_call = prev;
            Ok(Stmt::Expr(e))
        }
    }

    /// Parse import statement
    /// Syntax:
    ///   import "path"
    ///   import "path" as alias
    ///   import { a, b } from "path"
    ///   import { a as x, b } from "path"
    fn parse_import(&mut self) -> Result<Stmt> {
        let mut items = Vec::new();
        let mut alias = None;

        // Check for destructuring import: import { ... } from "path"
        if self.match_tok(Tok::LBrace) {
            // Parse import items
            loop {
                if self.check(Tok::RBrace) {
                    break;
                }
                let name = self.need_ident("expected import name")?;
                let item_alias = if self.match_tok(Tok::As) {
                    Some(self.need_ident("expected alias after 'as'")?)
                } else {
                    None
                };
                items.push(ImportItem {
                    name,
                    alias: item_alias,
                });
                if !self.match_tok(Tok::Comma) {
                    break;
                }
            }
            self.need(Tok::RBrace, "expected '}' after import list")?;
            self.need(Tok::From, "expected 'from' after import list")?;
        }

        // Parse source path (string)
        let source = self.need_string("expected module path string")?;

        // Check for alias: import "path" as name
        if items.is_empty() && self.match_tok(Tok::As) {
            alias = Some(self.need_ident("expected alias after 'as'")?);
        }

        Ok(Stmt::Import {
            items,
            source,
            alias,
        })
    }

    /// Parse export statement
    /// Syntax:
    ///   export { a, b }
    ///   export { a as x, b }
    ///   export { a, b } from "path"  (re-export)
    fn parse_export(&mut self) -> Result<Stmt> {
        self.need(Tok::LBrace, "expected '{' after export")?;

        let mut items = Vec::new();

        // Parse export items
        loop {
            if self.check(Tok::RBrace) {
                break;
            }
            let name = self.need_ident("expected export name")?;
            let item_alias = if self.match_tok(Tok::As) {
                Some(self.need_ident("expected alias after 'as'")?)
            } else {
                None
            };
            items.push(ExportItem {
                name,
                alias: item_alias,
            });
            if !self.match_tok(Tok::Comma) {
                break;
            }
        }

        self.need(Tok::RBrace, "expected '}' after export list")?;

        // Check for re-export: export { ... } from "path"
        let from_source = if self.match_tok(Tok::From) {
            Some(self.need_string("expected module path string after 'from'")?)
        } else {
            None
        };

        Ok(Stmt::Export { items, from_source })
    }

    /// Parse a cfg condition from attribute text like "cfg(windows)" or "cfg(feature = \"name\")"
    fn parse_cfg_condition(&self, attr: &str) -> Result<CfgCondition> {
        let attr = attr.trim();

        // Must start with "cfg("
        if !attr.starts_with("cfg(") || !attr.ends_with(')') {
            return Err(self.error_at_prev(&format!(
                "invalid cfg attribute: expected cfg(...), got '{}'",
                attr
            )));
        }

        // Extract the inner content
        let inner = &attr[4..attr.len() - 1];
        self.parse_cfg_inner(inner.trim())
    }

    fn parse_cfg_inner(&self, content: &str) -> Result<CfgCondition> {
        let content = content.trim();

        // Check for not(...)
        if content.starts_with("not(") && content.ends_with(')') {
            let inner = &content[4..content.len() - 1];
            let cond = self.parse_cfg_inner(inner.trim())?;
            return Ok(CfgCondition::Not(Box::new(cond)));
        }

        // Check for all(...)
        if content.starts_with("all(") && content.ends_with(')') {
            let inner = &content[4..content.len() - 1];
            let conditions = self.parse_cfg_list(inner)?;
            return Ok(CfgCondition::All(conditions));
        }

        // Check for any(...)
        if content.starts_with("any(") && content.ends_with(')') {
            let inner = &content[4..content.len() - 1];
            let conditions = self.parse_cfg_list(inner)?;
            return Ok(CfgCondition::Any(conditions));
        }

        // Check for feature = "name"
        if content.starts_with("feature") {
            let rest = content["feature".len()..].trim();
            if rest.starts_with('=') {
                let value = rest[1..].trim();
                // Remove quotes
                let value = value.trim_matches('"').trim_matches('\'');
                return Ok(CfgCondition::Feature(value.to_string()));
            }
            return Err(self.error_at_prev("invalid feature cfg: expected feature = \"name\""));
        }

        // Otherwise it's a platform name
        Ok(CfgCondition::Platform(content.to_string()))
    }

    fn parse_cfg_list(&self, content: &str) -> Result<Vec<CfgCondition>> {
        let mut conditions = Vec::new();
        let mut depth = 0;
        let mut current = String::new();

        for ch in content.chars() {
            match ch {
                '(' => {
                    depth += 1;
                    current.push(ch);
                }
                ')' => {
                    depth -= 1;
                    current.push(ch);
                }
                ',' if depth == 0 => {
                    if !current.trim().is_empty() {
                        conditions.push(self.parse_cfg_inner(current.trim())?);
                    }
                    current.clear();
                }
                _ => current.push(ch),
            }
        }

        if !current.trim().is_empty() {
            conditions.push(self.parse_cfg_inner(current.trim())?);
        }

        Ok(conditions)
    }

    /// Deepest expression nesting accepted.
    ///
    /// Far above anything hand-written or machine-generated in practice, and
    /// far below the ~15,000 at which the native stack gives out. Exceeding it
    /// is a parse error, which a caller can handle; overflowing the stack is
    /// not.
    const MAX_EXPR_DEPTH: usize = 512;

    /// Refuse an over-long operator/postfix chain.
    ///
    /// These loops are iterative, so they never overflow the *parser's* stack —
    /// but each turn adds one level to the AST, and evaluating (or dropping)
    /// that AST is recursive. `1 + 1 + 1 …` 40,000 times parsed happily and
    /// then overflowed the stack somewhere else entirely, which is the worst
    /// place for it to surface. Bounding the chain bounds the tree.
    fn chain_guard(&self, links: usize) -> Result<()> {
        if links > Self::MAX_EXPR_DEPTH {
            return Err(anyhow::anyhow!(
                "expression chain longer than {} at line {}, column {}",
                Self::MAX_EXPR_DEPTH,
                self.peek().line,
                self.peek().col
            ));
        }
        Ok(())
    }

    fn parse_expr(&mut self) -> Result<Expr> {
        self.depth += 1;
        if self.depth > Self::MAX_EXPR_DEPTH {
            self.depth -= 1;
            return Err(anyhow::anyhow!(
                "expression nested more than {} deep at line {}, column {}",
                Self::MAX_EXPR_DEPTH,
                self.peek().line,
                self.peek().col
            ));
        }
        let out = self.parse_pipe();
        self.depth -= 1;
        out
    }

    fn parse_pipe(&mut self) -> Result<Expr> {
        let mut left = self.parse_logic_or()?;
        let mut links = 0usize;
        while self.match_tok(Tok::Pipe) {
            links += 1;
            self.chain_guard(links)?;
            // Field projection: `xs |.name` / `xs |.a.b` desugars to
            // `xs | map(fn(__) => __.name)` directly in the grammar (Phase 5:
            // the terse form lives in the parser, not a text pre-pass).
            if self.check(Tok::Dot) {
                let mut body = Expr::Ident("__".to_string());
                while self.match_tok(Tok::Dot) {
                    let field = self.need_field("expected field name after '|.'")?;
                    body = Expr::MemberAccess {
                        object: Box::new(body),
                        field,
                    };
                }
                let projector = Expr::Call {
                    callee: Box::new(Expr::Ident("map".to_string())),
                    args: vec![Expr::Lambda {
                        params: vec!["__".to_string()],
                        body: Box::new(body),
                    }],
                    named: Vec::new(),
                };
                left = Expr::Pipe {
                    left: Box::new(left),
                    right: Box::new(projector),
                };
                continue;
            }
            self.reject_hyphenated_stage()?;
            let right = self.parse_call_like()?;
            left = Expr::Pipe {
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// The arguments of a parenthesised call, after the `(` has been seen by the
    /// caller: comma-separated expressions, each `.field` argument a row lambda.
    fn parse_paren_call_args(&mut self) -> Result<Vec<Expr>> {
        self.match_tok(Tok::LParen);
        let mut args = Vec::new();
        let mut arg_start = self.i;
        if !self.check(Tok::RParen) {
            loop {
                arg_start = self.i;
                // An argument that mentions `.field` without a receiver
                // is a function of the row: wrap it as one.
                let saved = (self.implicit_row, self.implicit_used);
                self.implicit_row = true;
                self.implicit_used = false;
                let parsed = self.parse_expr();
                let used = self.implicit_used;
                (self.implicit_row, self.implicit_used) = saved;
                let a = parsed?;
                args.push(if used {
                    Expr::Lambda {
                        params: vec!["__".to_string()],
                        body: Box::new(a),
                    }
                } else {
                    a
                });
                if self.match_tok(Tok::Comma) {
                    continue;
                }
                break;
            }
        }
        self.need_call_close(arg_start)?;
        Ok(args)
    }

    /// The `)` that ends a call's arguments, where the last argument began at
    /// `arg_start`. A `|` in that argument with no `)` after it is the pipe
    /// binding loosest, so the error says how to parenthesize it.
    fn need_call_close(&mut self, arg_start: usize) -> Result<()> {
        if !self.check(Tok::RParen) && self.pipe_in_span(arg_start) {
            let tok = self.peek();
            return Err(anyhow::Error::new(HintedSyntax {
                msg: format!(
                    "expected ')' after arguments at line {}, column {}",
                    tok.line, tok.col
                ),
                hint: PIPE_ARG_HINT.to_string(),
            }));
        }
        self.need(Tok::RParen, "expected ')' after arguments")
    }

    /// Whether a `|` outside any bracket lies in the tokens from `start` to the
    /// current one.
    fn pipe_in_span(&self, start: usize) -> bool {
        let span = match self.toks.get(start..self.i) {
            Some(span) => span,
            None => return false,
        };
        let mut depth = 0usize;
        for t in span {
            match t.kind {
                Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
                Tok::RParen | Tok::RBracket | Tok::RBrace => depth = depth.saturating_sub(1),
                Tok::Pipe if depth == 0 => return true,
                _ => {}
            }
        }
        false
    }

    /// `module.member(` as a pipeline stage: the callee chain when the tokens ahead
    /// are `ident (. ident)+`. Consumes them only on a match. Returns the callee and
    /// whether an argument list follows it. A module member with no list
    /// (`| arr.sum`) is a stage too, the same as `| sum`; any other dotted name
    /// without a list (`x | a.b` where `a` is not a module) is left to the caller.
    fn take_module_member_callee(&mut self) -> Option<(Expr, bool)> {
        let head = self.toks.get(self.i)?;
        if head.kind != Tok::Ident {
            return None;
        }
        #[cfg(feature = "native")]
        let module_head = crate::modules::is_module_name(&head.text);
        #[cfg(not(feature = "native"))]
        let module_head = false;
        let mut k = self.i + 1;
        let mut members = Vec::new();
        while self.toks.get(k).map(|t| t.kind) == Some(Tok::Dot) {
            let member = self.toks.get(k + 1)?;
            if member.kind != Tok::Ident {
                return None;
            }
            members.push(member.text.clone());
            k += 2;
        }
        let parens = self.toks.get(k).map(|t| t.kind) == Some(Tok::LParen);
        if members.is_empty() || (!parens && !module_head) {
            return None;
        }
        let mut callee = Expr::Ident(head.text.clone());
        self.i += 1;
        for member in members {
            self.i += 2;
            callee = Expr::MemberAccess {
                object: Box::new(callee),
                field: member,
            };
        }
        Some((callee, parens))
    }

    /// Word-call support: if the rhs begins with an identifier, slurp a sequence of
    /// atoms/lambdas as arguments to that identifier.
    fn parse_call_like(&mut self) -> Result<Expr> {
        if self.check(Tok::Fn) {
            return self.parse_lambda();
        }
        if self.check(Tok::Async) {
            return self.parse_async_lambda();
        }
        if let Some((callee, parens)) = self.take_module_member_callee() {
            if !parens {
                return Ok(Expr::Call {
                    callee: Box::new(callee),
                    args: Vec::new(),
                    named: Vec::new(),
                });
            }
            let args = self.parse_paren_call_args()?;
            return Ok(Expr::Call {
                callee: Box::new(callee),
                args,
                named: Vec::new(),
            });
        }
        let primary = self.parse_indexed_atom()?;
        // Remember the line where the primary expression ended (for word-call line tracking)
        let primary_line = self.prev().line;
        if let Expr::Ident(_) = primary {
            let callee = primary;
            // If the next token is a parenthesis, parse a normal call with
            // comma-separated expressions (handles e.g. map(fn(x)=>..., 0)).
            if self.check(Tok::LParen) {
                let args = self.parse_paren_call_args()?;
                return Ok(Expr::Call {
                    callee: Box::new(callee),
                    args,
                    named: Vec::new(),
                });
            }

            // Otherwise support word-call style: space-separated atoms and lambdas
            // Only continue if tokens are on the same line (newlines act as statement separators)
            let mut args: Vec<Expr> = Vec::new();
            while self.peek().line == primary_line
                && self.check_any(&[
                    Tok::Fn,
                    Tok::Async,
                    Tok::String,
                    Tok::Int,
                    Tok::Float,
                    Tok::LBrace,
                    Tok::LBracket,
                    Tok::Ident,
                    Tok::LParen,
                    Tok::True,
                    Tok::False,
                    Tok::Null,
                ])
            {
                if self.check(Tok::Fn) {
                    let lam = self.parse_lambda()?;
                    args.push(lam);
                    continue;
                }
                if self.check(Tok::Async) {
                    let lam = self.parse_async_lambda()?;
                    args.push(lam);
                    continue;
                }
                if self.check_any(&[
                    Tok::Pipe,
                    Tok::RBrace,
                    Tok::RParen,
                    Tok::Semicolon,
                    Tok::Eof,
                ]) {
                    break;
                }
                let a = self.parse_indexed_atom()?;
                args.push(a);
            }
            if args.is_empty() {
                Ok(callee)
            } else {
                Ok(Expr::Call {
                    callee: Box::new(callee),
                    args,
                    named: Vec::new(),
                })
            }
        } else {
            Ok(primary)
        }
    }

    fn parse_logic_or(&mut self) -> Result<Expr> {
        let mut e = self.parse_logic_and()?;
        let mut links = 0usize;
        while self.match_tok(Tok::OrOr) {
            links += 1;
            self.chain_guard(links)?;
            let r = self.parse_logic_and()?;
            e = Expr::Binary {
                left: Box::new(e),
                op: BinOp::Or,
                right: Box::new(r),
            };
        }
        Ok(e)
    }

    fn parse_logic_and(&mut self) -> Result<Expr> {
        let mut e = self.parse_not_word()?;
        let mut links = 0usize;
        while self.match_tok(Tok::AndAnd) {
            links += 1;
            self.chain_guard(links)?;
            let r = self.parse_not_word()?;
            e = Expr::Binary {
                left: Box::new(e),
                op: BinOp::And,
                right: Box::new(r),
            };
        }
        Ok(e)
    }

    /// Word `not`, with Python's and SQL's precedence: looser than the
    /// comparisons, tighter than `and`. `not .b > 5` is `!(.b > 5)`; the
    /// symbol `!` keeps its own, tighter binding.
    fn parse_not_word(&mut self) -> Result<Expr> {
        if self.match_word_not() {
            let e = self.parse_not_word()?;
            return Ok(Expr::Unary {
                op: UnOp::Not,
                expr: Box::new(e),
            });
        }
        self.parse_equality()
    }

    fn parse_equality(&mut self) -> Result<Expr> {
        let mut e = self.parse_comparison()?;
        let mut links = 0usize;
        loop {
            links += 1;
            self.chain_guard(links)?;
            if self.match_tok(Tok::EqEq) {
                let r = self.parse_comparison()?;
                e = Expr::Binary {
                    left: Box::new(e),
                    op: BinOp::Eq,
                    right: Box::new(r),
                };
            } else if self.match_tok(Tok::Ne) {
                let r = self.parse_comparison()?;
                e = Expr::Binary {
                    left: Box::new(e),
                    op: BinOp::Ne,
                    right: Box::new(r),
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_comparison(&mut self) -> Result<Expr> {
        let mut e = self.parse_term()?;
        let mut links = 0usize;
        loop {
            links += 1;
            self.chain_guard(links)?;
            if self.match_tok(Tok::Lt) {
                let r = self.parse_term()?;
                e = Expr::Binary {
                    left: Box::new(e),
                    op: BinOp::Lt,
                    right: Box::new(r),
                };
            } else if self.match_tok(Tok::Lte) {
                let r = self.parse_term()?;
                e = Expr::Binary {
                    left: Box::new(e),
                    op: BinOp::Lte,
                    right: Box::new(r),
                };
            } else if self.match_tok(Tok::Gt) {
                let r = self.parse_term()?;
                e = Expr::Binary {
                    left: Box::new(e),
                    op: BinOp::Gt,
                    right: Box::new(r),
                };
            } else if self.match_tok(Tok::Gte) {
                let r = self.parse_term()?;
                e = Expr::Binary {
                    left: Box::new(e),
                    op: BinOp::Gte,
                    right: Box::new(r),
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_term(&mut self) -> Result<Expr> {
        let mut e = self.parse_factor()?;
        let mut links = 0usize;
        loop {
            links += 1;
            self.chain_guard(links)?;
            if self.match_tok(Tok::Plus) {
                let r = self.parse_factor()?;
                e = Expr::Binary {
                    left: Box::new(e),
                    op: BinOp::Add,
                    right: Box::new(r),
                };
            } else if self.match_tok(Tok::Minus) {
                let r = self.parse_factor()?;
                e = Expr::Binary {
                    left: Box::new(e),
                    op: BinOp::Sub,
                    right: Box::new(r),
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_factor(&mut self) -> Result<Expr> {
        // Handle multiplicative operators and exponentiation (right-assoc)
        let mut e = self.parse_power()?;
        let mut links = 0usize;
        loop {
            links += 1;
            self.chain_guard(links)?;
            if self.match_tok(Tok::Star) {
                let r = self.parse_power()?;
                e = Expr::Binary {
                    left: Box::new(e),
                    op: BinOp::Mul,
                    right: Box::new(r),
                };
            } else if self.match_tok(Tok::Slash) {
                let r = self.parse_power()?;
                e = Expr::Binary {
                    left: Box::new(e),
                    op: BinOp::Div,
                    right: Box::new(r),
                };
            } else if self.match_tok(Tok::Percent) {
                let r = self.parse_power()?;
                e = Expr::Binary {
                    left: Box::new(e),
                    op: BinOp::Rem,
                    right: Box::new(r),
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn parse_power(&mut self) -> Result<Expr> {
        // Right-associative exponentiation: a ^ b ^ c -> a ^ (b ^ c)
        let mut left = self.parse_unary()?;
        if self.match_tok(Tok::Caret) {
            let right = self.parse_power()?;
            left = Expr::Binary {
                left: Box::new(left),
                op: BinOp::Pow,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        // `parse_unary` recurses into itself for prefix chains (`!!!x`, `---x`,
        // `await await x`) without passing through `parse_expr`, so it needs
        // its own guard: 50,000 leading `-` overflowed the stack even after
        // parenthesis nesting was bounded.
        self.depth += 1;
        if self.depth > Self::MAX_EXPR_DEPTH {
            self.depth -= 1;
            return Err(anyhow::anyhow!(
                "expression nested more than {} deep at line {}, column {}",
                Self::MAX_EXPR_DEPTH,
                self.peek().line,
                self.peek().col
            ));
        }
        let out = self.parse_unary_inner();
        self.depth -= 1;
        out
    }

    fn parse_unary_inner(&mut self) -> Result<Expr> {
        if self.match_tok(Tok::Bang) {
            let e = self.parse_unary()?;
            Ok(Expr::Unary {
                op: UnOp::Not,
                expr: Box::new(e),
            })
        } else if self.match_tok(Tok::Minus) {
            let e = self.parse_unary()?;
            Ok(Expr::Unary {
                op: UnOp::Neg,
                expr: Box::new(e),
            })
        } else if self.match_tok(Tok::Await) {
            // await expr - await the result of an async expression
            let e = self.parse_unary()?;
            Ok(Expr::Await(Box::new(e)))
        } else if self.match_tok(Tok::Throw) {
            // throw expr - raise an error
            let e = self.parse_unary()?;
            Ok(Expr::Throw(Box::new(e)))
        } else {
            self.parse_postfix()
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut e = self.parse_atom_expr()?;
        let mut links = 0usize;
        loop {
            links += 1;
            self.chain_guard(links)?;
            // A `(` that starts a new line starts a new statement, not a call
            // on the line before: `x = 5` then `(x + 1) * 2` called `5(...)`.
            if self.check(Tok::LParen) && self.continues_line() && self.match_tok(Tok::LParen) {
                let mut args = Vec::new();
                let mut arg_start = self.i;
                if !self.check(Tok::RParen) {
                    loop {
                        arg_start = self.i;
                        let a = self.parse_expr()?;
                        args.push(a);
                        if self.match_tok(Tok::Comma) {
                            continue;
                        }
                        break;
                    }
                }
                self.need_call_close(arg_start)?;
                e = Expr::Call {
                    callee: Box::new(e),
                    args,
                    named: Vec::new(),
                };
            } else if self.match_tok(Tok::Dot) {
                // Member access: obj.field
                let field = self.need_field("expected field name after '.'")?;
                e = Expr::MemberAccess {
                    object: Box::new(e),
                    field,
                };
            } else if self.at_index_bracket() || self.at_literal_index(&e) {
                e = self.parse_index_suffix(e)?;
            } else {
                break;
            }
        }
        // Support word-call (space-separated) style for top-level identifiers
        // e.g. `print "hi"` should parse like `print("hi")`. Only enable
        // this when the parser flag `allow_word_call` is set to avoid greedily
        // consuming tokens inside nested expressions like lambda bodies.
        if self.allow_word_call {
            if let Expr::Ident(_) = e {
                // A word-call may only take arguments on its own line.
                //
                // Without this guard a bare identifier statement swallowed the
                // statement after it: `hi` on one line followed by
                // `print("second")` on the next parsed as `hi(print, "second")`
                // — silently when the following call had one argument, and as a
                // baffling "expected ')'" pointing at a comma when it had two.
                // The sibling word-call loop in `parse_call_like` has always
                // had this check; this one did not.
                let call_line = self.prev().line;
                let mut args: Vec<Expr> = Vec::new();
                while self.peek().line == call_line
                    && self.check_any(&[
                        Tok::Fn,
                        Tok::Async,
                        Tok::String,
                        Tok::Int,
                        Tok::Float,
                        Tok::LBrace,
                        Tok::LBracket,
                        Tok::Ident,
                        Tok::LParen,
                        Tok::True,
                        Tok::False,
                        Tok::Null,
                    ])
                {
                    if self.check(Tok::Fn) {
                        let lam = self.parse_lambda()?;
                        args.push(lam);
                        continue;
                    }
                    if self.check(Tok::Async) {
                        let lam = self.parse_async_lambda()?;
                        args.push(lam);
                        continue;
                    }
                    if self.check_any(&[Tok::Pipe, Tok::RBrace, Tok::RParen, Tok::Eof]) {
                        break;
                    }
                    let a = self.parse_indexed_atom()?;
                    args.push(a);
                }
                if !args.is_empty() {
                    e = Expr::Call {
                        callee: Box::new(e),
                        args,
                        named: Vec::new(),
                    };
                }
            }
        }
        Ok(e)
    }

    /// Whether the next token is a `[` written directly after an indexable token
    /// on the same line: an identifier, a `)` or a `]`. Only that spelling is an
    /// index. `a[0]` and `f(x)[0]` index; `f [0]` and a `[` on the next line stay
    /// an array argument or a new statement.
    fn at_index_bracket(&self) -> bool {
        if !self.check(Tok::LBracket) || self.i == 0 || self.i >= self.toks.len() {
            return false;
        }
        let prev = &self.toks[self.i - 1];
        let cur = &self.toks[self.i];
        let prev_len = match prev.kind {
            Tok::Ident => prev.text.chars().count(),
            Tok::RParen | Tok::RBracket => 1,
            _ => return false,
        };
        prev.line == cur.line && prev.col + prev_len == cur.col
    }

    /// Whether the next token is a `[` written directly after a string or record
    /// literal that was just parsed: `"hello"[0]` and `{k: 1}["k"]` index. The
    /// atom decides, so a block's `}` (an `if`, `fn` or `match` body) followed
    /// by `[` is not affected. A spaced `"hello" [0]` is not glued and stays two
    /// expressions.
    fn at_literal_index(&self, atom: &Expr) -> bool {
        let literal = matches!(atom, Expr::LitStr(_) | Expr::Record(_));
        literal && self.check(Tok::LBracket) && self.glued_brackets.contains(&self.i)
    }

    /// Consume one `[index]` suffix and wrap `object` in an [`Expr::Index`].
    fn parse_index_suffix(&mut self, object: Expr) -> Result<Expr> {
        self.need(Tok::LBracket, "expected '['")?;
        let index = self.parse_expr()?;
        if self.check(Tok::Colon) {
            let msg = self
                .error_at_current("expected ']' after index")
                .to_string();
            return Err(anyhow::Error::new(SliceSpelling(msg)));
        }
        self.need(Tok::RBracket, "expected ']' after index")?;
        Ok(Expr::Index {
            object: Box::new(object),
            index: Box::new(index),
        })
    }

    /// An atom with any directly-attached `[index]` suffixes, for the operands of
    /// a word call and the head of a pipeline stage.
    fn parse_indexed_atom(&mut self) -> Result<Expr> {
        let mut e = self.parse_atom_expr()?;
        while self.at_index_bracket() || self.at_literal_index(&e) {
            e = self.parse_index_suffix(e)?;
        }
        Ok(e)
    }

    fn parse_atom_expr(&mut self) -> Result<Expr> {
        if self.implicit_row && self.check(Tok::Dot) {
            let mut e = Expr::Ident("__".to_string());
            while self.match_tok(Tok::Dot) {
                let field = self.need_field("expected a field name after '.'")?;
                e = Expr::MemberAccess {
                    object: Box::new(e),
                    field,
                };
            }
            self.implicit_used = true;
            return Ok(e);
        }
        if self.match_tok(Tok::LParen) {
            let e = self.parse_expr()?;
            self.need(Tok::RParen, "expected ')'")?;
            return Ok(e);
        }
        if self.match_tok(Tok::LBracket) {
            let mut items = Vec::new();
            if !self.check(Tok::RBracket) {
                loop {
                    let item = self.parse_expr()?;
                    items.push(item);
                    if self.match_tok(Tok::Comma) {
                        continue;
                    }
                    break;
                }
            }
            self.need(Tok::RBracket, "expected ']'")?;
            return Ok(Expr::Array(items));
        }
        if self.match_tok(Tok::LBrace) {
            let mut kvs = Vec::new();
            if !self.check(Tok::RBrace) {
                loop {
                    let key = self.need_field("expected key in record")?;
                    // A `{ ... }` with statements in it parses as a record, so
                    // the first statement fails here: `name = value`, or `let`.
                    if !self.check(Tok::Colon)
                        && (key == "let" || self.check(Tok::Equal) || self.check(Tok::ColonEqual))
                    {
                        let at = self.peek();
                        return Err(anyhow::Error::new(HintedSyntax {
                            msg: format!(
                                "expected ':' after key at line {}, column {}",
                                at.line, at.col
                            ),
                            hint: BLOCK_HINT.to_string(),
                        }));
                    }
                    // `fn() => { print("s"); 42 }`: a call or a `;` after the
                    // first word is a block of statements, not a record.
                    if !self.check(Tok::Colon)
                        && (self.check(Tok::LParen) || self.check(Tok::Semicolon))
                    {
                        let at = self.peek();
                        return Err(anyhow::Error::new(HintedSyntax {
                            msg: format!(
                                "expected ':' after key at line {}, column {}",
                                at.line, at.col
                            ),
                            hint: ONE_EXPR_HINT.to_string(),
                        }));
                    }
                    self.need(Tok::Colon, "expected ':' after key")?;
                    let val = self.parse_expr()?;
                    kvs.push((key, val));
                    if self.match_tok(Tok::Comma) {
                        continue;
                    }
                    break;
                }
            }
            self.need(Tok::RBrace, "expected '}'")?;
            return Ok(Expr::Record(kvs));
        }
        if self.match_tok(Tok::Match) {
            return self.parse_match();
        }
        // `?scrutinee { arms }` is terse sugar for `match scrutinee { arms }`,
        // parsed by the same routine (the `?`/`match` token is already consumed).
        if self.match_tok(Tok::Question) {
            return self.parse_match();
        }
        if self.match_tok(Tok::If) {
            return self.parse_if_expr();
        }
        if self.match_tok(Tok::Try) {
            return self.parse_try_catch();
        }
        if self.match_tok(Tok::Fn) {
            return self.parse_lambda_after_fn(false);
        }
        if self.match_tok(Tok::Async) {
            self.need(Tok::Fn, "expected 'fn' after 'async'")?;
            return self.parse_lambda_after_fn(true);
        }
        if self.match_tok(Tok::Tilde) {
            return self.parse_tilde_lambda();
        }
        if self.match_tok(Tok::String) {
            let s = self.prev().text.clone();
            return Ok(Expr::LitStr(s));
        }
        if self.match_tok(Tok::Float) {
            let x: f64 = self.prev().text.parse().unwrap_or(0.0);
            return Ok(Expr::LitFloat(x));
        }
        if self.match_tok(Tok::Int) {
            let n: i64 = self.prev().text.parse().unwrap_or(0);
            return Ok(Expr::LitInt(n));
        }
        if self.match_tok(Tok::True) {
            return Ok(Expr::LitBool(true));
        }
        if self.match_tok(Tok::False) {
            return Ok(Expr::LitBool(false));
        }
        if self.match_tok(Tok::Null) {
            return Ok(Expr::Null);
        }
        // `def f(...)`: a named function, which IronShell spells as a lambda value.
        if self.check(Tok::Ident)
            && self.peek().text.as_str() == "def"
            && self.peek_ahead(1) == Some(Tok::Ident)
            && self.peek_ahead(2) == Some(Tok::LParen)
        {
            let at = self.peek();
            return Err(anyhow::Error::new(HintedSyntax {
                msg: format!(
                    "`def` starts a function at line {}, column {}",
                    at.line, at.col
                ),
                hint: FUNCTION_HINT.to_string(),
            }));
        }
        // `while (...)`, `for x in ...`, `loop { ... }`: a loop head. The next
        // token is what makes it one, so `for = 1` and `for.x` stay names.
        if self.check(Tok::Ident)
            && matches!(self.peek().text.as_str(), "while" | "for" | "loop")
            && self.peek_ahead(1).is_some_and(|k| {
                matches!(
                    k,
                    Tok::LParen
                        | Tok::LBrace
                        | Tok::LBracket
                        | Tok::Ident
                        | Tok::Int
                        | Tok::Float
                        | Tok::String
                        | Tok::True
                        | Tok::False
                        | Tok::Null
                        | Tok::Bang
                )
            })
        {
            let at = self.peek();
            return Err(anyhow::Error::new(HintedSyntax {
                msg: format!(
                    "`{}` starts a loop at line {}, column {}",
                    at.text, at.line, at.col
                ),
                hint: LOOP_HINT.to_string(),
            }));
        }
        if self.match_tok(Tok::Ident) {
            let name = self.prev().text.clone();
            return Ok(Expr::Ident(name));
        }
        Err(self.error_at_current(&format!("unexpected token {:?}", self.peek().kind)))
    }

    fn parse_lambda(&mut self) -> Result<Expr> {
        self.need(Tok::Fn, "expected 'fn'")?;
        self.parse_lambda_after_fn(false)
    }

    /// Parse an async lambda: async fn(x) => expr
    fn parse_async_lambda(&mut self) -> Result<Expr> {
        self.need(Tok::Async, "expected 'async'")?;
        self.need(Tok::Fn, "expected 'fn' after 'async'")?;
        self.parse_lambda_after_fn(true)
    }

    /// Parse a terse lambda `~x: body` / `~x, y: body` (the `~` already consumed).
    /// Body is parsed at logic-or precedence so it's bounded by `|`/`,`/`)`/`]`/`}`
    /// and never swallows an enclosing pipeline. The implicit-parameter cipher
    /// form `~.field` is intentionally left to the `.ishg` transpiler — the legible
    /// grammar uses an explicit parameter.
    fn parse_tilde_lambda(&mut self) -> Result<Expr> {
        let mut params = Vec::new();
        loop {
            let p = self.need_ident("expected parameter name after '~'")?;
            params.push(p);
            if self.match_tok(Tok::Comma) {
                continue;
            }
            break;
        }
        self.need(Tok::Colon, "expected ':' after '~' lambda parameters")?;
        let prev_allow = self.allow_word_call;
        self.allow_word_call = false;
        self.lambda_param_stack.push(params.clone());
        let body = self.parse_logic_or()?;
        self.lambda_param_stack.pop();
        self.allow_word_call = prev_allow;
        Ok(Expr::Lambda {
            params,
            body: Box::new(body),
        })
    }

    /// The error for a lambda parameter followed by `=` (a default value) or `:`
    /// (a type annotation): lambda parameters have neither. `let` takes the type
    /// annotation, and it is checked; a default is written as a null check.
    fn reject_parameter_extra(&self, name: &str) -> Result<()> {
        let at = self.peek();
        let (what, hint) = if self.check(Tok::Equal) {
            (
                "lambda parameters take no default values",
                "lambda parameters take no default values: leave the parameter out of the \
                 list, or take it and check for null in the body, as in \
                 fn(a, b) => if b == null { a + 10 } else { a + b }",
            )
        } else if self.check(Tok::Colon) {
            (
                "lambda parameters take no type annotations",
                "lambda parameters take no type annotations: write fn(x) => x. Only \
                 `let x: Type = value` takes a type, and that type is checked",
            )
        } else {
            return Ok(());
        };
        Err(anyhow::Error::new(HintedSyntax {
            msg: format!("{what}: '{name}' at line {}, column {}", at.line, at.col),
            hint: hint.to_string(),
        }))
    }

    fn parse_lambda_after_fn(&mut self, is_async: bool) -> Result<Expr> {
        let saved = self.implicit_row;
        self.implicit_row = false;
        let out = self.parse_lambda_after_fn_inner(is_async);
        self.implicit_row = saved;
        out
    }

    fn parse_lambda_after_fn_inner(&mut self, is_async: bool) -> Result<Expr> {
        self.need(Tok::LParen, "expected '(' after fn")?;
        let mut params = Vec::new();
        if !self.check(Tok::RParen) {
            loop {
                let p = self.need_ident("expected parameter name")?;
                self.reject_parameter_extra(&p)?;
                params.push(p);
                if self.match_tok(Tok::Comma) {
                    continue;
                }
                break;
            }
        }
        self.need(Tok::RParen, "expected ')' after parameter list")?;
        // `fn(x) x.a` -- the arrow left out -- is what a model writes when its
        // prior is a language without one; the body that follows is
        // unambiguous, so accept it rather than fail the whole program.
        self.match_tok(Tok::FatArrow);
        // Parse the lambda body - include pipes as part of the body expression.
        // Disable word-call sugar while parsing to avoid greedily consuming
        // trailing atoms that belong to an outer call (e.g. `fn(a,b)=> a+b 0`).
        let prev_allow = self.allow_word_call;
        self.allow_word_call = false;
        // Push lambda params so `match { ... }` can use implicit scrutinee
        self.lambda_param_stack.push(params.clone());
        let body = self.parse_pipe()?; // Changed from parse_logic_or() to parse_pipe()
        self.lambda_param_stack.pop();
        self.allow_word_call = prev_allow;
        if is_async {
            Ok(Expr::AsyncLambda {
                params,
                body: Box::new(body),
            })
        } else {
            Ok(Expr::Lambda {
                params,
                body: Box::new(body),
            })
        }
    }

    fn parse_match(&mut self) -> Result<Expr> {
        // match expr { arms }         — explicit scrutinee
        // match { arms }              — implicit scrutinee (first param of enclosing lambda)
        // match {k: v} { arms }       — a record literal is the scrutinee: an arm
        //                               never starts `ident :`, so that `{` is one
        let scrutinee = if self.check(Tok::LBrace) && !self.at_record_literal() {
            // No explicit scrutinee — use first param of enclosing lambda
            let param = self
                .lambda_param_stack
                .last()
                .and_then(|params| params.first().cloned())
                .ok_or_else(|| {
                    self.error_at_current(
                        "`match { ... }` without scrutinee requires an enclosing lambda parameter",
                    )
                })?;
            Box::new(Expr::Ident(param))
        } else {
            // Explicit scrutinee expression
            let prev_allow = self.allow_word_call;
            self.allow_word_call = false;
            let expr = self.parse_pipe()?;
            self.allow_word_call = prev_allow;
            Box::new(expr)
        };

        // `c ? a : b`: the `?` opened a terse match, and the `:` is where that
        // match should have had its `{`. Name the ternary instead.
        if self.check(Tok::Colon) {
            let tok = self.peek();
            return Err(anyhow::Error::new(HintedSyntax {
                msg: format!(
                    "expected '{{' after match expression at line {}, column {}",
                    tok.line, tok.col
                ),
                hint: TERNARY_HINT.to_string(),
            }));
        }
        self.need(Tok::LBrace, "expected '{' after match expression")?;

        let mut arms = Vec::new();
        while !self.check(Tok::RBrace) && !self.check(Tok::Eof) {
            let arm = self.parse_match_arm()?;
            arms.push(arm);
            // Optional comma after each arm
            self.match_tok(Tok::Comma);
        }

        self.need(Tok::RBrace, "expected '}' after match arms")?;
        Ok(Expr::Match { scrutinee, arms })
    }

    /// Parse an `if` expression: `if cond { then } [else { else }]`.
    /// Desugars to a `match` on the boolean condition (the `if` is already
    /// consumed). `else` is detected as an identifier — no global keyword
    /// reservation. A missing `else` branch yields `null`.
    fn parse_if_expr(&mut self) -> Result<Expr> {
        let prev_allow = self.allow_word_call;
        self.allow_word_call = false;
        let cond = self.parse_pipe()?;
        self.allow_word_call = prev_allow;
        self.need(Tok::LBrace, "expected '{' after if condition")?;
        let then_expr = self.parse_expr()?;
        self.need(Tok::RBrace, "expected '}' after if branch")?;
        let else_expr = if self.check(Tok::Ident) && self.peek().text == "else" {
            self.i += 1; // consume 'else'
            if self.match_tok(Tok::If) {
                // `else if c { .. } else { .. }`: the chained `if` is a nested
                // if expression in the else position.
                self.parse_if_expr()?
            } else {
                self.need(Tok::LBrace, "expected '{' after else")?;
                let e = self.parse_expr()?;
                self.need(Tok::RBrace, "expected '}' after else branch")?;
                e
            }
        } else {
            Expr::Null
        };
        Ok(Expr::Match {
            scrutinee: Box::new(cond),
            arms: vec![
                crate::ast::MatchArm {
                    pattern: crate::ast::Pattern::LitBool(true),
                    guard: None,
                    body: Box::new(then_expr),
                },
                crate::ast::MatchArm {
                    pattern: crate::ast::Pattern::Wildcard,
                    guard: None,
                    body: Box::new(else_expr),
                },
            ],
        })
    }

    /// Parse try/catch expression:
    /// try { expr } catch { handler }
    /// try { expr } catch e { handler }  (with error binding)
    fn parse_try_catch(&mut self) -> Result<Expr> {
        // 'try' already consumed
        self.need(Tok::LBrace, "expected '{' after 'try'")?;
        let try_expr = self.parse_expr()?;
        self.need(Tok::RBrace, "expected '}' after try expression")?;

        self.need(Tok::Catch, "expected 'catch' after try block")?;

        // Optional error binding: catch e { ... }
        // Check if next token is Ident and token after that is LBrace
        let catch_var = if self.check(Tok::Ident) && self.peek_ahead(1) == Some(Tok::LBrace) {
            let name = self.peek().text.clone();
            self.i += 1;
            Some(name)
        } else {
            None
        };

        self.need(Tok::LBrace, "expected '{' after 'catch'")?;
        let catch_expr = self.parse_expr()?;
        self.need(Tok::RBrace, "expected '}' after catch expression")?;

        Ok(Expr::TryCatch {
            try_expr: Box::new(try_expr),
            catch_var,
            catch_expr: Box::new(catch_expr),
        })
    }

    fn parse_match_arm(&mut self) -> Result<crate::ast::MatchArm> {
        use crate::ast::MatchArm;

        let pattern = self.parse_pattern()?;

        // Optional guard: if condition
        let guard = if self.match_tok(Tok::If) {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };

        self.need(Tok::FatArrow, "expected '=>' in match arm")?;
        let body = Box::new(self.parse_expr()?);

        Ok(MatchArm {
            pattern,
            guard,
            body,
        })
    }

    fn parse_pattern(&mut self) -> Result<crate::ast::Pattern> {
        use crate::ast::Pattern;

        // Wildcard: _
        if self.check(Tok::Ident) && self.peek().text == "_" {
            self.i += 1;
            return Ok(Pattern::Wildcard);
        }

        // Literals. A number may carry a leading `-`.
        let negative =
            self.check(Tok::Minus) && matches!(self.peek_ahead(1), Some(Tok::Int | Tok::Float));
        if negative {
            self.i += 1;
        }
        if self.match_tok(Tok::Int) {
            let n: i64 = self.prev().text.parse().unwrap_or(0);
            return Ok(Pattern::LitInt(if negative { -n } else { n }));
        }
        if self.match_tok(Tok::Float) {
            let x: f64 = self.prev().text.parse().unwrap_or(0.0);
            return Ok(Pattern::LitFloat(if negative { -x } else { x }));
        }
        if self.match_tok(Tok::String) {
            let s = self.prev().text.clone();
            return Ok(Pattern::LitStr(s));
        }
        if self.match_tok(Tok::True) {
            return Ok(Pattern::LitBool(true));
        }
        if self.match_tok(Tok::False) {
            return Ok(Pattern::LitBool(false));
        }
        if self.match_tok(Tok::Null) {
            return Ok(Pattern::Null);
        }

        // Array pattern: [p1, p2, ...]
        if self.match_tok(Tok::LBracket) {
            let mut patterns = Vec::new();
            if !self.check(Tok::RBracket) {
                loop {
                    patterns.push(self.parse_pattern()?);
                    if self.match_tok(Tok::Comma) {
                        continue;
                    }
                    break;
                }
            }
            self.need(Tok::RBracket, "expected ']' in array pattern")?;
            return Ok(Pattern::Array(patterns));
        }

        // Record pattern: {field1, field2} or {field1: p1, field2: p2}
        if self.match_tok(Tok::LBrace) {
            let mut fields = Vec::new();
            if !self.check(Tok::RBrace) {
                loop {
                    let key = self.need_ident("expected field name in record pattern")?;
                    let pattern = if self.match_tok(Tok::Colon) {
                        self.parse_pattern()?
                    } else {
                        // Shorthand: {x} means {x: x}
                        Pattern::Ident(key.clone())
                    };
                    fields.push((key, pattern));
                    if self.match_tok(Tok::Comma) {
                        continue;
                    }
                    break;
                }
            }
            self.need(Tok::RBrace, "expected '}' in record pattern")?;
            return Ok(Pattern::Record(fields));
        }

        // Constructor pattern: Name(args...) or just Name
        if self.check(Tok::Ident) {
            let name = self.need_ident("expected pattern")?;

            // Check if it's a constructor call with args
            if self.match_tok(Tok::LParen) {
                let mut args = Vec::new();
                if !self.check(Tok::RParen) {
                    loop {
                        args.push(self.parse_pattern()?);
                        if self.match_tok(Tok::Comma) {
                            continue;
                        }
                        break;
                    }
                }
                self.need(Tok::RParen, "expected ')' in constructor pattern")?;
                return Ok(Pattern::Constructor { name, args });
            }

            // Otherwise it's just a variable binding
            return Ok(Pattern::Ident(name));
        }

        Err(self.error_at_current(&format!(
            "unexpected token in pattern: {:?}",
            self.peek().kind
        )))
    }

    // ---- small helpers ----
    fn check(&self, k: Tok) -> bool {
        self.peek().kind == k
    }
    /// `not` in prefix position -- followed by something that starts an
    /// operand -- is `!`. Anywhere else it stays a name, so the prelude's
    /// `let not = …`, `export { not }` and `map(not)` keep working, and
    /// `not(x)` means the same either way.
    fn match_word_not(&mut self) -> bool {
        let is_not = self.check(Tok::Ident) && self.peek().text == "not";
        if !is_not {
            return false;
        }
        let next = self.toks.get(self.i + 1).map(|t| &t.kind);
        let starts_operand = matches!(
            next,
            Some(
                Tok::Ident
                    | Tok::Int
                    | Tok::Float
                    | Tok::String
                    | Tok::True
                    | Tok::False
                    | Tok::Null
                    | Tok::LParen
                    | Tok::LBracket
                    | Tok::Bang
                    | Tok::Dot
            )
        );
        if starts_operand {
            self.i += 1;
        }
        starts_operand
    }

    /// Whether the current token is on the same line as the token before it.
    fn continues_line(&self) -> bool {
        match (
            self.i.checked_sub(1).and_then(|p| self.toks.get(p)),
            self.toks.get(self.i),
        ) {
            (Some(prev), Some(cur)) => prev.line == cur.line,
            _ => true,
        }
    }

    fn check_any(&self, ks: &[Tok]) -> bool {
        ks.iter().any(|k| self.peek().kind == *k)
    }
    /// Whether the `{` at the current token opens a record literal: `{ word :`.
    /// No match arm starts that way, since a pattern is never followed by `:`.
    fn at_record_literal(&self) -> bool {
        self.check(Tok::LBrace)
            && self.peek_ahead(1) == Some(Tok::Ident)
            && self.peek_ahead(2) == Some(Tok::Colon)
    }
    fn peek_ahead(&self, offset: usize) -> Option<Tok> {
        if self.i + offset < self.toks.len() {
            Some(self.toks[self.i + offset].kind)
        } else {
            None
        }
    }
    /// Whether the tokens ahead are `ident (. ident)+ =` (or `:=`): a field
    /// assignment. A plain `name = value` or a `==` comparison is not one.
    fn at_field_assignment(&self) -> bool {
        let kind = |k: usize| self.peek_ahead(k);
        if kind(0) != Some(Tok::Ident) || kind(1) != Some(Tok::Dot) {
            return false;
        }
        let mut k = 1;
        while kind(k) == Some(Tok::Dot) && kind(k + 1) == Some(Tok::Ident) {
            k += 2;
        }
        matches!(kind(k), Some(Tok::Equal) | Some(Tok::ColonEqual))
    }
    /// The chain of names, fields and indexes that ends just before the current
    /// token, as in `r.a[0].b`: whether it has a field step, whether it has an
    /// index step, and the index of its first token. None when the token before
    /// the current one does not end such a chain.
    fn assignable_chain_before(&self) -> Option<(bool, bool, usize)> {
        let mut end = self.i;
        let mut has_dot = false;
        let mut has_index = false;
        loop {
            let last = self.toks.get(end.checked_sub(1)?)?;
            match last.kind {
                Tok::RBracket => {
                    // Walk back to the `[` that opens this index step.
                    let mut depth = 0usize;
                    let mut k = end;
                    loop {
                        k = k.checked_sub(1)?;
                        match self.toks.get(k)?.kind {
                            Tok::RBracket => depth += 1,
                            Tok::LBracket => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    has_index = true;
                    end = k;
                }
                Tok::Ident => {
                    end -= 1;
                    // A field step: the name is preceded by `.`, so the chain
                    // goes on before it. Otherwise the name is the chain's root.
                    match end.checked_sub(1).and_then(|j| self.toks.get(j)) {
                        Some(t) if t.kind == Tok::Dot => {
                            has_dot = true;
                            end -= 1;
                        }
                        _ => return Some((has_dot, has_index, end)),
                    }
                }
                _ => return None,
            }
        }
    }
    fn match_tok(&mut self, k: Tok) -> bool {
        if self.check(k) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    /// Create an error message with line/column information at current token
    fn error_at_current(&self, msg: &str) -> anyhow::Error {
        let tok = self.peek();
        // A `(` left open on an earlier line is the real problem: name the line
        // it was opened on, not only the line the parser gave up on, and do not
        // offer a spelling hint about whatever token followed.
        // A second statement in a block is not an unclosed bracket: its hint wins.
        let block_statement = self.spelling_hint() == Some(ONE_EXPR_HINT);
        if let (false, Some((open_line, open_char))) = (block_statement, self.unclosed_opener(msg))
        {
            if open_line != tok.line {
                return anyhow!(
                    "'{}' opened at line {} is not closed; {} at line {}, column {}",
                    open_char,
                    open_line,
                    msg,
                    tok.line,
                    tok.col
                );
            }
        }
        let text = format!("{} at line {}, column {}", msg, tok.line, tok.col);
        match self.spelling_hint() {
            Some(hint) => anyhow::Error::new(HintedSyntax {
                msg: text,
                hint: hint.to_string(),
            }),
            None => anyhow!("{}", text),
        }
    }

    /// A builtin written the way a shell runs a command: `ls --help`, `mkdir -p
    /// x`, `grep -r x`. The word starts the statement, a space comes before its
    /// dash, and the dash (or the pair `--`) is glued to the flag's name. Without
    /// this, the parser reads the dash as minus and reports `unary -` on a
    /// builtin, or `unknown builtin: p`. `a - b`, `a-b`, `-1` and `n - -1` are
    /// not this shape, and neither is a word that does not start the statement.
    fn reject_shell_flag(&self) -> Result<()> {
        let (Some(word), Some(dash)) = (self.toks.get(self.i), self.toks.get(self.i + 1)) else {
            return Ok(());
        };
        let spaced = dash.line == word.line && dash.col > word.col + word.text.chars().count();
        if word.kind != Tok::Ident || dash.kind != Tok::Minus || !spaced {
            return Ok(());
        }
        let second_dash = self
            .toks
            .get(self.i + 2)
            .is_some_and(|t| t.kind == Tok::Minus && t.line == dash.line && t.col == dash.col + 1);
        let dashes = if second_dash { 2 } else { 1 };
        let Some(flag) = self.toks.get(self.i + 1 + dashes) else {
            return Ok(());
        };
        let glued =
            flag.kind == Tok::Ident && flag.line == dash.line && flag.col == dash.col + dashes;
        if !glued {
            return Ok(());
        }
        let name = word.text.as_str();
        // `total -discount` is a subtraction when `total` is a variable: a name
        // bound earlier in the source (`total = ...`) is left to the parser.
        let bound_earlier = self.toks[..self.i]
            .windows(2)
            .any(|w| w[0].kind == Tok::Ident && w[0].text == name && w[1].kind == Tok::Equal);
        if bound_earlier {
            return Ok(());
        }
        Err(anyhow::Error::new(HintedSyntax {
            msg: format!(
                "unexpected '-' after `{name}` at line {}, column {}",
                dash.line, dash.col
            ),
            hint: format!(
                "IronShell calls builtins as name(args), and has no -flags: write {name}(args) with each argument as a value; help(\"{name}\") describes a builtin; run shell commands with ish -b"
            ),
        }))
    }

    /// A pipeline stage written `from-csv` or `to-csv`. The lexer reads that as
    /// `from - csv` (or as the unknown name `to`, which is a word), and the
    /// error named a token the user never wrote. When the two words are joined
    /// by a hyphen and their underscore spelling is a builtin, say which name
    /// to write. Only the stage head is looked at, and only with no spaces.
    fn reject_hyphenated_stage(&self) -> Result<()> {
        let (Some(head), Some(minus), Some(tail)) = (
            self.toks.get(self.i),
            self.toks.get(self.i + 1),
            self.toks.get(self.i + 2),
        ) else {
            return Ok(());
        };
        let is_word = |t: &Spanned| {
            t.text
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
                && t.text.chars().all(|c| c.is_alphanumeric() || c == '_')
        };
        let joined = minus.line == head.line
            && minus.col == head.col + head.text.len()
            && tail.col == minus.col + 1;
        if !(is_word(head) && is_word(tail) && matches!(minus.kind, Tok::Minus) && joined) {
            return Ok(());
        }
        let underscore = format!("{}_{}", head.text, tail.text);
        // The signature table is part of the native build only; the web build
        // parses without this hint.
        #[cfg(feature = "native")]
        let known = crate::signature::signature_of(&underscore).is_some();
        #[cfg(not(feature = "native"))]
        let known = false;
        if !known {
            return Ok(());
        }
        Err(anyhow::Error::new(HintedSyntax {
            msg: format!(
                "unexpected '-' in `{}-{}` at line {}, column {}",
                head.text, tail.text, head.line, minus.col
            ),
            hint: format!(
                "IronShell builtin names use underscores: write `{underscore}`, not `{}-{}`",
                head.text, tail.text
            ),
        }))
    }

    /// The fix for a spelling from another language, when the token at the
    /// error is one: `=` where a comparison was meant, and `**` for a power.
    fn spelling_hint(&self) -> Option<&'static str> {
        let cur = self.peek().kind;
        let prev = self.i.checked_sub(1).and_then(|j| self.toks.get(j));
        // A `;` inside braces, or `name = value` right after an opening brace,
        // is a second statement in a block that holds one expression.
        if matches!(cur, Tok::Semicolon)
            && self
                .innermost_open_before(self.i)
                .is_some_and(|open| open.kind == Tok::LBrace)
        {
            return Some(ONE_EXPR_HINT);
        }
        if matches!(cur, Tok::Equal)
            && self.i >= 2
            && self
                .toks
                .get(self.i - 2)
                .is_some_and(|t| t.kind == Tok::LBrace)
            && prev.is_some_and(|p| p.kind == Tok::Ident)
        {
            return Some(ONE_EXPR_HINT);
        }
        // `=` after an assignable chain: `rows[0].n = 99` cannot assign an
        // element, and `{ r.a = 7 }` / `(r.a = v)` is a field assignment where
        // the block or the parentheses hold one expression.
        if matches!(cur, Tok::Equal) {
            if let Some((has_dot, has_index, start)) = self.assignable_chain_before() {
                if has_index {
                    return Some(INDEX_ASSIGN_HINT);
                }
                let opener = start
                    .checked_sub(1)
                    .and_then(|j| self.toks.get(j))
                    .map(|t| t.kind);
                if has_dot && matches!(opener, Some(Tok::LBrace | Tok::LParen)) {
                    return Some(ONE_EXPR_HINT);
                }
            }
        }
        // `=` is a comparison's spelling only directly after an operand, as in
        // `r.state = "open"`. After an operator or an opening bracket it is
        // not one, and the hint would name a comparison that was never written.
        if matches!(cur, Tok::Equal)
            && prev.is_some_and(|p| {
                matches!(
                    p.kind,
                    Tok::Ident
                        | Tok::String
                        | Tok::Int
                        | Tok::Float
                        | Tok::True
                        | Tok::False
                        | Tok::Null
                        | Tok::RParen
                        | Tok::RBracket
                        | Tok::RBrace
                )
            })
        {
            // `fn(x) => y = x * 2`: the body is `y`, and `=` assigns, which a
            // lambda body cannot do. The hint must say what to write instead.
            let after_arrow = self
                .i
                .checked_sub(2)
                .and_then(|j| self.toks.get(j))
                .is_some_and(|p| matches!(p.kind, Tok::FatArrow));
            if after_arrow {
                return Some(
                    "a lambda body is one expression: fn(x) => x * 2; bind values outside the lambda or use match",
                );
            }
            return Some("use `==` to compare; `=` assigns");
        }
        if matches!(cur, Tok::Star) && prev.is_some_and(|p| matches!(p.kind, Tok::Star)) {
            return Some("no `**` operator: use math.pow(a, b)");
        }
        // `a?.b` is optional chaining. The `?` opens a terse match, so the
        // error lands on the `?` before the dot or on the dot after it.
        if (matches!(cur, Tok::Question)
            && self
                .toks
                .get(self.i + 1)
                .is_some_and(|t| t.kind == Tok::Dot))
            || (matches!(cur, Tok::Dot) && prev.is_some_and(|p| p.kind == Tok::Question))
        {
            return Some(OPTIONAL_CHAIN_HINT);
        }
        // `c ? a : b` is the ternary. A `?` right after an operand is never
        // the start of a terse match, so it is the ternary's `?`.
        if matches!(cur, Tok::Question)
            && prev.is_some_and(|p| {
                matches!(
                    p.kind,
                    Tok::Ident
                        | Tok::String
                        | Tok::Int
                        | Tok::Float
                        | Tok::True
                        | Tok::False
                        | Tok::Null
                        | Tok::RParen
                        | Tok::RBracket
                        | Tok::RBrace
                )
            })
        {
            return Some(TERNARY_HINT);
        }
        None
    }

    /// Create an error message with line/column information at previous token
    fn error_at_prev(&self, msg: &str) -> anyhow::Error {
        let tok = self.prev();
        anyhow!("{} at line {}, column {}", msg, tok.line, tok.col)
    }

    fn need(&mut self, k: Tok, msg: &'static str) -> Result<()> {
        if self.match_tok(k) {
            Ok(())
        } else {
            Err(self.error_at_current(msg))
        }
    }
    /// A field name or record key: any word, reserved or not. `r.from`,
    /// `{match: 1}` and `x.not` failed to parse because the word after `.` had
    /// to be an identifier, and JSON data routinely has keys named `from`,
    /// `type` or `match`.
    fn need_field(&mut self, msg: &'static str) -> Result<String> {
        let t = self.peek();
        let wordlike = t
            .text
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
            && t.text.chars().all(|c| c.is_alphanumeric() || c == '_');
        if wordlike && !matches!(t.kind, Tok::String | Tok::Eof) {
            let text = t.text.clone();
            self.i += 1;
            return Ok(text);
        }
        self.need_ident(msg)
    }

    fn need_ident(&mut self, msg: &'static str) -> Result<String> {
        if self.match_tok(Tok::Ident) || self.match_tok(Tok::String) {
            Ok(self.prev().text.clone())
        } else {
            Err(self.error_with_suggestion(msg))
        }
    }
    fn need_string(&mut self, msg: &'static str) -> Result<String> {
        if self.match_tok(Tok::String) {
            Ok(self.prev().text.clone())
        } else {
            Err(self.error_with_suggestion(msg))
        }
    }
    fn peek(&self) -> &Spanned {
        // Safety: always return last token (Eof) if index is out of bounds
        if self.i >= self.toks.len() {
            self.toks.last().expect("token list should never be empty")
        } else {
            &self.toks[self.i]
        }
    }
    fn prev(&self) -> &Spanned {
        // Safety: return first token if i is 0
        if self.i == 0 {
            &self.toks[0]
        } else {
            &self.toks[self.i - 1]
        }
    }

    /// Generate error message with possible suggestion
    fn error_with_suggestion(&self, base_msg: &str) -> anyhow::Error {
        let tok = self.peek();
        // A `(` still open where a `)` was expected is the real problem, and it
        // is named where it was opened: the line the input ran out on says
        // nothing about where the missing `)` belonged.
        if let Some((open_line, open_char)) = self.unclosed_opener(base_msg) {
            return anyhow!(
                "'{}' opened at line {} is not closed; {} at line {}, column {}",
                open_char,
                open_line,
                base_msg,
                tok.line,
                tok.col
            );
        }
        let mut msg = format!("{} at line {}, column {}", base_msg, tok.line, tok.col);

        // Add suggestion based on context
        if let Some(suggestion) = self.get_suggestion() {
            msg.push_str(&format!("\n  suggestion: {}", suggestion));
        }

        anyhow!(msg)
    }

    /// For a message that wants a closer (`)`, `]` or `}`), the line and the
    /// character of the innermost bracket still open there, when it is the
    /// bracket that wants that closer. The error names it, not only the place
    /// where the input ran out.
    fn unclosed_opener(&self, msg: &str) -> Option<(usize, char)> {
        let (wanted, open_char) = if msg.starts_with("expected ')'") {
            (Tok::LParen, '(')
        } else if msg.starts_with("expected ']'") {
            (Tok::LBracket, '[')
        } else if msg.starts_with("expected '}'") {
            (Tok::LBrace, '{')
        } else {
            return None;
        };
        let open = self.innermost_open_before(self.i)?;
        (open.kind == wanted).then_some((open.line, open_char))
    }

    /// The innermost bracket opened before token `end` and not closed before it.
    fn innermost_open_before(&self, end: usize) -> Option<&Spanned> {
        let mut open: Vec<&Spanned> = Vec::new();
        for t in self.toks.iter().take(end) {
            match t.kind {
                Tok::LParen | Tok::LBracket | Tok::LBrace => open.push(t),
                Tok::RParen | Tok::RBracket | Tok::RBrace => {
                    open.pop();
                }
                _ => {}
            }
        }
        open.pop()
    }

    /// Get a suggestion based on the current parser state and token
    fn get_suggestion(&self) -> Option<String> {
        let tok = self.peek();

        // Check for common typos in keywords
        if tok.kind == Tok::Ident {
            let text = tok.text.to_lowercase();

            // Suggest 'let' for common typos
            if matches!(text.as_str(), "lte" | "elt" | "lt" | "le" | "lets" | "lett") {
                return Some(format!("did you mean 'let'? Found '{}'", tok.text));
            }

            // Suggest 'fn' for common typos
            if matches!(text.as_str(), "fun" | "func" | "function" | "fnn") {
                return Some(format!("did you mean 'fn'? Found '{}'", tok.text));
            }

            // Suggest 'match' for common typos
            if matches!(
                text.as_str(),
                "metch" | "mtch" | "swtich" | "switch" | "case"
            ) {
                return Some(format!("did you mean 'match'? Found '{}'", tok.text));
            }

            // Suggest 'import' for common typos
            if matches!(text.as_str(), "include" | "require" | "imprt" | "imoprt") {
                return Some(format!("did you mean 'import'? Found '{}'", tok.text));
            }

            // Suggest 'export' for common typos
            if matches!(text.as_str(), "exprt" | "exprot" | "exports") {
                return Some(format!("did you mean 'export'? Found '{}'", tok.text));
            }

            // Suggest 'true'/'false' for common typos
            if matches!(text.as_str(), "ture" | "treu" | "tre") {
                return Some(format!("did you mean 'true'? Found '{}'", tok.text));
            }
            if matches!(text.as_str(), "flase" | "fasle" | "fals") {
                return Some(format!("did you mean 'false'? Found '{}'", tok.text));
            }

            // Suggest 'null' for common typos
            if matches!(text.as_str(), "nil" | "none" | "nul" | "nill" | "undefined") {
                return Some(format!("did you mean 'null'? Found '{}'", tok.text));
            }

            // Suggest 'pub' for common typos
            if matches!(text.as_str(), "public" | "pbu") {
                return Some(format!("did you mean 'pub'? Found '{}'", tok.text));
            }

            // Suggest 'mut' for common typos
            if matches!(text.as_str(), "mutable" | "var" | "mtu") {
                return Some(format!("did you mean 'mut'? Found '{}'", tok.text));
            }
        }

        // Check for missing operators
        if tok.kind == Tok::Ident {
            // Previous was also ident - might be missing operator
            if self.i > 0 && self.toks[self.i - 1].kind == Tok::Ident {
                return Some(
                    "two identifiers in a row - did you forget an operator like '=' or '|'?"
                        .to_string(),
                );
            }
        }

        // Suggest closing bracket
        if tok.kind == Tok::Eof {
            // Check for unclosed delimiters
            let opens: Vec<_> = self
                .toks
                .iter()
                .filter(|t| matches!(t.kind, Tok::LParen | Tok::LBracket | Tok::LBrace))
                .collect();
            let closes: Vec<_> = self
                .toks
                .iter()
                .filter(|t| matches!(t.kind, Tok::RParen | Tok::RBracket | Tok::RBrace))
                .collect();

            if opens.len() > closes.len() {
                let last_open = opens.last().unwrap();
                let expected = match last_open.kind {
                    Tok::LParen => ")",
                    Tok::LBracket => "]",
                    Tok::LBrace => "}",
                    _ => "closing bracket",
                };
                return Some(format!(
                    "unclosed delimiter - expected '{}' to match '{}' at line {}",
                    expected, last_open.text, last_open.line
                ));
            }
        }

        None
    }

    /// Synchronize the parser state after encountering an error.
    /// This allows the parser to continue and report multiple errors.
    /// We skip tokens until we find a safe synchronization point.
    fn synchronize(&mut self) {
        // Advance past the current problematic token (if not at end)
        if self.i < self.toks.len().saturating_sub(1) {
            self.i += 1;
        }

        while !self.check(Tok::Eof) {
            // If the previous token indicates statement end, we're synchronized
            if self.i > 0 {
                let prev = &self.toks[self.i - 1];
                // After these tokens, a new statement likely begins
                if matches!(prev.kind, Tok::RBrace | Tok::RBracket | Tok::RParen) {
                    return;
                }
            }

            // These tokens often start new statements
            match self.peek().kind {
                Tok::Let
                | Tok::Fn
                | Tok::Async
                | Tok::If
                | Tok::Match
                | Tok::Import
                | Tok::Export
                | Tok::Pub => {
                    return;
                }
                // Attributes start new cfg-guarded statements
                Tok::Attribute => {
                    return;
                }
                _ => {}
            }

            // Don't advance past the last real token (Eof is always last)
            if self.i < self.toks.len().saturating_sub(1) {
                self.i += 1;
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The (kind, text) of every token, without the trailing Eof.
    fn toks(src: &str) -> Vec<(Tok, String)> {
        lex(src)
            .expect("lexes")
            .into_iter()
            .filter(|t| t.kind != Tok::Eof)
            .map(|t| (t.kind, t.text))
            .collect()
    }

    fn float_value(src: &str) -> f64 {
        let t = toks(src);
        assert_eq!(t.len(), 1, "one token for {src:?}: {t:?}");
        assert_eq!(t[0].0, Tok::Float, "{src:?} must be a Float literal");
        t[0].1.parse().expect("Float text parses")
    }

    fn parse_code(src: &str) -> String {
        let err = parse_program_strict(src).expect_err("must not parse");
        let msg = err.to_string();
        assert!(
            msg.contains("E_PARSE"),
            "{src:?} error is not E_PARSE: {msg}"
        );
        msg
    }

    #[test]
    fn exponent_literals_are_floats_with_the_right_value() {
        assert_eq!(float_value("1e10"), 1e10);
        assert_eq!(float_value("1E10"), 1e10);
        assert_eq!(float_value("1.5e3"), 1.5e3);
        assert_eq!(float_value("2.5E-3"), 2.5e-3);
        assert_eq!(float_value("1e+3"), 1e3);
        assert_eq!(float_value("6.02e23"), 6.02e23);
        assert_eq!(float_value("1e-0"), 1.0);
        assert_eq!(float_value("0e0"), 0.0);
    }

    #[test]
    fn negative_exponent_literals_keep_their_sign() {
        assert_eq!(float_value("-1.5e3"), -1.5e3);
        assert_eq!(float_value("-2e-2"), -2e-2);
    }

    #[test]
    fn exponent_literal_binds_as_a_float_value() {
        match parse_program_strict("x = 1e10").expect("parses").as_slice() {
            [Stmt::Let {
                value: Expr::LitFloat(x),
                ..
            }] => assert_eq!(*x, 1e10),
            other => panic!("x = 1e10 did not bind a float literal: {other:?}"),
        }
    }

    #[test]
    fn exponent_inside_a_call_parses() {
        // The original report: `print(1.5e3)` was E_PARSE "expected ')'".
        assert!(parse_program_strict("print(1.5e3)").is_ok());
        assert!(parse_program_strict("x = 1 + 2e3").is_ok());
    }

    #[test]
    fn integers_floats_and_si_suffixes_are_unchanged() {
        assert_eq!(toks("42"), vec![(Tok::Int, "42".into())]);
        assert_eq!(toks("3.14"), vec![(Tok::Float, "3.14".into())]);
        assert_eq!(toks("-7"), vec![(Tok::Int, "-7".into())]);
        assert_eq!(toks("1k"), vec![(Tok::Int, "1000".into())]);
        assert_eq!(toks("2M"), vec![(Tok::Int, "2000000".into())]);
        assert_eq!(toks("3G"), vec![(Tok::Int, "3000000000".into())]);
    }

    #[test]
    fn range_and_member_access_still_lex_as_before() {
        // `1..5` is Int, then the range dots, then Int: the fraction branch
        // must not swallow the first dot.
        assert_eq!(
            toks("1..5"),
            vec![
                (Tok::Int, "1".into()),
                (Tok::Dot, ".".into()),
                (Tok::Dot, ".".into()),
                (Tok::Int, "5".into()),
            ]
        );
        // `arr.range(10)`: the digits sit inside call parentheses.
        let t = toks("arr.range(10)");
        assert_eq!(t[0], (Tok::Ident, "arr".into()));
        assert_eq!(t[1], (Tok::Dot, ".".into()));
        assert_eq!(t[2], (Tok::Ident, "range".into()));
        assert_eq!(t[4], (Tok::Int, "10".into()));
        // `x.1`: a digit after a dot lexes as an Int field token, not a float.
        assert_eq!(toks("x.1")[2], (Tok::Int, "1".into()));
    }

    #[test]
    fn number_followed_by_identifier_character_is_e_parse() {
        for src in [
            "12abc", "1e", "1ex", "1e+", "1.5e", "1key", "1kb", "1_000", "1x",
        ] {
            let msg = parse_code(src);
            assert!(msg.contains("malformed number"), "{src:?}: {msg}");
        }
    }

    #[test]
    fn number_followed_by_space_or_punctuation_is_not_an_error() {
        assert!(parse_program_strict("print(1)").is_ok());
        assert!(parse_program_strict("x = [1, 2]").is_ok());
    }
}

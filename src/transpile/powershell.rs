//! PowerShell → IronShell Shell transpiler (compat mode).
//!
//! This is a pragmatic, line-oriented transpiler for a *useful subset* of PowerShell:
//!   - simple commands (cmdlets), args, and pipelines (`|`), split on `;`
//!   - single quotes:  'literal string'
//!   - double quotes:  expand $V and ${V} as IronShell string interpolations `${V}`
//!   - $VAR as a standalone arg becomes an identifier `VAR`
//!   - simple assignments: $NAME = value  →  let NAME = <expr>
//!   - arithmetic expression statements (`$x + 2`)
//!   - a small table of cmdlets whose IronShell target takes the same arguments
//!
//! Output is written as each statement runs: `Write-Host`, `Write-Output` and
//! every value-producing statement become `print(...)`, so a multi-statement
//! script prints each result, in order.
//!
//! What it cannot translate faithfully it does not guess at. A `-Parameter` of
//! a mapped cmdlet, a `$env:X` or `$(...)` expression, a here-string and the
//! like make the transpile fail before anything runs. Constructs that are
//! handed to `sh(["pwsh","-c", ...])` (blocks, operators, unmapped commands)
//! are reported through [`transpile_powershell_with_report`], so the caller
//! can refuse them by name and line when `sh` is not allowed.
//!
//! Variables whose names are IronShell module names (`$file`, `$user`) are
//! renamed with an `sh_` prefix in the translation.
//!
//! Output: IronShell source as a String.

use super::{Fallback, Kind};
use anyhow::{anyhow, Result};

/// Transpile a whole PowerShell script (multi-line) to IronShell code.
/// Multi-line blocks (if/foreach/for/while/switch/try/function/class) are
/// accumulated and emitted as a single `sh(["pwsh","-c","..."])` call rather
/// than falling back line-by-line.
pub fn transpile_powershell_to_ae(src: &str) -> Result<String> {
    transpile_powershell_with_report(src).map(|(code, _)| code)
}

/// [`transpile_powershell_to_ae`], plus every construct that was handed to
/// `sh(...)` rather than translated, with its line.
pub fn transpile_powershell_with_report(src: &str) -> Result<(String, Vec<Fallback>)> {
    let mut out = String::new();
    out.push_str("// Transpiled from PowerShell \u{2192} IronShell (compat mode)\n");

    let mut fb: Vec<Fallback> = Vec::new();
    let mut in_block_comment = false;
    let mut brace_depth: i32 = 0;
    let mut block_lines: Vec<String> = Vec::new();
    let mut block_start: usize = 0;

    for (idx, raw_line) in src.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw_line.trim();

        if in_block_comment {
            if line.contains("#>") {
                in_block_comment = false;
            }
            continue;
        }

        // Here-strings change what the lines after them mean; not translated.
        if line.contains("@\"") || line.contains("@'") {
            return Err(anyhow!(
                "powershell line {}: here-strings are not translated; nothing was run",
                line_no
            ));
        }

        if line.is_empty() {
            if brace_depth > 0 {
                block_lines.push(raw_line.to_string());
            }
            continue;
        }

        // PowerShell comments
        if line.starts_with('#') {
            if brace_depth > 0 {
                block_lines.push(raw_line.to_string());
            } else {
                out.push_str(&format!("// {}\n", line[1..].trim_start()));
            }
            continue;
        }

        // Block comments <# ... #>
        if line.starts_with("<#") {
            if brace_depth > 0 {
                block_lines.push(raw_line.to_string());
            } else {
                out.push_str(&format!(
                    "// {}\n",
                    line.trim_start_matches("<#").trim_end_matches("#>").trim()
                ));
            }
            if !line.contains("#>") {
                in_block_comment = true;
            }
            continue;
        }

        let openers = count_ps_brace_openers(line);
        let closers = count_ps_brace_closers(line);

        // Already inside a block
        if brace_depth > 0 {
            block_lines.push(raw_line.to_string());
            brace_depth += openers - closers;
            if brace_depth <= 0 {
                let full_block = block_lines.join("\n");
                out.push_str(&render_fallback_pwsh(&full_block));
                out.push('\n');
                fb.push(Fallback {
                    line: block_start,
                    construct: describe_ps_block(&block_lines[0]),
                });
                block_lines.clear();
                brace_depth = 0;
            }
            continue;
        }

        // New block starting (more open braces than close)
        if openers > closers {
            brace_depth = openers - closers;
            block_start = line_no;
            block_lines.push(raw_line.to_string());
            continue;
        }

        for stmt in split_ps_statements(line) {
            emit_ps_statement(&stmt, line_no, &mut out, &mut fb)?;
        }
    }

    // Flush any unterminated block
    if !block_lines.is_empty() {
        let full_block = block_lines.join("\n");
        out.push_str(&render_fallback_pwsh(&full_block));
        out.push('\n');
        fb.push(Fallback {
            line: block_start,
            construct: describe_ps_block(&block_lines[0]),
        });
    }

    Ok((out, fb))
}

/// The pipeline-stage cmdlet that takes a script block, when a statement has
/// one: `Where-Object { ... }`, `ForEach-Object { ... }` (and `?`, `%`, `where`
/// after a pipe), named so the refusal says what it is.
fn script_block_cmdlet(stmt: &str) -> Option<&'static str> {
    let stages = split_ps_pipes(stmt).ok()?;
    for (i, stage) in stages.iter().enumerate() {
        let w = stage.split_whitespace().next().unwrap_or("").to_lowercase();
        match w.as_str() {
            "where-object" => return Some("Where-Object"),
            "foreach-object" => return Some("ForEach-Object"),
            "?" | "where" if i > 0 => return Some("Where-Object"),
            "%" | "foreach" if i > 0 => return Some("ForEach-Object"),
            _ => {}
        }
    }
    None
}

fn describe_ps_block(first_line: &str) -> String {
    if let Some(c) = script_block_cmdlet(first_line) {
        return format!("command `{}`", c);
    }
    let t = first_line.trim();
    let w = t
        .split(|c: char| c.is_whitespace() || c == '(' || c == '{')
        .next()
        .unwrap_or("");
    match w.to_lowercase().as_str() {
        "if" | "foreach" | "for" | "while" | "do" | "switch" | "try" | "class" | "enum" => {
            format!("`{}` block", w.to_lowercase())
        }
        "function" | "filter" => "function definition".to_string(),
        _ => "script block".to_string(),
    }
}

fn describe_ps_complex(s: &str) -> String {
    if let Some(c) = script_block_cmdlet(s) {
        return format!("command `{}`", c);
    }
    let w = s
        .split(|c: char| c.is_whitespace() || c == '(' || c == '{')
        .next()
        .unwrap_or("")
        .to_lowercase();
    match w.as_str() {
        "if" | "foreach" | "for" | "while" | "do" | "switch" | "try" | "class" | "enum" => {
            format!("`{}` statement", w)
        }
        "function" => "function definition".to_string(),
        _ => {
            // A statement that starts with a cmdlet-like word (Verb-Noun) that
            // PS_MAP does not map is an unmapped command, not an operator.
            let word = s.split_whitespace().next().unwrap_or("");
            let mut parts = word.split('-');
            let verb_noun = matches!(
                (parts.next(), parts.next(), parts.next()),
                (Some(v), Some(n), None)
                    if !v.is_empty()
                        && !n.is_empty()
                        && v.chars().all(|c| c.is_ascii_alphabetic())
                        && n.chars().all(|c| c.is_ascii_alphanumeric())
            );
            if verb_noun
                && map_ps_cmdlet(&w).is_none()
                && !matches!(w.as_str(), "write-host" | "write-output")
            {
                format!("command `{}`", word)
            } else if s.contains("$(") {
                "subexpression `$(...)`".to_string()
            } else if s.contains(">>") || s.contains("2>&1") || s.contains("*>") {
                "redirection".to_string()
            } else {
                "expression or operator".to_string()
            }
        }
    }
}

/// Split a line at top-level `;`, outside quotes, parentheses, braces and
/// brackets; a trailing `# comment` is dropped.
fn split_ps_statements(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut sq = false;
    let mut dq = false;
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '`' && i + 1 < chars.len() {
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
                '(' | '{' | '[' => depth += 1,
                ')' | '}' | ']' => depth -= 1,
                ';' if depth <= 0 => {
                    out.push(std::mem::take(&mut cur));
                    i += 1;
                    continue;
                }
                '#' if cur.is_empty() || cur.ends_with(char::is_whitespace) => break,
                _ => {}
            }
        }
        cur.push(ch);
        i += 1;
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn emit_ps_statement(
    line: &str,
    line_no: usize,
    out: &mut String,
    fb: &mut Vec<Fallback>,
) -> Result<()> {
    // Skip if contains complex constructs that need fallback
    if contains_complex_construct(line) || script_block_cmdlet(line).is_some() {
        out.push_str(&render_fallback_pwsh(line));
        out.push('\n');
        fb.push(Fallback {
            line: line_no,
            construct: describe_ps_complex(line),
        });
        return Ok(());
    }

    // Try simple assignment: $VAR = value
    if let Some(assign) = parse_ps_assignment(line) {
        out.push_str(&assign);
        out.push('\n');
        return Ok(());
    }

    // `$x = <command or expression>`: not a literal, so not translated.
    if line.starts_with('$') && line.contains('=') && is_assignment_target(line) {
        out.push_str(&render_fallback_pwsh(line));
        out.push('\n');
        fb.push(Fallback {
            line: line_no,
            construct: "assignment of a command's output or an expression".to_string(),
        });
        return Ok(());
    }

    let toks = split_ps_words(line).map_err(|e| anyhow!("powershell line {}: {}", line_no, e))?;

    // Expression statement: `$x + 2`, `$x`, `"text"`, `5`.
    let piped = split_ps_pipes(line).map(|p| p.len() > 1).unwrap_or(false);
    if starts_like_expression(line) && !piped {
        return match parse_ps_expression(&toks) {
            Some(expr) => {
                out.push_str(&format!("print({});\n", expr));
                Ok(())
            }
            None => Err(anyhow!(
                "powershell line {}: the expression `{}` is not translated (only `+ - * %` between variables, numbers and strings are); nothing was run",
                line_no,
                line
            )),
        };
    }

    // Try to parse as a pipeline of commands
    let cmds =
        parse_ps_pipeline(line).map_err(|e| anyhow!("powershell line {}: {}", line_no, e))?;
    if cmds.is_empty() {
        return Ok(());
    }
    let code = render_ps_pipeline(&cmds, line_no, fb)?;
    out.push_str(&code);
    out.push('\n');
    Ok(())
}

/// `$name =` (and not `==`): the left side of an assignment.
fn is_assignment_target(line: &str) -> bool {
    match line.find('=') {
        Some(eq) => {
            let lhs = line[..eq]
                .trim_end_matches(|c: char| "+-*/%".contains(c))
                .trim();
            lhs.starts_with('$') && is_ps_ident(&lhs[1..])
        }
        None => false,
    }
}

fn starts_like_expression(line: &str) -> bool {
    match line.chars().next() {
        Some(c) => c == '$' || c == '(' || c == '"' || c == '\'' || c.is_ascii_digit(),
        None => false,
    }
}

/// `operand (op operand)*` with `+ - * %` between variables, numbers and
/// quoted text, as an IronShell expression. Division is not translated:
/// PowerShell's `/` yields a double and IronShell's does not always.
fn parse_ps_expression(toks: &[Token]) -> Option<String> {
    if toks.is_empty() || toks.len().is_multiple_of(2) {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if i % 2 == 1 {
            match t {
                Token::Interp(pcs) if pcs.len() == 1 => match &pcs[0] {
                    Piece::Text(op) if matches!(op.as_str(), "+" | "-" | "*" | "%") => {
                        out.push(op.clone())
                    }
                    _ => return None,
                },
                _ => return None,
            }
        } else {
            match t {
                Token::Single(_) | Token::Group(_) => out.push(render_token_expr(t).ok()?),
                Token::Interp(pcs) => {
                    let ok = match pcs.as_slice() {
                        [Piece::Var(_)] => true,
                        [Piece::Text(n)] => is_ps_number(n),
                        _ => pcs
                            .iter()
                            .all(|p| matches!(p, Piece::Text(_) | Piece::Var(_))),
                    };
                    if !ok {
                        return None;
                    }
                    out.push(render_token_expr(t).ok()?);
                }
                _ => return None,
            }
        }
    }
    Some(out.join(" "))
}

/* =============================================================================
Representation
============================================================================= */

#[derive(Debug, Clone)]
struct PsCommand {
    cmdlet: Token,    // The cmdlet or command name
    args: Vec<Token>, // Arguments (may include -Parameters)
}

#[derive(Debug, Clone)]
enum Token {
    /// Normal / double-quoted token with possible var expansions
    Interp(Vec<Piece>),
    /// Single-quoted string with no expansion
    Single(String),
    /// Parameter like -Name or -Path
    Parameter(String),
    /// Splatted variable @args
    Splat(String),
    /// A parenthesised expression, already translated to IronShell source
    /// (without its parentheses): `(1+2)` is `1 + 2`.
    Group(String),
}

#[derive(Debug, Clone)]
enum Piece {
    Text(String),
    Var(String), // VAR name (no leading '$')
}

/* =============================================================================
Block depth tracking (brace-based)
============================================================================= */

fn count_ps_brace_openers(line: &str) -> i32 {
    // Count unquoted opening braces
    let mut count = 0i32;
    let mut in_sq = false;
    let mut in_dq = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '`' && i + 1 < chars.len() {
            i += 2; // skip escaped char
            continue;
        }
        if ch == '\'' && !in_dq {
            in_sq = !in_sq;
        } else if ch == '\"' && !in_sq {
            in_dq = !in_dq;
        } else if ch == '{' && !in_sq && !in_dq {
            count += 1;
        }
        i += 1;
    }
    count
}

fn count_ps_brace_closers(line: &str) -> i32 {
    // Count unquoted closing braces
    let mut count = 0i32;
    let mut in_sq = false;
    let mut in_dq = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '`' && i + 1 < chars.len() {
            i += 2;
            continue;
        }
        if ch == '\'' && !in_dq {
            in_sq = !in_sq;
        } else if ch == '\"' && !in_sq {
            in_dq = !in_dq;
        } else if ch == '}' && !in_sq && !in_dq {
            count += 1;
        }
        i += 1;
    }
    count
}

/* =============================================================================
Complexity detection
============================================================================= */

fn contains_complex_construct(s: &str) -> bool {
    // Detect PowerShell constructs we don't handle yet
    s.contains("$(")           // Subexpression
        || s.contains("`(")    // Escaped parens
        || s.contains(">>")    // Append redirection
        || s.contains("2>&1")  // Error redirect
        || s.contains("*>")    // All streams redirect
        || s.starts_with("if ")
        || s.starts_with("if(")
        || s.starts_with("foreach ")
        || s.starts_with("foreach(")
        || s.starts_with("for ")
        || s.starts_with("for(")
        || s.starts_with("while ")
        || s.starts_with("while(")
        || s.starts_with("do ")
        || s.starts_with("switch ")
        || s.starts_with("switch(")
        || s.starts_with("try ")
        || s.starts_with("try{")
        || s.starts_with("function ")
        || s.starts_with("class ")
        || s.starts_with("enum ")
        || s.contains(" -join ")
        || s.contains(" -split ")
        || s.contains(" -match ")
        || s.contains(" -replace ")
        || s.contains(" -contains ")
        || s.contains(" -in ")
        || s.contains(" -notin ")
        || s.contains(" -eq ")
        || s.contains(" -ne ")
        || s.contains(" -gt ")
        || s.contains(" -ge ")
        || s.contains(" -lt ")
        || s.contains(" -le ")
        || s.contains(" -and ")
        || s.contains(" -or ")
        || s.contains("&&")
        || s.contains("||")
        || s.contains(" > ")
        || s.contains("[")     // Type casts/attributes
        || s.contains("::")    // Static method calls
        || has_range_operator(s) // Range operator
}

/// True for PowerShell's range operator (`1..10`, `$a..$b`, `(f)..3`): a `..`
/// with a number, variable or closing parenthesis before it and a number,
/// variable, sign or opening parenthesis after it. A `..` inside a path such
/// as `../x` is not a range.
fn has_range_operator(s: &str) -> bool {
    let b = s.as_bytes();
    (1..b.len().saturating_sub(1)).any(|i| {
        b[i] == b'.'
            && b[i + 1] == b'.'
            && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b')')
            && b.get(i + 2)
                .is_some_and(|c| c.is_ascii_digit() || matches!(*c, b'$' | b'(' | b'-'))
            && (b[i - 1].is_ascii_digit() || b[i - 1] == b')' || s[..i].contains('$'))
    })
}

fn render_fallback_pwsh(line: &str) -> String {
    let quoted = json_string_literal(line);
    format!("sh([\"pwsh\", \"-c\", {}]);", quoted)
}

/* =============================================================================
Assignment parsing
============================================================================= */

/// `$name = <expression>` with `+ - * %` between variables, numbers and text.
fn parse_ps_expression_assignment(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with('$') {
        return None;
    }
    let eq = trimmed.find('=')?;
    if trimmed[..eq].ends_with(|c: char| "+-*/%".contains(c)) || trimmed[eq + 1..].starts_with('=')
    {
        return None;
    }
    let var = trimmed[1..eq].trim();
    let val = trimmed[eq + 1..].trim();
    if !is_ps_ident(var) || val.is_empty() || !starts_like_expression(val) {
        return None;
    }
    if split_ps_pipes(val).map(|p| p.len() > 1).unwrap_or(true) {
        return None;
    }
    let toks = split_ps_words(val).ok()?;
    let expr = parse_ps_expression(&toks)?;
    Some(format!("let mut {} = {};", vname(var), expr))
}

fn parse_ps_assignment(line: &str) -> Option<String> {
    parse_ps_simple_assignment(line).or_else(|| parse_ps_expression_assignment(line))
}

fn parse_ps_simple_assignment(line: &str) -> Option<String> {
    // Match: $VarName = expression
    let trimmed = line.trim();

    // Must start with $
    if !trimmed.starts_with('$') {
        return None;
    }

    // Find the = sign
    let eq_pos = trimmed.find('=')?;

    // Check for compound assignment operators
    let before_eq = &trimmed[..eq_pos];
    if before_eq.ends_with('+')
        || before_eq.ends_with('-')
        || before_eq.ends_with('*')
        || before_eq.ends_with('/')
    {
        return None; // +=, -=, etc. - fallback
    }
    // `==` is not an assignment.
    if trimmed[eq_pos + 1..].starts_with('=') {
        return None;
    }

    let var_part = trimmed[1..eq_pos].trim();
    let val_part = trimmed[eq_pos + 1..].trim();

    // Validate variable name
    if !is_ps_ident(var_part) {
        return None;
    }
    let name = vname(var_part);

    // Parse the value
    if val_part.is_empty() {
        return Some(format!("let mut {} = null;", name));
    }

    // Handle different value types
    if val_part.starts_with('"') || val_part.starts_with('\'') {
        // A single quoted token only: `"a" + "b"` is an expression.
        let toks = split_ps_words(val_part).ok()?;
        if toks.len() != 1 {
            return None;
        }
        let expr = render_token_expr(&toks[0]).ok()?;
        return Some(format!("let mut {} = {};", name, expr));
    } else if val_part.starts_with('$') {
        // Variable reference
        if let Some(var) = parse_ps_var_ref(val_part) {
            return Some(format!("let mut {} = {};", name, var));
        }
    } else if val_part.starts_with("@(") {
        // Array literal - simplified handling
        if let Some(end) = val_part.rfind(')') {
            let inner = &val_part[2..end];
            let elements: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
            let ae_elements: Vec<String> = elements.iter().map(|e| parse_simple_value(e)).collect();
            return Some(format!("let mut {} = [{}];", name, ae_elements.join(", ")));
        }
    } else if val_part.starts_with("@{") {
        // Hashtable - simplified handling
        return None; // Complex, use fallback
    } else if is_ps_number(val_part) {
        return Some(format!("let mut {} = {};", name, val_part));
    }

    None
}

fn parse_simple_value(s: &str) -> String {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') {
        s.to_string()
    } else if s.starts_with('\'') && s.ends_with('\'') {
        format!("\"{}\"", &s[1..s.len() - 1])
    } else if s.starts_with('$') {
        vname(&s[1..])
    } else if is_ps_number(s) {
        s.to_string()
    } else {
        format!("\"{}\"", escape_string(s))
    }
}

/* =============================================================================
Pipeline parsing
============================================================================= */

fn parse_ps_pipeline(s: &str) -> Result<Vec<PsCommand>> {
    let parts = split_ps_pipes(s)?;
    let mut cmds = Vec::new();

    for p in parts {
        let toks = split_ps_words(&p)?;
        if toks.is_empty() {
            continue;
        }
        let cmdlet = toks[0].clone();
        let args = toks[1..].to_vec();
        cmds.push(PsCommand { cmdlet, args });
    }

    Ok(cmds)
}

fn split_ps_pipes(s: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut dq = false;
    let mut sq = false;
    let mut paren_depth: usize = 0;

    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        // Handle escape character
        if ch == '`' && i + 1 < chars.len() {
            cur.push(ch);
            cur.push(chars[i + 1]);
            i += 2;
            continue;
        }

        if ch == '"' && !sq {
            dq = !dq;
            cur.push(ch);
        } else if ch == '\'' && !dq {
            sq = !sq;
            cur.push(ch);
        } else if ch == '(' && !dq && !sq {
            paren_depth += 1;
            cur.push(ch);
        } else if ch == ')' && !dq && !sq {
            paren_depth = paren_depth.saturating_sub(1);
            cur.push(ch);
        } else if ch == '|' && !dq && !sq && paren_depth == 0 {
            out.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(ch);
        }

        i += 1;
    }

    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }

    Ok(out)
}

fn split_ps_words(s: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_dq = false;
    let mut in_sq = false;

    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        // Handle escape character (backtick in PowerShell)
        if ch == '`' && i + 1 < chars.len() {
            cur.push(chars[i + 1]);
            i += 2;
            continue;
        }

        // A parenthesised expression at the start of a word: `(1+2)`.
        if ch == '(' && !in_dq && !in_sq && cur.is_empty() {
            let close = matching_paren(&chars, i)
                .ok_or_else(|| anyhow!("the parenthesis in `{}` is not closed", s.trim()))?;
            if chars.get(close + 1).is_some_and(|c| !c.is_whitespace()) {
                return Err(anyhow!(
                    "the expression `{}` is not translated (only a whole parenthesised `+ - * %` expression is)",
                    s.trim()
                ));
            }
            let inner: String = chars[i + 1..close].iter().collect();
            tokens.push(Token::Group(translate_group(&inner)?));
            i = close + 1;
            continue;
        }

        if ch == '"' && !in_sq {
            if in_dq {
                // End of double-quoted string
                cur.push(ch);
                in_dq = false;
            } else {
                // Start of double-quoted string
                if !cur.is_empty() {
                    tokens.push(parse_ps_token(&cur)?);
                    cur.clear();
                }
                cur.push(ch);
                in_dq = true;
            }
        } else if ch == '\'' && !in_dq {
            if in_sq {
                // End of single-quoted string
                cur.push(ch);
                in_sq = false;
            } else {
                // Start of single-quoted string
                if !cur.is_empty() {
                    tokens.push(parse_ps_token(&cur)?);
                    cur.clear();
                }
                cur.push(ch);
                in_sq = true;
            }
        } else if ch.is_whitespace() && !in_dq && !in_sq {
            if !cur.is_empty() {
                tokens.push(parse_ps_token(&cur)?);
                cur.clear();
            }
        } else {
            cur.push(ch);
        }

        i += 1;
    }

    if !cur.is_empty() {
        tokens.push(parse_ps_token(&cur)?);
    }

    Ok(tokens)
}

/// The index of the `)` that closes the `(` at `open`, outside quotes.
fn matching_paren(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut i = open;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) => {
                if c == '`' && q == '"' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// The IronShell source of the expression inside `( ... )`, when it is in the
/// supported subset; otherwise a refusal that names it.
fn translate_group(inner: &str) -> Result<String> {
    tokenize_ps_expr(inner)
        .and_then(|toks| parse_ps_expression(&toks))
        .ok_or_else(|| {
            anyhow!(
                "the parenthesised expression `({})` is not translated (only `+ - * %` between variables, numbers and strings are)",
                inner.trim()
            )
        })
}

/// The operands and operators of an expression written with or without
/// spaces (`1+2`, `$x * -3`), as the tokens [`parse_ps_expression`] reads.
fn tokenize_ps_expr(s: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = s.chars().collect();
    let mut toks: Vec<Token> = Vec::new();
    let mut i = 0;
    let mut want_operand = true;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if !want_operand {
            if matches!(c, '+' | '-' | '*' | '%') {
                toks.push(Token::Interp(vec![Piece::Text(c.to_string())]));
                i += 1;
                want_operand = true;
                continue;
            }
            return None;
        }
        if c == '(' {
            let close = matching_paren(&chars, i)?;
            let inner: String = chars[i + 1..close].iter().collect();
            toks.push(Token::Group(translate_group(&inner).ok()?));
            i = close + 1;
        } else if c == '"' || c == '\'' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != c {
                if chars[j] == '`' {
                    return None;
                }
                j += 1;
            }
            if j >= chars.len() || chars.get(j + 1) == Some(&c) {
                return None;
            }
            let text: String = chars[i..=j].iter().collect();
            toks.push(parse_ps_token(&text).ok()?);
            i = j + 1;
        } else if c == '$' {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let text: String = chars[i..j].iter().collect();
            toks.push(parse_ps_token(&text).ok()?);
            i = j;
        } else if c.is_ascii_digit()
            || (c == '-' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '.') {
                j += 1;
            }
            let text: String = chars[i..j].iter().collect();
            if !is_ps_number(&text) {
                return None;
            }
            toks.push(Token::Interp(vec![Piece::Text(text)]));
            i = j;
        } else {
            return None;
        }
        want_operand = false;
    }
    if want_operand {
        None
    } else {
        Some(toks)
    }
}

fn parse_ps_token(s: &str) -> Result<Token> {
    let s = s.trim();

    if s.is_empty() {
        return Ok(Token::Single(String::new()));
    }

    // A negative number is a number, not a parameter.
    if s.starts_with('-') && is_ps_number(s) {
        return Ok(Token::Interp(vec![Piece::Text(s.to_string())]));
    }

    // Parameter (starts with -)
    if s.starts_with('-') && s.len() > 1 {
        let param_name = &s[1..];
        if is_ps_ident(param_name) || param_name.chars().all(|c| c.is_alphanumeric()) {
            return Ok(Token::Parameter(param_name.to_string()));
        }
    }

    // Splatted variable
    if s.starts_with('@') && s.len() > 1 {
        let var_name = &s[1..];
        if is_ps_ident(var_name) {
            return Ok(Token::Splat(var_name.to_string()));
        }
    }

    // Single-quoted string
    if s.starts_with('\'') && s.ends_with('\'') && s.len() >= 2 {
        let inner = &s[1..s.len() - 1];
        return Ok(Token::Single(inner.to_string()));
    }

    // Double-quoted string with variable interpolation
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        let inner = &s[1..s.len() - 1];
        let pieces = parse_ps_interpolation(inner)?;
        // Text only: a string, which is not a bare word (`"5"` is not `5`).
        if pieces.iter().all(|p| matches!(p, Piece::Text(_))) {
            let text: String = pieces
                .iter()
                .map(|p| match p {
                    Piece::Text(t) => t.as_str(),
                    Piece::Var(_) => "",
                })
                .collect();
            return Ok(Token::Single(text));
        }
        return Ok(Token::Interp(pieces));
    }

    // Bare word or variable
    if let Some(var_name) = s.strip_prefix('$') {
        if is_ps_ident(var_name) {
            return Ok(Token::Interp(vec![Piece::Var(var_name.to_string())]));
        }
    }

    // A `$` left in a bare word is a variable expression we do not read
    // (`$env:PATH`, `$x.Length`, `$x[0]`): never pass it on as text.
    if s.contains('$') {
        return Err(anyhow!(
            "the variable expression `{}` is not translated (only a plain `$name` is)",
            s
        ));
    }

    // Plain text
    Ok(Token::Interp(vec![Piece::Text(s.to_string())]))
}

fn parse_ps_interpolation(s: &str) -> Result<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut cur_text = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        // Escape character
        if ch == '`' && i + 1 < chars.len() {
            cur_text.push(chars[i + 1]);
            i += 2;
            continue;
        }

        // Variable interpolation
        if ch == '$' {
            // Save accumulated text
            if !cur_text.is_empty() {
                pieces.push(Piece::Text(cur_text.clone()));
                cur_text.clear();
            }

            // Parse variable name
            if i + 1 < chars.len() && chars[i + 1] == '{' {
                // ${variable} form
                let rest: String = chars[i + 2..].iter().collect();
                if let Some(end) = rest.find('}') {
                    let var_name = &rest[..end];
                    if is_ps_ident(var_name) {
                        pieces.push(Piece::Var(var_name.to_string()));
                        i += 2 + var_name.chars().count() + 1;
                        continue;
                    }
                }
            } else {
                // $variable form
                let mut var_name = String::new();
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    var_name.push(chars[j]);
                    j += 1;
                }
                if !var_name.is_empty() && is_ps_ident(&var_name) {
                    // `$env:PATH` is the variable `env:PATH`, not `$env` and text.
                    if j < chars.len() && chars[j] == ':' {
                        return Err(anyhow!(
                            "the variable `${}:...` in a string is not translated",
                            var_name
                        ));
                    }
                    pieces.push(Piece::Var(var_name));
                    i = j;
                    continue;
                }
            }

            // A `$` that starts an expression we do not read, in a string:
            // `$(...)`, `$_`, `$?`, `${a b}`. A lone `$` (or `$ `) is text.
            let next = chars.get(i + 1).copied();
            if matches!(next, Some(c) if c.is_alphanumeric() || "_{($?^".contains(c)) {
                return Err(anyhow!(
                    "the expression after `$` in a double-quoted string is not translated (only `$name` and `${{name}}` are)"
                ));
            }
            cur_text.push(ch);
        } else {
            cur_text.push(ch);
        }

        i += 1;
    }

    if !cur_text.is_empty() {
        pieces.push(Piece::Text(cur_text));
    }

    Ok(pieces)
}

/* =============================================================================
Rendering
============================================================================= */

fn cmdlet_name_of(tok: &Token) -> Option<String> {
    match tok {
        Token::Interp(pieces) if pieces.len() == 1 => match &pieces[0] {
            Piece::Text(t) => Some(t.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn render_ps_pipeline(
    cmds: &[PsCommand],
    line_no: usize,
    fb: &mut Vec<Fallback>,
) -> Result<String> {
    let last = cmds.len() - 1;
    let mut stages: Vec<String> = Vec::new();
    // Whether the last stage already writes its output, or is run for effect.
    let mut settled = false;
    let mut quiet = false;

    for (i, c) in cmds.iter().enumerate() {
        let name = match cmdlet_name_of(&c.cmdlet) {
            Some(n) => n,
            None => {
                // A variable as the first stage is a value to pipe on.
                if i == 0 && cmds.len() > 1 && c.args.is_empty() {
                    if let Token::Interp(p) = &c.cmdlet {
                        if let [Piece::Var(_)] = p.as_slice() {
                            stages.push(
                                render_token_expr(&c.cmdlet)
                                    .map_err(|e| anyhow!("powershell line {}: {}", line_no, e))?,
                            );
                            continue;
                        }
                    }
                }
                return Err(anyhow!(
                    "powershell line {}: a command name that is a variable, string or expression is not translated; nothing was run",
                    line_no
                ));
            }
        };
        let lower = name.to_lowercase();

        // Output cmdlets
        if matches!(lower.as_str(), "write-host" | "write-output" | "echo") {
            check_ps_params(&name, &c.args, line_no)?;
            if cmds.len() == 1 {
                if lower == "write-host" {
                    stages.push(format!("print({})", ps_joined(&c.args, line_no)?));
                } else if c.args.is_empty() {
                    stages.push("null".to_string());
                } else {
                    let mut prints = Vec::new();
                    for a in &c.args {
                        prints.push(format!(
                            "print({})",
                            render_token_expr(a).map_err(|e| anyhow!(
                                "powershell line {}: {}",
                                line_no,
                                e
                            ))?
                        ));
                    }
                    stages.push(prints.join("; "));
                }
                settled = true;
                continue;
            }
            if i == 0 && lower != "write-host" {
                let exprs: Vec<String> = c
                    .args
                    .iter()
                    .map(|a| {
                        render_token_expr(a)
                            .map_err(|e| anyhow!("powershell line {}: {}", line_no, e))
                    })
                    .collect::<Result<Vec<_>>>()?;
                stages.push(match exprs.len() {
                    0 => "null".to_string(),
                    1 => exprs[0].clone(),
                    _ => format!("[{}]", exprs.join(", ")),
                });
                continue;
            }
            fb.push(Fallback {
                line: line_no,
                construct: format!("`{}` inside a pipeline", name),
            });
            stages.push(render_generic_call(&name, &c.args));
            if i == last {
                settled = true;
            }
            continue;
        }

        if let Some((target, kind)) = map_ps_cmdlet(&lower) {
            let args = normalize_ps_args(&lower, &c.args);
            check_ps_params(&name, &args, line_no)?;
            let rendered: Vec<String> = args
                .iter()
                .map(|a| {
                    render_token_expr(a).map_err(|e| anyhow!("powershell line {}: {}", line_no, e))
                })
                .collect::<Result<Vec<_>>>()?;
            stages.push(format!("{}({})", target, rendered.join(", ")));
            if i == last {
                quiet = kind == Kind::Quiet;
            }
            continue;
        }

        // No faithful mapping: hand it to sh(), and say so.
        fb.push(Fallback {
            line: line_no,
            construct: format!("command `{}`", name),
        });
        stages.push(render_generic_call(&name, &c.args));
        if i == last {
            settled = true;
        }
    }

    let mut code = stages.join(" | ");
    if !settled && !quiet {
        code.push_str(" | print()");
    }
    code.push(';');
    if quiet && !settled {
        code.push_str("\nnull;");
    }
    Ok(code)
}

fn render_generic_call(name: &str, args: &[Token]) -> String {
    let mut all_args = vec![json_string_literal(name)];
    for arg in args {
        all_args.push(render_token_as_sh_arg(arg));
    }
    format!("sh([{}])", all_args.join(", "))
}

/// `Start-Sleep -Seconds 5` is `sleep(5)`; every other parameter is left as it
/// is, and refused by [`check_ps_params`].
fn normalize_ps_args(lower: &str, args: &[Token]) -> Vec<Token> {
    if matches!(lower, "start-sleep" | "sleep") && args.len() == 2 {
        if let Token::Parameter(p) = &args[0] {
            if p.eq_ignore_ascii_case("seconds") && !matches!(args[1], Token::Parameter(_)) {
                return vec![args[1].clone()];
            }
        }
    }
    args.to_vec()
}

/// A `-Parameter` is a named argument the IronShell target does not take:
/// refuse it by name rather than pass it as data.
fn check_ps_params(name: &str, args: &[Token], line_no: usize) -> Result<()> {
    for a in args {
        match a {
            Token::Parameter(p) => {
                return Err(anyhow!(
                    "powershell line {}: the parameter `-{}` of `{}` is not translated; nothing was run",
                    line_no,
                    p,
                    name
                ));
            }
            Token::Splat(s) => {
                return Err(anyhow!(
                    "powershell line {}: the splatted `@{}` in `{}` is not translated; nothing was run",
                    line_no,
                    s,
                    name
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// The words of a `Write-Host`, joined by single spaces, as one expression.
fn ps_joined(args: &[Token], line_no: usize) -> Result<String> {
    if args.is_empty() {
        return Ok("\"\"".to_string());
    }
    if args.len() == 1 {
        return render_token_expr(&args[0])
            .map_err(|e| anyhow!("powershell line {}: {}", line_no, e));
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
                        "powershell line {}: single-quoted text containing `${{` would be interpolated by IronShell; nothing was run",
                        line_no
                    ));
                }
                s.push_str(&escape_string(t));
            }
            Token::Interp(pieces) => {
                for p in pieces {
                    match p {
                        Piece::Text(t) => s.push_str(&escape_string(t)),
                        Piece::Var(v) => {
                            s.push_str("${");
                            s.push_str(&vname(v));
                            s.push('}');
                        }
                    }
                }
            }
            Token::Group(e) => {
                if e.contains('"') {
                    return Err(anyhow!(
                        "powershell line {}: a parenthesised expression with text in it, among several words, is not translated; nothing was run",
                        line_no
                    ));
                }
                s.push_str("${");
                s.push_str(e);
                s.push('}');
            }
            _ => {}
        }
    }
    s.push('"');
    Ok(s)
}

fn render_token_expr(tok: &Token) -> Result<String> {
    match tok {
        Token::Single(s) => {
            // IronShell interpolates `${...}` in single quotes too.
            if s.contains("${") {
                return Err(anyhow!(
                    "single-quoted text containing `${{` would be interpolated by IronShell"
                ));
            }
            Ok(format!("\"{}\"", escape_string(s)))
        }
        Token::Parameter(p) => Ok(format!("\"-{}\"", p)),
        Token::Splat(s) => Ok(vname(s)), // Pass as variable
        Token::Group(e) => Ok(format!("({})", e)),
        Token::Interp(pieces) => {
            if pieces.len() == 1 {
                match &pieces[0] {
                    // A bare number is a number; any other bare word is text.
                    Piece::Text(t) if is_ps_number(t) => Ok(t.clone()),
                    Piece::Text(t) => Ok(format!("\"{}\"", escape_string(t))),
                    Piece::Var(v) => Ok(vname(v)),
                }
            } else {
                // Build an interpolated string
                let mut s = String::from("\"");
                for piece in pieces {
                    match piece {
                        Piece::Text(t) => s.push_str(&escape_string(t)),
                        Piece::Var(v) => {
                            s.push_str("${");
                            s.push_str(&vname(v));
                            s.push('}');
                        }
                    }
                }
                s.push('"');
                Ok(s)
            }
        }
    }
}

fn render_token_as_sh_arg(tok: &Token) -> String {
    match tok {
        Token::Single(s) => json_string_literal(s),
        Token::Parameter(p) => json_string_literal(&format!("-{}", p)),
        Token::Splat(_) => "\"@args\"".to_string(), // Simplified
        Token::Group(e) => format!("({})", e),
        Token::Interp(pieces) => {
            if pieces.len() == 1 {
                match &pieces[0] {
                    Piece::Text(t) => json_string_literal(t),
                    Piece::Var(v) => vname(v), // Variable reference
                }
            } else {
                // Complex interpolation - render as expression
                render_token_expr(tok).unwrap_or_else(|_| "\"\"".to_string())
            }
        }
    }
}

/* =============================================================================
Cmdlet mapping
============================================================================= */

/// Cmdlet (lower case) → (IronShell target, kind). Only cmdlets whose target
/// takes the same positional arguments are listed; `-Parameters` are refused
/// rather than passed on, and everything else is handed to `sh(...)` and
/// refused by name when `sh` is not allowed. Every target is checked to
/// resolve by `tests/round30_transpile.rs`. (`Write-Host`, `Write-Output` and
/// `echo` are handled separately: they print.)
pub const PS_MAP: &[(&str, &str, Kind)] = &[
    // Navigation
    ("get-location", "pwd", Kind::Value),
    ("pwd", "pwd", Kind::Value),
    ("gl", "pwd", Kind::Value),
    ("set-location", "cd", Kind::Quiet),
    ("cd", "cd", Kind::Quiet),
    ("sl", "cd", Kind::Quiet),
    ("chdir", "cd", Kind::Quiet),
    // Files
    ("get-childitem", "ls", Kind::Value),
    ("dir", "ls", Kind::Value),
    ("ls", "ls", Kind::Value),
    ("gci", "ls", Kind::Value),
    ("get-item", "stat", Kind::Value),
    ("gi", "stat", Kind::Value),
    ("set-content", "file.write", Kind::Quiet),
    ("sc", "file.write", Kind::Quiet),
    ("add-content", "file.append", Kind::Quiet),
    ("ac", "file.append", Kind::Quiet),
    ("copy-item", "cp", Kind::Quiet),
    ("copy", "cp", Kind::Quiet),
    ("cp", "cp", Kind::Quiet),
    ("cpi", "cp", Kind::Quiet),
    ("move-item", "mv", Kind::Quiet),
    ("move", "mv", Kind::Quiet),
    ("mv", "mv", Kind::Quiet),
    ("mi", "mv", Kind::Quiet),
    ("remove-item", "rm", Kind::Quiet),
    ("del", "rm", Kind::Quiet),
    ("rm", "rm", Kind::Quiet),
    ("ri", "rm", Kind::Quiet),
    ("erase", "rm", Kind::Quiet),
    // Processes
    ("get-process", "proc.list", Kind::Value),
    ("ps", "proc.list", Kind::Value),
    ("gps", "proc.list", Kind::Value),
    ("stop-process", "proc.kill", Kind::Quiet),
    ("spps", "proc.kill", Kind::Quiet),
    // Date/time
    ("get-date", "now_iso", Kind::Value),
    // Pipeline operations
    ("measure-object", "len", Kind::Value),
    ("sort-object", "sort", Kind::Value),
    ("get-unique", "arr.unique", Kind::Value),
    // JSON
    ("convertto-json", "json.stringify", Kind::Value),
    ("convertfrom-json", "json.parse", Kind::Value),
    // Web
    ("invoke-webrequest", "http.get", Kind::Value),
    ("iwr", "http.get", Kind::Value),
    ("invoke-restmethod", "http.json_get", Kind::Value),
    ("irm", "http.json_get", Kind::Value),
    // Clipboard
    ("get-clipboard", "clip.get", Kind::Value),
    ("set-clipboard", "clip.set", Kind::Quiet),
    // Services
    ("get-service", "svc.list", Kind::Value),
    ("start-service", "svc.start", Kind::Quiet),
    ("stop-service", "svc.stop", Kind::Quiet),
    ("restart-service", "svc.restart", Kind::Quiet),
    // System info
    ("hostname", "hostname", Kind::Value),
    ("get-computerinfo", "sys.info", Kind::Value),
    ("get-culture", "sys.locale", Kind::Value),
    // Network
    ("test-connection", "net.ping", Kind::Value),
    ("resolve-dnsname", "net.dns_lookup", Kind::Value),
    ("get-netadapter", "net.interfaces", Kind::Value),
    ("get-netipaddress", "net.ip", Kind::Value),
    ("get-nettcpconnection", "net.connections", Kind::Value),
    // Help
    ("get-help", "help", Kind::Value),
    // Misc
    ("clear-host", "clear", Kind::Quiet),
    ("cls", "clear", Kind::Quiet),
    ("clear", "clear", Kind::Quiet),
    ("exit", "exit", Kind::Quiet),
    ("start-sleep", "sleep", Kind::Quiet),
    ("sleep", "sleep", Kind::Quiet),
];

fn map_ps_cmdlet(lower: &str) -> Option<(&'static str, Kind)> {
    PS_MAP
        .iter()
        .find(|(n, _, _)| *n == lower)
        .map(|(_, t, k)| (*t, *k))
}

/* =============================================================================
Helper functions
============================================================================= */

/// The IronShell name of a PowerShell variable: a module name (`file`, `user`)
/// cannot be bound, so it gets an `sh_` prefix, everywhere it appears.
fn vname(n: &str) -> String {
    if crate::modules::is_module_name(n) {
        format!("sh_{}", n)
    } else {
        n.to_string()
    }
}

fn parse_ps_var_ref(s: &str) -> Option<String> {
    if s.starts_with("${") {
        if let Some(end) = s.find('}') {
            let name = &s[2..end];
            if is_ps_ident(name) && s[end + 1..].trim().is_empty() {
                return Some(vname(name));
            }
        }
    } else if let Some(name) = s.strip_prefix('$') {
        let name = name.trim();
        if is_ps_ident(name) {
            return Some(vname(name));
        }
    }
    None
}

fn is_ps_ident(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    for ch in chars {
        if !(ch == '_' || ch.is_ascii_alphanumeric()) {
            return false;
        }
    }
    true
}

fn is_ps_number(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let s = s.trim();

    // Handle negative numbers
    let s = if s.starts_with('-') { &s[1..] } else { s };
    if s.is_empty() {
        return false;
    }

    // Integer
    if s.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }

    // Float
    if s.contains('.') {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() == 2 {
            return !parts[0].is_empty()
                && parts[0].chars().all(|c| c.is_ascii_digit())
                && !parts[1].is_empty()
                && parts[1].chars().all(|c| c.is_ascii_digit());
        }
    }

    false
}

fn escape_string(s: &str) -> String {
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

fn json_string_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/* =============================================================================
Tests
============================================================================= */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_assignment() {
        let result = transpile_powershell_to_ae("$x = 42").unwrap();
        assert!(result.contains("let mut x = 42;"));
    }

    #[test]
    fn test_string_assignment() {
        let result = transpile_powershell_to_ae("$name = \"hello\"").unwrap();
        assert!(result.contains("let mut name = \"hello\";"));
    }

    #[test]
    fn test_pipeline() {
        let result =
            transpile_powershell_to_ae("Get-Process | Where-Object { $_.CPU -gt 10 }").unwrap();
        // Script blocks are not translated: the line falls back to sh().
        assert!(
            result.contains("sh(") || result.contains("pwsh"),
            "Expected fallback, got: {}",
            result
        );
    }

    #[test]
    fn test_simple_cmdlet() {
        let result = transpile_powershell_to_ae("Get-ChildItem").unwrap();
        assert!(result.contains("ls()"));
    }

    #[test]
    fn test_get_content_is_reported_not_guessed() {
        let (code, fb) = transpile_powershell_with_report("Get-Content 'file.txt'").unwrap();
        assert!(code.contains("sh("), "got: {}", code);
        assert_eq!(fb.len(), 1);
        assert!(fb[0].construct.contains("Get-Content"), "got {:?}", fb);
    }

    #[test]
    fn test_get_process_maps_to_proc_list() {
        let result = transpile_powershell_to_ae("Get-Process").unwrap();
        assert!(result.contains("proc.list"));
    }

    #[test]
    fn test_invoke_webrequest_maps_to_http_get() {
        let result = transpile_powershell_to_ae("Invoke-WebRequest 'https://example.com'").unwrap();
        assert!(result.contains("http.get"));
    }

    #[test]
    fn test_get_service_maps_to_svc_list() {
        let result = transpile_powershell_to_ae("Get-Service").unwrap();
        assert!(result.contains("svc.list"));
    }

    #[test]
    fn test_hostname_maps_to_hostname() {
        let result = transpile_powershell_to_ae("hostname").unwrap();
        assert!(result.contains("hostname()"));
    }

    #[test]
    fn test_comment_preservation() {
        let result = transpile_powershell_to_ae("# This is a comment").unwrap();
        assert!(result.contains("// This is a comment"));
    }

    #[test]
    fn test_test_connection_maps_to_net_ping() {
        let result = transpile_powershell_to_ae("Test-Connection 'google.com'").unwrap();
        assert!(result.contains("net.ping"));
    }

    #[test]
    fn test_parameters_are_refused_by_name() {
        let e = transpile_powershell_to_ae("Get-ChildItem -Recurse")
            .unwrap_err()
            .to_string();
        assert!(e.contains("-Recurse"), "got: {}", e);
    }

    #[test]
    fn test_sleep_maps() {
        let result = transpile_powershell_to_ae("Start-Sleep -Seconds 5").unwrap();
        assert!(result.contains("sleep(5)"), "got: {}", result);
    }

    #[test]
    fn test_semicolons_split() {
        assert_eq!(
            split_ps_statements("$x = 5; $x + 2"),
            vec!["$x = 5", "$x + 2"]
        );
        assert_eq!(
            split_ps_statements("Write-Output 'a;b'"),
            vec!["Write-Output 'a;b'"]
        );
    }

    #[test]
    fn test_write_output_prints_each_statement() {
        let r = transpile_powershell_to_ae("Write-Output one\nWrite-Output two").unwrap();
        assert!(
            r.contains("print(\"one\");") && r.contains("print(\"two\");"),
            "got: {}",
            r
        );
    }

    #[test]
    fn test_expression_statement() {
        let r = transpile_powershell_to_ae("$x = 5; $x + 2").unwrap();
        assert!(r.contains("print(x + 2);"), "got: {}", r);
    }
}

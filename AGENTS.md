# AGENTS.md - IronShell Agent & AI Discovery

> This file helps AI coding assistants (GitHub Copilot, Claude, ChatGPT, Cursor, Windsurf, Cody, local models) understand how to work with IronShell.

## Identity

- **Project**: IronShell
- **Tagline**: One language, every platform, deterministic typed output
- **Binary**: `ish` (`ae`, the pre-14.0.0 name, is installed as an alias)
- **Language**: Rust
- **Version**: 14.0.5
- **Repository**: https://github.com/nervosys/IronShell
- **License**: AGPL-3.0-or-later
- **File Extension**: `.ish`

## Why IronShell Exists

AI agents today are forced to interact with shells designed decades ago for humans — Bash on Linux, PowerShell on Windows, Zsh on macOS. Each has its own syntax, conventions, and quirks. Worse, their output is unstructured text that varies across OS versions, locales, and tool installations, creating non-deterministic results that make agent workflows fundamentally brittle. An agent that parses `df -h` output on Ubuntu will break on Alpine. A script that scrapes `Get-Process` output will fail when column widths change. Every shell command becomes a fragile integration point.

## What Is IronShell?

IronShell eliminates this entire class of failure. It provides a single cross-platform shell language with deterministic, typed output — every command returns structured data (Int, Float, String, Array, Record, Lambda), not raw text. An ontology built into the shell makes commands, their arguments, and their return types discoverable by AI agents without documentation scraping or prompt engineering. It has 1,200+ builtins (as its ontology lists them; 1,400+ callable names counting aliases) in 108 modules, native AI agents with multi-modal support, and implements the MCP, A2A, A2UI, and NANDA agentic protocols.

## How To Use IronShell (for AI agents)

### Option 1: Generate IronShell Code

Write `.ish` scripts or REPL expressions:

```ish
# Typed pipeline
ls("./src") | where(fn(f) => f.size > 1024) | map(fn(f) => f.name)

# AI query
code = file.read("main.rs")
ai("Explain this code: ${code}")

# File editing (safe, cross-platform)
file.replace("config.rs", "DEBUG = false", "DEBUG = true")
```

### Option 2: Call the Agent API (HTTP)

Start: `ish agent serve` (port 3002)

```bash
# Call a builtin
curl -X POST http://localhost:3002/api/v1/execute \
  -H "Content-Type: application/json" \
  -d '{"action": "call", "builtin": "ls", "args": {"path": "."}}'

# Get schema for your AI provider
curl http://localhost:3002/api/v1/schema/openai
```

The body of `POST /api/v1/call/:builtin` is the arguments object itself (e.g. `{"path": "."}`), not `{"args": ...}`: that wrapper is the `/execute` request shape (a lone `args` key is unwrapped for you). `/builtins/:name` and `/call/:builtin` also take a module-qualified name (`file.read`).

### Option 3: Python SDK + LangChain

```python
from ironshell import IronShellRuntime
from ironshell.langchain import get_agent_api_tools

runtime = IronShellRuntime()
result = runtime.eval('sys.hostname()')

# LangChain tools (connect to Agent API server)
tools = get_agent_api_tools("http://localhost:3002")
```

### Option 4: Run Existing Shell Scripts

`ish -b` (and `.sh` files) run the ordinary POSIX subset **in-process**
(`src/posix.rs`): pipelines, `&&`/`||`, variables, `$(…)`, redirections,
`if`/`for`/`while`, `test`, and GNU-compatible `ls`/`find`/`grep`/`head`/`tail`/
`wc`/`sort`/`uniq`/`cut`/`sed`/`cat`/`tee`/`tr`/`mkdir`/`touch`/`rm`/`rmdir`/`echo`/`printf` (the common GNU flags, not all: `ls` takes `-a` but not `-l`;
`sort -k`/`-t`, `cat -n` and printf `%x` and `%.Nf` run in-process too). `tee` takes `-a`, and each file it names
is judged as a `>` redirection is, so `tee ../f` is refused and writes nothing; so is each operand of `mkdir [-p]` and `touch`. `sed -i[SUFFIX]` edits each file in place under the same jail and journal (a backup FILE+SUFFIX first). That refusal is the workspace jail's, so it applies in agent mode (`--agent`) or when a workspace is set (`--workspace`, or `IRONSHELL_WORKSPACE`); in a plain human session with no workspace, `ish -b` writes where bash would. `tr` takes `-d`, `-s`, ranges and `[:upper:]`-style classes. `rm` takes `-r`/`-R`, `-f` and `--`, and `rmdir` takes only operands; both run on the code of the `rm()` and `rmdir()` builtins, so the workspace jail and the journal judge them, a link (a symlink, or a junction on Windows) is removed itself and never followed, a missing file is `rm: cannot remove 'x': No such file or directory` (silent with `-f`), and a name with a trailing `/`, or ending in `.` or `..`, is refused as `rm()` refuses it (write `rm -r d`, not `rm -r d/`). Any other program a script runs (`cp`, `mv`, `xargs`, or an `rm` flag the subset lacks, such as `-i`) is an external program, and the workspace jail does not confine it outside agent mode: in agent mode each one goes through the approval gate, and in a human session with a workspace set only the in-process tools above are confined. On Windows, an unsupported flag
runs a GNU-compatible program if one is on PATH, and otherwise exits 2 naming the flag (elsewhere it runs the
PATH program of that name). Other commands run as the
real program, directly, through the effect gate — never via `bash -c` — so
agent mode gates each one. On E6 (`benches/agentic/bashcompat.mjs`), 32 of 32
matched bash byte-for-byte, and 32 of 32 on a held-out corpus.

Also in the subset: `grep -r` takes `--include=GLOB`, `--exclude=GLOB` and `--exclude-dir=GLOB` (also written `--include GLOB`; a glob is matched against a file's or directory's base name, and a file or directory named on the command line is judged by its name too); `printf` takes the C flags `-` `+` space `0` `#` with `d i u x X o f e g` and a precision on an integer (`%+d`, `%05d`, `%#x`, `%+.2f`); `${x:offset}` and `${x:offset:length}` (a negative offset is written `${x: -2}` or `${x:(-2)}`; a negative length stops that many characters before the end; offset and length are sums of integers and variable names; counted in characters, as bash does in a UTF-8 locale, where the C locale counts bytes; a negative length that ends before the offset is bash's "substring expression < 0" error, which here also ends the script); here-strings `cmd <<< word` (the word expands as a double-quoted word and gets a newline; the last one on a command wins); and `/dev/null` as input (an empty file) in `<` and as an operand of `cat`/`grep`/`wc`/`head`/`tail`/`sort`/`cut`/`uniq` on every platform. An argument that starts with `-` and is no option of `tr`, `grep`, `sort` or `cut` fails as GNU does (`tr: invalid option -- '_'`, status 1 for `tr` and `cut`, 2 for `grep` and `sort`), so `tr '-_' ' '` is an error and `tr -- '-_' ' '` is not. When an option is real but the in-process tool lacks it, the refusal names the tool first and the flag it does not support (`ish: tr: the in-process tr does not support -c`), then says a GNU-compatible one on PATH would run it. Also in the subset: the `${...}` operators `${#x}`, `${x-w}`, `${x:-w}`, `${x=w}`, `${x:=w}`, `${x+w}`, `${x:+w}`, `${x?w}`, `${x:?w}`, and the glob trims `${x#p}`, `${x##p}`, `${x%p}`, `${x%%p}`; `$'...'` (ANSI-C quoting: `\n \t \r \a \b \e \f \v \\ \' \" \NNN \xHH`) and `cd -` (prints the directory it goes to). Still refused: here-documents (`<<EOF`), brace expansion (`{a,b}`, `{1..3}`; `{}` and `{a}` stay literal), and the other `${...}` operators (`${x/a/b}`, `${x^^}`, `${x,,}`, `${!x}`, arrays).

`set -e` (errexit), `set -u` (nounset) and `set -o pipefail` run in-process too, with bash's
exemptions (conditions, `&&`/`||` operands, `!` pipelines) and `+` forms to turn them off. `set -x`
and other `set` options are refused before anything runs, naming the option.

In the subset, `uniq` drops only adjacent duplicates, as Unix `uniq` does
(`sort | uniq` drops all of them); the `arr.unique` builtin drops all duplicates.

A script outside the subset (functions, `case`, `$((…))`, here-documents, subshells,
background jobs) is refused before any of it runs, naming the construct and its line, unless
`IRONSHELL_ALLOW_SH` is set. With it, the unsupported parts fall back to the transpiler, which may
delegate them to `bash -lc`; that fallback is not contained.
Its own refusals exit 2 for a syntax error (`E_PARSE`, which runs nothing in either mode) and for a flag no GNU-compatible program can run on Windows, 77 for a construct outside the subset (refused before anything runs), 1 for a write or read the workspace jail refuses (that command fails and the script goes on unless `set -e` stops it), and 75 for a program that needs approval, whose command fails the same way.
Zsh input runs the same engine after a small rewrite of zsh-only words (`setopt`/`unsetopt` of history, prompt, completion and beep options are no-ops, `errexit`/`nounset`/`pipefail` map to `set`, `nullglob` is accepted only in a script with no glob (a glob after it on the same line counts); any other option is refused by name; `print -l/-n/-r` become `printf`/`echo`; `echo` interprets backslash escapes as zsh does), so `;`, `&&`/`||`, flags, `grep`/`wc`/`sort`/`tr`/`head`/`find`/`date`, `cd` and globs behave as under `ish -b`. Differences from zsh that remain: an unquoted `$var` word-splits as in POSIX sh, and an unmatched glob stays literal instead of failing. A zsh-only construct (arrays, `[[ ]]`, `${(f)x}`, functions) is refused before anything runs, naming the construct and its line. The one thing zsh does that the engine does not is `**`: a lone `ish --zsh -c 'ls **/*.txt'` goes through the transpiler, and `**` mixed with other commands is refused. PowerShell input goes through the transpiler only.
On Windows, a script run as a command (`./x.sh` whose first line is a `#!/bin/sh` or `#!/bin/bash` shebang, or `sh FILE` / `bash FILE`) runs in a child `ish -b` with the same mode and workspace, and its status becomes the command's status; the child gates its own commands, so the script itself has no Exec gate.

```bash
# Auto-detected by file extension
ish deploy.sh           # Bash → IronShell
ish setup.zsh           # Zsh  → IronShell
ish provision.ps1       # PowerShell → IronShell

# Explicit mode flags
ish -b -c 'ls | grep .rs | wc -l'        # -b (same flag as --bash): in-process POSIX subset; ls takes -a, not -l
ish --zsh  -c 'ls **/*.txt'              # typed records of every .txt, subfolders included (no setopt needed)
ish --pwsh -c 'Get-ChildItem *.rs | Measure-Object'   # PowerShell subset: see note below

# Pipe via stdin
echo 'cat /etc/hosts' | ish --bash
```

The PowerShell transpiler covers a subset only. `;` separates statements; `Write-Host`, `Write-Output` and every value-producing statement print as they run, in order; `$x = 5; $x + 2` prints 7 (`+ - * %` between variables, numbers and strings; no `/` or comparisons). `+` follows PowerShell: the right operand takes the left one's type, so `'a' + 1` is `a1` and `1 + '2'` is 3 (translated to the `ps_add` builtin). Only cmdlets whose IronShell target takes the same positional arguments are mapped (`PS_MAP` in `src/transpile/powershell.rs`, each target checked by `tests/round30_transpile.rs`). A parenthesised argument or statement (`Write-Host (1+2)`) is translated as an expression when it holds only those operators (a unary minus on a number is allowed, `$b = $a + 1` assigns), and is refused otherwise; variables are mutable, so `$x = 5; $x = 6` works; a runtime error names no line, as the translation has no line map back to the PowerShell source. A `-Parameter` of a mapped cmdlet (`-Recurse`, `-Path`), `$env:X`, `$(...)`, here-strings and `Where-Object { ... }` / `ForEach-Object { ... }` make the script fail before anything runs, naming the parameter or construct and its line; an unmapped command (`Get-Content`), block or operator is refused the same way unless `IRONSHELL_ALLOW_SH` runs it through `pwsh`. Every such refusal has one shape: exit 77, `E_POLICY_DENY` (JSON under `--agent`), with a hint. A variable named like an IronShell module (`$file`, `$user`) is renamed `sh_file` in the translation.

The zsh and PowerShell transpilers map only commands whose IronShell builtin takes the same arguments; block accumulation (`if`/`for`/`while`/`case`/`function`) hands multi-line constructs to `sh()` and is refused by name unless `IRONSHELL_ALLOW_SH` is set.

### Option 5: Agentic Syntax (legacy; not recommended)

`ish -a` and `.ishg` files accept a compressed syntax (`l"./src"|w~.size>1k|.name`).
It is kept for scripts that use it, and its reference is no longer part of
this file, because measured it costs more than it saves:

- **Tokens:** 27.5% fewer command tokens and 0% fewer output tokens on the E1
  corpus, against a 1,196-token reference; a net loss for sessions under ~129
  queries (`docs/LANGUAGE_FIRST_PRINCIPLES.md` §2). The legible syntax with
  `open` and `.field` is now cheaper than it outright (E1: 331 vs 336 tokens).
- **Correctness:** a Claude Haiku 4.5 run of E7 scored **2/10** with the
  reference in its prompt and **7/10** without it, when it fell back to the
  standard syntax (`benches/agentic/results/e7-haiku-4-5-subagent.md`).

Write the standard syntax below. The reference, for maintaining old scripts,
is `docs/AGENTIC_SYNTAX.md` and `ish agent schema`.

### Reading Output, Config Files and Names (for AI agents)

- **Agent-mode output** (`--agent`, `IRONSHELL_MODE=agent`) is compact AECON text, not JSON: a Record prints as `{k=v k2=v2}` (a string inside is JSON-quoted, nested values are inline), and an Array of Records prints as a TSV table (a String cell is JSON-quoted when bare text would read as another type: empty, `null`/`true`/`false`, a number, opening with `@` `"` `[` or `{`, with leading or trailing whitespace, or holding a tab or newline; ordinary strings stay bare) whose header may be followed by compression directives, `@const` (a column that is the same in every row, stated once), `@prefix` (a shared leading run), `@same` (an empty cell repeats the row above) and `@delta` (a running sum), plus `@dict` and `@suffix`, and `@type col:f` (a column whose Floats are all written without a decimal point reads as Float, so `3` in that column is `3.0`, not an Int). No row of a table prints as an empty line. A value too large to send whole prints as an `@handle` summary (`@handle`, `@shape`, `@items`, `@bytes`, a preview, `@omitted`); narrow it with `handle("h1") | where(...) | take(5)`. A handle is kept in a session directory (keyed by `IRONSHELL_SESSION_DIR`, `IRONSHELL_SESSION`, or the working directory), so a later `ish` process in the same session can read it, but another session cannot, and a session holds at most 4096.
- **Secrets are redacted from agent-mode output.** A value that looks like a secret (an API-key shape, a JWT, a PEM block, URL credentials, `password=...`/`token: ...` text, or the value of a secret-named field) prints as `[REDACTED]`, so `open("conf/app.toml")` or `cat` may show `key = "API_KEY=[REDACTED]"`. That is the shell's scrubbing, not the file's text. When anything was redacted, one line goes to stderr (`ish: 1 secret redacted from the output as [REDACTED]; IRONSHELL_REDACT=off shows raw values`) and stdout is unchanged. The note is a count only; the value is never shown, and the human pretty-printer is not redacted.
- **Approval tokens are permanent grants.** A call that needs approval fails with `E_NEEDS_APPROVAL` (exit 75) and an `apv_...` token. The token is the same every time for the same action on the same target, in any run or session. Once the operator puts it in `IRONSHELL_APPROVE` (comma-separated), it approves that exact action on that exact target every time it is presented, in any later run, until it is removed from the list. It approves nothing else. A recursive delete's `blast_radius` counts the files and directories under the tree (`10000+` when cut off); the counts do not change the token.
- **Parse `--deterministic` output.** `ish --deterministic ...` (or `IRONSHELL_DETERMINISTIC=1`) prints every result as canonical JSON, with sorted keys, no whitespace, a Float that keeps its point (`1.0`, not `1`) and `null` for null. It takes precedence over the AECON form above, is never summarised as a handle, and is the form to parse. An uncaught error under `--deterministic` is JSON on stderr (`{"error":{"code":...}}`), as in agent mode, not the `error[CODE]:` prose a human sees.
- **Config round trips lose comments.** `to_toml`, `to_yaml` and `to_json` write a Record's keys sorted and keep no comments, no flow style and no original key order, so `from_toml(text) | to_toml` on a hand-written file drops its comments and reorders it. Edit config text with `file.replace` when comments matter.
- **Module names are not variable names.** The names of the built-in modules (`file`, `user`, `net`, `sys`, `perm`, `proc`, `http`, ...) are bound to the modules, so `user = 1` or `let file = 2` fails with "is the name of a built-in module and cannot be used as a variable name"; pick another name (`owner`, `path`).

## IronShell Syntax Quick Reference

```
# Variables (inferred types)
x = 42                              # Int
s = "hello"                         # String
a = [1, 2, 3]                       # Array<Int>
r = {name: "ae", version: "1.5.0"} # Record

# Lambdas
double = fn(x) => x * 2
add = fn(a, b) => a + b

# Pipelines
[1,2,3] | map(fn(x) => x * 2) | reduce(fn(a,b) => a + b, 0)
# IronShell has no loop statements (`for`/`while`); iterate with map / where / reduce / each

# `|` binds loosest (as in jq): parenthesize a pipe inside && / || or before ==, e.g. ("Hello" | upper) == "HELLO"
# `x | f(a)[i]` and `x | f(a).field` index or read the result of the pipeline so far, i.e. `(x | f(a))[i]`; `x | f[i]` likewise. Statements are separated by a newline or `;`, so `x = 5 6` is E_PARSE, not two statements. Trailing commas in `[...]` and `{...}` are fine; a duplicate record key is E_PARSE.
open("issues.json") | where(fn(r) => (r.title | lower | contains("x")) || (r.body | lower | contains("x")))

# Rows: `.field` in a pipeline stage's argument is the row (jq's spelling),
# `|.field` projects, `open` parses by extension (JSON, JSONL, TOML, YAML, CSV, TSV)
open("issues.json") | where(.state == "open" && .comments > 5) |.number
open("Cargo.toml").package.version
cat("src/*.rs") | lines | len          # globs in cat/ls; glob("src/**/*.rs"); {a,b} alternation: glob("**/*.{rs,py}")
grep("TODO\\|FIXME", "src/*.rs")       # grep is BRE (as grep without -E): a bare | + ? ( ) { } is literal, so "TODO|FIXME" matches nothing; write \\| or pass {extended: true}; no recursive option, grep a glob

# SQL over values or files (read-only, in-process)
open("issues.json") | sql("select user, count(*) as n from t group by user")
sql_value("issues.json", "select count(*) from issues where state = 'open'")
open("issues.json") | count_by(.user)  # [{key, count}] sorted by count descending; the short form of group_by + sort_by("Count", "desc"). The key may be a field name, `.field`, a lambda or a builtin (`count_by(len)`); 1 and 1.0 are one key; a row without the field counts under the key null (when no row has the field it is E_UNKNOWN_FIELD, and a non-Record row with a field key is E_BAD_ARG). group_by takes the same keys and treats a missing field the same way (the group of an explicit null, Name "<null>"), and merges 1 and 1.0 into one group like count_by. group_by rows carry `Name` (the key's text, "<null>" for null) and `Key` (the key with its type)
# SQL stores a Bool as 1/0, so Bool values come back from sql as 1 and 0; every sql row (not only select *) has its columns in sorted key order, because a Record sorts its keys: to control the order prefix the aliases (a_region, b_rev) or write the table with to_csv(rows, [cols])

# Check effects before running
explain_effects("rm(\"x\")")            # or: ish --explain -c '...'; a decision of "approve" means the call needs approval and would stop with E_NEEDS_APPROVAL (exit 75)

# Pattern matching
match score { s if s >= 90 => "A", s if s >= 80 => "B", _ => "F" }

# Error handling
result = try { risky() } catch e { "fallback" }
try { error("bad", {field: "age"}) } catch e { e.code }   # "E_USER"; error(message[, extra Record]) raises your own
# error.retryable is true when retrying the same call later, or with the arguments corrected as the hint says, can succeed (bad arguments, an unknown name that has a suggestion, not found, needs approval) and false when the same call can never succeed as asked (policy deny, outside the workspace, a value that is undefined or overflows, an unknown name with no suggestion).
# In agent mode `try` does not catch E_NEEDS_APPROVAL, E_OUTSIDE_WORKSPACE or E_POLICY_DENY: it re-raises them and the script stops with their exit status (75/77). Human mode catches them.
# Limits: memory past IRONSHELL_MEMORY_LIMIT_MB (or `--memory-limit MB`; default the smaller of 2048 and half of RAM, 0 = none) and a run past `--timeout SECONDS` (or IRONSHELL_TIMEOUT; no default, 0 = none) stop with E_BUDGET_EXCEEDED, as do strings over 8 MB and Arrays over 10M elements; `sleep` is cut at the deadline

# Script arguments: `ish script.ish a b` gives the script args() == ["a", "b"] (empty under -c)
# A script file echoes the value of its last expression only; earlier values are not printed.
int("42") + 1                  # int(x) / float(x) convert text or numbers; int("1.5") is refused, float("1.5") is 1.5; int("ff", 16) parses a base 2..36 (0x/0b/0o prefix allowed for 16/2/8; int("0x1F") alone is refused), to_base(255, 16) is "ff"; there are no 0xff literals
now_iso()                      # the current time, e.g. 2026-11-08T14:03:05Z; now_iso("local") carries the offset

"abc" | regex_match("b"); regex_find("a1b22", "[0-9]+").start; regex_replace("a1", "([0-9])", "<$1>")    # regex on text; a match is {match, groups, start, end}
"7" | pad_left(3, "0"); str.pad_right("ab", 5)                  # padding counts characters; the fill is one character
date_add("2024-02-28", 1, "days"); date_diff("2024-02-29", "2026-11-09"); date_format("2024-02-29", "%d %B %Y")    # ISO 8601 dates, in UTC
from_toml(to_toml({a: 1})).a           # TOML: to_toml(record) -> String; from_toml(text) -> Record
merge({a: 1, b: 2}, {b: 3}).b         # merge(a, b): shallow Record merge, b's fields win; `a | merge(b)` pipes
[1, 2] + [3]   # [1, 2, 3]: + concatenates Arrays
-7.5 % 2       # -1.5: % takes the sign of the dividend, unlike Python; Int and Float mix as Float
6 / 2          # 3.0: / is true division, so two Ints give a Float; use math.floor(6 / 2) or int(6 / 2) for integer division
!0             # true: `!` follows truthiness (0, 0.0, "", null, [] and {} are falsy; everything else is truthy), but `if` takes only a Bool (or null, which is false) and `&&` / `||` take only Bool operands: `if 0 {...}` and `1 && 2` are errors, so write `x != 0`
let x: Int = 1                         # an annotation is one bare type name (Int, Float, String, Bool, Array, Record, Lambda, Null); an unknown name is E_BAD_ARG, and Array<Int> or String? is E_PARSE; `mut m: Int = 1` is `let mut m: Int = 1`, and an annotated binding keeps its type when assigned again (`m = "x"` is E_BAD_ARG)
["bb", "a"] | sort_by(len)             # a builtin name works where a lambda is expected (map, where, each, any, all, sort_by, group_by): it is called with the element
mut r = {a: 1}
r.a = 2                                # set a field of a `mut` record; r.a.b = v needs r.a to exist already; other copies keep theirs
rows | sql("select * from t where amount > ?", [100])   # a trailing Array binds the ? placeholders
to_csv(rows, ["id", "customer"])       # column order (Records sort their keys); csv_columns("orders.csv") is the header in file order
from_csv("v\n1\n2.5\n")          # a CSV column's type is inferred from ALL its cells: any decimal makes the column Float in every row ([1.0, 2.5]), and a "1.0" written by to_csv reads back as the Float 1.0
date_add("2024-01-31", 1, "months")    # "2024-02-29": weeks, months and years clamp to the month's end
journal().entries                      # the journal rows (journal() is a Record {entries, steps, reversible, irreversible, enabled}: steps is a count, each row in entries carries id and step)
undo({id: 3})                          # undo one journal step out of order (needs the journal: agent mode, or IRONSHELL_JOURNAL=on); refused if a later step changed the same file
format_number(1234567.891, 2, ",")    # "1,234,567.89": exactly `decimals` digits, optional thousands separator
path_join("src", "main.rs")            # "src/main.rs"; path_basename, path_dirname, path_extension, path_stem, path_normalize: text only, always "/"

# String interpolation
msg = "Hello ${name}, you have ${count} items"
greet = 'Hello ${name}'                # single-quoted strings interpolate ${...} too

# Module calls
file.read("path"); sys.hostname(); net.ping("host")
crypto.uuid(); math.sqrt(16); arr.range(10)
```

## Module Directory (108 modules)

| Module       | Purpose        | Example                                               |
| ------------ | -------------- | ----------------------------------------------------- |
| `file`       | File I/O       | `file.read("f.txt")`, `file.write("f.txt", data)`     |
| `sys`        | System info    | `sys.hostname()`, `sys.uptime()`, `sys.cpu_info()`    |
| `proc`       | Processes      | `proc.list()`, `proc.kill(pid)`                       |
| `net`        | Network        | `net.ping("host")`, `net.dns_lookup("host")`          |
| `http`       | HTTP client    | `http.get(url)`, `http.post(url, body)`               |
| `crypto`     | Cryptography   | `crypto.uuid()`, `crypto.hash("sha256", data)`        |
| `db`         | Database       | `db.sqlite_open("app.db")`, `db.sqlite_query("app.db", sql)` |
| `math`       | Mathematics    | `math.sqrt(x)`, `math.pow(a, b)`, `math.pi` (3.14159…), `math.e` (2.71828…) constants, read without a call |
| `str`        | Strings        | `str.upper(s)`, `str.split(s, ",")`                   |
| `arr`        | Arrays         | `arr.range(n)`, `arr.flatten(a)` (one level), `arr.unique(a)` |
| `json`       | JSON           | `json.parse(s)`, `json.stringify(v)`                  |
| `platform`   | Platform       | `platform.arch()`, `platform.cpu()`, `platform.gpus()` |
| `ai`         | AI queries     | `ai("prompt")`, `ai("prompt", {images: ["photo.jpg"]})`          |
| `agent`      | Agents         | `agent("goal", tools)`, `swarm({...})`                |
| `mcp`        | MCP protocol   | `mcp.tools()`, `mcp.call("tool", args)`               |
| `a2a`        | Agent-to-Agent | `a2a.send("agent", msg)`                              |
| `a2ui`       | Agent-to-UI    | `a2ui.notify("msg", "type")`                          |
| `nanda`      | Consensus      | `nanda.propose("id", config)`                         |
| `rbac`       | Access control | `rbac.create("role", perms)`                          |
| `audit`      | Audit logging  | `audit.log("action", "target", meta)`                 |
| `sso`        | Single sign-on | `sso.init("provider", config)`                        |
| `nn`         | Neural nets    | `nn.create("name", layers)`                           |
| `evo`        | Evolution      | `evo.population(n, type, config)`                     |
| `rl`         | Reinforcement  | `rl.agent("type", states, actions, config)`           |
| `fs`         | Filesystem     | Filesystem operations                                 |
| `gui`        | GUI            | GUI operations                                        |
| `web`        | Web            | Web operations                                        |
| `svc`        | Services       | Service management                                    |
| `cron`       | Scheduling     | Cron/schedule operations                              |
| `archive`    | Archives       | Compression/extraction                                |
| `user`       | Users          | User management                                       |
| `perm`       | Permissions    | Permission operations                                 |
| `pkg`        | Packages       | Package management                                    |
| `hw`         | Hardware       | Hardware access                                       |
| `clip`       | Clipboard      | Clipboard operations                                  |
| `input`      | Input          | User input                                            |
| `shell`      | Shell          | Shell operations                                      |
| `cluster`    | Cluster        | Distributed computing                                 |
| `docker`     | Docker         | `docker.ps()`, `docker.run(image, args)`              |
| `podman`     | Podman         | `podman.ps()`, `podman.run(image, args)`              |
| `k8s`        | Kubernetes     | `k8s.pods()`, `k8s.apply(manifest)`                   |
| `helm`       | Helm           | `helm.list()`, `helm.install(chart)`                  |
| `terraform`  | Terraform      | `terraform.plan()`, `terraform.apply()`               |
| `ansible`    | Ansible        | `ansible.playbook(file)`                              |
| `tmux`       | Terminal mux   | `tmux.new(name)`, `tmux.list()`                       |
| `screen`     | Screen         | `screen.new(name)`, `screen.list()`                   |
| `valgrind`   | Memory debug   | `valgrind.run(prog)`, `valgrind.memcheck(prog)`       |
| `gdb`        | Debugger       | `gdb.run(prog)`, `gdb.bt(prog)`                       |
| `objdump`    | Binary inspect | `objdump.disasm(file)`, `objdump.headers(file)`       |
| `readelf`    | ELF inspect    | `readelf.headers(file)`, `readelf.sections(file)`     |
| `zoxide`     | Smart cd       | `zoxide.add(path)`, `zoxide.query(term)`              |
| `just`       | Task runner    | `just.run(recipe)`, `just.list()`                     |
| `direnv`     | Dir envs       | `direnv.allow()`, `direnv.status()`                   |
| `asdf`       | Version mgr    | `asdf.install(plugin, ver)`, `asdf.list(plugin)`      |
| `mise`       | Dev tools      | `mise.use_version(tool, ver)`, `mise.list()`          |
| `uv`         | Python pkg     | `uv.pip("install", pkg)`, `uv.run(script)`            |
| `pipx`       | Python apps    | `pipx.install(pkg)`, `pipx.list()`                    |
| `poetry`     | Python deps    | `poetry.install()`, `poetry.run(cmd)`                 |
| `cargo`      | Rust pkg       | `cargo.build()`, `cargo.test()`, `cargo.run()`        |
| `rustup`     | Rust toolchain | `rustup.update()`, `rustup.show()`                    |
| `go`         | Go toolchain   | `go.build()`, `go.test()`, `go.run(file)`             |
| `node`       | Node.js        | `node.run(script)`                                    |
| `npm`        | npm            | `npm.install(pkg)`, `npm.run(script)`                 |
| `pnpm`       | pnpm           | `pnpm.install(pkg)`, `pnpm.run(script)`               |
| `yarn`       | Yarn           | `yarn.install()`, `yarn.run(script)`                  |
| `bun`        | Bun            | `bun.run(script)`, `bun.install(pkg)`                 |
| `deno`       | Deno           | `deno.run(script)`, `deno.task(name)`                 |
| `gh`         | GitHub CLI     | `gh.pr()` (lists), `gh.issue("create", title)`        |
| `glab`       | GitLab CLI     | `glab.mr()` (lists), `glab.issue("create", title)`    |
| `pre_commit` | Git hooks      | `pre_commit.run()`, `pre_commit.install()`            |
| `buildah`    | OCI build      | `buildah.build(file)`, `buildah.images()`             |
| `skopeo`     | Container img  | `skopeo.inspect(img)`, `skopeo.copy(src, dst)`        |
| `trivy`      | Security scan  | `trivy.scan(target)`, `trivy.image(img)`              |
| `ruff`       | Python lint    | `ruff.check(path)`, `ruff.format(path)`               |
| `iperf3`     | Network perf   | `iperf3.client(host)`, `iperf3.server()`              |
| `nc`         | Netcat         | `nc.connect(host, port)`, `nc.listen(port)`           |

## Agent API Server Endpoints

| Method | Path                     | Description               |
| ------ | ------------------------ | ------------------------- |
| `POST` | `/api/v1/execute`        | Execute any AgentRequest  |
| `POST` | `/api/v1/call/:builtin`  | Call a single builtin     |
| `POST` | `/api/v1/pipeline`       | Execute a pipeline        |
| `POST` | `/api/v1/eval`           | Evaluate IronShell code |
| `POST` | `/api/v1/stream/execute` | Stream execution (SSE)    |
| `GET`  | `/api/v1/ws`             | WebSocket connection      |
| `GET`  | `/api/v1/schema`         | Full JSON schema          |
| `GET`  | `/api/v1/schema/:format` | Provider-specific schema  |
| `GET`  | `/api/v1/builtins`       | List builtins             |
| `GET`  | `/api/v1/builtins/:name` | Describe a builtin        |
| `GET`  | `/api/v1/types`          | Type information          |
| `GET`  | `/health`                | Health check              |

Schema formats: `openai`, `claude`, `gemini`, `llama`, `mistral`, `cohere`, `grok`, `deepseek`, `bedrock`, `azure_openai`, `qwen`, `ollama`, `lmstudio`, `vllm`, `huggingface`, `openrouter`, `together`, `groq`, `fireworks`, `ontology`

## Development

```bash
cargo build --bins          # Build ish + aimodel
cargo test --workspace      # Run workspace tests
cargo run -- tui          # TUI mode
cargo run -- -c 'expr'      # Quick eval
```

### Key Source Files

| File               | Purpose                                            |
| ------------------ | -------------------------------------------------- |
| `src/main.rs`      | CLI entry point                                    |
| `src/ast.rs`       | AST definitions                                    |
| `src/parser.rs`    | Parser                                             |
| `src/eval.rs`      | Evaluator                                          |
| `src/value.rs`     | Value types                                        |
| `src/builtins.rs`  | 1,200+ builtins                                    |
| `src/ai.rs`        | AI provider router                                 |
| `src/agent.rs`     | Agent framework                                    |
| `src/agent_api.rs` | Agent API + HTTP server                            |
| `src/typecheck.rs` | Hindley-Milner inference                           |
| `src/transpile/`   | Shell transpilers (Bash, Zsh, PowerShell, Agentic) |
| `src/tui/`         | Terminal UI                                        |

## Context Files

| File                              | Purpose                              |
| --------------------------------- | ------------------------------------ |
| `llms.txt`                        | Short AI context (llms.txt standard) |
| `llms-full.txt`                   | Complete AI context                  |
| `AGENTS.md`                       | This file - agent discovery          |
| `.github/copilot-instructions.md` | GitHub Copilot instructions          |
| `.well-known/ai-plugin.json`      | OpenAI plugin manifest               |
| `.well-known/openapi.yaml`        | OpenAPI 3.1 spec for Agent API       |
| `docs/specs/SPEC.md`              | Language specification               |
| `ROADMAP.md`                      | Development roadmap                  |

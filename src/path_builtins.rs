//! The `path` builtins: text-only path manipulation.
//!
//! `path_join`, `path_basename` and the rest are also the `path` module
//! (`path.join("a", "b")`, `path.basename(p)`). None of them touches the
//! filesystem, so an answer does not depend on whether the path exists; the
//! reads that do are `path_exists` and `fs_realpath`.
//!
//! Both `/` and `\` separate components on every platform, so `C:\a\b.txt`
//! gives the same answer on Linux, macOS and Windows. Every result uses `/`,
//! on every platform. That departs from Python's `os.path`, which follows the
//! host's separator: an agent's answer should not change with the machine it
//! runs on. A drive is kept as a root (`C:/`). A UNC prefix is not special, so
//! `\\server\share\x` is `/server/share/x`.

use crate::value::Value;
use anyhow::Result;

/// Whether `c` separates two components, on every platform.
fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

/// The root a path starts with, as text, and the rest of the path after it.
/// The root is `/` for an absolute path, `C:/` for a drive-absolute one, and
/// empty for a relative one.
fn split_root(path: &str) -> (String, &str) {
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && is_sep(bytes[2] as char)
    {
        return (format!("{}:/", bytes[0] as char), &path[3..]);
    }
    if path.starts_with(is_sep) {
        return ("/".to_string(), path);
    }
    (String::new(), path)
}

/// The components of a path below its root. Empty parts and `.` are dropped,
/// so `a//./b/` has the components `a` and `b`. A `..` is kept.
fn components(rest: &str) -> Vec<&str> {
    rest.split(is_sep)
        .filter(|part| !part.is_empty() && *part != ".")
        .collect()
}

/// The last component of a path, or empty when it has none (`/`, `.`, "").
fn name_of(path: &str) -> &str {
    components(split_root(path).1).last().copied().unwrap_or("")
}

/// A name's stem and suffix, as pathlib splits them. A dot at the start of a
/// name (`.bashrc`) or at its end (`a.`) does not begin a suffix.
fn split_suffix(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// The parent of a path. A relative path with one component has the parent
/// `.`, and the parent of a root is the root.
fn dirname_of(path: &str) -> String {
    let (root, rest) = split_root(path);
    let parts = components(rest);
    let kept = &parts[..parts.len().saturating_sub(1)];
    if kept.is_empty() {
        if root.is_empty() {
            ".".to_string()
        } else {
            root
        }
    } else {
        format!("{root}{}", kept.join("/"))
    }
}

/// The path with `.`, repeated separators and `name/..` pairs removed. It never
/// reads the disk, so a `..` after a symlink is resolved as text. A relative
/// path keeps the `..` it cannot resolve; an absolute path drops them.
fn normalize_text(path: &str) -> String {
    let (root, rest) = split_root(path);
    let mut out: Vec<&str> = Vec::new();
    for part in components(rest) {
        if part != ".." {
            out.push(part);
        } else if out.last().is_some_and(|top| *top != "..") {
            out.pop();
        } else if root.is_empty() {
            out.push("..");
        }
    }
    let body = out.join("/");
    if root.is_empty() && body.is_empty() {
        ".".to_string()
    } else {
        format!("{root}{body}")
    }
}

/// Join path parts as `os.path.join` does: a part with a root replaces every
/// part before it, and any other part follows a `/` unless the path so far
/// already ends with one.
fn join_text(parts: &[String]) -> String {
    let mut out = String::new();
    for part in parts {
        let part = part.replace('\\', "/");
        if !split_root(&part).0.is_empty() {
            out = part;
        } else if out.is_empty() || out.ends_with('/') {
            out.push_str(&part);
        } else {
            out.push('/');
            out.push_str(&part);
        }
    }
    out
}

/// The components of a path, with its root first when it has one.
fn split_text(path: &str) -> Vec<String> {
    let (root, rest) = split_root(path);
    let mut out = Vec::new();
    if !root.is_empty() {
        out.push(root);
    }
    out.extend(components(rest).into_iter().map(str::to_string));
    out
}

fn text_of(builtin: &str, value: &Value) -> Result<String> {
    match value {
        Value::Str(s) | Value::Uri(s) => Ok(s.clone()),
        other => Err(crate::safety::bad_arg(
            builtin,
            "a path String",
            other.type_name(),
        )),
    }
}

/// The one path a unary builtin takes, passed or piped.
fn one_path(builtin: &str, args: Vec<Value>, input: Option<Value>) -> Result<String> {
    let values: Vec<Value> = input.into_iter().chain(args).collect();
    match values.as_slice() {
        [value] => text_of(builtin, value),
        _ => Err(crate::safety::bad_arg(
            builtin,
            "one path String, passed or piped",
            &format!("{} values", values.len()),
        )),
    }
}

/// The parts `path_join` takes: passed or piped, or one Array of them.
fn join_parts(builtin: &str, args: Vec<Value>, input: Option<Value>) -> Result<Vec<String>> {
    let mut values: Vec<Value> = input.into_iter().chain(args).collect();
    let expanded = match values.as_slice() {
        [Value::Array(items)] => Some(items.clone()),
        _ => None,
    };
    if let Some(items) = expanded {
        values = items;
    }
    if values.is_empty() {
        return Err(crate::safety::bad_arg(
            builtin,
            "at least one path String",
            "no paths",
        ));
    }
    values.iter().map(|v| text_of(builtin, v)).collect()
}

/// path_join(part, ...) - Join path parts with `/`, as os.path.join does.
/// Args:
///   - part: String, or an Array of Strings. Any number of parts.
/// Returns: String, with `/` separators on every platform
///
/// A part that has a root (`/etc`, `C:\x`) replaces the parts before it.
/// Backslashes in the parts become `/`; the parts are not otherwise changed.
///
/// Example:
///   path_join("a", "b")                # "a/b"
///   path_join("a", "/etc", "b")        # "/etc/b"
///   path_join(["data", "c.txt"])       # "data/c.txt"
pub fn bi_path_join(args: Vec<Value>, input: Option<Value>) -> Result<Value> {
    const B: &str = "path_join";
    let parts = join_parts(B, args, input)?;
    Ok(Value::Str(join_text(&parts)))
}

/// path_basename(path) - The last component of a path.
/// Args:
///   - path: String (or the piped String)
/// Returns: String, "" when the path has no component (`/`, "")
///
/// A trailing separator is ignored, as pathlib's `name` ignores it: `a/b/` is
/// `b`. `..` is returned as it is.
///
/// Example:
///   path_basename("a/b/c.txt")         # "c.txt"
///   path_basename("C:\\a\\b.txt")      # "b.txt"
///   path_basename("a/b/")              # "b"
pub fn bi_path_basename(args: Vec<Value>, input: Option<Value>) -> Result<Value> {
    const B: &str = "path_basename";
    let path = one_path(B, args, input)?;
    Ok(Value::Str(name_of(&path).to_string()))
}

/// path_dirname(path) - Everything above the last component of a path.
/// Args:
///   - path: String (or the piped String)
/// Returns: String
///
/// A relative path with one component has the directory `.`, so `c.txt` gives
/// `.`, not "". The parent of a root is the root: `/` gives `/`.
///
/// Example:
///   path_dirname("a/b/c.txt")          # "a/b"
///   path_dirname("c.txt")              # "."
///   path_dirname("/a")                 # "/"
pub fn bi_path_dirname(args: Vec<Value>, input: Option<Value>) -> Result<Value> {
    const B: &str = "path_dirname";
    let path = one_path(B, args, input)?;
    Ok(Value::Str(dirname_of(&path)))
}

/// path_extension(path) - The suffix of the last component, with its dot.
/// Args:
///   - path: String (or the piped String)
/// Returns: String, "" when there is none
///
/// Only the last dot counts: `archive.tar.gz` is `.gz`. A dot at the start of
/// the name (`.bashrc`) or at its end (`a.`) is not a suffix.
///
/// Example:
///   path_extension("archive.tar.gz")   # ".gz"
///   path_extension(".bashrc")          # ""
pub fn bi_path_extension(args: Vec<Value>, input: Option<Value>) -> Result<Value> {
    const B: &str = "path_extension";
    let path = one_path(B, args, input)?;
    Ok(Value::Str(split_suffix(name_of(&path)).1.to_string()))
}

/// path_stem(path) - The last component without its suffix.
/// Args:
///   - path: String (or the piped String)
/// Returns: String
///
/// Example:
///   path_stem("c.txt")                 # "c"
///   path_stem("archive.tar.gz")        # "archive.tar"
///   path_stem(".bashrc")               # ".bashrc"
pub fn bi_path_stem(args: Vec<Value>, input: Option<Value>) -> Result<Value> {
    const B: &str = "path_stem";
    let path = one_path(B, args, input)?;
    Ok(Value::Str(split_suffix(name_of(&path)).0.to_string()))
}

/// path_normalize(path) - Remove `.`, repeated separators and `name/..` pairs.
/// Args:
///   - path: String (or the piped String)
/// Returns: String, with `/` separators
///
/// Purely lexical: the disk is never read, so a `..` is resolved as text, which
/// differs from the OS when a component is a symlink. A relative path keeps
/// the leading `..` it cannot resolve; an absolute path drops a `..` above
/// its root. An empty result is `.`.
///
/// Example:
///   path_normalize("a/./b/../c//d/")   # "a/c/d"
///   path_normalize("../x/../../y")     # "../../y"
///   path_normalize("/../a")            # "/a"
pub fn bi_path_normalize(args: Vec<Value>, input: Option<Value>) -> Result<Value> {
    const B: &str = "path_normalize";
    let path = one_path(B, args, input)?;
    Ok(Value::Str(normalize_text(&path)))
}

/// path_is_absolute(path) - Whether the path has a root.
/// Args:
///   - path: String (or the piped String)
/// Returns: Bool
///
/// A path is absolute when it starts with a separator (`/`, `\`) or with a
/// drive and a separator (`C:\`, `C:/`). Nothing is checked on the disk.
///
/// Example:
///   path_is_absolute("/etc")           # true
///   path_is_absolute("C:\\x")          # true
///   path_is_absolute("a/b")            # false
pub fn bi_path_is_absolute(args: Vec<Value>, input: Option<Value>) -> Result<Value> {
    const B: &str = "path_is_absolute";
    let path = one_path(B, args, input)?;
    Ok(Value::Bool(!split_root(&path).0.is_empty()))
}

/// path_split(path) - The components of a path, root first.
/// Args:
///   - path: String (or the piped String)
/// Returns: Array of Strings
///
/// The root, when there is one, is the first element: `/` or `C:/`. Empty and
/// `.` parts are dropped, as `path_normalize` drops them; `..` is kept.
///
/// Example:
///   path_split("a/b/c.txt")            # ["a", "b", "c.txt"]
///   path_split("/a/b")                 # ["/", "a", "b"]
pub fn bi_path_split(args: Vec<Value>, input: Option<Value>) -> Result<Value> {
    const B: &str = "path_split";
    let path = one_path(B, args, input)?;
    Ok(Value::Array(
        split_text(&path).into_iter().map(Value::Str).collect(),
    ))
}

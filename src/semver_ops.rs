//! Semantic Versioning 2.0.0 (<https://semver.org>): parse, precedence, bump.
//!
//! In-process and dependency free. `semver_parse`, `semver_compare`,
//! `semver_sort`, `semver_bump` and `semver_valid` in `builtins.rs` call it.

use std::cmp::Ordering;

/// One prerelease identifier: digits only is numeric, anything else text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ident {
    Num(u64),
    Text(String),
}

impl Ident {
    fn parse(s: &str) -> Result<Ident, String> {
        if s.is_empty() {
            return Err("an empty identifier".to_string());
        }
        if !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(format!("`{s}` holds a character outside [0-9A-Za-z-]"));
        }
        if s.bytes().all(|b| b.is_ascii_digit()) {
            if s.len() > 1 && s.starts_with('0') {
                return Err(format!("the numeric identifier `{s}` has a leading zero"));
            }
            return s
                .parse::<u64>()
                .map(Ident::Num)
                .map_err(|_| format!("the numeric identifier `{s}` is too large"));
        }
        Ok(Ident::Text(s.to_string()))
    }

    pub fn text(&self) -> String {
        match self {
            Ident::Num(n) => n.to_string(),
            Ident::Text(t) => t.clone(),
        }
    }
}

impl Ord for Ident {
    /// Numeric identifiers compare as numbers and rank below text ones; text
    /// compares in ASCII order (semver.org section 11).
    fn cmp(&self, other: &Ident) -> Ordering {
        match (self, other) {
            (Ident::Num(a), Ident::Num(b)) => a.cmp(b),
            (Ident::Num(_), Ident::Text(_)) => Ordering::Less,
            (Ident::Text(_), Ident::Num(_)) => Ordering::Greater,
            (Ident::Text(a), Ident::Text(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Ident) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A parsed version. `build` is kept as text and never compared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Semver {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<Ident>,
    pub build: Vec<String>,
}

fn core_number(label: &str, s: &str) -> Result<u64, String> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("the {label} number `{s}` is not digits"));
    }
    if s.len() > 1 && s.starts_with('0') {
        return Err(format!("the {label} number `{s}` has a leading zero"));
    }
    s.parse::<u64>()
        .map_err(|_| format!("the {label} number `{s}` is too large"))
}

/// Parse `1.2.3`, `1.2.3-rc.1`, `1.2.3+build` or both; a leading `v` and
/// surrounding whitespace are accepted. The error says what is wrong.
pub fn parse(input: &str) -> Result<Semver, String> {
    let s = input.trim();
    let s = s.strip_prefix('v').unwrap_or(s);
    if s.is_empty() {
        return Err("it is empty".to_string());
    }
    let (rest, build) = match s.split_once('+') {
        Some((r, b)) => (r, Some(b)),
        None => (s, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (rest, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return Err(format!(
            "`{core}` needs three dot-separated numbers (major.minor.patch)"
        ));
    }
    let major = core_number("major", parts[0])?;
    let minor = core_number("minor", parts[1])?;
    let patch = core_number("patch", parts[2])?;
    let pre = match pre {
        Some(p) => p.split('.').map(Ident::parse).collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    let build = match build {
        Some(b) => b
            .split('.')
            .map(|id| {
                if !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
                    Ok(id.to_string())
                } else {
                    Err(format!(
                        "the build identifier `{id}` is empty or has a character outside [0-9A-Za-z-]"
                    ))
                }
            })
            .collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    Ok(Semver {
        major,
        minor,
        patch,
        pre,
        build,
    })
}

/// Precedence by semver 2.0.0 section 11. Build metadata is ignored, so two
/// versions that differ only in it are equal.
pub fn compare(a: &Semver, b: &Semver) -> Ordering {
    a.major
        .cmp(&b.major)
        .then(a.minor.cmp(&b.minor))
        .then(a.patch.cmp(&b.patch))
        .then_with(|| match (a.pre.is_empty(), b.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            // A version with a prerelease is lower than the same without.
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            // Identifier by identifier; with all equal, the longer list is higher.
            (false, false) => a.pre.cmp(&b.pre),
        })
}

impl Semver {
    /// The version as text, without a leading `v`; build metadata is kept.
    pub fn render(&self) -> String {
        let mut s = format!("{}.{}.{}", self.major, self.minor, self.patch);
        if !self.pre.is_empty() {
            let pre: Vec<String> = self.pre.iter().map(Ident::text).collect();
            s.push('-');
            s.push_str(&pre.join("."));
        }
        if !self.build.is_empty() {
            s.push('+');
            s.push_str(&self.build.join("."));
        }
        s
    }

    /// The next version at `level` (`major`, `minor`, `patch` or `prerelease`),
    /// with npm's rules: a prerelease of the next major, minor or patch is
    /// released by bumping that level (`2.0.0-rc.1` major is `2.0.0`); a
    /// prerelease bump of a release raises the patch and starts the
    /// prerelease at `.0`, of a prerelease of the same `id` raises its last
    /// number, and of another `id` restarts at `id.0`. Build metadata is dropped.
    pub fn bump(&self, level: &str, id: Option<&str>) -> Result<Semver, String> {
        let mut v = Semver {
            build: Vec::new(),
            ..self.clone()
        };
        let has_pre = !v.pre.is_empty();
        match level {
            "major" => {
                if !(has_pre && v.minor == 0 && v.patch == 0) {
                    v.major += 1;
                }
                v.minor = 0;
                v.patch = 0;
                v.pre.clear();
            }
            "minor" => {
                if !(has_pre && v.patch == 0) {
                    v.minor += 1;
                }
                v.patch = 0;
                v.pre.clear();
            }
            "patch" => {
                if !has_pre {
                    v.patch += 1;
                }
                v.pre.clear();
            }
            "prerelease" => {
                let id_idents: Vec<Ident> = match id {
                    Some(i) if !i.is_empty() => {
                        i.split('.').map(Ident::parse).collect::<Result<_, _>>()?
                    }
                    _ => Vec::new(),
                };
                if !has_pre {
                    v.patch += 1;
                    v.pre = id_idents;
                    v.pre.push(Ident::Num(0));
                } else if id_idents.is_empty() || v.pre.starts_with(&id_idents) {
                    // Same line of prereleases: raise the last number, or add .0.
                    match v.pre.iter().rposition(|i| matches!(i, Ident::Num(_))) {
                        Some(pos) => {
                            if let Ident::Num(n) = &mut v.pre[pos] {
                                *n += 1;
                            }
                        }
                        None => v.pre.push(Ident::Num(0)),
                    }
                } else {
                    v.pre = id_idents;
                    v.pre.push(Ident::Num(0));
                }
            }
            other => {
                return Err(format!(
                    "`{other}` is not major, minor, patch or prerelease"
                ))
            }
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Semver {
        parse(s).unwrap()
    }

    #[test]
    fn precedence_chain_of_semver_org_section_11() {
        let chain = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
            "2.0.0",
            "2.1.0",
            "2.1.1",
        ];
        for w in chain.windows(2) {
            assert_eq!(compare(&p(w[0]), &p(w[1])), Ordering::Less, "{w:?}");
        }
    }

    #[test]
    fn build_is_ignored_and_bad_input_refused() {
        assert_eq!(compare(&p("1.0.0+a"), &p("1.0.0+b")), Ordering::Equal);
        for bad in [
            "1.0",
            "01.0.0",
            "1.0.0-",
            "1.0.0-01",
            "1.0.0-a..b",
            "a.b.c",
            "",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn bumps() {
        let b = |v: &str, l: &str, id: Option<&str>| p(v).bump(l, id).unwrap().render();
        assert_eq!(b("1.2.3", "patch", None), "1.2.4");
        assert_eq!(b("1.2.3", "minor", None), "1.3.0");
        assert_eq!(b("1.2.3", "major", None), "2.0.0");
        assert_eq!(b("2.0.0-rc.1", "major", None), "2.0.0");
        assert_eq!(b("1.3.0-rc.1", "minor", None), "1.3.0");
        assert_eq!(b("1.2.4-rc.1", "patch", None), "1.2.4");
        assert_eq!(b("1.2.3", "prerelease", Some("rc")), "1.2.4-rc.0");
        assert_eq!(b("1.2.3", "prerelease", None), "1.2.4-0");
        assert_eq!(b("1.2.4-rc.0", "prerelease", Some("rc")), "1.2.4-rc.1");
        assert_eq!(b("1.2.4-rc.1", "prerelease", Some("beta")), "1.2.4-beta.0");
        assert_eq!(b("1.2.4-rc.1+x", "prerelease", None), "1.2.4-rc.2");
    }
}

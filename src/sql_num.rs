//! Strict `sum`, `total` and `avg` for the in-process SQL tables.
//!
//! SQLite converts text to a number by its leading digits, so over a text
//! column `sum` answered 5.0 for "1,234.50", "2.000,10", "1 234", "$5" and
//! "1,234", and 12.0 for "12x": a wrong number with no error. These functions
//! replace the built-in aggregates of the same names on tables built from
//! shell values. A TEXT argument is accepted only when the whole text is a
//! number ("12", " 1.5 ", "1e3"); anything else is an SQL error that names the
//! value. Numbers, NULLs and the arithmetic (integer sums stay integers,
//! floating sums use Kahan-Babuska-Neumaier compensation, as SQLite's do) are
//! unchanged. `count`, `min`, `max` and `group_concat` stay SQLite's.
//!
//! Arithmetic such as `p * 2` is not a function, so it cannot be replaced:
//! SQLite still reads the leading number of a text operand there.

use rusqlite::functions::{Aggregate, Context, FunctionFlags, WindowAggregate};
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, Error, Result};

/// Part of every message this module raises, so the query runner can find it
/// and attach its hint.
pub(crate) const MARKER: &str = "is not a clean number";

#[derive(Clone, Copy)]
enum Kind {
    Sum,
    Total,
    Avg,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Sum => "sum",
            Kind::Total => "total",
            Kind::Avg => "avg",
        }
    }
}

#[derive(Default)]
struct Acc {
    count: i64,
    int_sum: i64,
    approx: bool,
    overflowed: bool,
    sum: f64,
    err: f64,
}

impl Acc {
    fn kbn_init(&mut self, v: f64) {
        self.sum = v;
        self.err = 0.0;
    }

    fn kbn_step(&mut self, r: f64) {
        let s = self.sum;
        let t = s + r;
        if s.abs() > r.abs() {
            self.err += (s - t) + r;
        } else {
            self.err += (r - t) + s;
        }
        self.sum = t;
    }

    fn add_int(&mut self, v: i64) {
        if self.approx {
            self.kbn_step(v as f64);
        } else if let Some(sum) = self.int_sum.checked_add(v) {
            self.int_sum = sum;
        } else {
            self.approx = true;
            self.overflowed = true;
            self.kbn_init(self.int_sum as f64);
            self.kbn_step(v as f64);
        }
    }

    fn add_real(&mut self, v: f64) {
        if !self.approx {
            self.kbn_init(self.int_sum as f64);
            self.approx = true;
        }
        self.kbn_step(v);
    }

    fn real_total(&self) -> f64 {
        if self.approx {
            let t = self.sum + self.err;
            if t.is_nan() {
                self.sum
            } else {
                t
            }
        } else {
            self.int_sum as f64
        }
    }
}

/// A number read from an argument.
enum Num {
    Int(i64),
    Real(f64),
}

fn user_error(message: String) -> Error {
    Error::UserFunctionError(message.into())
}

/// The whole of `text` as a number, or None.
fn parse_clean(text: &str) -> Option<Num> {
    let t = text.trim();
    if t.is_empty() || !t.bytes().all(|b| b"0123456789+-.eE".contains(&b)) {
        return None;
    }
    if let Ok(i) = t.parse::<i64>() {
        return Some(Num::Int(i));
    }
    t.parse::<f64>()
        .ok()
        .filter(|f| f.is_finite())
        .map(Num::Real)
}

/// The argument as a number; None for NULL.
fn read_arg(kind: Kind, ctx: &Context<'_>) -> Result<Option<Num>> {
    match ctx.get_raw(0) {
        ValueRef::Null => Ok(None),
        ValueRef::Integer(i) => Ok(Some(Num::Int(i))),
        ValueRef::Real(f) => Ok(Some(Num::Real(f))),
        ValueRef::Text(bytes) => {
            let text = String::from_utf8_lossy(bytes);
            match parse_clean(&text) {
                Some(n) => Ok(Some(n)),
                None => Err(user_error(format!(
                    "{}() was given {}, which {MARKER}",
                    kind.name(),
                    describe_text(&text)
                ))),
            }
        }
        ValueRef::Blob(_) => Err(user_error(format!(
            "{}() was given a blob, which {MARKER}",
            kind.name()
        ))),
    }
}

/// How an error names the text a strict aggregate refused. A person at a
/// terminal sees the text, cut to 40 characters. In agent mode with redaction
/// on it is not shown: the cell is often a password or token, a bare value has
/// no shape or field name for the redactor to recognise, and the text would
/// reach the agent through the error. The length and the first character's
/// kind say enough to find the cell.
fn describe_text(text: &str) -> String {
    let hide = crate::safety::current_mode() == crate::safety::Mode::Agent
        && crate::safety::redaction_enabled();
    if !hide {
        let shown: String = text.chars().take(40).collect();
        let cut = if text.chars().count() > 40 { "..." } else { "" };
        return format!("the text '{shown}{cut}'");
    }
    let kind = match text.trim().chars().next() {
        None => "empty",
        Some(c) if c.is_alphabetic() => "starting with a letter",
        Some(c) if c.is_ascii_digit() => "starting with a digit",
        Some(_) => "starting with a symbol",
    };
    format!(
        "a text of {} characters ({kind}; the value is not shown in agent mode)",
        text.chars().count()
    )
}

struct Strict(Kind);

impl Strict {
    fn result(&self, acc: Option<&Acc>) -> Result<Value> {
        let Some(acc) = acc.filter(|a| a.count > 0) else {
            return Ok(match self.0 {
                Kind::Total => Value::Real(0.0),
                _ => Value::Null,
            });
        };
        Ok(match self.0 {
            Kind::Sum if acc.overflowed => return Err(user_error("integer overflow".into())),
            Kind::Sum if acc.approx => Value::Real(acc.real_total()),
            Kind::Sum => Value::Integer(acc.int_sum),
            Kind::Total => Value::Real(acc.real_total()),
            Kind::Avg => Value::Real(acc.real_total() / acc.count as f64),
        })
    }
}

impl Aggregate<Acc, Value> for Strict {
    fn init(&self, _: &mut Context<'_>) -> Result<Acc> {
        Ok(Acc::default())
    }

    fn step(&self, ctx: &mut Context<'_>, acc: &mut Acc) -> Result<()> {
        match read_arg(self.0, ctx)? {
            None => {}
            Some(Num::Int(i)) => {
                acc.count += 1;
                acc.add_int(i);
            }
            Some(Num::Real(f)) => {
                acc.count += 1;
                acc.add_real(f);
            }
        }
        Ok(())
    }

    fn finalize(&self, _: &mut Context<'_>, acc: Option<Acc>) -> Result<Value> {
        self.result(acc.as_ref())
    }
}

impl WindowAggregate<Acc, Value> for Strict {
    fn value(&self, acc: Option<&mut Acc>) -> Result<Value> {
        self.result(acc.map(|a| &*a))
    }

    fn inverse(&self, ctx: &mut Context<'_>, acc: &mut Acc) -> Result<()> {
        match read_arg(self.0, ctx)? {
            None => {}
            Some(Num::Int(i)) => {
                acc.count -= 1;
                if acc.approx {
                    acc.kbn_step(-(i as f64));
                } else {
                    acc.int_sum = acc.int_sum.wrapping_sub(i);
                }
            }
            Some(Num::Real(f)) => {
                acc.count -= 1;
                acc.kbn_step(-f);
            }
        }
        Ok(())
    }
}

/// Replace `sum`, `total` and `avg` on `conn` with the strict versions.
pub(crate) fn register(conn: &Connection) -> Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    for kind in [Kind::Sum, Kind::Total, Kind::Avg] {
        conn.create_window_function(kind.name(), 1, flags, Strict(kind))?;
    }
    Ok(())
}

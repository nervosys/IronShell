//! Dates and times, on `jiff`, in the text forms the shell has always written.
//!
//! The shell used to take its dates from `chrono`, which is unmaintained
//! (RUSTSEC-2026-0543). This module is the small part of it the shell used,
//! kept byte-for-byte: the strftime layouts (`%f`, `%6f`, `%.3f`, `%:z`, `%-d`,
//! `%e`, the errors for a specifier it does not read), the RFC 3339 text a
//! serialized timestamp is written as (`Z`, and a fraction of 3, 6 or 9 digits
//! only when there is one), and the layout parser behind `date_parse`.
//! `jiff` supplies the clock, the system time zone and the stored instant.
//!
//! Years run from -262143 to 262142 here, as they did, so the refusals the date
//! builtins make for a year outside 0001 to 9999 are the same ones.

use jiff::{tz::TimeZone, SignedDuration, Timestamp};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{self, Write as _};

// ───────────────────────────── calendar arithmetic ─────────────────────────────

/// The first year of the calendar this module reads and writes.
pub const MIN_YEAR: i32 = -262_143;
/// The last year of the calendar this module reads and writes.
pub const MAX_YEAR: i32 = 262_142;

fn is_leap(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(year) => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(if month <= 2 { year - 1 } else { year });
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from((month + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

/// The number of ISO weeks (52 or 53) of an ISO year.
fn iso_weeks_in_year(year: i32) -> u32 {
    let p = |y: i32| (y + y.div_euclid(4) - y.div_euclid(100) + y.div_euclid(400)).rem_euclid(7);
    if p(year) == 4 || p(year - 1) == 3 {
        53
    } else {
        52
    }
}

/// A date and time with no zone, as a reading of the clock and calendar. The
/// nanosecond may reach 1_999_999_999 for a leap second written `:60`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Naive {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub nano: u32,
}

impl Naive {
    /// The date and time, or None when either is not one.
    pub fn new(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        nano: u32,
    ) -> Option<Naive> {
        if !(MIN_YEAR..=MAX_YEAR).contains(&year)
            || !(1..=12).contains(&month)
            || day < 1
            || day > days_in_month(year, month)
            || hour > 23
            || minute > 59
            || second > 59
            || nano >= 2_000_000_000
            || (nano >= 1_000_000_000 && second != 59)
        {
            return None;
        }
        Some(Naive {
            year,
            month,
            day,
            hour,
            minute,
            second,
            nano,
        })
    }

    /// Midnight of a date, or None when it is not one.
    pub fn date(year: i32, month: u32, day: u32) -> Option<Naive> {
        Naive::new(year, month, day, 0, 0, 0, 0)
    }

    fn from_days_and_time(days: i64, secs_of_day: u32, nano: u32) -> Option<Naive> {
        let (year, month, day) = civil_from_days(days);
        if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
            return None;
        }
        Some(Naive {
            year,
            month,
            day,
            hour: secs_of_day / 3600,
            minute: secs_of_day % 3600 / 60,
            second: secs_of_day % 60,
            nano,
        })
    }

    /// The date and time `secs` whole seconds after 1970-01-01T00:00:00.
    pub fn from_unix(secs: i64, nano: u32) -> Option<Naive> {
        Naive::from_days_and_time(
            secs.div_euclid(86_400),
            secs.rem_euclid(86_400) as u32,
            nano,
        )
    }

    /// Days since 1970-01-01 of the date.
    pub fn days(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    /// Whole seconds since 1970-01-01T00:00:00, the leap second counted as the
    /// last second of its minute.
    pub fn unix_secs(&self) -> i64 {
        self.days() * 86_400
            + i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second)
    }

    /// The same clock reading `secs` seconds later (or earlier), None when it
    /// leaves the calendar.
    pub fn add_secs(&self, secs: i64) -> Option<Naive> {
        Naive::from_unix(self.unix_secs().checked_add(secs)?, self.nano)
    }

    /// 0 for Sunday .. 6 for Saturday.
    pub fn weekday_from_sun(&self) -> u32 {
        (self.days() + 4).rem_euclid(7) as u32
    }

    /// 0 for Monday .. 6 for Sunday.
    pub fn weekday_from_mon(&self) -> u32 {
        (self.weekday_from_sun() + 6) % 7
    }

    /// The day of the year, from 1.
    pub fn ordinal(&self) -> u32 {
        (self.days() - days_from_civil(self.year, 1, 1)) as u32 + 1
    }

    /// The ISO year and week.
    pub fn iso_week(&self) -> (i32, u32) {
        let w = (self.ordinal() as i32 - self.weekday_from_mon() as i32 - 1 + 10) / 7;
        if w < 1 {
            (self.year - 1, iso_weeks_in_year(self.year - 1))
        } else if w as u32 > iso_weeks_in_year(self.year) {
            (self.year + 1, 1)
        } else {
            (self.year, w as u32)
        }
    }

    /// The weeks of the year counted from the first `start` (0 = Monday ..
    /// 6 = Sunday), as `%U` and `%W` count them.
    fn weeks_from(&self, start: u32) -> i32 {
        let since = (self.weekday_from_mon() + 7 - start) % 7;
        (self.ordinal() as i32 - since as i32 + 6) / 7
    }

    fn quarter(&self) -> u32 {
        (self.month - 1) / 3 + 1
    }

    /// The date of an ISO week date; weekday is 1 (Monday) to 7 (Sunday).
    pub fn from_iso_week(year: i32, week: u32, weekday: u32) -> Option<Naive> {
        if !(MIN_YEAR..=MAX_YEAR).contains(&year)
            || week == 0
            || week > iso_weeks_in_year(year)
            || !(1..=7).contains(&weekday)
        {
            return None;
        }
        let jan4 = Naive::date(year, 1, 4)?;
        let monday_of_week_1 = jan4.days() - i64::from(jan4.weekday_from_mon());
        let days = monday_of_week_1 + i64::from(week - 1) * 7 + i64::from(weekday - 1);
        Naive::from_days_and_time(days, 0, 0)
    }

    /// The date of the day-of-year `ordinal` of `year`.
    pub fn from_ordinal(year: i32, ordinal: u32) -> Option<Naive> {
        if !(MIN_YEAR..=MAX_YEAR).contains(&year)
            || ordinal < 1
            || ordinal > if is_leap(year) { 366 } else { 365 }
        {
            return None;
        }
        Naive::from_days_and_time(days_from_civil(year, 1, 1) + i64::from(ordinal) - 1, 0, 0)
    }

    /// The date and time in a strftime layout; a layout that needs a zone
    /// (`%Z`, `%z`, `%+`) cannot be written.
    pub fn format(&self, layout: &str) -> Formatted {
        Formatted {
            text: format_naive(layout, self, None),
        }
    }

    /// The same time of day on another date.
    pub fn with_date(&self, date: Naive) -> Naive {
        Naive {
            hour: self.hour,
            minute: self.minute,
            second: self.second,
            nano: self.nano,
            ..date
        }
    }

    /// The text `YYYY-MM-DDTHH:MM:SS[.fff[fff[fff]]]` followed by the offset in
    /// RFC 3339 form: `Z` for a zero offset when `use_z`, else `+hh:mm`.
    pub fn rfc3339(&self, offset: i32, use_z: bool) -> String {
        let mut out = String::new();
        let _ = write_rfc3339(&mut out, self, offset, true, use_z);
        out
    }
}

// ─────────────────────────────── strftime items ────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pad {
    None,
    Zero,
    Space,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Num {
    Year,
    YearDiv100,
    YearMod100,
    IsoYear,
    IsoYearMod100,
    Quarter,
    Month,
    Day,
    WeekFromSun,
    WeekFromMon,
    IsoWeek,
    DaysFromSun,
    WeekdayFromMon,
    Ordinal,
    Hour,
    Hour12,
    Minute,
    Second,
    Nanosecond,
    Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fix {
    ShortMonth,
    LongMonth,
    ShortWeekday,
    LongWeekday,
    LowerAmPm,
    UpperAmPm,
    Nano,
    Nano3,
    Nano6,
    Nano9,
    Nano3NoDot,
    Nano6NoDot,
    Nano9NoDot,
    TzName,
    Offset,
    OffsetColon,
    OffsetDoubleColon,
    OffsetTripleColon,
    OffsetPermissive,
    Rfc3339,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Item<'a> {
    Literal(&'a str),
    Space(&'a str),
    Numeric(Num, Pad),
    Fixed(Fix),
    Error,
}

const SHORT_MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const LONG_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const SHORT_WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const LONG_WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

fn num0(n: Num) -> Item<'static> {
    Item::Numeric(n, Pad::Zero)
}
fn nums(n: Num) -> Item<'static> {
    Item::Numeric(n, Pad::Space)
}
fn num(n: Num) -> Item<'static> {
    Item::Numeric(n, Pad::None)
}
fn fixed(f: Fix) -> Item<'static> {
    Item::Fixed(f)
}

/// The items of a strftime layout; an unreadable specifier is an `Item::Error`
/// in its place, and reading goes on after it.
fn strftime_items(layout: &str) -> Vec<Item<'_>> {
    let mut out = Vec::new();
    let mut rest = layout;
    while let Some(first) = rest.chars().next() {
        if first == '%' {
            let (items, next) = specifier(rest);
            out.extend(items);
            rest = next;
        } else if first.is_whitespace() {
            let end = rest
                .find(|c: char| !c.is_whitespace())
                .unwrap_or(rest.len());
            out.push(Item::Space(&rest[..end]));
            rest = &rest[end..];
        } else {
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '%')
                .unwrap_or(rest.len());
            out.push(Item::Literal(&rest[..end]));
            rest = &rest[end..];
        }
    }
    out
}

/// One `%` specifier at the start of `text`: its items and the text after it.
fn specifier(text: &str) -> (Vec<Item<'_>>, &str) {
    let mut remainder = &text[1..];
    // A premature end of the text is an error that has consumed everything.
    macro_rules! next {
        () => {
            match remainder.chars().next() {
                Some(c) => {
                    remainder = &remainder[c.len_utf8()..];
                    c
                }
                None => return (vec![Item::Error], remainder),
            }
        };
    }
    macro_rules! bad {
        () => {
            return (vec![Item::Error], remainder)
        };
    }
    let spec = next!();
    let pad_override = match spec {
        '-' => Some(Pad::None),
        '0' => Some(Pad::Zero),
        '_' => Some(Pad::Space),
        _ => None,
    };
    let is_alternate = spec == '#';
    let spec = if pad_override.is_some() || is_alternate {
        next!()
    } else {
        spec
    };
    if is_alternate && spec != 'z' {
        bad!();
    }
    use Num::*;
    let items: Vec<Item<'static>> = match spec {
        'A' => vec![fixed(Fix::LongWeekday)],
        'B' => vec![fixed(Fix::LongMonth)],
        'C' => vec![num0(YearDiv100)],
        'D' => vec![
            num0(Month),
            Item::Literal("/"),
            num0(Day),
            Item::Literal("/"),
            num0(YearMod100),
        ],
        'F' => vec![
            num0(Year),
            Item::Literal("-"),
            num0(Month),
            Item::Literal("-"),
            num0(Day),
        ],
        'G' => vec![num0(IsoYear)],
        'H' => vec![num0(Hour)],
        'I' => vec![num0(Hour12)],
        'M' => vec![num0(Minute)],
        'P' => vec![fixed(Fix::LowerAmPm)],
        'R' => vec![num0(Hour), Item::Literal(":"), num0(Minute)],
        'S' => vec![num0(Second)],
        'T' | 'X' => vec![
            num0(Hour),
            Item::Literal(":"),
            num0(Minute),
            Item::Literal(":"),
            num0(Second),
        ],
        'U' => vec![num0(WeekFromSun)],
        'V' => vec![num0(IsoWeek)],
        'W' => vec![num0(WeekFromMon)],
        'Y' => vec![num0(Year)],
        'Z' => vec![fixed(Fix::TzName)],
        'a' => vec![fixed(Fix::ShortWeekday)],
        'b' | 'h' => vec![fixed(Fix::ShortMonth)],
        'c' => vec![
            fixed(Fix::ShortWeekday),
            Item::Space(" "),
            fixed(Fix::ShortMonth),
            Item::Space(" "),
            nums(Day),
            Item::Space(" "),
            num0(Hour),
            Item::Literal(":"),
            num0(Minute),
            Item::Literal(":"),
            num0(Second),
            Item::Space(" "),
            num0(Year),
        ],
        'd' => vec![num0(Day)],
        'e' => vec![nums(Day)],
        'f' => vec![num0(Nanosecond)],
        'g' => vec![num0(IsoYearMod100)],
        'j' => vec![num0(Ordinal)],
        'k' => vec![nums(Hour)],
        'l' => vec![nums(Hour12)],
        'm' => vec![num0(Month)],
        'n' => vec![Item::Space("\n")],
        'p' => vec![fixed(Fix::UpperAmPm)],
        'q' => vec![num(Quarter)],
        'r' => vec![
            num0(Hour12),
            Item::Literal(":"),
            num0(Minute),
            Item::Literal(":"),
            num0(Second),
            Item::Space(" "),
            fixed(Fix::UpperAmPm),
        ],
        's' => vec![num(Timestamp)],
        't' => vec![Item::Space("\t")],
        'u' => vec![num(WeekdayFromMon)],
        'v' => vec![
            nums(Day),
            Item::Literal("-"),
            fixed(Fix::ShortMonth),
            Item::Literal("-"),
            num0(Year),
        ],
        'w' => vec![num(DaysFromSun)],
        'x' => vec![
            num0(Month),
            Item::Literal("/"),
            num0(Day),
            Item::Literal("/"),
            num0(YearMod100),
        ],
        'y' => vec![num0(YearMod100)],
        'z' => vec![fixed(if is_alternate {
            Fix::OffsetPermissive
        } else {
            Fix::Offset
        })],
        '+' => vec![fixed(Fix::Rfc3339)],
        ':' => {
            if remainder.starts_with("::z") {
                remainder = &remainder[3..];
                vec![fixed(Fix::OffsetTripleColon)]
            } else if remainder.starts_with(":z") {
                remainder = &remainder[2..];
                vec![fixed(Fix::OffsetDoubleColon)]
            } else if remainder.starts_with('z') {
                remainder = &remainder[1..];
                vec![fixed(Fix::OffsetColon)]
            } else {
                bad!()
            }
        }
        '.' => match next!() {
            '3' if next!() == 'f' => vec![fixed(Fix::Nano3)],
            '6' if next!() == 'f' => vec![fixed(Fix::Nano6)],
            '9' if next!() == 'f' => vec![fixed(Fix::Nano9)],
            'f' => vec![fixed(Fix::Nano)],
            _ => bad!(),
        },
        '3' => {
            if next!() == 'f' {
                vec![fixed(Fix::Nano3NoDot)]
            } else {
                bad!()
            }
        }
        '6' => {
            if next!() == 'f' {
                vec![fixed(Fix::Nano6NoDot)]
            } else {
                bad!()
            }
        }
        '9' => {
            if next!() == 'f' {
                vec![fixed(Fix::Nano9NoDot)]
            } else {
                bad!()
            }
        }
        '%' => vec![Item::Literal("%")],
        _ => bad!(),
    };
    if let Some(new_pad) = pad_override {
        // A padding flag belongs to one numeric item, not to a composite.
        return match items.as_slice() {
            [Item::Numeric(kind, _)] => (vec![Item::Numeric(*kind, new_pad)], remainder),
            _ => (vec![Item::Error], remainder),
        };
    }
    (items, remainder)
}

/// Whether a strftime layout holds a specifier that is not read.
pub fn layout_has_error(layout: &str) -> bool {
    strftime_items(layout)
        .iter()
        .any(|item| matches!(item, Item::Error))
}

// ───────────────────────────────── formatting ──────────────────────────────────

/// A zone as formatting sees it: the name `%Z` writes and the offset in seconds.
#[derive(Clone, Debug)]
pub struct Zone {
    pub name: String,
    pub offset: i32,
}

impl Zone {
    /// UTC.
    pub fn utc() -> Zone {
        Zone {
            name: "UTC".to_string(),
            offset: 0,
        }
    }

    /// A fixed offset in seconds, named as `+hh:mm`.
    pub fn fixed(offset: i32) -> Zone {
        Zone {
            name: offset_display(offset),
            offset,
        }
    }
}

fn offset_display(offset: i32) -> String {
    let (sign, off) = if offset < 0 {
        ('-', -offset)
    } else {
        ('+', offset)
    };
    let sec = off.rem_euclid(60);
    let mins = off.div_euclid(60);
    let (min, hour) = (mins.rem_euclid(60), mins.div_euclid(60));
    if sec == 0 {
        format!("{sign}{hour:02}:{min:02}")
    } else {
        format!("{sign}{hour:02}:{min:02}:{sec:02}")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Precision {
    Hours,
    Minutes,
    Seconds,
}

fn write_offset(
    w: &mut impl fmt::Write,
    offset: i32,
    precision: Precision,
    colons: bool,
) -> fmt::Result {
    let (sign, off) = if offset < 0 {
        ('-', -offset)
    } else {
        ('+', offset)
    };
    let (hours, mins, secs) = match precision {
        Precision::Hours => ((off / 3600) as u8, 0, 0),
        Precision::Minutes => {
            let minutes = (off + 30) / 60;
            ((minutes / 60) as u8, (minutes % 60) as u8, 0)
        }
        Precision::Seconds => {
            let minutes = off / 60;
            ((minutes / 60) as u8, (minutes % 60) as u8, (off % 60) as u8)
        }
    };
    if hours >= 100 || mins >= 100 || secs >= 100 {
        return Err(fmt::Error);
    }
    write!(w, "{sign}{hours:02}")?;
    if precision != Precision::Hours {
        if colons {
            w.write_char(':')?;
        }
        write!(w, "{mins:02}")?;
    }
    if precision == Precision::Seconds {
        if colons {
            w.write_char(':')?;
        }
        write!(w, "{secs:02}")?;
    }
    Ok(())
}

/// `%Y-%m-%dT%H:%M:%S%.f%:z`; the fraction is the digits that are there (3, 6
/// or 9) when `auto_fraction`, none otherwise.
fn write_rfc3339(
    w: &mut impl fmt::Write,
    n: &Naive,
    offset: i32,
    auto_fraction: bool,
    use_z: bool,
) -> fmt::Result {
    if (0..=9999).contains(&n.year) {
        write!(w, "{:04}", n.year)?;
    } else {
        write!(w, "{:+05}", n.year)?;
    }
    let (mut second, mut nano) = (n.second, n.nano);
    if nano >= 1_000_000_000 {
        second += 1;
        nano -= 1_000_000_000;
    }
    write!(
        w,
        "-{:02}-{:02}T{:02}:{:02}:{:02}",
        n.month, n.day, n.hour, n.minute, second
    )?;
    if auto_fraction {
        write_auto_fraction(w, nano)?;
    }
    if use_z && offset == 0 {
        w.write_char('Z')
    } else {
        write_offset(w, offset, Precision::Minutes, true)
    }
}

/// `.123`, `.123456` or `.123456789`: as many digits as the value needs, in
/// groups of three; nothing for zero.
fn write_auto_fraction(w: &mut impl fmt::Write, nano: u32) -> fmt::Result {
    if nano == 0 {
        Ok(())
    } else if nano.is_multiple_of(1_000_000) {
        write!(w, ".{:03}", nano / 1_000_000)
    } else if nano.is_multiple_of(1_000) {
        write!(w, ".{:06}", nano / 1_000)
    } else {
        write!(w, ".{nano:09}")
    }
}

fn write_two(w: &mut impl fmt::Write, v: u32, pad: Pad) -> fmt::Result {
    let ones = v % 10;
    match (v / 10, pad) {
        (0, Pad::None) => {}
        (0, Pad::Space) => w.write_char(' ')?,
        (tens, _) => write!(w, "{tens}")?,
    }
    write!(w, "{ones}")
}

fn write_n(
    w: &mut impl fmt::Write,
    width: usize,
    v: i64,
    pad: Pad,
    always_sign: bool,
) -> fmt::Result {
    if always_sign {
        match pad {
            Pad::None => write!(w, "{v:+}"),
            Pad::Zero => write!(w, "{:+01$}", v, width + 1),
            Pad::Space => write!(w, "{:+1$}", v, width + 1),
        }
    } else {
        match pad {
            Pad::None => write!(w, "{v}"),
            Pad::Zero => write!(w, "{v:0width$}"),
            Pad::Space => write!(w, "{v:width$}"),
        }
    }
}

fn write_year(w: &mut impl fmt::Write, year: i32, pad: Pad) -> fmt::Result {
    if (1000..=9999).contains(&year) {
        write!(w, "{year}")
    } else {
        write_n(w, 4, i64::from(year), pad, !(0..10_000).contains(&year))
    }
}

fn hour12(hour: u32) -> (bool, u32) {
    let pm = hour >= 12;
    let h = hour % 12;
    (pm, if h == 0 { 12 } else { h })
}

fn format_item(w: &mut String, item: &Item<'_>, n: &Naive, zone: Option<&Zone>) -> fmt::Result {
    match *item {
        Item::Literal(s) | Item::Space(s) => w.write_str(s),
        Item::Error => Err(fmt::Error),
        Item::Numeric(kind, pad) => {
            use Num::*;
            match kind {
                Year => write_year(w, n.year, pad),
                YearDiv100 => write_two(w, n.year.div_euclid(100) as u32, pad),
                YearMod100 => write_two(w, n.year.rem_euclid(100) as u32, pad),
                IsoYear => write_year(w, n.iso_week().0, pad),
                IsoYearMod100 => write_two(w, n.iso_week().0.rem_euclid(100) as u32, pad),
                Quarter => write!(w, "{}", n.quarter()),
                Month => write_two(w, n.month, pad),
                Day => write_two(w, n.day, pad),
                WeekFromSun => write_two(w, n.weeks_from(6) as u32, pad),
                WeekFromMon => write_two(w, n.weeks_from(0) as u32, pad),
                IsoWeek => write_two(w, n.iso_week().1, pad),
                DaysFromSun => write!(w, "{}", n.weekday_from_sun()),
                WeekdayFromMon => write!(w, "{}", n.weekday_from_mon() + 1),
                Ordinal => write_n(w, 3, i64::from(n.ordinal()), pad, false),
                Hour => write_two(w, n.hour, pad),
                Hour12 => write_two(w, hour12(n.hour).1, pad),
                Minute => write_two(w, n.minute, pad),
                Second => write_two(w, n.second + n.nano / 1_000_000_000, pad),
                Nanosecond => write_n(w, 9, i64::from(n.nano % 1_000_000_000), pad, false),
                Timestamp => {
                    let at = n.unix_secs() - zone.map_or(0, |z| i64::from(z.offset));
                    write_n(w, 9, at, pad, false)
                }
            }
        }
        Item::Fixed(kind) => {
            let nano = n.nano % 1_000_000_000;
            match kind {
                Fix::ShortMonth => w.write_str(SHORT_MONTHS[n.month as usize - 1]),
                Fix::LongMonth => w.write_str(LONG_MONTHS[n.month as usize - 1]),
                Fix::ShortWeekday => w.write_str(SHORT_WEEKDAYS[n.weekday_from_sun() as usize]),
                Fix::LongWeekday => w.write_str(LONG_WEEKDAYS[n.weekday_from_sun() as usize]),
                Fix::LowerAmPm => w.write_str(if hour12(n.hour).0 { "pm" } else { "am" }),
                Fix::UpperAmPm => w.write_str(if hour12(n.hour).0 { "PM" } else { "AM" }),
                Fix::Nano => write_auto_fraction(w, nano),
                Fix::Nano3 => write!(w, ".{:03}", nano / 1_000_000 % 1000),
                Fix::Nano6 => write!(w, ".{:06}", nano / 1_000 % 1_000_000),
                Fix::Nano9 => write!(w, ".{nano:09}"),
                Fix::Nano3NoDot => write!(w, "{:03}", nano / 1_000_000 % 1000),
                Fix::Nano6NoDot => write!(w, "{:06}", nano / 1_000 % 1_000_000),
                Fix::Nano9NoDot => write!(w, "{nano:09}"),
                // `%#z` is for reading; writing it is an error, as it always was.
                Fix::OffsetPermissive => Err(fmt::Error),
                Fix::TzName => match zone {
                    Some(z) => w.write_str(&z.name),
                    None => Err(fmt::Error),
                },
                Fix::Offset => match zone {
                    Some(z) => write_offset(w, z.offset, Precision::Minutes, false),
                    None => Err(fmt::Error),
                },
                Fix::OffsetColon => match zone {
                    Some(z) => write_offset(w, z.offset, Precision::Minutes, true),
                    None => Err(fmt::Error),
                },
                Fix::OffsetDoubleColon => match zone {
                    Some(z) => write_offset(w, z.offset, Precision::Seconds, true),
                    None => Err(fmt::Error),
                },
                Fix::OffsetTripleColon => match zone {
                    Some(z) => write_offset(w, z.offset, Precision::Hours, false),
                    None => Err(fmt::Error),
                },
                Fix::Rfc3339 => match zone {
                    Some(z) => write_rfc3339(w, n, z.offset, true, false),
                    None => Err(fmt::Error),
                },
            }
        }
    }
}

/// Format a date and time with a strftime layout. An error for a layout with a
/// specifier that is not read, or one that needs a zone when `zone` is None.
pub fn format_naive(layout: &str, n: &Naive, zone: Option<&Zone>) -> Result<String, fmt::Error> {
    let mut out = String::new();
    for item in strftime_items(layout) {
        format_item(&mut out, &item, n, zone)?;
    }
    Ok(out)
}

/// A strftime layout applied lazily, so it is written with `{}`. Writing it
/// fails for a layout the formatter cannot write, as `to_string()` then panics.
pub struct Formatted {
    text: Result<String, fmt::Error>,
}

impl Formatted {
    /// The text, or an error for a layout that cannot be written.
    pub fn write(&self) -> Result<String, fmt::Error> {
        self.text.clone()
    }
}

impl fmt::Display for Formatted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.text {
            Ok(text) => f.pad(text),
            Err(_) => Err(fmt::Error),
        }
    }
}

// ─────────────────────────────────── reading ───────────────────────────────────

type Scan<T> = Result<T, ()>;

fn scan_number(s: &str, min: usize, max: usize) -> Scan<(&str, i64)> {
    let bytes = s.as_bytes();
    if bytes.len() < min {
        return Err(());
    }
    let mut n = 0i64;
    for (i, c) in bytes.iter().take(max).copied().enumerate() {
        if !c.is_ascii_digit() {
            if i < min {
                return Err(());
            }
            return Ok((&s[i..], n));
        }
        n = n
            .checked_mul(10)
            .and_then(|n| n.checked_add(i64::from(c - b'0')))
            .ok_or(())?;
    }
    Ok((&s[max.min(bytes.len())..], n))
}

const NANO_SCALE: [i64; 10] = [
    0,
    100_000_000,
    10_000_000,
    1_000_000,
    100_000,
    10_000,
    1_000,
    100,
    10,
    1,
];

fn scan_nanosecond(s: &str) -> Scan<(&str, i64)> {
    let before = s.len();
    let (s, v) = scan_number(s, 1, 9)?;
    let consumed = before - s.len();
    let v = v * NANO_SCALE[consumed];
    Ok((s.trim_start_matches(|c: char| c.is_ascii_digit()), v))
}

fn scan_nanosecond_fixed(s: &str, digits: usize) -> Scan<(&str, i64)> {
    let (s, v) = scan_number(s, digits, digits)?;
    Ok((s, v * NANO_SCALE[digits]))
}

fn scan_short_month0(s: &str) -> Scan<(&str, u32)> {
    let b = s.as_bytes();
    if b.len() < 3 {
        return Err(());
    }
    let lower = [b[0] | 32, b[1] | 32, b[2] | 32];
    let month0 = SHORT_MONTHS
        .iter()
        .position(|m| m.as_bytes().eq_ignore_ascii_case(&lower))
        .ok_or(())?;
    Ok((&s[3..], month0 as u32))
}

fn scan_short_or_long_month0(s: &str) -> Scan<(&str, u32)> {
    const SUFFIXES: [&[u8]; 12] = [
        b"uary", b"ruary", b"ch", b"il", b"", b"e", b"y", b"ust", b"tember", b"ober", b"ember",
        b"ember",
    ];
    let (mut s, month0) = scan_short_month0(s)?;
    let suffix = SUFFIXES[month0 as usize];
    if s.len() >= suffix.len() && s.as_bytes()[..suffix.len()].eq_ignore_ascii_case(suffix) {
        s = &s[suffix.len()..];
    }
    Ok((s, month0))
}

/// 0 for Sunday .. 6 for Saturday.
fn scan_short_weekday(s: &str) -> Scan<(&str, u32)> {
    let b = s.as_bytes();
    if b.len() < 3 {
        return Err(());
    }
    let lower = [b[0] | 32, b[1] | 32, b[2] | 32];
    let day = SHORT_WEEKDAYS
        .iter()
        .position(|d| d.as_bytes().eq_ignore_ascii_case(&lower))
        .ok_or(())?;
    Ok((&s[3..], day as u32))
}

fn scan_short_or_long_weekday(s: &str) -> Scan<(&str, u32)> {
    // Indexed from Sunday.
    const SUFFIXES: [&[u8]; 7] = [
        b"day", b"day", b"sday", b"nesday", b"rsday", b"day", b"urday",
    ];
    let (mut s, day) = scan_short_weekday(s)?;
    let suffix = SUFFIXES[day as usize];
    if s.len() >= suffix.len() && s.as_bytes()[..suffix.len()].eq_ignore_ascii_case(suffix) {
        s = &s[suffix.len()..];
    }
    Ok((s, day))
}

fn colon_or_space(s: &str) -> Scan<&str> {
    Ok(s.trim_start_matches(|c: char| c == ':' || c.is_whitespace()))
}

fn require_colon(s: &str) -> Scan<&str> {
    match s.as_bytes().first() {
        Some(b':') => Ok(&s[1..]),
        _ => Err(()),
    }
}

fn scan_timezone_offset<F>(
    mut s: &str,
    consume_colon: F,
    allow_zulu: bool,
    allow_missing_minutes: bool,
) -> Scan<(&str, i32)>
where
    F: Fn(&str) -> Scan<&str>,
{
    if allow_zulu {
        if let Some(b'Z' | b'z') = s.as_bytes().first() {
            return Ok((&s[1..], 0));
        }
    }
    let negative = match s.chars().next() {
        Some('+') => {
            s = &s[1..];
            false
        }
        Some('-') => {
            s = &s[1..];
            true
        }
        Some('\u{2212}') => {
            s = &s['\u{2212}'.len_utf8()..];
            true
        }
        _ => return Err(()),
    };
    let b = s.as_bytes();
    if b.len() < 2 || !b[0].is_ascii_digit() || !b[1].is_ascii_digit() {
        return Err(());
    }
    let hours = i32::from((b[0] - b'0') * 10 + (b[1] - b'0'));
    s = &s[2..];
    s = consume_colon(s)?;
    let b = s.as_bytes();
    let minutes = if b.len() >= 2 {
        match (b[0], b[1]) {
            (m1 @ b'0'..=b'5', m2 @ b'0'..=b'9') => i32::from((m1 - b'0') * 10 + (m2 - b'0')),
            _ => return Err(()),
        }
    } else if allow_missing_minutes {
        0
    } else {
        return Err(());
    };
    s = match s.len() {
        len if len >= 2 => &s[2..],
        0 => s,
        _ => return Err(()),
    };
    let seconds = hours * 3600 + minutes * 60;
    Ok((s, if negative { -seconds } else { seconds }))
}

/// The fields of a date and time as they are read, checked against each other
/// when the date and time are put together.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Parsed {
    year: Option<i32>,
    year_div_100: Option<i32>,
    year_mod_100: Option<i32>,
    isoyear: Option<i32>,
    isoyear_div_100: Option<i32>,
    isoyear_mod_100: Option<i32>,
    quarter: Option<u32>,
    month: Option<u32>,
    week_from_sun: Option<u32>,
    week_from_mon: Option<u32>,
    isoweek: Option<u32>,
    /// 0 for Monday .. 6 for Sunday.
    weekday: Option<u32>,
    ordinal: Option<u32>,
    day: Option<u32>,
    hour_div_12: Option<u32>,
    hour_mod_12: Option<u32>,
    minute: Option<u32>,
    second: Option<u32>,
    nanosecond: Option<u32>,
    timestamp: Option<i64>,
    offset: Option<i32>,
}

/// Why a date or time could not be put together: a field missing, or wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Miss {
    NotEnough,
    Bad,
}

/// A time field that must be present and at most `max`.
fn miss(v: Option<u32>, max: u32) -> Result<u32, Miss> {
    match v {
        Some(v) if v <= max => Ok(v),
        Some(_) => Err(Miss::Bad),
        None => Err(Miss::NotEnough),
    }
}

fn set<T: PartialEq>(slot: &mut Option<T>, value: T) -> Scan<()> {
    match slot {
        Some(old) if *old != value => Err(()),
        _ => {
            *slot = Some(value);
            Ok(())
        }
    }
}

fn ranged(value: i64, lo: i64, hi: i64) -> Scan<i64> {
    if (lo..=hi).contains(&value) {
        Ok(value)
    } else {
        Err(())
    }
}

impl Parsed {
    fn set_numeric(&mut self, kind: Num, v: i64) -> Scan<()> {
        use Num::*;
        let int = |v: i64| i32::try_from(v).map_err(|_| ());
        match kind {
            Year => set(&mut self.year, int(v)?),
            YearDiv100 => set(
                &mut self.year_div_100,
                ranged(v, 0, i64::from(i32::MAX))? as i32,
            ),
            YearMod100 => set(&mut self.year_mod_100, ranged(v, 0, 99)? as i32),
            IsoYear => set(&mut self.isoyear, int(v)?),
            IsoYearMod100 => set(&mut self.isoyear_mod_100, ranged(v, 0, 99)? as i32),
            Quarter => set(&mut self.quarter, ranged(v, 1, 4)? as u32),
            Month => set(&mut self.month, ranged(v, 1, 12)? as u32),
            Day => set(&mut self.day, ranged(v, 1, 31)? as u32),
            WeekFromSun => set(&mut self.week_from_sun, ranged(v, 0, 53)? as u32),
            WeekFromMon => set(&mut self.week_from_mon, ranged(v, 0, 53)? as u32),
            IsoWeek => set(&mut self.isoweek, ranged(v, 1, 53)? as u32),
            DaysFromSun => {
                let day = ranged(v, 0, 6)? as u32;
                set(&mut self.weekday, (day + 6) % 7)
            }
            WeekdayFromMon => {
                let day = ranged(v, 1, 7)? as u32;
                set(&mut self.weekday, day - 1)
            }
            Ordinal => set(&mut self.ordinal, ranged(v, 1, 366)? as u32),
            Hour => {
                let hour = ranged(v, 0, 23)? as u32;
                set(&mut self.hour_div_12, hour / 12)?;
                set(&mut self.hour_mod_12, hour % 12)
            }
            Hour12 => {
                let hour = ranged(v, 1, 12)? as u32;
                set(&mut self.hour_mod_12, hour % 12)
            }
            Minute => set(&mut self.minute, ranged(v, 0, 59)? as u32),
            Second => set(&mut self.second, ranged(v, 0, 60)? as u32),
            Nanosecond => set(&mut self.nanosecond, ranged(v, 0, 999_999_999)? as u32),
            Timestamp => set(&mut self.timestamp, v),
        }
    }

    fn set_offset(&mut self, v: i64) -> Scan<()> {
        set(&mut self.offset, i32::try_from(v).map_err(|_| ())?)
    }

    fn set_nanosecond(&mut self, v: i64) -> Scan<()> {
        set(&mut self.nanosecond, ranged(v, 0, 999_999_999)? as u32)
    }

    fn to_date(&self) -> Result<Naive, Miss> {
        fn resolve_year(
            y: Option<i32>,
            q: Option<i32>,
            r: Option<i32>,
        ) -> Result<Option<i32>, Miss> {
            match (y, q, r) {
                (y, None, None) => Ok(y),
                (Some(y), q, r @ Some(0..=99)) | (Some(y), q, r @ None) => {
                    if y < 0 {
                        return Err(Miss::Bad);
                    }
                    let (q_, r_) = (y / 100, y % 100);
                    if q.unwrap_or(q_) == q_ && r.unwrap_or(r_) == r_ {
                        Ok(Some(y))
                    } else {
                        Err(Miss::Bad)
                    }
                }
                (None, Some(q), Some(r @ 0..=99)) => {
                    if q < 0 {
                        return Err(Miss::Bad);
                    }
                    Ok(Some(
                        q.checked_mul(100)
                            .and_then(|v| v.checked_add(r))
                            .ok_or(Miss::Bad)?,
                    ))
                }
                (None, None, Some(r @ 0..=99)) => Ok(Some(r + if r < 70 { 2000 } else { 1900 })),
                (None, Some(_), None) => Err(Miss::NotEnough),
                (_, _, Some(_)) => Err(Miss::Bad),
            }
        }
        let given_year = resolve_year(self.year, self.year_div_100, self.year_mod_100)?;
        let given_isoyear = resolve_year(self.isoyear, self.isoyear_div_100, self.isoyear_mod_100)?;

        let verify_ymd = |d: &Naive| {
            let (div, rem) = if d.year >= 0 {
                (Some(d.year / 100), Some(d.year % 100))
            } else {
                (None, None)
            };
            self.year.unwrap_or(d.year) == d.year
                && self.year_div_100.or(div) == div
                && self.year_mod_100.or(rem) == rem
                && self.month.unwrap_or(d.month) == d.month
                && self.day.unwrap_or(d.day) == d.day
        };
        let verify_iso = |d: &Naive| {
            let (isoyear, isoweek) = d.iso_week();
            let (div, rem) = if isoyear >= 0 {
                (Some(isoyear / 100), Some(isoyear % 100))
            } else {
                (None, None)
            };
            self.isoyear.unwrap_or(isoyear) == isoyear
                && self.isoyear_div_100.or(div) == div
                && self.isoyear_mod_100.or(rem) == rem
                && self.isoweek.unwrap_or(isoweek) == isoweek
                && self.weekday.unwrap_or(d.weekday_from_mon()) == d.weekday_from_mon()
        };
        let verify_ordinal = |d: &Naive| {
            let from_sun = d.weeks_from(6);
            let from_mon = d.weeks_from(0);
            self.ordinal.unwrap_or(d.ordinal()) == d.ordinal()
                && self.week_from_sun.map_or(from_sun, |v| v as i32) == from_sun
                && self.week_from_mon.map_or(from_mon, |v| v as i32) == from_mon
        };
        let resolve_week_date = |year: i32, week: u32, weekday: u32, start: u32| {
            if week > 53 {
                return Err(Miss::Bad);
            }
            let first = Naive::from_ordinal(year, 1).ok_or(Miss::Bad)?;
            let first_week_start = 1 + ((start + 7 - first.weekday_from_mon()) % 7) as i32;
            let weekday = ((weekday + 7 - start) % 7) as i32;
            let ordinal = first_week_start + (week as i32 - 1) * 7 + weekday;
            if ordinal <= 0 {
                return Err(Miss::Bad);
            }
            Naive::from_ordinal(year, ordinal as u32).ok_or(Miss::Bad)
        };

        let (verified, date) =
            if let (Some(year), Some(month), Some(day)) = (given_year, self.month, self.day) {
                let date = Naive::date(year, month, day).ok_or(Miss::Bad)?;
                (verify_iso(&date) && verify_ordinal(&date), date)
            } else if let (Some(year), Some(ordinal)) = (given_year, self.ordinal) {
                let date = Naive::from_ordinal(year, ordinal).ok_or(Miss::Bad)?;
                (
                    verify_ymd(&date) && verify_iso(&date) && verify_ordinal(&date),
                    date,
                )
            } else if let (Some(year), Some(week), Some(weekday)) =
                (given_year, self.week_from_sun, self.weekday)
            {
                let date = resolve_week_date(year, week, weekday, 6)?;
                (
                    verify_ymd(&date) && verify_iso(&date) && verify_ordinal(&date),
                    date,
                )
            } else if let (Some(year), Some(week), Some(weekday)) =
                (given_year, self.week_from_mon, self.weekday)
            {
                let date = resolve_week_date(year, week, weekday, 0)?;
                (
                    verify_ymd(&date) && verify_iso(&date) && verify_ordinal(&date),
                    date,
                )
            } else if let (Some(isoyear), Some(isoweek), Some(weekday)) =
                (given_isoyear, self.isoweek, self.weekday)
            {
                let date = Naive::from_iso_week(isoyear, isoweek, weekday + 1).ok_or(Miss::Bad)?;
                (verify_ymd(&date) && verify_ordinal(&date), date)
            } else {
                return Err(Miss::NotEnough);
            };
        if !verified {
            return Err(Miss::Bad);
        }
        if let Some(q) = self.quarter {
            if q != date.quarter() {
                return Err(Miss::Bad);
            }
        }
        Ok(date)
    }

    /// The hour, minute, second and nanosecond.
    fn to_time(&self) -> Result<(u32, u32, u32, u32), Miss> {
        let hour_div_12 = miss(self.hour_div_12, 1)?;
        let hour_mod_12 = miss(self.hour_mod_12, 11)?;
        let minute = miss(self.minute, 59)?;
        let (second, mut nano) = match self.second.unwrap_or(0) {
            v @ 0..=59 => (v, 0),
            60 => (59, 1_000_000_000),
            _ => return Err(Miss::Bad),
        };
        nano += match self.nanosecond {
            Some(v) if self.second.is_some() => v,
            Some(_) => return Err(Miss::NotEnough),
            None => 0,
        };
        Ok((hour_div_12 * 12 + hour_mod_12, minute, second, nano))
    }

    fn to_naive_datetime_with_offset(&self, offset: i32) -> Scan<Naive> {
        let date = self.to_date();
        let time = self.to_time();
        if let (Ok(date), Ok((hour, minute, second, nano))) = (&date, &time) {
            let datetime = Naive::new(
                date.year, date.month, date.day, *hour, *minute, *second, *nano,
            )
            .ok_or(())?;
            let timestamp = datetime.unix_secs() - i64::from(offset);
            if let Some(given) = self.timestamp {
                if given != timestamp && !(datetime.nano >= 1_000_000_000 && given == timestamp + 1)
                {
                    return Err(());
                }
            }
            Ok(datetime)
        } else if let Some(timestamp) = self.timestamp {
            // A date or time that is wrong, not merely missing, ends it here.
            if matches!(date, Err(Miss::Bad)) || matches!(time, Err(Miss::Bad)) {
                return Err(());
            }
            let ts = timestamp.checked_add(i64::from(offset)).ok_or(())?;
            let mut datetime = Naive::from_unix(ts, 0).ok_or(())?;
            let mut parsed = self.clone();
            if parsed.second == Some(60) {
                match datetime.second {
                    59 => {}
                    0 => datetime = datetime.add_secs(-1).ok_or(())?,
                    _ => return Err(()),
                }
            } else {
                parsed.set_numeric(Num::Second, i64::from(datetime.second))?;
            }
            parsed.set_numeric(Num::Year, i64::from(datetime.year))?;
            parsed.set_numeric(Num::Ordinal, i64::from(datetime.ordinal()))?;
            parsed.set_numeric(Num::Hour, i64::from(datetime.hour))?;
            parsed.set_numeric(Num::Minute, i64::from(datetime.minute))?;
            let date = parsed.to_date().map_err(|_| ())?;
            let (hour, minute, second, nano) = parsed.to_time().map_err(|_| ())?;
            Naive::new(date.year, date.month, date.day, hour, minute, second, nano).ok_or(())
        } else {
            Err(())
        }
    }

    /// The instant, in UTC, of a reading with an offset (or a timestamp).
    fn to_utc(&self) -> Scan<Naive> {
        let offset = match (self.offset, self.timestamp) {
            (Some(off), _) => off,
            (None, Some(_)) => 0,
            (None, None) => return Err(()),
        };
        let naive = self.to_naive_datetime_with_offset(offset)?;
        if offset.abs() >= 86_400 {
            return Err(());
        }
        naive.add_secs(-i64::from(offset)).ok_or(())
    }
}

fn parse_items<'a>(parsed: &mut Parsed, mut s: &'a str, items: &[Item<'_>]) -> Scan<&'a str> {
    for item in items {
        match *item {
            Item::Literal(prefix) => {
                s = s.strip_prefix(prefix).ok_or(())?;
            }
            Item::Space(_) => s = s.trim_start(),
            Item::Numeric(kind, _) => {
                use Num::*;
                let (width, signed) = match kind {
                    Year | IsoYear => (4, true),
                    YearDiv100 | YearMod100 | IsoYearMod100 | Month | Day | WeekFromSun
                    | WeekFromMon | IsoWeek | Hour | Hour12 | Minute | Second => (2, false),
                    Quarter | DaysFromSun | WeekdayFromMon => (1, false),
                    Ordinal => (3, false),
                    Nanosecond => (9, false),
                    Timestamp => (usize::MAX, false),
                };
                s = s.trim_start();
                let v = if signed {
                    if let Some(rest) = s.strip_prefix('-') {
                        let (rest, v) = scan_number(rest, 1, usize::MAX)?;
                        s = rest;
                        0i64.checked_sub(v).ok_or(())?
                    } else if let Some(rest) = s.strip_prefix('+') {
                        let (rest, v) = scan_number(rest, 1, usize::MAX)?;
                        s = rest;
                        v
                    } else {
                        let (rest, v) = scan_number(s, 1, width)?;
                        s = rest;
                        v
                    }
                } else {
                    let (rest, v) = scan_number(s, 1, width)?;
                    s = rest;
                    v
                };
                parsed.set_numeric(kind, v)?;
            }
            Item::Fixed(kind) => match kind {
                Fix::ShortMonth => {
                    let (rest, m) = scan_short_month0(s)?;
                    s = rest;
                    parsed.set_numeric(Num::Month, i64::from(m) + 1)?;
                }
                Fix::LongMonth => {
                    let (rest, m) = scan_short_or_long_month0(s)?;
                    s = rest;
                    parsed.set_numeric(Num::Month, i64::from(m) + 1)?;
                }
                Fix::ShortWeekday => {
                    let (rest, d) = scan_short_weekday(s)?;
                    s = rest;
                    parsed.set_numeric(Num::DaysFromSun, i64::from(d))?;
                }
                Fix::LongWeekday => {
                    let (rest, d) = scan_short_or_long_weekday(s)?;
                    s = rest;
                    parsed.set_numeric(Num::DaysFromSun, i64::from(d))?;
                }
                Fix::LowerAmPm | Fix::UpperAmPm => {
                    let b = s.as_bytes();
                    if b.len() < 2 {
                        return Err(());
                    }
                    let pm = match (b[0] | 32, b[1] | 32) {
                        (b'a', b'm') => 0,
                        (b'p', b'm') => 1,
                        _ => return Err(()),
                    };
                    set(&mut parsed.hour_div_12, pm)?;
                    s = &s[2..];
                }
                Fix::Nano => {
                    if let Some(rest) = s.strip_prefix('.') {
                        let (rest, v) = scan_nanosecond(rest)?;
                        s = rest;
                        parsed.set_nanosecond(v)?;
                    }
                }
                Fix::Nano3 | Fix::Nano6 | Fix::Nano9 => {
                    let digits = match kind {
                        Fix::Nano3 => 3,
                        Fix::Nano6 => 6,
                        _ => 9,
                    };
                    if let Some(rest) = s.strip_prefix('.') {
                        let (rest, v) = scan_nanosecond_fixed(rest, digits)?;
                        s = rest;
                        parsed.set_nanosecond(v)?;
                    }
                }
                Fix::Nano3NoDot | Fix::Nano6NoDot | Fix::Nano9NoDot => {
                    let digits = match kind {
                        Fix::Nano3NoDot => 3,
                        Fix::Nano6NoDot => 6,
                        _ => 9,
                    };
                    if s.len() < digits {
                        return Err(());
                    }
                    let (rest, v) = scan_nanosecond_fixed(s, digits)?;
                    s = rest;
                    parsed.set_nanosecond(v)?;
                }
                Fix::TzName => {
                    s = s.trim_start_matches(|c: char| !c.is_whitespace());
                }
                Fix::Offset
                | Fix::OffsetColon
                | Fix::OffsetDoubleColon
                | Fix::OffsetTripleColon => {
                    let (rest, off) =
                        scan_timezone_offset(s.trim_start(), colon_or_space, false, false)?;
                    s = rest;
                    parsed.set_offset(i64::from(off))?;
                }
                Fix::OffsetPermissive => {
                    let (rest, off) =
                        scan_timezone_offset(s.trim_start(), colon_or_space, true, true)?;
                    s = rest;
                    parsed.set_offset(i64::from(off))?;
                }
                Fix::Rfc3339 => s = parse_rfc3339_relaxed(parsed, s)?,
            },
            Item::Error => return Err(()),
        }
    }
    Ok(s)
}

fn parse_all(text: &str, layout: &str) -> Scan<Parsed> {
    let mut parsed = Parsed::default();
    let items = strftime_items(layout);
    match parse_items(&mut parsed, text, &items)? {
        "" => Ok(parsed),
        _ => Err(()),
    }
}

/// Date, time and offset in the relaxed RFC 3339 form: numbers need no padding,
/// a space may stand between parts, `UTC` may stand for the offset, and the
/// offset's colon may be left out.
fn parse_rfc3339_relaxed<'a>(parsed: &mut Parsed, s: &'a str) -> Scan<&'a str> {
    use Num::*;
    const DATE: &[Item<'static>] = &[
        Item::Numeric(Year, Pad::Zero),
        Item::Space(""),
        Item::Literal("-"),
        Item::Numeric(Month, Pad::Zero),
        Item::Space(""),
        Item::Literal("-"),
        Item::Numeric(Day, Pad::Zero),
    ];
    const TIME: &[Item<'static>] = &[
        Item::Numeric(Hour, Pad::Zero),
        Item::Space(""),
        Item::Literal(":"),
        Item::Numeric(Minute, Pad::Zero),
        Item::Space(""),
        Item::Literal(":"),
        Item::Numeric(Second, Pad::Zero),
        Item::Fixed(Fix::Nano),
        Item::Space(""),
    ];
    let mut s = parse_items(parsed, s, DATE)?;
    s = match s.as_bytes().first() {
        Some(b't' | b'T' | b' ') => &s[1..],
        _ => return Err(()),
    };
    s = parse_items(parsed, s, TIME)?;
    s = s.trim_start();
    let (s, offset) = if s.len() >= 3 && b"UTC".eq_ignore_ascii_case(&s.as_bytes()[..3]) {
        (&s[3..], 0)
    } else {
        scan_timezone_offset(s, colon_or_space, true, false)?
    };
    parsed.set_offset(i64::from(offset))?;
    Ok(s)
}

/// An RFC 3339 date and time (`2024-02-29T10:00:00[.f]Z` or with a `+hh:mm`
/// offset, the `T` also `t` or a space), as the UTC reading of the instant.
pub fn parse_rfc3339(text: &str) -> Option<Naive> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let d = |i: usize| -> Option<u32> {
        bytes[i]
            .is_ascii_digit()
            .then(|| u32::from(bytes[i] - b'0'))
    };
    let year = d(0)? * 1000 + d(1)? * 100 + d(2)? * 10 + d(3)?;
    if bytes[4] != b'-' {
        return None;
    }
    let month = d(5)? * 10 + d(6)?;
    if bytes[7] != b'-' {
        return None;
    }
    let day = d(8)? * 10 + d(9)?;
    if !matches!(bytes[10], b't' | b'T' | b' ') {
        return None;
    }
    let hour = d(11)? * 10 + d(12)?;
    if bytes[13] != b':' {
        return None;
    }
    let minute = d(14)? * 10 + d(15)?;
    if bytes[16] != b':' {
        return None;
    }
    let second = d(17)? * 10 + d(18)?;
    let (second, extra) = if second == 60 {
        (59, 1_000_000_000)
    } else {
        (second, 0)
    };
    let mut s = &text[19..];
    let mut nano = extra;
    if s.starts_with('.') {
        let (rest, v) = scan_nanosecond(&s[1..]).ok()?;
        s = rest;
        nano += v as u32;
    }
    let (rest, offset) = scan_timezone_offset(s, require_colon, true, false).ok()?;
    if !rest.is_empty() || offset.abs() >= 86_400 {
        return None;
    }
    Naive::new(year as i32, month, day, hour, minute, second, nano)?.add_secs(-i64::from(offset))
}

/// Text in a strftime layout that carries an offset, as a UTC reading of the
/// instant; None when the text does not match or has no offset.
pub fn parse_with_offset(text: &str, layout: &str) -> Option<Naive> {
    parse_all(text, layout).ok()?.to_utc().ok()
}

/// Text in a strftime layout read as a date and time with no zone.
pub fn parse_naive_datetime(text: &str, layout: &str) -> Option<Naive> {
    parse_all(text, layout)
        .ok()?
        .to_naive_datetime_with_offset(0)
        .ok()
}

/// Text in a strftime layout read as a date alone (midnight); the layout may
/// hold other fields, which are not used.
pub fn parse_naive_date(text: &str, layout: &str) -> Option<Naive> {
    parse_all(text, layout).ok()?.to_date().ok()
}

// ───────────────────────────── instants and the clock ──────────────────────────

/// The whole seconds from `a` to `b`, truncated toward zero, the fractions of
/// a second counted.
pub fn whole_secs_between(a: &Naive, b: &Naive) -> i64 {
    let nanos = i128::from(b.unix_secs() - a.unix_secs()) * 1_000_000_000 + i128::from(b.nano)
        - i128::from(a.nano);
    (nanos / 1_000_000_000) as i64
}

/// A span of time, in the units the shell's tools count in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Duration(SignedDuration);

impl Duration {
    pub fn seconds(n: i64) -> Duration {
        Duration(SignedDuration::from_secs(n))
    }
    pub fn milliseconds(n: i64) -> Duration {
        Duration(SignedDuration::from_millis(n))
    }
    pub fn minutes(n: i64) -> Duration {
        Duration(SignedDuration::from_mins(n))
    }
    pub fn hours(n: i64) -> Duration {
        Duration(SignedDuration::from_hours(n))
    }
    pub fn days(n: i64) -> Duration {
        Duration(SignedDuration::from_hours(n.saturating_mul(24)))
    }
    /// Whole seconds, truncated toward zero.
    pub fn num_seconds(&self) -> i64 {
        self.0.as_secs()
    }
    /// Whole milliseconds, truncated toward zero.
    pub fn num_milliseconds(&self) -> i64 {
        i64::try_from(self.0.as_millis()).unwrap_or(i64::MAX)
    }
    /// Whole days, truncated toward zero.
    pub fn num_days(&self) -> i64 {
        self.0.as_secs() / 86_400
    }
}

/// An instant, kept in UTC. It is written as RFC 3339 text ending in `Z`, with
/// a fraction of 3, 6 or 9 digits only when there is one, and read from the
/// relaxed RFC 3339 forms.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UtcTime(Timestamp);

/// Written as the RFC 3339 text with `Z`, as it always was.
impl fmt::Debug for UtcTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.naive().rfc3339(0, true))
    }
}

impl UtcTime {
    /// The time now.
    pub fn now() -> UtcTime {
        UtcTime(Timestamp::now())
    }

    /// The instant `secs` seconds and `nanos` nanoseconds after the epoch.
    pub fn from_timestamp(secs: i64, nanos: u32) -> Option<UtcTime> {
        Timestamp::new(secs, i32::try_from(nanos).ok()?)
            .ok()
            .map(UtcTime)
    }

    /// The instant `ms` milliseconds after the epoch.
    pub fn from_timestamp_millis(ms: i64) -> Option<UtcTime> {
        Timestamp::from_millisecond(ms).ok().map(UtcTime)
    }

    /// Whole seconds since 1970-01-01T00:00:00Z.
    pub fn timestamp(&self) -> i64 {
        self.0.as_second()
    }

    /// Whole milliseconds since 1970-01-01T00:00:00Z.
    pub fn timestamp_millis(&self) -> i64 {
        self.0.as_millisecond()
    }

    /// Nanoseconds since 1970-01-01T00:00:00Z, None past what an i64 holds.
    pub fn timestamp_nanos_opt(&self) -> Option<i64> {
        i64::try_from(self.0.as_nanosecond()).ok()
    }

    fn naive(&self) -> Naive {
        let nanos = self.0.subsec_nanosecond();
        // A negative fraction borrows from the second.
        let (secs, nanos) = if nanos < 0 {
            (self.0.as_second() - 1, (nanos + 1_000_000_000) as u32)
        } else {
            (self.0.as_second(), nanos as u32)
        };
        Naive::from_unix(secs, nanos).expect("a Timestamp lies within the calendar")
    }

    /// The calendar date in UTC, as (year, month, day).
    pub fn date_naive(&self) -> (i32, u32, u32) {
        let n = self.naive();
        (n.year, n.month, n.day)
    }

    /// RFC 3339 text with a `+00:00` offset and the fraction it needs.
    pub fn to_rfc3339(&self) -> String {
        self.naive().rfc3339(0, false)
    }

    /// The instant in a strftime layout; the zone is named `UTC`.
    pub fn format(&self, layout: &str) -> Formatted {
        Formatted {
            text: format_naive(layout, &self.naive(), Some(&Zone::utc())),
        }
    }

    /// Time between this instant and an earlier one.
    pub fn signed_duration_since(&self, earlier: UtcTime) -> Duration {
        Duration(self.0.duration_since(earlier.0))
    }

    /// The instant in the system time zone.
    pub fn to_local(&self) -> LocalTime {
        LocalTime::at(self.0)
    }
}

impl std::ops::Add<Duration> for UtcTime {
    type Output = UtcTime;
    fn add(self, d: Duration) -> UtcTime {
        UtcTime(self.0.checked_add(d.0).unwrap_or(if d.0.is_negative() {
            Timestamp::MIN
        } else {
            Timestamp::MAX
        }))
    }
}

impl std::ops::Sub<Duration> for UtcTime {
    type Output = UtcTime;
    fn sub(self, d: Duration) -> UtcTime {
        UtcTime(self.0.checked_sub(d.0).unwrap_or(if d.0.is_negative() {
            Timestamp::MAX
        } else {
            Timestamp::MIN
        }))
    }
}

impl std::ops::Sub<UtcTime> for UtcTime {
    type Output = Duration;
    fn sub(self, other: UtcTime) -> Duration {
        self.signed_duration_since(other)
    }
}

impl From<std::time::SystemTime> for UtcTime {
    fn from(t: std::time::SystemTime) -> UtcTime {
        UtcTime(Timestamp::try_from(t).unwrap_or(Timestamp::UNIX_EPOCH))
    }
}

impl Serialize for UtcTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.naive().rfc3339(0, true))
    }
}

impl<'de> Deserialize<'de> for UtcTime {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<UtcTime, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = UtcTime;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an RFC 3339 formatted date and time string")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<UtcTime, E> {
                parse_relaxed_utc(value)
                    .ok_or_else(|| E::custom("input is not a valid RFC 3339 date and time"))
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

/// The instant of relaxed RFC 3339 text.
fn parse_relaxed_utc(text: &str) -> Option<UtcTime> {
    let mut parsed = Parsed::default();
    let rest = parse_rfc3339_relaxed(&mut parsed, text).ok()?;
    if !rest.trim_start().is_empty() {
        return None;
    }
    let n = parsed.to_utc().ok()?;
    UtcTime::from_timestamp(n.unix_secs(), n.nano.min(999_999_999))
}

/// An instant in the system time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    at: Timestamp,
    offset: i32,
}

impl LocalTime {
    fn at(at: Timestamp) -> LocalTime {
        LocalTime {
            at,
            offset: TimeZone::system().to_offset(at).seconds(),
        }
    }

    /// The time now.
    pub fn now() -> LocalTime {
        LocalTime::at(Timestamp::now())
    }

    /// The local time of `secs` seconds after the epoch.
    pub fn from_timestamp(secs: i64) -> Option<LocalTime> {
        Timestamp::new(secs, 0).ok().map(LocalTime::at)
    }

    /// Whole seconds since 1970-01-01T00:00:00Z.
    pub fn timestamp(&self) -> i64 {
        self.at.as_second()
    }

    /// Whole milliseconds since 1970-01-01T00:00:00Z.
    pub fn timestamp_millis(&self) -> i64 {
        self.at.as_millisecond()
    }

    fn naive(&self) -> Naive {
        UtcTime(self.at)
            .naive()
            .add_secs(i64::from(self.offset))
            .expect("a local time lies within the calendar")
    }

    /// The hour of the day, 0 to 23.
    pub fn hour(&self) -> u32 {
        self.naive().hour
    }

    /// RFC 3339 text with the local offset (`+hh:mm`) and the fraction it needs.
    pub fn to_rfc3339(&self) -> String {
        self.naive().rfc3339(self.offset, false)
    }

    /// RFC 3339 text to the second, with the local offset.
    pub fn to_rfc3339_secs(&self) -> String {
        let mut n = self.naive();
        n.nano = 0;
        n.rfc3339(self.offset, false)
    }

    /// The time in a strftime layout; `%Z` writes the offset as `+hh:mm`.
    pub fn format(&self, layout: &str) -> Formatted {
        Formatted {
            text: format_naive(layout, &self.naive(), Some(&Zone::fixed(self.offset))),
        }
    }

    /// Time between this instant and an earlier one.
    pub fn signed_duration_since(&self, earlier: LocalTime) -> Duration {
        Duration(self.at.duration_since(earlier.at))
    }
}

impl PartialOrd for LocalTime {
    fn partial_cmp(&self, other: &LocalTime) -> Option<std::cmp::Ordering> {
        self.at.partial_cmp(&other.at)
    }
}

/// The instant of a local date and time in the system time zone: None when the
/// time does not exist there or is ambiguous (a clock change).
pub fn local_instant(n: &Naive) -> Option<LocalTime> {
    let civil = jiff::civil::DateTime::new(
        i16::try_from(n.year).ok()?,
        i8::try_from(n.month).ok()?,
        i8::try_from(n.day).ok()?,
        i8::try_from(n.hour).ok()?,
        i8::try_from(n.minute).ok()?,
        i8::try_from(n.second).ok()?,
        i32::try_from(n.nano).ok()?,
    )
    .ok()?;
    let at = TimeZone::system()
        .to_ambiguous_timestamp(civil)
        .unambiguous()
        .ok()?;
    Some(LocalTime::at(at))
}

/// RFC 3339 text of the time now, in UTC with `Z`, to the second.
pub fn now_utc_secs() -> String {
    let mut n = UtcTime::now().naive();
    n.nano = 0;
    n.rfc3339(0, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64, nanos: u32) -> UtcTime {
        UtcTime::from_timestamp(secs, nanos).unwrap()
    }

    #[test]
    fn serde_writes_and_reads_the_text_chrono_wrote() {
        // 2026-11-08T14:03:05Z with each fraction width chrono chose.
        let secs = 1_794_146_585;
        for (nanos, text) in [
            (0, "\"2026-11-08T14:03:05Z\""),
            (120_000_000, "\"2026-11-08T14:03:05.120Z\""),
            (123_456_000, "\"2026-11-08T14:03:05.123456Z\""),
            (123_456_789, "\"2026-11-08T14:03:05.123456789Z\""),
            (5_000_000, "\"2026-11-08T14:03:05.005Z\""),
        ] {
            let t = at(secs, nanos);
            assert_eq!(serde_json::to_string(&t).unwrap(), text);
            assert_eq!(serde_json::from_str::<UtcTime>(text).unwrap(), t);
        }
        // Forms an older writer used: an offset, a space, a bare `+00:00`.
        let t = at(secs, 0);
        for text in [
            "\"2026-11-08T14:03:05+00:00\"",
            "\"2026-11-08 14:03:05Z\"",
            "\"2026-11-08T15:03:05+01:00\"",
            "\"2026-11-08T14:03:05.000000000Z\"",
        ] {
            assert_eq!(serde_json::from_str::<UtcTime>(text).unwrap(), t, "{text}");
        }
        assert!(serde_json::from_str::<UtcTime>("\"yesterday\"").is_err());
    }

    #[test]
    fn a_time_before_the_epoch_and_the_edges_of_the_calendar() {
        let t = at(-2, 500_000_000);
        assert_eq!(t.to_rfc3339(), "1969-12-31T23:59:58.500+00:00");
        assert_eq!(format!("{t:?}"), "1969-12-31T23:59:58.500Z");
        assert!(Naive::date(262_142, 12, 31).is_some());
        assert!(Naive::date(262_143, 1, 1).is_none());
        assert!(Naive::date(-262_143, 1, 1).is_some());
        assert!(Naive::date(-262_144, 1, 1).is_none());
        assert!(Naive::date(-1, 2, 29).is_none());
        assert!(Naive::date(0, 2, 29).is_some());
    }

    #[test]
    fn to_rfc3339_uses_plus_zero_offset_like_chrono() {
        assert_eq!(at(0, 0).to_rfc3339(), "1970-01-01T00:00:00+00:00");
        assert_eq!(
            at(0, 1_500_000).to_rfc3339(),
            "1970-01-01T00:00:00.001500+00:00"
        );
    }

    #[test]
    fn strftime_matches_chrono_on_the_layouts_the_shell_documents() {
        let t = at(1_709_201_400, 123_456_789); // 2024-02-29T10:10:00.123456789Z
        let f = |layout: &str| t.format(layout).to_string();
        assert_eq!(f("%d %B %Y"), "29 February 2024");
        assert_eq!(f("%f"), "123456789");
        assert_eq!(f("%6f"), "123456");
        assert_eq!(f("%.3f"), ".123");
        assert_eq!(f("%3f"), "123");
        assert_eq!(f("%.f"), ".123456789");
        assert_eq!(f("%e|%-d|%_m|%0e"), "29|29| 2|29");
        assert_eq!(
            f("%a %A %b %h %p %P %I %l"),
            "Thu Thursday Feb Feb AM am 10 10"
        );
        assert_eq!(
            f("%j %U %W %V %G %g %u %w %C %y"),
            "060 08 09 09 2024 24 4 4 20 24"
        );
        assert_eq!(
            f("%F %T %R %D %x %X %v"),
            "2024-02-29 10:10:00 10:10 02/29/24 02/29/24 10:10:00 29-Feb-2024"
        );
        assert_eq!(f("%c"), "Thu Feb 29 10:10:00 2024");
        assert_eq!(f("%z %:z %::z %:::z %Z"), "+0000 +00:00 +00:00:00 +00 UTC");
        assert_eq!(f("%s %q %%"), "1709201400 1 %");
        assert_eq!(f("%+"), "2024-02-29T10:10:00.123456789+00:00");
        assert!(layout_has_error("%^a"));
        assert!(layout_has_error("%L"));
        assert!(layout_has_error("%-D"));
        assert!(layout_has_error("%5Y"));
        assert!(layout_has_error("%"));
        assert!(!layout_has_error("%#z"));
    }

    #[test]
    fn layouts_are_read_as_chrono_read_them() {
        let n = parse_naive_datetime("29/02/2024 13:05", "%d/%m/%Y %H:%M").unwrap();
        assert_eq!(
            (n.year, n.month, n.day, n.hour, n.minute),
            (2024, 2, 29, 13, 5)
        );
        assert!(parse_naive_datetime("30/02/2024 13:05", "%d/%m/%Y %H:%M").is_none());
        assert!(parse_naive_date("2024", "%Y").is_none());
        let utc = parse_with_offset("2024-02-29 10:00:00 +0200", "%Y-%m-%d %H:%M:%S %z").unwrap();
        assert_eq!((utc.hour, utc.day), (8, 29));
        assert!(parse_with_offset("2024-02-29 10:00:00", "%Y-%m-%d %H:%M:%S").is_none());
        assert_eq!(
            parse_naive_date("Feb 29 2024", "%b %d %Y").map(|d| (d.year, d.month, d.day)),
            Some((2024, 2, 29))
        );
        // A full month name is read by %B, and a leap second by %S.
        assert!(parse_naive_datetime("February 29 2024 10:00:60", "%B %d %Y %H:%M:%S").is_some());
    }

    #[test]
    fn rfc3339_is_read_strictly() {
        assert_eq!(
            parse_rfc3339("2024-02-29T10:00:00+02:00").map(|n| n.hour),
            Some(8)
        );
        assert!(parse_rfc3339("2024-02-29T10:00:00+0200").is_none());
        assert!(parse_rfc3339("2024-02-29T10:00:00").is_none());
        assert!(parse_rfc3339("2024-02-30T10:00:00Z").is_none());
        assert_eq!(
            parse_rfc3339("2016-12-31T23:59:60Z").map(|n| n.nano),
            Some(1_000_000_000)
        );
    }

    #[test]
    fn calendar_helpers_agree_with_known_dates() {
        let n = Naive::date(2024, 12, 30).unwrap();
        assert_eq!(n.iso_week(), (2025, 1));
        assert_eq!(Naive::date(2021, 1, 3).unwrap().iso_week(), (2020, 53));
        let w = Naive::from_iso_week(2024, 9, 4).unwrap();
        assert_eq!((w.year, w.month, w.day), (2024, 2, 29));
        assert!(Naive::from_iso_week(2024, 53, 1).is_none());
        assert!(Naive::from_ordinal(2023, 366).is_none());
        assert_eq!(Naive::date(1970, 1, 1).unwrap().weekday_from_sun(), 4);
        assert_eq!(Naive::from_unix(-1, 0).unwrap().second, 59);
    }
}

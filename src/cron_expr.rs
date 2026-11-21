//! Cron expressions: parsing the five-field form and finding the times it fires.
//!
//! This is the schedule text itself, not the scheduler: `cron_next` and
//! `cron_parse` read an expression and answer from the calendar, and run nothing.
//! The rules follow Vixie cron:
//!
//! * the fields are minute (0-59), hour (0-23), day of month (1-31), month
//!   (1-12 or JAN-DEC) and day of week (0-6 or SUN-SAT, with 7 also Sunday);
//! * a field is a comma list of `*`, `N`, `A-B`, with an optional `/STEP`;
//!   `N/STEP` runs from N to the field's maximum;
//! * when both day of month and day of week are restricted (neither field
//!   starts with `*`), a day matches if either field matches; when one starts
//!   with `*`, both must match;
//! * `@yearly`, `@annually`, `@monthly`, `@weekly`, `@daily`, `@midnight` and
//!   `@hourly` are the usual shorthands; `@reboot` has no time and is refused.
//!
//! Times are UTC. A day-of-month that no month in the search window has (for
//! example `0 0 31 2 *`) is an error rather than a loop.

use crate::clock::Naive;

/// A parsed expression: one bit per allowed value of each field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    pub minute: u64,
    pub hour: u32,
    pub day_of_month: u32,
    pub month: u16,
    pub day_of_week: u8,
    /// The day-of-month field began with `*`.
    pub dom_star: bool,
    /// The day-of-week field began with `*`.
    pub dow_star: bool,
}

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const DAYS: [&str; 7] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

/// The longest search for a matching day, in days: a little over eight years, the
/// longest gap between two 29 Februaries (2096 to 2104).
const SEARCH_DAYS: i64 = 366 * 9;

fn value(
    token: &str,
    names: &[&str],
    first_name: u32,
    lo: u32,
    hi: u32,
    what: &str,
) -> Result<u32, String> {
    let upper = token.to_ascii_uppercase();
    if !names.is_empty() {
        if let Some(i) = names.iter().position(|n| *n == upper) {
            return Ok(first_name + i as u32);
        }
    }
    let n: u32 = token
        .parse()
        .map_err(|_| format!("`{token}` is not a number or a name in the {what} field"))?;
    if n < lo || n > hi {
        return Err(format!("{n} is outside {lo}-{hi} in the {what} field"));
    }
    Ok(n)
}

/// One field as a bit set over `lo..=hi`.
fn field(
    text: &str,
    names: &[&str],
    first_name: u32,
    lo: u32,
    hi: u32,
    what: &str,
) -> Result<u64, String> {
    if text.is_empty() {
        return Err(format!("the {what} field is empty"));
    }
    let mut bits = 0u64;
    for item in text.split(',') {
        let (range, step) = match item.split_once('/') {
            Some((r, s)) => {
                let step: u32 = s
                    .parse()
                    .map_err(|_| format!("`{s}` is not a step in the {what} field"))?;
                if step == 0 {
                    return Err(format!("a step of 0 in the {what} field"));
                }
                (r, step)
            }
            None => (item, 1),
        };
        let (from, to) = if range == "*" {
            (lo, hi)
        } else if let Some((a, b)) = range.split_once('-') {
            let a = value(a, names, first_name, lo, hi, what)?;
            let b = value(b, names, first_name, lo, hi, what)?;
            if a > b {
                return Err(format!(
                    "the range {range} runs backwards in the {what} field"
                ));
            }
            (a, b)
        } else {
            let a = value(range, names, first_name, lo, hi, what)?;
            // `5/15` runs from 5 to the maximum; a bare `5` is just 5.
            (a, if item.contains('/') { hi } else { a })
        };
        let mut v = from;
        while v <= to {
            bits |= 1u64 << v;
            v += step;
        }
    }
    Ok(bits)
}

/// Parse a five-field expression or one of the `@` shorthands.
pub fn parse(expr: &str) -> Result<Cron, String> {
    let text = expr.trim();
    let expanded;
    let text = if let Some(word) = text.strip_prefix('@') {
        expanded = match word.to_ascii_lowercase().as_str() {
            "yearly" | "annually" => "0 0 1 1 *",
            "monthly" => "0 0 1 * *",
            "weekly" => "0 0 * * 0",
            "daily" | "midnight" => "0 0 * * *",
            "hourly" => "0 * * * *",
            "reboot" => {
                return Err("@reboot runs once at start-up and has no next time".to_string())
            }
            _ => {
                return Err(format!(
                    "`@{word}` is not a cron shorthand; the shorthands are @yearly, @annually, @monthly, @weekly, @daily, @midnight, @hourly"
                ))
            }
        };
        expanded
    } else {
        text
    };
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(format!(
            "a cron expression has 5 fields (minute hour day-of-month month day-of-week), and this has {}",
            parts.len()
        ));
    }
    let minute = field(parts[0], &[], 0, 0, 59, "minute")?;
    let hour = field(parts[1], &[], 0, 0, 23, "hour")? as u32;
    let day_of_month = field(parts[2], &[], 0, 1, 31, "day-of-month")? as u32;
    let month = field(parts[3], &MONTHS, 1, 1, 12, "month")? as u16;
    let mut dow = field(parts[4], &DAYS, 0, 0, 7, "day-of-week")?;
    if dow & (1 << 7) != 0 {
        dow = (dow & !(1 << 7)) | 1; // 7 is Sunday
    }
    Ok(Cron {
        minute,
        hour,
        day_of_month,
        month,
        day_of_week: dow as u8,
        dom_star: parts[2].starts_with('*'),
        dow_star: parts[4].starts_with('*'),
    })
}

fn bits_to_vec(bits: u64, lo: u32, hi: u32) -> Vec<i64> {
    (lo..=hi)
        .filter(|v| bits & (1u64 << v) != 0)
        .map(i64::from)
        .collect()
}

impl Cron {
    pub fn minutes(&self) -> Vec<i64> {
        bits_to_vec(self.minute, 0, 59)
    }
    pub fn hours(&self) -> Vec<i64> {
        bits_to_vec(u64::from(self.hour), 0, 23)
    }
    pub fn days_of_month(&self) -> Vec<i64> {
        bits_to_vec(u64::from(self.day_of_month), 1, 31)
    }
    pub fn months(&self) -> Vec<i64> {
        bits_to_vec(u64::from(self.month), 1, 12)
    }
    pub fn days_of_week(&self) -> Vec<i64> {
        bits_to_vec(u64::from(self.day_of_week), 0, 6)
    }

    fn day_matches(&self, day: &Naive) -> bool {
        if self.month & (1 << day.month) == 0 {
            return false;
        }
        let dom = self.day_of_month & (1 << day.day) != 0;
        let dow = self.day_of_week & (1 << day.weekday_from_sun()) != 0;
        if self.dom_star || self.dow_star {
            dom && dow
        } else {
            dom || dow
        }
    }

    /// The next `n` times after `after` (strictly later, to the minute), as Unix
    /// seconds. An error when no day in the next nine years matches.
    pub fn next_times(&self, after: &Naive, n: usize) -> Result<Vec<Naive>, String> {
        let mut out = Vec::with_capacity(n);
        // The first minute strictly after `after`.
        let start = after.unix_secs().div_euclid(60) * 60 + 60;
        let first_day = start.div_euclid(86_400);
        let first_minute_of_day = (start.rem_euclid(86_400) / 60) as u32;
        let mut empty_days = 0i64;
        let mut day = first_day;
        while out.len() < n {
            if empty_days > SEARCH_DAYS {
                return Err("no day within nine years matches this expression (for example, a day-of-month that its months never have)".to_string());
            }
            let date = Naive::from_unix(day * 86_400, 0)
                .filter(|d| d.year <= 9999)
                .ok_or_else(|| "the next run is past the year 9999".to_string())?;
            let mut found = false;
            if self.day_matches(&date) {
                let from = if day == first_day {
                    first_minute_of_day
                } else {
                    0
                };
                'times: for h in 0..24u32 {
                    if self.hour & (1 << h) == 0 {
                        continue;
                    }
                    for m in 0..60u32 {
                        if h * 60 + m < from || self.minute & (1u64 << m) == 0 {
                            continue;
                        }
                        let t = Naive::from_unix(
                            day * 86_400 + i64::from(h) * 3600 + i64::from(m) * 60,
                            0,
                        )
                        .ok_or_else(|| "the next run is out of range".to_string())?;
                        out.push(t);
                        found = true;
                        if out.len() == n {
                            break 'times;
                        }
                    }
                }
            }
            empty_days = if found { 0 } else { empty_days + 1 };
            day += 1;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Naive {
        crate::clock::parse_rfc3339(text).unwrap()
    }

    fn next(expr: &str, from: &str, n: usize) -> Vec<String> {
        parse(expr)
            .unwrap()
            .next_times(&at(from), n)
            .unwrap()
            .iter()
            .map(|t| t.rfc3339(0, true))
            .collect()
    }

    #[test]
    fn steps_and_lists() {
        assert_eq!(
            next("*/15 * * * *", "2026-11-21T10:07:30Z", 3),
            [
                "2026-11-21T10:15:00Z",
                "2026-11-21T10:30:00Z",
                "2026-11-21T10:45:00Z"
            ]
        );
        assert_eq!(
            next("0 9,17 * * *", "2026-11-21T09:00:00Z", 2),
            ["2026-11-21T17:00:00Z", "2026-11-22T09:00:00Z"]
        );
        assert_eq!(parse("5/20 * * * *").unwrap().minutes(), [5, 25, 45]);
    }

    #[test]
    fn leap_day_and_month_ends() {
        assert_eq!(
            next("0 0 29 2 *", "2026-11-21T00:00:00Z", 2),
            ["2028-02-29T00:00:00Z", "2032-02-29T00:00:00Z"]
        );
        assert_eq!(
            next("0 0 29 2 *", "2096-03-01T00:00:00Z", 1),
            ["2104-02-29T00:00:00Z"]
        );
        assert_eq!(
            next("0 0 31 * *", "2026-01-31T00:00:00Z", 3),
            [
                "2026-03-31T00:00:00Z",
                "2026-05-31T00:00:00Z",
                "2026-07-31T00:00:00Z"
            ]
        );
        assert!(parse("0 0 31 2 *")
            .unwrap()
            .next_times(&at("2026-01-01T00:00:00Z"), 1)
            .is_err());
    }

    #[test]
    fn names_and_the_or_rule() {
        // 13th or a Friday.
        assert_eq!(
            next("0 0 13 * FRI", "2026-11-01T00:00:00Z", 3),
            [
                "2026-11-06T00:00:00Z",
                "2026-11-13T00:00:00Z",
                "2026-11-20T00:00:00Z"
            ]
        );
        // A star on either side means both must match: Mondays in March.
        assert_eq!(
            next("0 0 * MAR MON", "2026-11-21T00:00:00Z", 1),
            ["2027-03-01T00:00:00Z"]
        );
        assert_eq!(parse("0 0 * * 7").unwrap().days_of_week(), [0]);
        assert_eq!(parse("0 0 * jan-mar *").unwrap().months(), [1, 2, 3]);
    }

    #[test]
    fn shorthands_and_errors() {
        assert_eq!(
            next("@weekly", "2026-11-21T00:00:00Z", 1),
            ["2026-11-22T00:00:00Z"]
        );
        assert_eq!(
            next("@hourly", "2026-11-21T10:00:00Z", 1),
            ["2026-11-21T11:00:00Z"]
        );
        assert!(parse("@reboot").unwrap_err().contains("no next time"));
        assert!(parse("* * * *").is_err());
        assert!(parse("60 * * * *").is_err());
        assert!(parse("*/0 * * * *").is_err());
        assert!(parse("5-1 * * * *").is_err());
    }
}

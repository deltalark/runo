//! Playbook schedules beyond "every N minutes": a time of day on chosen weekdays, or a
//! cron expression. A time of day is stored as the cron expression it stands for.

use chrono::{Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Timelike};

/// Weekday names from Sunday, as cron counts them.
pub const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// A parsed five-field cron expression: minute, hour, day of month, month, weekday.
#[derive(Clone, Debug, PartialEq)]
pub struct Cron {
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    any_day: bool,
    any_weekday: bool,
}

/// Parse a cron expression. Fields take `*`, numbers, ranges, lists and `/steps`;
/// months and weekdays also take names. `@hourly`, `@daily`, `@weekly`, `@monthly`
/// and `@yearly` work too.
pub fn parse(expr: &str) -> Result<Cron, String> {
    let expr = match expr.trim() {
        "@hourly" => "0 * * * *",
        "@daily" | "@midnight" => "0 0 * * *",
        "@weekly" => "0 0 * * 0",
        "@monthly" => "0 0 1 * *",
        "@yearly" | "@annually" => "0 0 1 1 *",
        other => other,
    };
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return Err("A cron schedule has five fields: minute hour day month weekday.".into());
    }
    let minutes = field(fields[0], 0, 59, &[])?;
    let hours = field(fields[1], 0, 23, &[])?;
    let days = field(fields[2], 1, 31, &[])?;
    let months = field(fields[3], 1, 12, &MONTHS)?;
    let mut weekdays = field(
        fields[4],
        0,
        7,
        &["sun", "mon", "tue", "wed", "thu", "fri", "sat"],
    )?;
    // 7 is Sunday too.
    if weekdays & (1 << 7) != 0 {
        weekdays |= 1;
    }
    Ok(Cron {
        minutes,
        hours: hours as u32,
        days: days as u32,
        months: months as u16,
        weekdays: (weekdays & 0x7f) as u8,
        any_day: fields[2] == "*",
        any_weekday: fields[4] == "*",
    })
}

/// One field as a bit per allowed value.
fn field(text: &str, min: u32, max: u32, names: &[&str]) -> Result<u64, String> {
    let value = |v: &str| -> Result<u32, String> {
        let lower = v.to_ascii_lowercase();
        if let Some(i) = names.iter().position(|n| *n == lower) {
            // Names start at the field's first value: January is 1, Sunday is 0.
            return Ok(min + i as u32);
        }
        v.parse::<u32>()
            .ok()
            .filter(|n| (min..=max).contains(n))
            .ok_or_else(|| format!("“{v}” is not between {min} and {max}."))
    };
    let mut bits = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (
                r,
                s.parse::<u32>()
                    .ok()
                    .filter(|s| *s > 0)
                    .ok_or_else(|| format!("“{s}” is not a step."))?,
            ),
            None => (part, 1),
        };
        let (from, to) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (value(a)?, value(b)?)
        } else {
            let v = value(range)?;
            // `5/15` runs from 5 to the end in steps.
            (v, if part.contains('/') { max } else { v })
        };
        if from > to {
            return Err(format!("“{range}” runs backwards."));
        }
        let mut v = from;
        while v <= to {
            bits |= 1 << v;
            v += step;
        }
    }
    Ok(bits)
}

impl Cron {
    fn day_matches(&self, date: NaiveDate) -> bool {
        let dom = self.days & (1 << date.day()) != 0;
        let dow = self.weekdays & (1 << date.weekday().num_days_from_sunday()) != 0;
        // As in cron: when both are limited, either one will do.
        match (self.any_day, self.any_weekday) {
            (false, false) => dom || dow,
            _ => dom && dow,
        }
    }

    /// The first time after `after` (Unix seconds) that matches, in the time zone.
    pub fn next_after<Tz: TimeZone>(&self, after: u64, tz: &Tz) -> Option<u64> {
        let start = (after / 60 + 1) * 60;
        let mut t: NaiveDateTime = tz.timestamp_opt(start as i64, 0).single()?.naive_local();
        // Days, hours and minutes are skipped whole, so this stays short even for a
        // schedule that runs once a year; an impossible one (February 30) gives up.
        for _ in 0..100_000 {
            if self.months & (1 << t.month()) == 0 {
                let (y, m) = if t.month() == 12 {
                    (t.year() + 1, 1)
                } else {
                    (t.year(), t.month() + 1)
                };
                t = NaiveDate::from_ymd_opt(y, m, 1)?.and_hms_opt(0, 0, 0)?;
                continue;
            }
            if !self.day_matches(t.date()) {
                t = (t.date() + Duration::days(1)).and_hms_opt(0, 0, 0)?;
                continue;
            }
            if self.hours & (1 << t.hour()) == 0 {
                t = t.with_minute(0)? + Duration::hours(1);
                continue;
            }
            if self.minutes & (1 << t.minute()) == 0 {
                t += Duration::minutes(1);
                continue;
            }
            match tz.from_local_datetime(&t) {
                LocalResult::Single(at) | LocalResult::Ambiguous(at, _) => {
                    return Some(at.timestamp() as u64);
                }
                // A time skipped by a daylight-saving change.
                LocalResult::None => t += Duration::minutes(1),
            }
        }
        None
    }
}

/// The next run of a cron schedule after `after`, in local time.
pub fn next_local(expr: &str, after: u64) -> Option<u64> {
    parse(expr).ok()?.next_after(after, &chrono::Local)
}

/// How many runs fall after `from` and at or before `to`, counting at most `cap`.
pub fn runs_between(expr: &str, from: u64, to: u64, cap: u64) -> u64 {
    let Ok(cron) = parse(expr) else {
        return 0;
    };
    let mut n = 0;
    let mut at = from;
    while n < cap {
        match cron.next_after(at, &chrono::Local) {
            Some(next) if next <= to => {
                n += 1;
                at = next;
            }
            _ => break,
        }
    }
    n
}

/// The cron expression for a time of day on chosen weekdays (from Sunday). No weekday
/// chosen means every day.
pub fn at_time(hour: u32, minute: u32, weekdays: &[bool; 7]) -> String {
    let days: Vec<String> = (0..7)
        .filter(|&d| weekdays[d])
        .map(|d| d.to_string())
        .collect();
    let days = if days.is_empty() || days.len() == 7 {
        "*".to_string()
    } else {
        days.join(",")
    };
    format!("{minute} {hour} * * {days}")
}

/// The time of day and weekdays of an expression `at_time` could have made.
pub fn as_time(expr: &str) -> Option<(u32, u32, [bool; 7])> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    let [m, h, "*", "*", w] = fields.as_slice() else {
        return None;
    };
    let minute = m.parse::<u32>().ok().filter(|m| *m < 60)?;
    let hour = h.parse::<u32>().ok().filter(|h| *h < 24)?;
    let cron = parse(expr).ok()?;
    let mut days = [false; 7];
    if *w != "*" {
        for (d, day) in days.iter_mut().enumerate() {
            *day = cron.weekdays & (1 << d) != 0;
        }
    }
    Some((hour, minute, days))
}

/// "09:30" or "9:30" as hours and minutes.
pub fn parse_time(text: &str) -> Option<(u32, u32)> {
    let (h, m) = text.trim().split_once(':')?;
    let hour = h.trim().parse::<u32>().ok().filter(|h| *h < 24)?;
    let minute = m.trim().parse::<u32>().ok().filter(|m| *m < 60)?;
    Some((hour, minute))
}

/// "Weekdays at 09:00", "Mon, Thu at 18:30", or the expression itself.
pub fn describe(expr: &str) -> String {
    let Some((hour, minute, days)) = as_time(expr) else {
        return format!("Cron {}", expr.trim());
    };
    let picked: Vec<usize> = (0..7).filter(|&d| days[d]).collect();
    let when = match picked.as_slice() {
        [] => "Daily".to_string(),
        [1, 2, 3, 4, 5] => "Weekdays".to_string(),
        [0, 6] => "Weekends".to_string(),
        some => some
            .iter()
            .map(|&d| WEEKDAYS[d])
            .collect::<Vec<_>>()
            .join(", "),
    };
    format!("{when} at {hour:02}:{minute:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> u64 {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp() as u64
    }

    #[test]
    fn finds_the_next_run() {
        // Monday 13 Oct 2025, 10:00 UTC.
        let now = at(2025, 10, 13, 10, 0);
        let weekdays = parse("30 9 * * mon-fri").unwrap();
        assert_eq!(
            weekdays.next_after(now, &Utc),
            Some(at(2025, 10, 14, 9, 30))
        );
        // Friday evening goes to Monday.
        let friday = at(2025, 10, 17, 18, 0);
        assert_eq!(
            weekdays.next_after(friday, &Utc),
            Some(at(2025, 10, 20, 9, 30))
        );
        let quarter = parse("*/15 * * * *").unwrap();
        assert_eq!(
            quarter.next_after(now, &Utc),
            Some(at(2025, 10, 13, 10, 15))
        );
        // Strictly after: a run at this very minute is not the next one.
        assert_eq!(
            parse("0 10 * * *").unwrap().next_after(now, &Utc),
            Some(at(2025, 10, 14, 10, 0))
        );
        assert_eq!(
            parse("@monthly").unwrap().next_after(now, &Utc),
            Some(at(2025, 11, 1, 0, 0))
        );
        assert_eq!(
            parse("0 8 29 feb *").unwrap().next_after(now, &Utc),
            Some(at(2028, 2, 29, 8, 0))
        );
        assert_eq!(parse("0 0 30 2 *").unwrap().next_after(now, &Utc), None);
        // Day of month or weekday, as cron does when both are set.
        assert_eq!(
            parse("0 12 1 * sun").unwrap().next_after(now, &Utc),
            Some(at(2025, 10, 19, 12, 0))
        );
        assert_eq!(
            parse("0 0 * * 7").unwrap().next_after(now, &Utc),
            Some(at(2025, 10, 19, 0, 0))
        );
    }

    #[test]
    fn rejects_bad_expressions() {
        assert!(parse("* * * *").is_err());
        assert!(parse("60 * * * *").is_err());
        assert!(parse("5-1 * * * *").is_err());
        assert!(parse("*/0 * * * *").is_err());
        assert!(parse("0 9 * * funday").is_err());
        assert!(parse("0 9 1-15/2 jan,jul *").is_ok());
    }

    #[test]
    fn times_of_day_round_trip() {
        let mut days = [false; 7];
        days[1..6].fill(true);
        let expr = at_time(9, 5, &days);
        assert_eq!(expr, "5 9 * * 1,2,3,4,5");
        assert_eq!(as_time(&expr), Some((9, 5, days)));
        assert_eq!(describe(&expr), "Weekdays at 09:05");
        assert_eq!(at_time(18, 30, &[false; 7]), "30 18 * * *");
        assert_eq!(describe("30 18 * * *"), "Daily at 18:30");
        assert_eq!(describe("0 7 * * 1,4"), "Mon, Thu at 07:00");
        assert_eq!(describe("0 */2 * * *"), "Cron 0 */2 * * *");
        assert_eq!(as_time("0 */2 * * *"), None);
        assert_eq!(parse_time(" 7:45"), Some((7, 45)));
        assert_eq!(parse_time("24:00"), None);
    }
}

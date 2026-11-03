//! `datetime.fromtimestamp(ts).strftime(fmt)` for the viewer's timestamp
//! prefix: local time, the C locale's names, glibc's conversions.

use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Timelike};

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
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

/// A local timestamp with microseconds (naive, like `fromtimestamp`).
pub struct Stamp {
    local: NaiveDateTime,
    micros: u32,
    epoch: i64,
}

impl Stamp {
    /// `datetime.fromtimestamp(seconds)`: floats round half-even to the
    /// microsecond; years outside 1..=9999 are an error.
    pub fn from_float(value: f64) -> Option<Stamp> {
        if !value.is_finite() {
            return None;
        }
        let mut whole = value.trunc();
        let mut micros = ((value - whole) * 1e6).round_ties_even();
        if micros >= 1e6 {
            micros -= 1e6;
            whole += 1.0;
        } else if micros < 0.0 {
            micros += 1e6;
            whole -= 1.0;
        }
        if !(-9.2e18..9.2e18).contains(&whole) {
            return None;
        }
        Stamp::from_parts(whole as i64, micros as u32)
    }

    pub fn from_parts(seconds: i64, micros: u32) -> Option<Stamp> {
        let local: DateTime<Local> = Local.timestamp_opt(seconds, micros * 1000).earliest()?;
        let naive = local.naive_local();
        if !(1..=9999).contains(&naive.year()) {
            return None;
        }
        Some(Stamp {
            local: naive,
            micros,
            epoch: seconds,
        })
    }

    pub fn format(&self, fmt: &str) -> String {
        let t = &self.local;
        let mut out = String::new();
        let mut chars = fmt.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            let Some(mut spec) = chars.next() else {
                out.push('%');
                break;
            };
            // %E and %O modifiers change nothing in the C locale.
            let mut modifier = None;
            if (spec == 'E' || spec == 'O') && chars.peek().is_some() {
                modifier = Some(spec);
                spec = chars.next().unwrap_or(spec);
            }
            let hour12 = match t.hour() % 12 {
                0 => 12,
                h => h,
            };
            let weekday = t.weekday().num_days_from_sunday() as usize;
            let month = t.month0() as usize;
            let iso = t.iso_week();
            match spec {
                'a' => out.push_str(&DAYS[weekday][..3]),
                'A' => out.push_str(DAYS[weekday]),
                'b' | 'h' => out.push_str(&MONTHS[month][..3]),
                'B' => out.push_str(MONTHS[month]),
                'c' => out.push_str(&self.format("%a %b %e %H:%M:%S %Y")),
                'C' => out.push_str(&format!("{:02}", t.year().div_euclid(100))),
                'd' => out.push_str(&format!("{:02}", t.day())),
                'D' => out.push_str(&self.format("%m/%d/%y")),
                'e' => out.push_str(&format!("{:>2}", t.day())),
                'f' if modifier.is_none() => out.push_str(&format!("{:06}", self.micros)),
                'F' => out.push_str(&self.format("%Y-%m-%d")),
                'g' => out.push_str(&format!("{:02}", iso.year().rem_euclid(100))),
                'G' => out.push_str(&iso.year().to_string()),
                'H' => out.push_str(&format!("{:02}", t.hour())),
                'I' => out.push_str(&format!("{hour12:02}")),
                'j' => out.push_str(&format!("{:03}", t.ordinal())),
                'k' => out.push_str(&format!("{:>2}", t.hour())),
                'l' => out.push_str(&format!("{hour12:>2}")),
                'm' => out.push_str(&format!("{:02}", t.month())),
                'M' => out.push_str(&format!("{:02}", t.minute())),
                'n' => out.push('\n'),
                'p' => out.push_str(if t.hour() < 12 { "AM" } else { "PM" }),
                'P' => out.push_str(if t.hour() < 12 { "am" } else { "pm" }),
                'r' => out.push_str(&self.format("%I:%M:%S %p")),
                'R' => out.push_str(&self.format("%H:%M")),
                's' => out.push_str(&self.epoch.to_string()),
                'S' => out.push_str(&format!("{:02}", t.second())),
                't' => out.push('\t'),
                'T' => out.push_str(&self.format("%H:%M:%S")),
                'u' => out.push_str(&t.weekday().number_from_monday().to_string()),
                'U' => {
                    let week = (t.ordinal0() as usize + 7 - weekday) / 7;
                    out.push_str(&format!("{week:02}"));
                }
                'V' => out.push_str(&format!("{:02}", iso.week())),
                'w' => out.push_str(&weekday.to_string()),
                'W' => {
                    let monday_based = t.weekday().num_days_from_monday() as usize;
                    let week = (t.ordinal0() as usize + 7 - monday_based) / 7;
                    out.push_str(&format!("{week:02}"));
                }
                'x' => out.push_str(&self.format("%m/%d/%y")),
                'X' => out.push_str(&self.format("%H:%M:%S")),
                'y' => out.push_str(&format!("{:02}", t.year().rem_euclid(100))),
                'Y' => out.push_str(&t.year().to_string()),
                // A naive datetime has no offset or zone name.
                'z' | 'Z' if modifier.is_none() => {}
                '%' if modifier.is_none() => out.push('%'),
                ':' if modifier.is_none() && chars.peek() == Some(&'z') => {
                    chars.next();
                }
                other => {
                    // glibc copies an unknown conversion verbatim.
                    out.push('%');
                    if let Some(m) = modifier {
                        out.push(m);
                    }
                    out.push(other);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graded_specifiers_in_utc() {
        std::env::set_var("TZ", "UTC");
        let stamp = Stamp::from_float(1577836861.123456).unwrap();
        assert_eq!(
            stamp.format("%H %M %S %Y %m %d %a %b %p %%"),
            "00 01 01 2020 01 01 Wed Jan AM %"
        );
        assert_eq!(stamp.format("%Y-%m-%d %H:%M"), "2020-01-01 00:01");
        assert_eq!(stamp.format("TS"), "TS");
    }
}

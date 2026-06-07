//! Cron-like scheduling: parse expressions like `"0 18 * * *"`, compute next fire time,
//! support day-of-week and month fields.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// Parsed cron expression with five fields: minute, hour, day-of-month, month, day-of-week.
#[derive(Debug, Clone)]
pub struct CronExpr {
    minute: CronField,
    hour: CronField,
    day_of_month: CronField,
    month: CronField,
    day_of_week: CronField,
}

/// A single cron field representing a set of allowed values.
#[derive(Debug, Clone)]
enum CronField {
    All,
    Exact(u32),
    List(Vec<u32>),
    Range(u32, u32),
    Step(u32, u32), // start, step
}

impl CronField {
    fn matches(&self, value: u32) -> bool {
        match self {
            CronField::All => true,
            CronField::Exact(v) => value == *v,
            CronField::List(vs) => vs.contains(&value),
            CronField::Range(lo, hi) => value >= *lo && value <= *hi,
            CronField::Step(start, step) => {
                if value < *start {
                    return false;
                }
                (value - start).is_multiple_of(*step)
            }
        }
    }

    /// Returns the smallest value >= `from` that matches, or None.
    #[allow(dead_code)]
    fn next_match(&self, from: u32, max: u32) -> Option<u32> {
        (from..=max).find(|&v| self.matches(v))
    }
}

/// Errors that can occur during cron parsing.
#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    InvalidFieldCount(usize),
    InvalidNumber(String),
    InvalidRange(String),
    InvalidStep(String),
    OutOfBounds { field: String, value: u32, min: u32, max: u32 },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::InvalidFieldCount(n) => write!(f, "expected 5 fields, got {}", n),
            ParseError::InvalidNumber(s) => write!(f, "invalid number: {}", s),
            ParseError::InvalidRange(s) => write!(f, "invalid range: {}", s),
            ParseError::InvalidStep(s) => write!(f, "invalid step: {}", s),
            ParseError::OutOfBounds { field, value, min, max } => {
                write!(f, "field '{}' value {} out of bounds [{}, {}]", field, value, min, max)
            }
        }
    }
}

impl std::error::Error for ParseError {}

fn parse_field(token: &str, field_name: &str, min: u32, max: u32) -> Result<CronField, ParseError> {
    if token == "*" {
        return Ok(CronField::All);
    }

    // Step: */N or A-B/N or N/N
    if let Some(slash_pos) = token.rfind('/') {
        let base = &token[..slash_pos];
        let step_str = &token[slash_pos + 1..];
        let step: u32 = step_str.parse().map_err(|_| ParseError::InvalidStep(step_str.to_string()))?;
        if step == 0 {
            return Err(ParseError::InvalidStep(step_str.to_string()));
        }
        let start = if base == "*" {
            min
        } else {
            base.parse().map_err(|_| ParseError::InvalidNumber(base.to_string()))?
        };
        return Ok(CronField::Step(start, step));
    }

    // Range: A-B
    if let Some(dash_pos) = token.find('-') {
        let lo: u32 = token[..dash_pos].parse().map_err(|_| ParseError::InvalidRange(token.to_string()))?;
        let hi: u32 = token[dash_pos + 1..].parse().map_err(|_| ParseError::InvalidRange(token.to_string()))?;
        if lo < min || hi > max || lo > hi {
            return Err(ParseError::InvalidRange(token.to_string()));
        }
        return Ok(CronField::Range(lo, hi));
    }

    // List: comma-separated
    if token.contains(',') {
        let mut values = Vec::new();
        for part in token.split(',') {
            let v: u32 = part.parse().map_err(|_| ParseError::InvalidNumber(part.to_string()))?;
            if v < min || v > max {
                return Err(ParseError::OutOfBounds {
                    field: field_name.to_string(),
                    value: v,
                    min,
                    max,
                });
            }
            values.push(v);
        }
        return Ok(CronField::List(values));
    }

    // Single exact value
    let v: u32 = token.parse().map_err(|_| ParseError::InvalidNumber(token.to_string()))?;
    if v < min || v > max {
        return Err(ParseError::OutOfBounds {
            field: field_name.to_string(),
            value: v,
            min,
            max,
        });
    }
    Ok(CronField::Exact(v))
}

impl CronExpr {
    /// Parse a cron expression string with 5 fields: minute hour day-of-month month day-of-week.
    /// Example: `"0 18 * * *"` fires at 18:00 every day.
    pub fn parse(expr: &str) -> Result<Self, ParseError> {
        let tokens: Vec<&str> = expr.split_whitespace().collect();
        if tokens.len() != 5 {
            return Err(ParseError::InvalidFieldCount(tokens.len()));
        }
        Ok(CronExpr {
            minute: parse_field(tokens[0], "minute", 0, 59)?,
            hour: parse_field(tokens[1], "hour", 0, 23)?,
            day_of_month: parse_field(tokens[2], "day-of-month", 1, 31)?,
            month: parse_field(tokens[3], "month", 1, 12)?,
            day_of_week: parse_field(tokens[4], "day-of-week", 0, 6)?,
        })
    }

    /// Check whether a given timestamp (seconds since epoch) matches this cron expression.
    pub fn matches(&self, timestamp_secs: u64) -> bool {
        let secs = timestamp_secs as i64;
        // Calculate date components from unix timestamp
        let days = secs / 86400;
        let time_secs = secs % 86400;
        let hour = (time_secs / 3600) as u32;
        let minute = ((time_secs % 3600) / 60) as u32;

        // Day of week: 1970-01-01 was Thursday. 0=Sunday, 1=Monday, ..., 6=Saturday
        // (days + 4) % 7 gives: day0=Thu=4, day3=Sun=0, day4=Mon=1 — correct.
        let dow = ((days + 4) % 7) as u32;

        // Simple date calculation
        let (_year, month, day) = days_to_date(days);

        self.minute.matches(minute)
            && self.hour.matches(hour)
            && self.day_of_month.matches(day as u32)
            && self.month.matches(month as u32)
            && self.day_of_week.matches(dow)
    }

    /// Compute the next fire time at or after `after` (seconds since epoch).
    /// Returns `None` if no matching time is found within a reasonable window (~4 years).
    pub fn next_after(&self, after: u64) -> Option<u64> {
        // Brute-force: scan minute-by-minute for up to 4 years (~2.1M minutes)
        let start = (after / 60) * 60; // Round down to minute
        for offset in 0..2_200_000 {
            let candidate = start + offset * 60;
            if candidate > after && self.matches(candidate) {
                return Some(candidate);
            }
        }
        None
    }
}

/// Convert days since epoch to (year, month, day).
fn days_to_date(mut days: i64) -> (i64, i64, i64) {
    let mut year = 1970;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let month_days: [i64; 12] = if leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 0;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    (year, month + 1, days + 1)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Convenience: get current time as seconds since epoch.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wildcard_all() {
        let expr = CronExpr::parse("* * * * *").unwrap();
        assert!(expr.minute.matches(0));
        assert!(expr.minute.matches(59));
        assert!(expr.hour.matches(0));
        assert!(expr.hour.matches(23));
    }

    #[test]
    fn parse_exact_value() {
        let expr = CronExpr::parse("30 6 * * *").unwrap();
        assert!(expr.minute.matches(30));
        assert!(!expr.minute.matches(29));
        assert!(expr.hour.matches(6));
        assert!(!expr.hour.matches(7));
    }

    #[test]
    fn parse_list() {
        let expr = CronExpr::parse("0,15,30,45 * * * *").unwrap();
        assert!(expr.minute.matches(0));
        assert!(expr.minute.matches(15));
        assert!(expr.minute.matches(30));
        assert!(expr.minute.matches(45));
        assert!(!expr.minute.matches(10));
    }

    #[test]
    fn parse_range() {
        let expr = CronExpr::parse("0 9-17 * * *").unwrap();
        assert!(expr.hour.matches(9));
        assert!(expr.hour.matches(12));
        assert!(expr.hour.matches(17));
        assert!(!expr.hour.matches(8));
        assert!(!expr.hour.matches(18));
    }

    #[test]
    fn parse_step() {
        let expr = CronExpr::parse("*/15 * * * *").unwrap();
        assert!(expr.minute.matches(0));
        assert!(expr.minute.matches(15));
        assert!(expr.minute.matches(30));
        assert!(expr.minute.matches(45));
        assert!(!expr.minute.matches(5));
    }

    #[test]
    fn parse_step_with_start() {
        let expr = CronExpr::parse("10/10 * * * *").unwrap();
        assert!(expr.minute.matches(10));
        assert!(expr.minute.matches(20));
        assert!(expr.minute.matches(30));
        assert!(!expr.minute.matches(0));
        assert!(!expr.minute.matches(5));
    }

    #[test]
    fn reject_wrong_field_count() {
        assert_eq!(
            CronExpr::parse("0 18 * *").unwrap_err(),
            ParseError::InvalidFieldCount(4)
        );
        assert_eq!(
            CronExpr::parse("0 18 * * * extra").unwrap_err(),
            ParseError::InvalidFieldCount(6)
        );
    }

    #[test]
    fn reject_out_of_bounds() {
        let err = CronExpr::parse("60 0 * * *").unwrap_err();
        assert!(matches!(err, ParseError::OutOfBounds { field, .. } if field == "minute"));

        let err = CronExpr::parse("0 24 * * *").unwrap_err();
        assert!(matches!(err, ParseError::OutOfBounds { field, .. } if field == "hour"));
    }

    #[test]
    fn matches_specific_time() {
        // 2025-01-01 00:00:00 UTC = 1735689600
        // 2025-01-01 18:00:00 UTC = 1735689600 + 64800 = 1735754400
        let expr = CronExpr::parse("0 18 * * *").unwrap();
        assert!(expr.matches(1735754400)); // 2025-01-01 18:00 UTC
        assert!(!expr.matches(1735754460)); // 18:01
        assert!(!expr.matches(1735732800)); // 12:00
    }

    #[test]
    fn next_after_finds_next_occurrence() {
        let expr = CronExpr::parse("0 18 * * *").unwrap();
        // 2025-01-01 12:00 UTC = 1735732800
        let next = expr.next_after(1735732800);
        assert!(next.is_some());
        let next = next.unwrap();
        // Should be 2025-01-01 18:00 UTC = 1735754400
        assert!(expr.matches(next));
        assert!(next > 1735732800);
    }

    #[test]
    fn next_after_skips_current() {
        let expr = CronExpr::parse("0 18 * * *").unwrap();
        // Exactly at 18:00 - should find next day's 18:00
        let at = 1735754400u64; // 2025-01-01 18:00 UTC
        let next = expr.next_after(at);
        assert!(next.is_some());
        assert!(next.unwrap() > at);
    }

    #[test]
    fn day_of_week_filter() {
        // Monday only (day_of_week=1): "0 12 * * 1"
        let expr = CronExpr::parse("0 12 * * 1").unwrap();
        // 2025-01-06 is a Monday, 12:00 UTC
        assert!(expr.matches(1736164800));
        // 2025-01-07 is a Tuesday, 12:00 UTC
        assert!(!expr.matches(1736251200));
    }

    #[test]
    fn month_filter() {
        // Only January: "0 0 1 1 *"
        let expr = CronExpr::parse("0 0 1 1 *").unwrap();
        // 2025-01-01 00:00 UTC = 1735689600
        assert!(expr.matches(1735689600));
        // 2025-02-01 00:00 UTC ≈ 1738368000
        assert!(!expr.matches(1738368000));
    }
}

//! `<time>` elements as the API's create pipeline generates them (DM§6.2 step 11,
//! text-and-content §7), and the compact Markdown form `📅 YYYY-MM-DD[ HH:MM]` (§4.2).

use super::node::{El, N};

/// The date emoji that starts the compact form.
pub(crate) const CALENDAR: char = '📅';

/// A date (and optional time) that a `<time>` element was generated from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Date {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub time: Option<(u32, u32)>,
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn is_leap(y: u32) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar (H. Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

impl Date {
    /// Valid as the API converts it: years 1–9999, a real calendar day, hour < 24,
    /// minute < 60 (DM§6.2 step 11).
    pub(crate) fn is_valid(&self) -> bool {
        (1..=9999).contains(&self.year)
            && (1..=12).contains(&self.month)
            && (1..=days_in_month(self.year, self.month)).contains(&self.day)
            && self.time.is_none_or(|(h, m)| h < 24 && m < 60)
    }

    /// The element text: `Thu, Jan 15, 2026`, plus ` at 2:30 PM` with a time; English
    /// abbreviations, unpadded day and year, 12-hour clock (0 -> 12 AM, 12 -> 12 PM).
    /// UNVERIFIED: locale and 24-hour account settings (§13 Q3) are not modelled.
    pub(crate) fn text(&self) -> String {
        let days = days_from_civil(self.year.into(), self.month.into(), self.day.into());
        // 1970-01-01 was a Thursday (index 3 with Monday = 0).
        let wd = (days + 3).rem_euclid(7) as usize;
        let mut s = format!(
            "{}, {} {}, {}",
            WEEKDAYS[wd],
            MONTHS[self.month as usize - 1],
            self.day,
            self.year
        );
        if let Some((h, m)) = self.time {
            let h12 = if h % 12 == 0 { 12 } else { h % 12 };
            let ampm = if h < 12 { "AM" } else { "PM" };
            s.push_str(&format!(" at {h12}:{m:02} {ampm}"));
        }
        s
    }

    /// The attributes, in the API's order, unpadded.
    pub(crate) fn attrs(&self) -> Vec<(String, String)> {
        let mut a = vec![
            ("startYear".to_owned(), self.year.to_string()),
            ("startMonth".to_owned(), self.month.to_string()),
            ("startDay".to_owned(), self.day.to_string()),
        ];
        if let Some((h, m)) = self.time {
            a.push(("startHour".to_owned(), h.to_string()));
            a.push(("startMinute".to_owned(), m.to_string()));
        }
        a
    }

    /// The `<time>` node the API would generate.
    pub(crate) fn node(&self) -> N {
        N::El(El::Time(self.attrs()), vec![N::Text(self.text())])
    }

    /// The compact Markdown form: `📅 2026-01-15` or `📅 2026-01-15 14:30`.
    pub(crate) fn compact(&self) -> String {
        let mut s = format!(
            "{CALENDAR} {:04}-{:02}-{:02}",
            self.year, self.month, self.day
        );
        if let Some((h, m)) = self.time {
            s.push_str(&format!(" {h:02}:{m:02}"));
        }
        s
    }

    /// The date a `<time>` element was generated from, if its attributes and text are
    /// exactly what the API generates (only then is the compact form exact, §4.2).
    pub(crate) fn of_element(attrs: &[(String, String)], children: &[N]) -> Option<Date> {
        fn num(v: &str) -> Option<u32> {
            let ok = !v.is_empty()
                && v.len() <= 4
                && v.bytes().all(|c| c.is_ascii_digit())
                && (v == "0" || !v.starts_with('0'));
            ok.then(|| v.parse().ok()).flatten()
        }
        let names: Vec<&str> = attrs.iter().map(|(k, _)| k.as_str()).collect();
        let time = match names.as_slice() {
            ["startYear", "startMonth", "startDay"] => None,
            [
                "startYear",
                "startMonth",
                "startDay",
                "startHour",
                "startMinute",
            ] => Some((num(&attrs[3].1)?, num(&attrs[4].1)?)),
            _ => return None,
        };
        let d = Date {
            year: num(&attrs[0].1)?,
            month: num(&attrs[1].1)?,
            day: num(&attrs[2].1)?,
            time,
        };
        if !d.is_valid() {
            return None;
        }
        match children {
            [N::Text(t)] if *t == d.text() => Some(d),
            _ => None,
        }
    }
}

fn digits(s: &str, min: usize, max: usize) -> Option<(u32, usize)> {
    let n = s.bytes().take(max).take_while(u8::is_ascii_digit).count();
    (n >= min)
        .then(|| s[..n].parse().ok().map(|v| (v, n)))
        .flatten()
}

/// Parse the compact form at the start of `s`: `📅 ` + `YYYY-MM-DD` (ASCII digits),
/// optionally followed by ` H:MM` or ` HH:MM`. An invalid date is not a date; an invalid
/// time leaves the date alone (the time stays literal text). Returns the date and the
/// bytes consumed.
pub(crate) fn parse_compact(s: &str) -> Option<(Date, usize)> {
    let rest = s.strip_prefix(CALENDAR)?.strip_prefix(' ')?;
    let mut pos = s.len() - rest.len();
    let b = s.as_bytes();
    let (year, n) = digits(&s[pos..], 4, 4)?;
    pos += n;
    if b.get(pos) != Some(&b'-') {
        return None;
    }
    let (month, n) = digits(&s[pos + 1..], 2, 2)?;
    pos += 1 + n;
    if b.get(pos) != Some(&b'-') {
        return None;
    }
    let (day, n) = digits(&s[pos + 1..], 2, 2)?;
    pos += 1 + n;
    let date = Date {
        year,
        month,
        day,
        time: None,
    };
    if !date.is_valid() {
        return None;
    }
    if b.get(pos) == Some(&b' ')
        && let Some((h, n)) = digits(&s[pos + 1..], 1, 2)
    {
        let p2 = pos + 1 + n;
        if b.get(p2) == Some(&b':')
            && let Some((m, n2)) = digits(&s[p2 + 1..], 2, 2)
        {
            let with = Date {
                time: Some((h, m)),
                ..date
            };
            if with.is_valid() {
                return Some((with, p2 + 1 + n2));
            }
        }
    }
    Some((date, pos))
}

/// True if `s` starts like a time that would be read as part of a preceding date-only
/// compact form (` H:MM` / ` HH:MM`).
pub(crate) fn starts_like_time(s: &str) -> bool {
    let Some(rest) = s.strip_prefix(' ') else {
        return false;
    };
    let Some((_, n)) = digits(rest, 1, 2) else {
        return false;
    };
    rest.as_bytes().get(n) == Some(&b':') && digits(&rest[n + 1..], 2, 2).is_some()
}

/// True if `s` (starting right after a `📅`) looks like the rest of a compact date, so a
/// literal `📅` before it must be escaped.
pub(crate) fn looks_like_compact_tail(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 11
        && b[0] == b' '
        && b[1..5].iter().all(u8::is_ascii_digit)
        && b[5] == b'-'
        && b[6..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'-'
        && b[9..11].iter().all(u8::is_ascii_digit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: u32, month: u32, day: u32, time: Option<(u32, u32)>) -> Date {
        Date {
            year,
            month,
            day,
            time,
        }
    }

    #[test]
    fn texts_match_the_api() {
        // text-and-content §7 and DM§6.2 step 11.
        assert_eq!(d(2026, 1, 15, None).text(), "Thu, Jan 15, 2026");
        assert_eq!(
            d(2026, 1, 15, Some((14, 30))).text(),
            "Thu, Jan 15, 2026 at 2:30 PM"
        );
        assert_eq!(
            d(2026, 1, 15, Some((0, 0))).text(),
            "Thu, Jan 15, 2026 at 12:00 AM"
        );
        assert_eq!(
            d(2026, 1, 15, Some((12, 0))).text(),
            "Thu, Jan 15, 2026 at 12:00 PM"
        );
        assert_eq!(
            d(2026, 1, 15, Some((9, 5))).text(),
            "Thu, Jan 15, 2026 at 9:05 AM"
        );
        assert_eq!(
            d(2026, 1, 15, Some((23, 59))).text(),
            "Thu, Jan 15, 2026 at 11:59 PM"
        );
        assert_eq!(d(2026, 1, 5, None).text(), "Mon, Jan 5, 2026");
        assert_eq!(d(2024, 2, 29, None).text(), "Thu, Feb 29, 2024");
        assert_eq!(d(1999, 12, 31, None).text(), "Fri, Dec 31, 1999");
        assert_eq!(d(9999, 12, 31, None).text(), "Fri, Dec 31, 9999");
        assert_eq!(d(1, 1, 1, None).text(), "Mon, Jan 1, 1");
        assert_eq!(d(2026, 7, 5, None).text(), "Sun, Jul 5, 2026");
        assert_eq!(d(2026, 9, 1, None).text(), "Tue, Sep 1, 2026");
    }

    #[test]
    fn validity() {
        assert!(!d(2025, 2, 29, None).is_valid());
        assert!(!d(2026, 4, 31, None).is_valid());
        assert!(!d(2026, 13, 1, None).is_valid());
        assert!(!d(0, 1, 1, None).is_valid());
        assert!(!d(2026, 1, 15, Some((24, 0))).is_valid());
        assert!(!d(2026, 1, 15, Some((12, 60))).is_valid());
        assert!(d(2000, 2, 29, None).is_valid());
        assert!(!d(1900, 2, 29, None).is_valid());
    }

    #[test]
    fn compact_forms() {
        assert_eq!(d(2026, 1, 15, None).compact(), "📅 2026-01-15");
        assert_eq!(d(1, 1, 1, Some((9, 5))).compact(), "📅 0001-01-01 09:05");
        assert_eq!(
            parse_compact("📅 2026-01-15 x"),
            Some((d(2026, 1, 15, None), "📅 2026-01-15".len()))
        );
        assert_eq!(
            parse_compact("📅 2026-01-15 9:05!"),
            Some((d(2026, 1, 15, Some((9, 5))), "📅 2026-01-15 9:05".len()))
        );
        assert_eq!(
            parse_compact("📅 2026-01-15 25:00"),
            Some((d(2026, 1, 15, None), "📅 2026-01-15".len()))
        );
        assert_eq!(parse_compact("📅 2026-02-30"), None);
        assert_eq!(parse_compact("📅 2026-1-5"), None);
        assert_eq!(parse_compact("📅2026-01-15"), None);
        assert_eq!(parse_compact("📅 ٢٠٢٦-01-15"), None);
        assert!(starts_like_time(" 14:30"));
        assert!(starts_like_time(" 1:30 x"));
        assert!(!starts_like_time(" 14:3"));
        assert!(!starts_like_time("14:30"));
    }

    #[test]
    fn element_matching() {
        let date = d(2026, 1, 15, Some((14, 30)));
        let N::El(El::Time(attrs), kids) = date.node() else {
            unreachable!()
        };
        assert_eq!(Date::of_element(&attrs, &kids), Some(date));
        let mut padded = attrs.clone();
        padded[1].1 = "01".to_owned();
        assert_eq!(Date::of_element(&padded, &kids), None);
        assert_eq!(
            Date::of_element(&attrs, &[N::Text("Thursday".to_owned())]),
            None
        );
        let mut reordered = attrs.clone();
        reordered.swap(0, 1);
        assert_eq!(Date::of_element(&reordered, &kids), None);
    }
}

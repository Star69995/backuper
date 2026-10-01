//! Next-occurrence math for task schedules (local time, DST-safe).

use crate::model::Schedule;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

pub fn parse_time(s: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M").map_err(|_| format!("שעה לא תקינה: {s}"))
}

pub fn parse_datetime(s: &str) -> Result<NaiveDateTime, String> {
    let s = s.trim();
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|_| format!("תאריך לא תקין: {s}"))
}

pub fn validate(s: &Schedule) -> Result<(), String> {
    match s {
        Schedule::Manual => Ok(()),
        Schedule::Once { at } => parse_datetime(at).map(|_| ()),
        Schedule::Daily { time } => parse_time(time).map(|_| ()),
        Schedule::Weekly { days, time } => {
            if days.is_empty() || days.iter().any(|d| *d > 6) {
                return Err("יש לבחור לפחות יום אחד בשבוע".into());
            }
            parse_time(time).map(|_| ())
        }
        Schedule::Monthly { day, time } => {
            if !(1..=31).contains(day) {
                return Err("יום בחודש חייב להיות בין 1 ל-31".into());
            }
            parse_time(time).map(|_| ())
        }
        Schedule::Interval { minutes } => {
            if *minutes == 0 {
                return Err("מרווח הזמן חייב להיות לפחות דקה אחת".into());
            }
            Ok(())
        }
    }
}

/// Resolve a local wall-clock time; times inside a DST gap move forward to the first valid instant.
fn to_local(naive: NaiveDateTime) -> DateTime<Local> {
    let mut n = naive;
    for _ in 0..4 {
        if let Some(dt) = Local.from_local_datetime(&n).earliest() {
            return dt;
        }
        n += Duration::minutes(30);
    }
    Local.from_utc_datetime(&naive)
}

fn last_day_of_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1).unwrap().pred_opt().unwrap().day()
}

/// First occurrence strictly after `after`, or None if the schedule never fires again.
pub fn next_after(s: &Schedule, after: DateTime<Local>) -> Option<DateTime<Local>> {
    let today = after.date_naive();
    match s {
        Schedule::Manual => None,
        Schedule::Once { at } => {
            let at = to_local(parse_datetime(at).ok()?);
            (at > after).then_some(at)
        }
        Schedule::Interval { minutes } => Some(after + Duration::minutes(*minutes as i64)),
        Schedule::Daily { time } => {
            let t = parse_time(time).ok()?;
            (0..3)
                .map(|i| to_local((today + Duration::days(i)).and_time(t)))
                .find(|dt| *dt > after)
        }
        Schedule::Weekly { days, time } => {
            let t = parse_time(time).ok()?;
            (0..9).map(|i| today + Duration::days(i)).find_map(|d| {
                let wd = d.weekday().num_days_from_sunday() as u8;
                let dt = to_local(d.and_time(t));
                (days.contains(&wd) && dt > after).then_some(dt)
            })
        }
        Schedule::Monthly { day, time } => {
            let t = parse_time(time).ok()?;
            let (mut y, mut m) = (today.year(), today.month());
            for _ in 0..14 {
                let d = (*day as u32).min(last_day_of_month(y, m));
                let dt = to_local(NaiveDate::from_ymd_opt(y, m, d)?.and_time(t));
                if dt > after {
                    return Some(dt);
                }
                if m == 12 {
                    y += 1;
                    m = 1;
                } else {
                    m += 1;
                }
            }
            None
        }
    }
}

/// The next `count` occurrences after now, for the editor preview.
pub fn preview(s: &Schedule, count: usize) -> Vec<DateTime<Local>> {
    let mut out = Vec::new();
    let mut cur = Local::now();
    while out.len() < count {
        match next_after(s, cur) {
            Some(n) => {
                out.push(n);
                cur = n;
            }
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Local> {
        to_local(parse_datetime(s).unwrap())
    }

    #[test]
    fn daily_today_or_tomorrow() {
        let s = Schedule::Daily { time: "03:00".into() };
        assert_eq!(next_after(&s, at("2026-10-01T02:00")), Some(at("2026-10-01T03:00")));
        assert_eq!(next_after(&s, at("2026-10-01T03:00")), Some(at("2026-10-02T03:00")));
    }

    #[test]
    fn weekly_picks_next_selected_day() {
        // 2026-10-01 is a Thursday (4)
        let s = Schedule::Weekly {
            days: vec![0, 2],
            time: "10:00".into(),
        };
        assert_eq!(next_after(&s, at("2026-10-01T12:00")), Some(at("2026-10-04T10:00")));
    }

    #[test]
    fn monthly_clamps_to_month_end() {
        let s = Schedule::Monthly {
            day: 31,
            time: "01:00".into(),
        };
        assert_eq!(next_after(&s, at("2026-11-01T00:00")), Some(at("2026-11-30T01:00")));
    }

    #[test]
    fn once_fires_once() {
        let s = Schedule::Once {
            at: "2026-10-05T08:30".into(),
        };
        assert_eq!(next_after(&s, at("2026-10-01T00:00")), Some(at("2026-10-05T08:30")));
        assert_eq!(next_after(&s, at("2026-10-05T08:30")), None);
    }
}

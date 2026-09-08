//! Calendar facts computed by the harness, so an assistant need not invent
//! weekday/date arithmetic. Resolve the local date before advancing calendar days.
use chrono::{DateTime, Days, Local, NaiveDate, Utc};
use chrono_tz::Tz;

pub fn render(now: DateTime<Utc>, timezone: Option<Tz>) -> String {
    let (date, mut context) = match timezone {
        Some(tz) => {
            let local = now.with_timezone(&tz);
            (local.date_naive(), format!(
                "Today is {}, and the user's timezone is {tz} (currently {}). Give times in that zone unless asked otherwise.",
                local.format("%A, %-d %B %Y"), local.format("%Z, UTC%:z")))
        }
        None => {
            let local = now.with_timezone(&Local);
            (
                local.date_naive(),
                format!("Today is {}.", local.format("%A, %-d %B %Y")),
            )
        }
    };
    context.push_str(" Work out relative dates from this calendar reference. Use its date/weekday pairs when naming near-term commitments; do not attach a conflicting weekday or relative label.\n");
    context.push_str(&calendar_reference(date));
    context
}

fn calendar_reference(date: NaiveDate) -> String {
    let mut facts = Vec::new();
    if let Some(yesterday) = date.checked_sub_days(Days::new(1)) {
        facts.push(format!("yesterday = {}", yesterday.format("%A %Y-%m-%d")));
    }
    for offset in 0..=7 {
        if let Some(day) = date.checked_add_days(Days::new(offset)) {
            let label = match offset {
                0 => "today".into(),
                1 => "tomorrow".into(),
                n => format!("in {n} days"),
            };
            facts.push(format!("{label} = {}", day.format("%A %Y-%m-%d")));
        }
    }
    format!("Calendar reference: {}.", facts.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(s: &str, tz: Tz) -> String {
        render(s.parse().unwrap(), Some(tz))
    }
    #[test]
    fn a_tuesday_followup_does_not_make_tomorrow_thursday() {
        let context = at("2026-10-13T12:00:00Z", chrono_tz::UTC);
        assert!(context.contains("yesterday = Monday 2026-10-12"));
        assert!(context.contains("tomorrow = Wednesday 2026-10-14"));
        assert!(context.contains("in 2 days = Thursday 2026-10-15"));
    }
    #[test]
    fn local_calendar_days_cross_dst_month_and_year_boundaries() {
        let context = at("2026-03-08T04:30:00Z", chrono_tz::America::New_York);
        assert!(context.contains("today = Saturday 2026-03-07"));
        assert!(context.contains("tomorrow = Sunday 2026-03-08"));
        assert!(context.contains("in 2 days = Monday 2026-03-09"));
        let context = at("2026-12-31T23:00:00Z", chrono_tz::UTC);
        assert!(context.contains("tomorrow = Friday 2027-01-01"));
        let context = at("2028-02-28T12:00:00Z", chrono_tz::UTC);
        assert!(context.contains("tomorrow = Tuesday 2028-02-29"));
        assert!(context.contains("in 2 days = Wednesday 2028-03-01"));
    }
}

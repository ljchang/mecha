//! Rendering calendar times in the user's zone.
//!
//! Both providers hand back UTC (Graph as `2026-08-04T16:00:00.0000000Z`,
//! Google as RFC 3339), and a model reading UTC reports UTC — so a noon
//! meeting is announced at four in the afternoon. It is not obviously wrong
//! on the page, which is what makes it worth fixing here rather than hoping
//! the model converts.
//!
//! The zone comes from `MECHA_TZ`, set on the server in the `[[mcp]]` block's
//! `env`, falling back to `TZ` and then to leaving the stamp alone.

use chrono_tz::Tz;

/// The zone to render in, if one is configured.
pub fn configured_zone() -> Option<Tz> {
    for var in ["MECHA_TZ", "TZ"] {
        if let Ok(name) = std::env::var(var) {
            if let Ok(tz) = name.parse::<Tz>() {
                return Some(tz);
            }
        }
    }
    None
}

/// Render one timestamp in `tz`. Returns the input unchanged when it does not
/// parse or carries no zone — an all-day event's bare `2026-08-10` is a date,
/// not an instant, and converting it would move the day.
pub fn in_zone(raw: &str, tz: Option<Tz>) -> String {
    let Some(tz) = tz else { return raw.to_string() };
    let trimmed = raw.trim();
    // Graph pads to seven fractional digits, which is valid RFC 3339.
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) else {
        return raw.to_string();
    };
    parsed.with_timezone(&tz).format("%Y-%m-%d %H:%M %Z").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eastern() -> Option<Tz> {
        Some("America/New_York".parse().unwrap())
    }

    /// The bug this exists for: a noon Eastern meeting was announced as 4pm.
    #[test]
    fn a_utc_stamp_renders_in_the_users_zone() {
        assert_eq!(in_zone("2026-08-04T16:00:00Z", eastern()), "2026-08-04 12:00 EDT");
        // Graph's seven-digit fraction must parse too.
        assert_eq!(
            in_zone("2026-08-04T16:00:00.0000000Z", eastern()),
            "2026-08-04 12:00 EDT"
        );
    }

    /// An IANA zone, not an offset, so this is right on both sides of the
    /// DST boundary.
    #[test]
    fn winter_and_summer_differ() {
        assert!(in_zone("2026-01-15T17:00:00Z", eastern()).contains("12:00 EST"));
        assert!(in_zone("2026-07-15T16:00:00Z", eastern()).contains("12:00 EDT"));
    }

    #[test]
    fn all_day_dates_and_unparseable_input_pass_through() {
        // A date is not an instant; converting it could move the day.
        assert_eq!(in_zone("2026-08-10", eastern()), "2026-08-10");
        assert_eq!(in_zone("", eastern()), "");
        // No zone configured means no change.
        assert_eq!(in_zone("2026-08-04T16:00:00Z", None), "2026-08-04T16:00:00Z");
    }
}

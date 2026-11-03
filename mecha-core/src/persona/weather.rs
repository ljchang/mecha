//! Her weather: a day's sky and temperatures where she lives, as shelf A
//! parses a fact (`docs/PERSONA-LIFE-DESIGN.md` §8.2), pulled ahead of the
//! rest of the world outside so storylets can read it (§4.3, the owner's
//! word on 2026-11-02).
//!
//! **Closed types, or refused.** The source's answer becomes a [`Sky`] from
//! a closed set and two whole degrees inside a plausible range, and nothing
//! else of it is kept: no place name, no text, no field this module does not
//! name. A code outside the set, a temperature outside the range, or a date
//! outside the window refuses that day, never guesses at it, so what reaches
//! her days carries no string the source wrote.
//!
//! **Fetched by the harness, never a model**, from one owner-written place
//! (`story.toml`'s `[place]`): one read of Open-Meteo's daily endpoint, the
//! last [`PAST_DAYS`] and the next [`AHEAD_DAYS`], capped at the door. What
//! leaves the machine is the place's latitude and longitude, nothing about
//! her or the owner.

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Days back a fetch asks for: the window her stored days cover.
pub const PAST_DAYS: u32 = 14;
/// Days ahead a fetch asks for.
pub const AHEAD_DAYS: u32 = 7;
/// The most bytes a fetch reads; the answer for three weeks is a few KB.
pub const MAX_BYTES: usize = 64 * 1024;
/// The range a temperature must fall in, in °C.
pub const COLDEST: i16 = -60;
pub const HOTTEST: i16 = 60;

/// The day's sky, from the WMO weather code the source reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sky {
    Clear,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Storm,
}

impl Sky {
    pub const ALL: [Sky; 7] = [
        Sky::Clear,
        Sky::Cloudy,
        Sky::Fog,
        Sky::Drizzle,
        Sky::Rain,
        Sky::Snow,
        Sky::Storm,
    ];

    /// The sky a WMO 4677 code names, or `None` for a code outside the set.
    pub fn from_wmo(code: i64) -> Option<Sky> {
        Some(match code {
            0 | 1 => Sky::Clear,
            2 | 3 => Sky::Cloudy,
            45 | 48 => Sky::Fog,
            51..=57 => Sky::Drizzle,
            61..=67 | 80..=82 => Sky::Rain,
            71..=77 | 85 | 86 => Sky::Snow,
            95..=99 => Sky::Storm,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Sky::Clear => "clear",
            Sky::Cloudy => "cloudy",
            Sky::Fog => "fog",
            Sky::Drizzle => "drizzle",
            Sky::Rain => "rain",
            Sky::Snow => "snow",
            Sky::Storm => "storm",
        }
    }

    pub fn from_id(id: &str) -> Option<Sky> {
        Sky::ALL.into_iter().find(|s| s.id() == id)
    }
}

/// One day's weather.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Weather {
    pub date: NaiveDate,
    pub sky: Sky,
    pub high_c: i16,
    pub low_c: i16,
}

impl Weather {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "date": self.date.to_string(),
            "sky": self.sky,
            "high_c": self.high_c,
            "low_c": self.low_c,
        })
    }
}

/// Where she lives, as the owner wrote it: a label for the page and the
/// coordinates a fetch sends.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

impl Place {
    /// Refused unless the label is a short line and the coordinates are on
    /// the globe.
    pub fn check(&self) -> Result<()> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
            bail!("a place's name is a short line, at most 60 characters");
        }
        if !(-90.0..=90.0).contains(&self.lat) || !(-180.0..=180.0).contains(&self.lon) {
            bail!("a place's lat is -90 to 90 and its lon -180 to 180");
        }
        Ok(())
    }
}

fn whole_degrees(v: f64) -> Option<i16> {
    let r = v.round();
    (r.is_finite() && r >= f64::from(COLDEST) && r <= f64::from(HOTTEST)).then_some(r as i16)
}

/// The days in Open-Meteo's daily answer that parse, each within `from..=to`;
/// refused whole when the answer is not that shape. A day with a code
/// outside the set, a missing or implausible temperature, a low above its
/// high, or a date outside the window is left out, never guessed.
pub fn parse(body: &str, from: NaiveDate, to: NaiveDate) -> Result<Vec<Weather>> {
    #[derive(Deserialize)]
    struct Daily {
        time: Vec<String>,
        weather_code: Vec<Option<f64>>,
        temperature_2m_max: Vec<Option<f64>>,
        temperature_2m_min: Vec<Option<f64>>,
    }
    #[derive(Deserialize)]
    struct Answer {
        daily: Daily,
    }
    let a: Answer = serde_json::from_str(body).context("the weather answer is not daily data")?;
    let d = a.daily;
    let n = d.time.len();
    if [
        d.weather_code.len(),
        d.temperature_2m_max.len(),
        d.temperature_2m_min.len(),
    ]
    .iter()
    .any(|l| *l != n)
    {
        bail!("the weather answer's columns differ in length");
    }
    let mut out: Vec<Weather> = Vec::new();
    for i in 0..n {
        let Ok(date) = NaiveDate::parse_from_str(&d.time[i], "%Y-%m-%d") else {
            continue;
        };
        if date < from || date > to || out.iter().any(|w| w.date == date) {
            continue;
        }
        let sky = d.weather_code[i]
            .filter(|c| c.fract() == 0.0)
            .and_then(|c| Sky::from_wmo(c as i64));
        let high = d.temperature_2m_max[i].and_then(whole_degrees);
        let low = d.temperature_2m_min[i].and_then(whole_degrees);
        if let (Some(sky), Some(high_c), Some(low_c)) = (sky, high, low) {
            if low_c <= high_c {
                out.push(Weather {
                    date,
                    sky,
                    high_c,
                    low_c,
                });
            }
        }
    }
    Ok(out)
}

/// The request for `place`'s days around `today`.
pub fn url(place: &Place) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}\
         &daily=weather_code,temperature_2m_max,temperature_2m_min&timezone=auto\
         &past_days={PAST_DAYS}&forecast_days={AHEAD_DAYS}",
        place.lat, place.lon
    )
}

/// Fetch and parse `place`'s days around `today`. The answer is read up to
/// [`MAX_BYTES`] and refused past it, never cut.
pub async fn fetch(place: &Place, today: NaiveDate) -> Result<Vec<Weather>> {
    place.check()?;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let mut resp = http
        .get(url(place))
        .send()
        .await
        .context("asking for the weather")?;
    if !resp.status().is_success() {
        bail!("the weather service answered {}", resp.status());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.context("reading the weather")? {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BYTES {
            bail!("the weather answer is over {MAX_BYTES} bytes; refused");
        }
    }
    let body = String::from_utf8(body).context("the weather answer is not text")?;
    let from = today - chrono::Days::new(u64::from(PAST_DAYS));
    let to = today + chrono::Days::new(u64::from(AHEAD_DAYS));
    parse(&body, from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn a_day_parses_into_closed_types_or_is_left_out() {
        let body = r#"{"latitude": 1, "daily": {
            "time": ["2026-11-01", "2026-11-02", "2026-11-03", "2026-11-04", "2026-11-05", "2030-01-01", "2026-11-06"],
            "weather_code": [61, 3, 999, 71, 0, 0, 95.5],
            "temperature_2m_max": [12.4, 9.6, 10, 1.2, 80, 5, 5],
            "temperature_2m_min": [6.5, 2.0, 3, -3.7, 1, 1, 1]
        }}"#;
        let got = parse(body, d("2026-10-25"), d("2026-11-10")).unwrap();
        assert_eq!(
            got,
            vec![
                Weather {
                    date: d("2026-11-01"),
                    sky: Sky::Rain,
                    high_c: 12,
                    low_c: 7
                },
                Weather {
                    date: d("2026-11-02"),
                    sky: Sky::Cloudy,
                    high_c: 10,
                    low_c: 2
                },
                Weather {
                    date: d("2026-11-04"),
                    sky: Sky::Snow,
                    high_c: 1,
                    low_c: -4
                },
            ],
            "an unknown code, 80°C, a date outside the window and a fractional code are refused"
        );
        assert!(parse(r#"{"daily": {"time": ["2026-11-01"], "weather_code": [], "temperature_2m_max": [], "temperature_2m_min": []}}"#, d("2026-10-01"), d("2026-12-01")).is_err());
        assert!(parse("<html>", d("2026-10-01"), d("2026-12-01")).is_err());
    }

    #[test]
    fn every_wmo_code_names_a_sky_or_none() {
        for code in 0..=100 {
            if let Some(s) = Sky::from_wmo(code) {
                assert_eq!(Sky::from_id(s.id()), Some(s));
            }
        }
        assert_eq!(Sky::from_wmo(4), None);
    }

    #[test]
    fn a_place_is_checked() {
        let ok = Place {
            name: "Hanover, NH".into(),
            lat: 43.7,
            lon: -72.29,
        };
        ok.check().unwrap();
        assert!(Place {
            lat: 91.0,
            ..ok.clone()
        }
        .check()
        .is_err());
        assert!(Place {
            name: "a\nb".into(),
            ..ok.clone()
        }
        .check()
        .is_err());
        assert!(url(&ok).contains("latitude=43.7000&longitude=-72.2900"));
    }
}

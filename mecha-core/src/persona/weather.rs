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
//! **Fetched by the harness, never a model**, from one owner-written place:
//! `[life] place` in `persona.toml` ("Springfield, Illinois"), resolved once
//! by Open-Meteo's geocoder and cached beside the persona ([`PLACE_FILE`]),
//! or `story.toml`'s `[place]` with its own coordinates. One read of the
//! daily endpoint takes the last [`PAST_DAYS`] and the next [`AHEAD_DAYS`],
//! capped at the door. What leaves the machine is the town, to the
//! geocoder, and its latitude and longitude, to the weather service: nothing
//! about the persona or the owner. [`refresh`] runs it when the stored
//! weather is older than [`STALE_HOURS`], from the Week tab and a chat turn.
//!
//! **What the geocoder answers is shown to the owner, never told.** Its
//! label (town, region, country code) is how the Week tab says which place
//! it found; no prompt reads it. What the teller folds is a [`Sky`] and two
//! whole degrees, the closed fields above.

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Days back a fetch asks for: the window her stored days cover.
pub const PAST_DAYS: u32 = 14;
/// Days ahead a fetch asks for.
pub const AHEAD_DAYS: u32 = 7;
/// The most bytes a fetch reads; the answer for three weeks is a few KB.
pub const MAX_BYTES: usize = 64 * 1024;
/// Stored weather older than this is fetched again.
pub const STALE_HOURS: i64 = 6;
/// The resolved place, cached beside `persona.toml`.
pub const PLACE_FILE: &str = "weather.place.json";
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

/// A place the geocoder found for a name the owner wrote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resolved {
    /// What the owner wrote, as it was looked up.
    pub query: String,
    /// Town, region and country code, as the geocoder named them: shown to
    /// the owner, never in a prompt.
    pub label: String,
    pub lat: f64,
    pub lon: f64,
    /// The query whose weather was stored successfully. A newly resolved
    /// place has none, so a failed forecast stays due on the next ask.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weather_for: Option<String>,
}

impl Resolved {
    pub fn place(&self) -> Place {
        Place {
            name: self.label.clone(),
            lat: self.lat,
            lon: self.lon,
        }
    }
}

/// US state codes, so "Springfield, IL" finds Illinois: the geocoder names
/// the region in full.
const US_STATES: [(&str, &str); 51] = [
    ("AL", "Alabama"),
    ("AK", "Alaska"),
    ("AZ", "Arizona"),
    ("AR", "Arkansas"),
    ("CA", "California"),
    ("CO", "Colorado"),
    ("CT", "Connecticut"),
    ("DE", "Delaware"),
    ("DC", "District of Columbia"),
    ("FL", "Florida"),
    ("GA", "Georgia"),
    ("HI", "Hawaii"),
    ("ID", "Idaho"),
    ("IL", "Illinois"),
    ("IN", "Indiana"),
    ("IA", "Iowa"),
    ("KS", "Kansas"),
    ("KY", "Kentucky"),
    ("LA", "Louisiana"),
    ("ME", "Maine"),
    ("MD", "Maryland"),
    ("MA", "Massachusetts"),
    ("MI", "Michigan"),
    ("MN", "Minnesota"),
    ("MS", "Mississippi"),
    ("MO", "Missouri"),
    ("MT", "Montana"),
    ("NE", "Nebraska"),
    ("NV", "Nevada"),
    ("NH", "New Hampshire"),
    ("NJ", "New Jersey"),
    ("NM", "New Mexico"),
    ("NY", "New York"),
    ("NC", "North Carolina"),
    ("ND", "North Dakota"),
    ("OH", "Ohio"),
    ("OK", "Oklahoma"),
    ("OR", "Oregon"),
    ("PA", "Pennsylvania"),
    ("RI", "Rhode Island"),
    ("SC", "South Carolina"),
    ("SD", "South Dakota"),
    ("TN", "Tennessee"),
    ("TX", "Texas"),
    ("UT", "Utah"),
    ("VT", "Vermont"),
    ("VA", "Virginia"),
    ("WA", "Washington"),
    ("WV", "West Virginia"),
    ("WI", "Wisconsin"),
    ("WY", "Wyoming"),
];

/// The name the geocoder is asked for, and what narrows its answers: the
/// text before the first comma, and the rest.
fn split_query(query: &str) -> (String, Option<String>) {
    match query.split_once(',') {
        Some((town, rest)) if !rest.trim().is_empty() => {
            (town.trim().to_string(), Some(rest.trim().to_string()))
        }
        _ => (query.trim().trim_end_matches(',').trim().to_string(), None),
    }
}

/// The geocoder's answer to `query`, the first result whose region,
/// country or country code matches what follows the comma (a US state's
/// code standing for its name), or the first result when nothing follows.
/// Refused when nothing matches or the answer is not that shape; a name,
/// region or country outside a short line is left out of the label.
pub fn parse_geocode(body: &str, query: &str) -> Result<Resolved> {
    #[derive(Deserialize)]
    struct Hit {
        name: Option<String>,
        latitude: f64,
        longitude: f64,
        admin1: Option<String>,
        country: Option<String>,
        country_code: Option<String>,
    }
    #[derive(Deserialize)]
    struct Answer {
        #[serde(default)]
        results: Vec<Hit>,
    }
    let a: Answer =
        serde_json::from_str(body).context("the geocoder's answer is not a list of places")?;
    let (_, narrow) = split_query(query);
    let wanted: Vec<String> = narrow
        .iter()
        .flat_map(|n| {
            let n = n.trim().to_lowercase();
            let full = US_STATES
                .iter()
                .find(|(code, _)| code.eq_ignore_ascii_case(&n))
                .map(|(_, name)| name.to_lowercase());
            std::iter::once(n).chain(full)
        })
        .collect();
    let short = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|t| {
                !t.is_empty() && t.chars().count() <= 60 && !t.chars().any(char::is_control)
            })
            .map(str::to_string)
    };
    let hit = a
        .results
        .into_iter()
        .find(|h| {
            wanted.is_empty()
                || [&h.admin1, &h.country, &h.country_code].iter().any(|f| {
                    f.as_deref()
                        .is_some_and(|v| wanted.iter().any(|w| v.trim().to_lowercase() == *w))
                })
        })
        .with_context(|| {
            format!(
                "no place named {query:?} was found; try the town and its state or country spelled out"
            )
        })?;
    let label: Vec<String> = [
        short(&hit.name),
        short(&hit.admin1),
        short(&hit.country_code),
    ]
    .into_iter()
    .flatten()
    .collect();
    let r = Resolved {
        query: query.trim().to_string(),
        label: if label.is_empty() {
            query.trim().to_string()
        } else {
            label.join(", ")
        },
        lat: hit.latitude,
        lon: hit.longitude,
        weather_for: None,
    };
    r.place().check()?;
    Ok(r)
}

/// Ask the geocoder for `query`.
pub async fn geocode(query: &str) -> Result<Resolved> {
    let (town, _) = split_query(query);
    if town.is_empty() {
        bail!("a place needs a town's name");
    }
    let mut url = reqwest::Url::parse("https://geocoding-api.open-meteo.com/v1/search")?;
    url.query_pairs_mut()
        .append_pair("name", &town)
        .append_pair("count", "10")
        .append_pair("language", "en")
        .append_pair("format", "json");
    let body = read_capped(url.as_str(), "the geocoder").await?;
    parse_geocode(&body, query)
}

/// One GET, its body read up to [`MAX_BYTES`] and refused past it, never cut.
async fn read_capped(url: &str, what: &str) -> Result<String> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()?;
    let mut resp = http
        .get(url)
        .send()
        .await
        .with_context(|| format!("asking {what}"))?;
    if !resp.status().is_success() {
        bail!("{what} answered {}", resp.status());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .with_context(|| format!("reading {what}"))?
    {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BYTES {
            bail!("{what}'s answer is over {MAX_BYTES} bytes; refused");
        }
    }
    String::from_utf8(body).with_context(|| format!("{what}'s answer is not text"))
}

/// The place cached for `pdir`, if it was resolved from `query`.
pub fn cached(pdir: &std::path::Path, query: &str) -> Option<Resolved> {
    let text = std::fs::read_to_string(pdir.join(PLACE_FILE)).ok()?;
    let r: Resolved = serde_json::from_str(&text).ok()?;
    (r.query == query.trim() && r.place().check().is_ok()).then_some(r)
}

/// Where `pdir`'s weather is for: `[life] place`, resolved through the
/// cache or the geocoder, or `story.toml`'s `[place]` when that is unset.
/// `None` when neither names one.
pub async fn place_of(pdir: &std::path::Path, life_place: Option<&str>) -> Result<Option<Place>> {
    if let Some(q) = life_place.map(str::trim).filter(|q| !q.is_empty()) {
        if let Some(r) = cached(pdir, q) {
            return Ok(Some(r.place()));
        }
        let r = geocode(q).await?;
        super::write_private(
            &pdir.join(PLACE_FILE),
            serde_json::to_string(&r)?.as_bytes(),
        )?;
        return Ok(Some(r.place()));
    }
    let Ok(text) = std::fs::read_to_string(pdir.join(super::story::FILE)) else {
        return Ok(None);
    };
    Ok(super::story::parse(&text)?.place)
}

/// Whether `pdir`'s weather is due: none stored, the last fetch older than
/// [`STALE_HOURS`] at `now`, or `[life] place` changed since it was
/// resolved.
pub fn due(
    pdir: &std::path::Path,
    life_place: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool> {
    if let Some(q) = life_place.map(str::trim).filter(|q| !q.is_empty()) {
        let Some(place) = cached(pdir, q) else {
            return Ok(true);
        };
        if place.weather_for.as_deref() != Some(q) {
            return Ok(true);
        }
    }
    let last = super::world::World::open(pdir)?.weather_fetched_at()?;
    Ok(last.is_none_or(|at| now - at > chrono::Duration::hours(STALE_HOURS)))
}

/// Fetch `pdir`'s weather around `today` and store it, and the place it is
/// for; `None` when no place is named. Refused when every day was refused.
pub async fn refresh(
    pdir: &std::path::Path,
    life_place: Option<&str>,
    today: NaiveDate,
) -> Result<Option<(Place, usize)>> {
    let Some(place) = place_of(pdir, life_place).await? else {
        return Ok(None);
    };
    let days = fetch(&place, today).await?;
    if days.is_empty() {
        bail!("the weather service answered, but every day was refused");
    }
    let n = super::world::World::open(pdir)?.set_weather(&days)?;
    if let Some(q) = life_place.map(str::trim).filter(|q| !q.is_empty()) {
        let mut resolved = cached(pdir, q).context("the resolved place cache was not kept")?;
        resolved.weather_for = Some(q.to_string());
        super::write_private(
            &pdir.join(PLACE_FILE),
            serde_json::to_string(&resolved)?.as_bytes(),
        )?;
    }
    Ok(Some((place, n)))
}

/// Fetch and parse `place`'s days around `today`. The answer is read up to
/// [`MAX_BYTES`] and refused past it, never cut.
pub async fn fetch(place: &Place, today: NaiveDate) -> Result<Vec<Weather>> {
    place.check()?;
    let body = read_capped(&url(place), "the weather service").await?;
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

    #[test]
    fn geocoding_picks_the_named_region_and_expands_a_us_code() {
        let body = r#"{"results":[
            {"name":"Springfield","latitude":37.2,"longitude":-93.3,"admin1":"Missouri","country":"United States","country_code":"US"},
            {"name":"Springfield","latitude":39.8,"longitude":-89.6,"admin1":"Illinois","country":"United States","country_code":"US"},
            {"name":"Springfield","latitude":45.0,"longitude":-64.0,"admin1":"Nova Scotia","country":"Canada","country_code":"CA"}
        ]}"#;
        let state = parse_geocode(body, "Springfield, IL").unwrap();
        assert_eq!(state.label, "Springfield, Illinois, US");
        assert_eq!((state.lat, state.lon), (39.8, -89.6));
        let country = parse_geocode(body, "Springfield, Canada").unwrap();
        assert_eq!(country.label, "Springfield, Nova Scotia, CA");
    }

    #[test]
    fn weather_is_due_when_empty_old_or_for_a_changed_place() {
        let dir = std::env::temp_dir().join(format!(
            "mecha-weather-due-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let now = chrono::Utc::now();
        assert!(due(&dir, Some("Springfield, Illinois"), now).unwrap());

        let mut resolved = Resolved {
            query: "Springfield, Illinois".into(),
            label: "Springfield, Illinois, US".into(),
            lat: 39.8,
            lon: -89.6,
            weather_for: None,
        };
        std::fs::write(dir.join(PLACE_FILE), serde_json::to_vec(&resolved).unwrap()).unwrap();
        assert!(
            due(&dir, Some("Springfield, Illinois"), now).unwrap(),
            "a matching place with no weather is due"
        );
        let mut world = super::super::world::World::open(&dir).unwrap();
        world
            .set_weather(&[Weather {
                date: now.date_naive(),
                sky: Sky::Clear,
                high_c: 12,
                low_c: 4,
            }])
            .unwrap();
        resolved.weather_for = Some(resolved.query.clone());
        std::fs::write(dir.join(PLACE_FILE), serde_json::to_vec(&resolved).unwrap()).unwrap();
        assert!(!due(&dir, Some("Springfield, Illinois"), now).unwrap());
        assert!(due(&dir, Some("Springfield, Missouri"), now).unwrap());

        let conn = rusqlite::Connection::open(dir.join(super::super::world::WORLD_DB)).unwrap();
        conn.execute(
            "UPDATE weather SET fetched_at = ?1",
            [(now - chrono::Duration::hours(STALE_HOURS + 1)).to_rfc3339()],
        )
        .unwrap();
        assert!(due(&dir, Some("Springfield, Illinois"), now).unwrap());
        std::fs::remove_dir_all(dir).ok();
    }
}

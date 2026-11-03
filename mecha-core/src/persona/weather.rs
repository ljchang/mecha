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
//! weather is older than [`STALE_HOURS`], from the Week tab and a chat turn;
//! either caller records a failure and pauses the same place for
//! [`FAILURE_BACKOFF_HOURS`].
//!
//! **What the geocoder answers is shown to the owner, never told.** Its
//! label (town, region, country code) is how the Week tab says which place
//! it found; no prompt reads it. What the teller folds is a [`Sky`] and two
//! whole degrees, the closed fields above.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Days back a fetch asks for: the window her stored days cover.
pub const PAST_DAYS: u32 = 14;
/// Days ahead a fetch asks for.
pub const AHEAD_DAYS: u32 = 7;
/// The most bytes a fetch reads; the answer for three weeks is a few KB.
pub const MAX_BYTES: usize = 64 * 1024;
/// Stored weather older than this is fetched again.
pub const STALE_HOURS: i64 = 6;
/// A failed geocode or forecast is not retried before this many hours.
pub const FAILURE_BACKOFF_HOURS: i64 = 1;
/// The resolved place, cached beside `persona.toml`.
pub const PLACE_FILE: &str = "weather.place.json";
/// The last failed refresh, private beside the place cache.
pub const FAILURE_FILE: &str = ".weather.failed";
/// The range a temperature must fall in, in °C.
pub const COLDEST: i16 = -60;
pub const HOTTEST: i16 = 60;
/// The longest resolved label shown on the owner's Week page.
pub const MAX_PLACE_LABEL: usize = 120;

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
        if name.is_empty()
            || name.chars().count() > MAX_PLACE_LABEL
            || name.chars().any(char::is_control)
        {
            bail!("a place's name is a short line, at most {MAX_PLACE_LABEL} characters");
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

/// The name the geocoder is asked for, and each comma-separated part that
/// narrows its answers after the town.
fn split_query(query: &str) -> (String, Vec<String>) {
    let mut parts = query.split(',').map(str::trim);
    let town = parts.next().unwrap_or_default().to_string();
    let narrow = parts
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect();
    (town, narrow)
}

/// Spellings one owner-written region or country part may match. Country
/// aliases live beside the state-code aliases they complement: `USA`
/// matches either Open-Meteo's `US` code or its `United States` country.
fn place_aliases(part: &str) -> Vec<String> {
    let part = part.trim().to_lowercase();
    let mut aliases = vec![part.clone()];
    if let Some((_, name)) = US_STATES
        .iter()
        .find(|(code, _)| code.eq_ignore_ascii_case(&part))
    {
        aliases.push(name.to_lowercase());
    }
    if part == "usa" {
        aliases.push("us".into());
        aliases.push("united states".into());
    }
    aliases
}

/// The geocoder's answer to `query`, the first result whose region,
/// country or country code matches every comma part after the town (a US
/// state's code standing for its name), or the first result when nothing
/// follows. All parts must match fields on the same result.
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
    let wanted: Vec<Vec<String>> = narrow.iter().map(|part| place_aliases(part)).collect();
    let short = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|t| {
                !t.is_empty()
                    && t.chars().count() <= MAX_PLACE_LABEL
                    && !t.chars().any(char::is_control)
            })
            .map(str::to_string)
    };
    let hit = a
        .results
        .into_iter()
        .find(|h| {
            wanted.iter().all(|aliases| {
                [&h.admin1, &h.country, &h.country_code]
                    .iter()
                    .any(|field| {
                        field.as_deref().is_some_and(|value| {
                            let value = value.trim().to_lowercase();
                            aliases.contains(&value)
                        })
                    })
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
    r.place().check().with_context(|| {
        format!("the place found for the owner's input {query:?} is not usable")
    })?;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Failure {
    query: String,
    at: String,
    message: String,
}

/// The recorded failure message while `query` is inside its retry backoff.
/// A changed query never inherits the old place's pause.
pub fn backoff_error(
    pdir: &std::path::Path,
    query: &str,
    now: DateTime<Utc>,
) -> Result<Option<String>> {
    let text = match std::fs::read_to_string(pdir.join(FAILURE_FILE)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("reading the last weather failure"),
    };
    let failure: Failure =
        serde_json::from_str(&text).context("the last weather failure does not load")?;
    if failure.query != query.trim() {
        return Ok(None);
    }
    let at = DateTime::parse_from_rfc3339(&failure.at)
        .map(|at| at.with_timezone(&Utc))
        .context("the last weather failure has an unreadable time")?;
    Ok((now - at < chrono::Duration::hours(FAILURE_BACKOFF_HOURS)).then_some(failure.message))
}

/// Pace a failed refresh for this exact owner-written place.
pub fn record_failure(
    pdir: &std::path::Path,
    query: &str,
    now: DateTime<Utc>,
    message: &str,
) -> Result<()> {
    let failure = Failure {
        query: query.trim().to_string(),
        at: now.to_rfc3339(),
        message: message.to_string(),
    };
    let bytes = serde_json::to_vec(&failure)?;
    super::write_private(&pdir.join(FAILURE_FILE), &bytes)
}

fn clear_failure(pdir: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(pdir.join(FAILURE_FILE)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).context("clearing the last weather failure"),
    }
}

/// Clear weather unless the place cache proves that the rows were fetched
/// for this exact owner-written query. This runs before backoff: a failed new
/// place must not leave the previous place in the teller while it waits.
fn clear_weather_for_changed_place(pdir: &std::path::Path, query: &str) -> Result<()> {
    let query = query.trim();
    let current = cached(pdir, query)
        .and_then(|place| place.weather_for)
        .is_some_and(|weather_for| weather_for == query);
    if !current {
        super::world::World::open(pdir)?.clear_weather()?;
    }
    Ok(())
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
/// resolved. A recorded failure for the same place pauses retries for
/// [`FAILURE_BACKOFF_HOURS`].
pub fn due(
    pdir: &std::path::Path,
    life_place: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<bool> {
    if let Some(q) = life_place.map(str::trim).filter(|q| !q.is_empty()) {
        clear_weather_for_changed_place(pdir, q)?;
        if backoff_error(pdir, q, now)?.is_some() {
            return Ok(false);
        }
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
    match life_place.map(str::trim).filter(|q| !q.is_empty()) {
        Some(query) => clear_weather_for_changed_place(pdir, query)?,
        // Exact coordinates in story.toml have no geocoder cache to prove
        // ownership. A manual fallback refresh therefore clears first: if
        // its place changed and the request fails, no old place is shown.
        None => super::world::World::open(pdir)?.clear_weather()?,
    }
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
    clear_failure(pdir)?;
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
            name: "Springfield, Illinois".into(),
            lat: 39.8,
            lon: -89.6,
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
        assert!(url(&ok).contains("latitude=39.8000&longitude=-89.6000"));
    }

    #[test]
    fn geocoding_matches_each_named_part_and_expands_us_aliases() {
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
        let three = parse_geocode(body, "Springfield, Illinois, US").unwrap();
        assert_eq!(three.label, "Springfield, Illinois, US");
        assert!(parse_geocode(body, "Springfield, Illinois, CA").is_err());

        let austin = r#"{"results":[
            {"name":"Austin","latitude":30.1,"longitude":-97.7,"admin1":"Texas","country":"Canada","country_code":"CA"},
            {"name":"Austin","latitude":30.3,"longitude":-97.7,"admin1":"Texas","country":"United States","country_code":"US"}
        ]}"#;
        let austin = parse_geocode(austin, "Austin, TX, USA").unwrap();
        assert_eq!(austin.label, "Austin, Texas, US");
        assert_eq!(
            place_aliases("USA"),
            vec![
                "usa".to_string(),
                "us".to_string(),
                "united states".to_string()
            ]
        );

        let paris = r#"{"results":[
            {"name":"Paris","latitude":48.9,"longitude":2.3,"admin1":"Île-de-France","country":"France","country_code":"FR"}
        ]}"#;
        let paris = parse_geocode(paris, "Paris, Île-de-France, France").unwrap();
        assert_eq!(paris.label, "Paris, Île-de-France, FR");

        let long = "w".repeat(65);
        let body =
            format!(r#"{{"results":[{{"name":"{long}","latitude":39.8,"longitude":-89.6}}]}}"#);
        assert_eq!(parse_geocode(&body, "Springfield").unwrap().label, long);
        let component = "w".repeat(50);
        let body = format!(
            r#"{{"results":[{{"name":"{component}","latitude":39.8,"longitude":-89.6,"admin1":"{component}","country_code":"{component}"}}]}}"#
        );
        let why = format!("{:#}", parse_geocode(&body, "Springfield").unwrap_err());
        assert!(why.contains("owner's input \"Springfield\""), "{why}");
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

    #[test]
    fn a_failed_place_backs_off_for_one_hour_but_not_for_a_changed_place() {
        let dir = std::env::temp_dir().join(format!(
            "mecha-weather-failed-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let now = "2026-11-03T10:00:00Z".parse().unwrap();
        record_failure(
            &dir,
            "Springfield, Illinois",
            now,
            "the geocoder found no match",
        )
        .unwrap();
        assert!(!due(&dir, Some("Springfield, Illinois"), now).unwrap());
        assert_eq!(
            backoff_error(&dir, "Springfield, Illinois", now)
                .unwrap()
                .as_deref(),
            Some("the geocoder found no match")
        );
        assert!(due(
            &dir,
            Some("Springfield, Illinois"),
            now + chrono::Duration::hours(FAILURE_BACKOFF_HOURS)
        )
        .unwrap());
        assert!(due(&dir, Some("Springfield, Missouri"), now).unwrap());
        clear_failure(&dir).unwrap();
        assert!(backoff_error(&dir, "Springfield, Illinois", now)
            .unwrap()
            .is_none());
        std::fs::remove_dir_all(dir).ok();
    }

    /// Regression gate part 1 for shelf A: strings from every weather and
    /// geocoder field may reach the owner-facing place label, but no canary
    /// survives the closed weather row or either prompt fold.
    #[test]
    fn regression_shelf_a_weather_canary_never_reaches_world_or_prompt() {
        const CANARY: &str = "CANARY-wren-7731";
        let from = d("2026-11-01");
        let to = d("2026-11-05");
        let body = format!(
            r#"{{"outside":"{CANARY}","daily":{{
                "time":["2026-11-03","{CANARY}"],
                "weather_code":[61,61],
                "temperature_2m_max":[9,9],
                "temperature_2m_min":[4,4],
                "extra":"{CANARY}"
            }}}}"#
        );
        let parsed = parse(&body, from, to).unwrap();
        assert_eq!(parsed.len(), 1, "the canary date is refused");
        for bad in [
            format!(
                r#"{{"daily":{{"time":["2026-11-03"],"weather_code":["{CANARY}"],"temperature_2m_max":[9],"temperature_2m_min":[4]}}}}"#
            ),
            format!(
                r#"{{"daily":{{"time":["2026-11-03"],"weather_code":[61],"temperature_2m_max":["{CANARY}"],"temperature_2m_min":[4]}}}}"#
            ),
            format!(
                r#"{{"daily":{{"time":["2026-11-03"],"weather_code":[61],"temperature_2m_max":[9],"temperature_2m_min":["{CANARY}"]}}}}"#
            ),
        ] {
            assert!(parse(&bad, from, to).is_err(), "string columns are refused");
        }

        let geocode = format!(
            r#"{{"extra":"{CANARY}","results":[{{
                "name":"{CANARY}","latitude":39.8,"longitude":-89.6,
                "admin1":"Illinois","country":"{CANARY}",
                "country_code":"US","extra":"{CANARY}"
            }}]}}"#
        );
        let resolved = parse_geocode(&geocode, &format!("Springfield, {CANARY}")).unwrap();
        assert!(resolved.label.contains(CANARY));
        let admin = format!(
            r#"{{"results":[{{"name":"Springfield","latitude":39.8,"longitude":-89.6,"admin1":"{CANARY}","country":"United States","country_code":"US"}}]}}"#
        );
        assert!(parse_geocode(&admin, &format!("Springfield, {CANARY}"))
            .unwrap()
            .label
            .contains(CANARY));
        let country_code = format!(
            r#"{{"results":[{{"name":"Springfield","latitude":39.8,"longitude":-89.6,"admin1":"Illinois","country":"United States","country_code":"{CANARY}"}}]}}"#
        );
        assert!(
            parse_geocode(&country_code, &format!("Springfield, {CANARY}"))
                .unwrap()
                .label
                .contains(CANARY)
        );
        for (lat, lon) in [
            (format!(r#""{CANARY}""#), "-89.6".to_string()),
            ("39.8".to_string(), format!(r#""{CANARY}""#)),
        ] {
            let bad = format!(
                r#"{{"results":[{{"name":"Springfield","latitude":{lat},"longitude":{lon}}}]}}"#
            );
            assert!(parse_geocode(&bad, "Springfield").is_err());
        }

        let dir = std::env::temp_dir().join(format!(
            "mecha-weather-canary-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut world = super::super::world::World::open(&dir).unwrap();
        world.set_weather(&parsed).unwrap();
        let stored = world.weather(d("2026-11-03")).unwrap().unwrap();
        assert!(!stored.to_json().to_string().contains(CANARY));

        let story = super::super::story::StoryDay {
            date: stored.date,
            weather: Some(stored.into()),
            morning: super::super::story::Levels::default(),
            evening: super::super::story::Levels::default(),
            events: Vec::new(),
            kept: true,
        };
        let lines = super::super::today::teller_lines(
            "2026-11-03T12:00:00".parse().unwrap(),
            &story,
            None,
            &std::collections::BTreeMap::new(),
        )
        .unwrap();
        assert!(!lines.contains(CANARY));
        assert!(!lines.contains(&resolved.label));
        let now = super::super::world::Now {
            date: stored.date,
            minute: 12 * 60,
            stretch: super::super::sampler::Stretch {
                activity: super::super::activity::Activity::Reading,
                start: 12 * 60,
                end: 13 * 60,
                company: None,
                anchor: None,
            },
            source: super::super::world::Source::Sampler,
            with_names: Vec::new(),
        };
        let note = super::super::today::note_with_events(&now, Some(&lines));
        assert!(!note.contains(CANARY));
        assert!(!note.contains(&resolved.label));
        assert!(
            resolved.label.contains(CANARY),
            "the label remains page-only"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    /// Gate part 2 for shelf A derives the rendered weather item's capacity
    /// from the shipped enum and degree bounds, and refuses more than 16 bits.
    #[test]
    fn shelf_a_weather_item_stays_within_sixteen_bits() {
        let degrees = u128::from((i32::from(HOTTEST) - i32::from(COLDEST) + 1) as u32);
        let ordered_low_high = degrees * (degrees + 1) / 2;
        let states = Sky::ALL.len() as u128 * ordered_low_high;
        let bits = (states as f64).log2();
        assert!(
            bits <= 16.0,
            "weather renders {bits:.2} bits ({states} states)"
        );
    }
}

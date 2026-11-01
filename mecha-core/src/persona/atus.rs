//! The day sampler's table: American Time Use Survey counts, compiled into
//! the binary (`docs/PERSONA-LIFE-DESIGN.md` §2.2, §4.2).
//!
//! `scripts/atus-table.py` writes `data/atus/table.tsv.gz` from pinned
//! public files, and this module reads it (`zcat` shows the text). The table holds weighted counts, never
//! probabilities, so how they become a draw (backing off from a thin cell to
//! the pool, smoothing an hour that saw nothing) is decided in code here and
//! in the sampler, where a test can see it. Counts are in diaries: weights
//! are scaled to a mean of 1 per day type, one scale across every cell, so a
//! cell's counts add up to the pool's.
//!
//! What a row holds, by its first field:
//!
//! - `T`: stretches that began at a clock hour, keyed by the activity before
//!   them. One activity's consecutive episodes are one stretch, so a stretch
//!   never follows itself.
//! - `D`: 21 duration quantiles in minutes, per start band. The first
//!   stretch of a diary is left out (it began before the 4am the diary
//!   starts at); a cell with fewer than 20 stretches has no row, and the
//!   pool's (`all`) stands in.
//! - `W`: who was there, as weighted counts of stretches with each kind of
//!   company present. ATUS does not ask during sleep, grooming or private
//!   activities, so those have no row.
//! - `M`: every lexicon code the files used, and the activity the script gave
//!   it, so a test can hold the script's mapping to [`Activity::of_atus`].

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::OnceLock;

use super::activity::Activity;

const TABLE: &[u8] = include_bytes!("../../data/atus/table.tsv.gz");
/// The format `V` row this reader understands.
const VERSION: &str = "1";
/// Quantiles per duration row: 0%, 5%, …, 100%.
pub const QUANTILES: usize = 21;
const MINUTES_PER_DAY: u32 = 24 * 60;

/// A demographic cell: whose diaries a persona's days are drawn from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cell {
    /// Enrolled full time in college or university.
    Student,
    /// Employed full time, no household child under 13.
    FullTime,
    /// Employed full time, with a household child under 13.
    FullTimeParent,
    /// Employed part time.
    PartTime,
    /// Not employed, under 65.
    AtHome,
    /// Not employed, 65 or older.
    Retired,
}

impl Cell {
    pub const ALL: [Cell; 6] = [
        Cell::Student,
        Cell::FullTime,
        Cell::FullTimeParent,
        Cell::PartTime,
        Cell::AtHome,
        Cell::Retired,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Cell::Student => "student",
            Cell::FullTime => "full_time",
            Cell::FullTimeParent => "full_time_parent",
            Cell::PartTime => "part_time",
            Cell::AtHome => "at_home",
            Cell::Retired => "retired",
        }
    }

    pub fn from_id(id: &str) -> Option<Cell> {
        Cell::ALL.into_iter().find(|c| c.id() == id)
    }
}

/// ATUS holidays are left out, so a day is one or the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DayType {
    Weekday,
    Weekend,
}

impl DayType {
    pub const ALL: [DayType; 2] = [DayType::Weekday, DayType::Weekend];

    pub fn id(self) -> &'static str {
        match self {
            DayType::Weekday => "weekday",
            DayType::Weekend => "weekend",
        }
    }

    fn from_id(id: &str) -> Option<DayType> {
        DayType::ALL.into_iter().find(|d| d.id() == id)
    }
}

/// When a stretch starts, for its duration: the same activity runs longer
/// begun at 22:00 than at 14:00.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Band {
    /// 22:00 to 06:00.
    Night,
    /// 06:00 to 12:00.
    Morning,
    /// 12:00 to 17:00.
    Afternoon,
    /// 17:00 to 22:00.
    Evening,
}

impl Band {
    pub const ALL: [Band; 4] = [Band::Night, Band::Morning, Band::Afternoon, Band::Evening];

    pub fn id(self) -> &'static str {
        match self {
            Band::Night => "night",
            Band::Morning => "morning",
            Band::Afternoon => "afternoon",
            Band::Evening => "evening",
        }
    }

    fn from_id(id: &str) -> Option<Band> {
        Band::ALL.into_iter().find(|b| b.id() == id)
    }

    /// The band a clock hour (0–23) falls in.
    pub fn of(hour: u8) -> Band {
        match hour % 24 {
            6..=11 => Band::Morning,
            12..=16 => Band::Afternoon,
            17..=21 => Band::Evening,
            _ => Band::Night,
        }
    }
}

/// Who was there, from the Who file's codes, in the table's `C` order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Company {
    Alone,
    /// Spouse or unmarried partner.
    Partner,
    /// Her own child or grandchild, in the household or not.
    Child,
    Parent,
    /// Siblings and other relatives.
    Family,
    /// A housemate, roommate or boarder who is not family.
    Housemate,
    Friend,
    /// Neighbours and acquaintances.
    Acquaintance,
    /// A boss, someone she supervises, or a co-worker.
    Colleague,
    /// Customers and other people outside the household: no one in her
    /// network.
    Public,
}

impl Company {
    pub const ALL: [Company; 10] = [
        Company::Alone,
        Company::Partner,
        Company::Child,
        Company::Parent,
        Company::Family,
        Company::Housemate,
        Company::Friend,
        Company::Acquaintance,
        Company::Colleague,
        Company::Public,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Company::Alone => "alone",
            Company::Partner => "partner",
            Company::Child => "child",
            Company::Parent => "parent",
            Company::Family => "family",
            Company::Housemate => "housemate",
            Company::Friend => "friend",
            Company::Acquaintance => "acquaintance",
            Company::Colleague => "colleague",
            Company::Public => "public",
        }
    }
}

/// How long a stretch lasts, as quantiles of the stretches seen.
#[derive(Debug, Clone, PartialEq)]
pub struct Durations {
    /// The weighted count of stretches behind the quantiles.
    pub weight: f64,
    /// Minutes at 0%, 5%, …, 100%, never decreasing.
    pub quantiles: [u32; QUANTILES],
}

impl Durations {
    /// The duration at quantile `u` (clamped to 0–1), interpolated between
    /// the two stored quantiles around it.
    pub fn at(&self, u: f64) -> u32 {
        let pos = u.clamp(0.0, 1.0) * (QUANTILES - 1) as f64;
        let i = (pos.floor() as usize).min(QUANTILES - 2);
        let (lo, hi) = (self.quantiles[i] as f64, self.quantiles[i + 1] as f64);
        (lo + (hi - lo) * (pos - i as f64)).round() as u32
    }
}

/// Who was there across an activity's stretches.
#[derive(Debug, Clone, PartialEq)]
pub struct CompanyCounts {
    /// The weighted count of stretches the Who file covered.
    pub weight: f64,
    /// For each of [`Company::ALL`], the weighted count of those stretches
    /// it was present for. They overlap: a dinner can hold a partner and a
    /// child.
    pub present: [f64; 10],
}

impl CompanyCounts {
    fn add(&mut self, other: &CompanyCounts) {
        self.weight += other.weight;
        for (a, b) in self.present.iter_mut().zip(other.present) {
            *a += b;
        }
    }

    /// The share of stretches `who` was present for; `None` over nothing.
    pub fn share(&self, who: Company) -> Option<f64> {
        (self.weight > 0.0).then(|| self.present[who as usize] / self.weight)
    }
}

/// What a transition count is kept under: cell (`None` for the pool), day
/// type, the clock hour the next stretch began, and the activity before it.
type Step = (Option<Cell>, DayType, u8, Activity);

/// The parsed table. `None` as a cell means the pool of every cell.
#[derive(Debug, Default)]
pub struct Table {
    diaries: BTreeMap<(Cell, DayType), u32>,
    transitions: BTreeMap<Step, BTreeMap<Activity, f64>>,
    durations: BTreeMap<(Option<Cell>, DayType, Band, Activity), Durations>,
    company: BTreeMap<(Option<Cell>, DayType, Activity), CompanyCounts>,
    codes: Vec<(String, Option<Activity>)>,
}

/// The table compiled into this binary.
pub fn table() -> &'static Table {
    static PARSED: OnceLock<Table> = OnceLock::new();
    // The bytes are a constant of the build, and `the_shipped_table_parses`
    // proves they read, so a failure here is a build that skipped its tests.
    PARSED.get_or_init(|| {
        shipped_text()
            .and_then(|text| Table::parse(&text))
            .expect("data/atus/table.tsv.gz parses")
    })
}

fn shipped_text() -> Result<String> {
    let mut text = String::new();
    flate2::read::GzDecoder::new(TABLE)
        .read_to_string(&mut text)
        .context("data/atus/table.tsv.gz does not decompress")?;
    Ok(text)
}

impl Table {
    pub fn parse(text: &str) -> Result<Table> {
        let mut t = Table::default();
        let mut version = false;
        let mut columns = false;
        for (n, line) in text.lines().enumerate() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            t.row(line, &mut version, &mut columns)
                .with_context(|| format!("ATUS table line {}", n + 1))?;
        }
        ensure!(version, "the ATUS table has no V row");
        ensure!(columns, "the ATUS table has no C row");
        // The pool is the sum of the cells, which the shared weight scale
        // makes exact.
        let cells: Vec<_> = t.transitions.iter().map(|(k, v)| (*k, v.clone())).collect();
        for ((_, day, hour, from), tos) in cells {
            let pool = t.transitions.entry((None, day, hour, from)).or_default();
            for (to, n) in tos {
                *pool.entry(to).or_default() += n;
            }
        }
        let cells: Vec<_> = t.company.iter().map(|(k, v)| (*k, v.clone())).collect();
        for ((_, day, act), counts) in cells {
            t.company
                .entry((None, day, act))
                .or_insert_with(|| CompanyCounts {
                    weight: 0.0,
                    present: [0.0; 10],
                })
                .add(&counts);
        }
        Ok(t)
    }

    fn row(&mut self, line: &str, version: &mut bool, columns: &mut bool) -> Result<()> {
        let f: Vec<&str> = line.split('\t').collect();
        match f[0] {
            "V" => {
                ensure!(
                    f.get(1) == Some(&VERSION),
                    "format {:?} is not {VERSION}",
                    f.get(1)
                );
                *version = true;
            }
            "C" => {
                let want: Vec<_> = Company::ALL.iter().map(|c| c.id()).collect();
                ensure!(
                    f[1..] == want[..],
                    "company columns {:?} are not {want:?}",
                    &f[1..]
                );
                *columns = true;
            }
            "M" => {
                let [_, code, act] = f[..] else {
                    bail!("an M row has 3 fields")
                };
                ensure!(
                    code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()),
                    "code {code:?}"
                );
                let act = if act == "-" {
                    None
                } else {
                    Some(activity(act)?)
                };
                self.codes.push((code.to_string(), act));
            }
            "N" => {
                let [_, cell, day, n] = f[..] else {
                    bail!("an N row has 4 fields")
                };
                self.diaries
                    .insert((self::cell(cell)?, day_type(day)?), n.parse()?);
            }
            "T" => {
                let [_, cell, day, hour, from, pairs] = f[..] else {
                    bail!("a T row has 6 fields")
                };
                let hour: u8 = hour.parse()?;
                ensure!(hour < 24, "hour {hour}");
                let from = activity(from)?;
                let mut tos = BTreeMap::new();
                for pair in pairs.split(' ') {
                    let (to, n) = pair
                        .split_once('=')
                        .ok_or_else(|| anyhow!("pair {pair:?}"))?;
                    let to = activity(to)?;
                    ensure!(to != from, "{} follows itself", to.id());
                    ensure!(tos.insert(to, count(n)?).is_none(), "{} twice", to.id());
                }
                let key = (Some(self::cell(cell)?), day_type(day)?, hour, from);
                ensure!(
                    self.transitions.insert(key, tos).is_none(),
                    "a repeated T row"
                );
            }
            "D" => {
                ensure!(
                    f.len() == 6 + QUANTILES,
                    "a D row has {} fields",
                    6 + QUANTILES
                );
                let cell = if f[1] == "all" {
                    None
                } else {
                    Some(self::cell(f[1])?)
                };
                let band = Band::from_id(f[3]).ok_or_else(|| anyhow!("band {:?}", f[3]))?;
                let mut quantiles = [0; QUANTILES];
                for (q, s) in quantiles.iter_mut().zip(&f[6..]) {
                    *q = s.parse()?;
                }
                ensure!(
                    quantiles.windows(2).all(|w| w[0] <= w[1]),
                    "quantiles decrease"
                );
                ensure!(
                    quantiles[QUANTILES - 1] <= MINUTES_PER_DAY,
                    "a stretch past a day"
                );
                let key = (cell, day_type(f[2])?, band, activity(f[4])?);
                let row = Durations {
                    weight: count(f[5])?,
                    quantiles,
                };
                ensure!(
                    self.durations.insert(key, row).is_none(),
                    "a repeated D row"
                );
            }
            "W" => {
                ensure!(
                    f.len() == 5 + Company::ALL.len(),
                    "a W row has {} fields",
                    5 + Company::ALL.len()
                );
                let weight = count(f[4])?;
                let mut present = [0.0; 10];
                for (p, s) in present.iter_mut().zip(&f[5..]) {
                    *p = count(s)?;
                    ensure!(
                        *p <= weight + 0.01,
                        "company present more often than the activity"
                    );
                }
                let key = (Some(self::cell(f[1])?), day_type(f[2])?, activity(f[3])?);
                let row = CompanyCounts { weight, present };
                ensure!(self.company.insert(key, row).is_none(), "a repeated W row");
            }
            other => bail!("unknown row kind {other:?}"),
        }
        Ok(())
    }

    /// How many diaries a cell's counts came from, for a day type.
    pub fn diaries(&self, cell: Cell, day: DayType) -> u32 {
        self.diaries.get(&(cell, day)).copied().unwrap_or(0)
    }

    /// What began at `hour` after `from`, as weighted counts by activity, in
    /// [`Activity`] order. `None` pools every cell. Empty when the hour saw
    /// no stretch of `from` end.
    pub fn next(
        &self,
        cell: Option<Cell>,
        day: DayType,
        hour: u8,
        from: Activity,
    ) -> Option<&BTreeMap<Activity, f64>> {
        self.transitions.get(&(cell, day, hour, from))
    }

    /// How long `act` lasts begun in `band`. A cell too thin for a row has
    /// none; the pool's (`None`) is there for every activity it saw enough.
    pub fn durations(
        &self,
        cell: Option<Cell>,
        day: DayType,
        band: Band,
        act: Activity,
    ) -> Option<&Durations> {
        self.durations.get(&(cell, day, band, act))
    }

    /// Who was there for `act`. `None` for an activity ATUS does not ask
    /// about, or one the cell never did.
    pub fn company(
        &self,
        cell: Option<Cell>,
        day: DayType,
        act: Activity,
    ) -> Option<&CompanyCounts> {
        self.company.get(&(cell, day, act))
    }

    /// Every lexicon code the source files used, with the activity the table
    /// was built with; `None` for a data code (`50xxxx`).
    pub fn codes(&self) -> &[(String, Option<Activity>)] {
        &self.codes
    }
}

fn activity(id: &str) -> Result<Activity> {
    Activity::from_id(id).ok_or_else(|| anyhow!("unknown activity {id:?}"))
}

fn cell(id: &str) -> Result<Cell> {
    Cell::from_id(id).ok_or_else(|| anyhow!("unknown cell {id:?}"))
}

fn day_type(id: &str) -> Result<DayType> {
    DayType::from_id(id).ok_or_else(|| anyhow!("unknown day type {id:?}"))
}

fn count(s: &str) -> Result<f64> {
    let n: f64 = s.parse().with_context(|| format!("count {s:?}"))?;
    ensure!(n.is_finite() && n >= 0.0, "count {s:?}");
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "V\t1\nC\talone\tpartner\tchild\tparent\tfamily\thousemate\tfriend\tacquaintance\tcolleague\tpublic\n";

    fn most_likely(tos: &BTreeMap<Activity, f64>) -> Activity {
        *tos.iter().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0
    }

    fn median(cell: Option<Cell>, day: DayType, band: Band, act: Activity) -> u32 {
        table().durations(cell, day, band, act).unwrap().at(0.5)
    }

    #[test]
    fn the_shipped_table_parses() {
        let t = Table::parse(&shipped_text().unwrap()).unwrap();
        for cell in Cell::ALL {
            for day in DayType::ALL {
                assert!(
                    t.diaries(cell, day) >= 200,
                    "{} {} is thin",
                    cell.id(),
                    day.id()
                );
            }
        }
    }

    #[test]
    fn the_table_was_built_with_this_activity_mapping() {
        // The script holds a copy of `Activity::atus`; this is what keeps it
        // one. A change to either fails here until the table is rewritten.
        let codes = table().codes();
        assert!(codes.len() > 300, "{} codes", codes.len());
        for (code, built) in codes {
            assert_eq!(
                *built,
                Activity::of_atus(code),
                "{code}: re-run scripts/atus-table.py"
            );
            assert_eq!(built.is_none(), code.starts_with("50"), "{code}");
        }
        for act in Activity::ALL {
            assert!(
                codes.iter().any(|(_, a)| *a == Some(act)),
                "no code in the data is {}",
                act.id()
            );
        }
    }

    #[test]
    fn every_activity_can_be_reached_left_and_timed_in_the_pool() {
        for day in DayType::ALL {
            for act in Activity::ALL {
                let reached = (0..24).any(|h| {
                    Activity::ALL.iter().any(|from| {
                        table()
                            .next(None, day, h, *from)
                            .is_some_and(|tos| tos.contains_key(&act))
                    })
                });
                let left = (0..24).any(|h| table().next(None, day, h, act).is_some());
                let timed = Band::ALL
                    .iter()
                    .any(|b| table().durations(None, day, *b, act).is_some());
                assert!(reached && left && timed, "{} on a {}", act.id(), day.id());
            }
        }
    }

    #[test]
    fn the_pool_is_the_sum_of_the_cells() {
        let t = table();
        let (day, hour, from) = (DayType::Weekday, 7, Activity::Sleep);
        let pooled = t.next(None, day, hour, from).unwrap()[&Activity::PersonalCare];
        let summed: f64 = Cell::ALL
            .iter()
            .filter_map(|c| t.next(Some(*c), day, hour, from))
            .filter_map(|tos| tos.get(&Activity::PersonalCare))
            .sum();
        assert!((pooled - summed).abs() < 1e-9, "{pooled} vs {summed}");
    }

    #[test]
    fn the_table_describes_an_ordinary_day() {
        // Coarse facts any day of ATUS shows; a broken estimator (weights,
        // censoring, the stretch merge) breaks at least one.
        let t = table();
        let wd = DayType::Weekday;
        let woke = t.next(None, wd, 7, Activity::Sleep).unwrap();
        assert_eq!(most_likely(woke), Activity::PersonalCare);
        let night = median(None, wd, Band::Evening, Activity::Sleep);
        assert!(
            (360..=600).contains(&night),
            "an evening's sleep runs {night} minutes"
        );
        let shift = median(Some(Cell::FullTime), wd, Band::Morning, Activity::Work);
        assert!(shift >= 180, "a morning's work runs {shift} minutes");
        let student = |c| {
            (0..24)
                .filter_map(|h| t.next(Some(c), wd, h, Activity::Commute))
                .filter_map(|tos| tos.get(&Activity::Class))
                .sum::<f64>()
                / f64::from(t.diaries(c, wd))
        };
        assert!(student(Cell::Student) > 10.0 * student(Cell::Retired));
        let work = t.company(None, wd, Activity::Work).unwrap();
        assert!(work.share(Company::Colleague).unwrap() > work.share(Company::Partner).unwrap());
        assert!(
            t.company(None, wd, Activity::Sleep).is_none(),
            "ATUS does not ask during sleep"
        );
    }

    #[test]
    fn a_duration_is_read_between_quantiles() {
        let mut quantiles = [0; QUANTILES];
        for (i, q) in quantiles.iter_mut().enumerate() {
            *q = 10 * i as u32;
        }
        let d = Durations {
            weight: 1.0,
            quantiles,
        };
        assert_eq!(d.at(0.0), 0);
        assert_eq!(d.at(1.0), 200);
        assert_eq!(d.at(0.5), 100);
        assert_eq!(d.at(0.525), 105);
        assert_eq!(d.at(-3.0), 0);
        assert_eq!(d.at(f64::INFINITY), 200);
    }

    #[test]
    fn a_damaged_table_is_refused_by_line() {
        let q = (0..QUANTILES)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\t");
        for (body, says) in [
            (
                "T\tstudent\tweekday\t7\tsleep\tnapping=1.0\n",
                "unknown activity",
            ),
            ("T\tstudent\tweekday\t24\tsleep\teating=1.0\n", "hour 24"),
            (
                "T\tstudent\tweekday\t7\tsleep\tsleep=1.0\n",
                "follows itself",
            ),
            ("T\tstudent\tweekday\t7\tsleep\teating=-1\n", "count"),
            ("T\tstudent\tweekday\t7\tsleep\teating=NaN\n", "count"),
            ("T\tnurse\tweekday\t7\tsleep\teating=1\n", "unknown cell"),
            ("X\tanything\n", "unknown row kind"),
            ("M\t0101\tsleep\n", "code"),
            (
                &format!(
                    "D\tall\tweekday\tnight\tsleep\t1\t{}\n",
                    q.replace("\t20", "\t1")
                ),
                "decrease",
            ),
            (
                "W\tstudent\tweekday\twork\t1\t2\t0\t0\t0\t0\t0\t0\t0\t0\t0\n",
                "more often",
            ),
        ] {
            let err = format!("{:#}", Table::parse(&format!("{HEAD}{body}")).unwrap_err());
            assert!(
                err.contains("line 3") && err.contains(says),
                "{body:?}: {err}"
            );
        }
        let err = Table::parse("V\t2\n").unwrap_err();
        assert!(format!("{err:#}").contains("is not 1"));
        let err = Table::parse("V\t1\nC\tpartner\talone\n").unwrap_err();
        assert!(format!("{err:#}").contains("company columns"));
        assert!(Table::parse(HEAD).is_ok());
    }

    #[test]
    fn a_clock_hour_has_one_band() {
        let bands: Vec<_> = (0..24).map(Band::of).collect();
        assert_eq!(bands[5], Band::Night);
        assert_eq!(bands[6], Band::Morning);
        assert_eq!(bands[12], Band::Afternoon);
        assert_eq!(bands[17], Band::Evening);
        assert_eq!(bands[22], Band::Night);
        assert!(Band::ALL.iter().all(|b| bands.contains(b)));
        assert!(Cell::ALL.iter().all(|c| Cell::from_id(c.id()) == Some(*c)));
    }
}

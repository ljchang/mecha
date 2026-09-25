//! The conditions a run happened under.
//!
//! Every counter on [`RunStats`](crate::session::RunStats) says what a run
//! *did*. Nothing said what it was working *against* — whether the context was
//! nearly full, whether the machine was loaded, how much was already waiting on
//! the owner. `docs/GOAL-SYSTEM-DESIGN.md` §4 is the argument; the short form
//! is that an outcome is not interpretable without the state it happened in.
//! A run that failed under a saturated machine and one that failed on an idle
//! one are the same row today, and they are not the same event: appraisal
//! separates *regret* from *disappointment* on exactly whether an alternative
//! existed.
//!
//! **Read-only.** It writes no store, and it is recorded so that later rungs
//! — predictive compaction, the diagnostician's brief, the appraiser — have a
//! series to reason from, and so that the corpus exists before anything is
//! built on it. The one thing it decides is a narrowing: a charter line
//! saturated over the recorded runs is withheld from what the run's in-run
//! consumers read ([`Homeostat::in_run_readings`], S5), and the only such
//! consumer, `Decision::assess`, reaches the model only as fixed advice
//! behind `goal_guidance`.
//!
//! ## Three rules it inherits
//!
//! - **Opt-in, never automatic.** It rides on [`RunContext`] the way
//!   cancellation does. Sampling costs directory scans, and more importantly
//!   `mecha eval` and the replay probes must not read *live* machine state:
//!   a scorecard that varies with how busy the box was is not a scorecard, and
//!   an arm that samples today's backlog is measuring the afternoon rather than
//!   the change. Anything reconstructing a run reads the recorded snapshot.
//! - **Absent is not zero.** Every field is `Option` and `None` means the
//!   sensor could not be read, never that its value was zero — the rule
//!   [`crate::backlog`] states at length and for the same reason.
//! - **It never reaches the system prompt.** Render order is tools → system →
//!   messages with the cache breakpoint on the last system block, so a
//!   per-turn value there would re-pay the whole prefix, tools included, every
//!   request. Whatever eventually shows this to a model puts it in the turn
//!   tail or in a tool result.
//!
//! **Deliberately not sampled yet: llama-server's `/slots`.** It is the best
//! load signal available — occupancy directly rather than by proxy — but it is
//! an HTTP call, and nothing reads it yet. A sensor with no consumer should not
//! put a request in the path of every run's start; it goes in beside whatever
//! first needs it.
//!
//! [`RunContext`]: crate::agent::RunContext

use crate::backlog::{Backlog, BacklogDelta, Inventory};
use crate::pressure::ContextTracker;
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// A run's conditions: sampled at start, completed at end.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Homeostat {
    /// One-minute load average.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_avg_1m: Option<f32>,
    /// `MemAvailable`, in kB. On unified-memory hardware this is the *only*
    /// memory sensor — `nvidia-smi` reports `[N/A]` for GPU memory on GB10,
    /// because there is one pool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_available_kb: Option<u64>,
    /// What was waiting on the owner when the run began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backlog: Option<Backlog>,
    /// What this run added to that, or took off it. See [`BacklogDelta`] —
    /// the level alone cannot separate a run's own output from what it
    /// inherited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backlog_delta: Option<BacklogDelta>,
    /// The largest prompt this run actually sent.
    ///
    /// The **peak**, not the last and not the total. The total is what
    /// `RunOutcome::usage` already reports and it is the wrong number for this
    /// — it sums every prompt the run ever sent, so a long conversation shows
    /// a figure several times the window and reads as impossible pressure. The
    /// last is the wrong number too: a run that spent twenty turns at the
    /// threshold and then compacted would record the small number, which is
    /// the one moment it was not under pressure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_prompt_tokens: Option<u64>,
    /// That peak as a share of the context window, when the window is known.
    ///
    /// `None` where it is not — the fraction is unknowable rather than zero,
    /// and a provider with no declared `context_window` is common enough that
    /// recording 0.0 there would put a floor of healthy-looking rows under
    /// every later reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_context_pressure: Option<f32>,
    /// A harness-computed proxy for anticipated guilt
    /// (`docs/GOAL-SYSTEM-DESIGN.md` §7.4) — predicted error against another
    /// party's expectation: the standing **level**, folded from how long the
    /// oldest recorded commitment in [`backlog`](Self::backlog)'s stores
    /// has waited and how much room this run had to act on it. What this
    /// run did about it is the next field, deliberately not folded in here:
    /// `Corpus::mean_anticipated_guilt` averages this across every row the
    /// store holds, and `anticipated_guilt`'s own doc chose `None` over a
    /// differently-computed number precisely so that mean stays one
    /// quantity — writing the relief-scaled reading into the same field
    /// blended two formulas with nothing marking which (found on review).
    /// See [`crate::guilt`] for the formula and, importantly, for what this
    /// is *not* used for yet: only the diagnostician's brief reads it. It is
    /// recorded so the corpus exists before anything is built on it, on
    /// `runlog`'s own rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anticipated_guilt: Option<f32>,
    /// The level above, scaled down by the owner-facing share of the
    /// inherited backlog this run cleared (`guilt::with_backlogs`) — the
    /// run's *act* on the situation, in its own field so the level stays
    /// comparable across every row. `None` wherever the level is, or where
    /// the delta could not be read. Nothing consumes this yet either.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guilt_after_relief: Option<f32>,
    /// Each sensored charter line, read against its store as the run began
    /// (`docs/GOAL-SYSTEM-DESIGN.md` §11.1's readings; [`crate::reading`]):
    /// the line-specific form of the guilt above, one reading per line
    /// instead of one number over three stores. Taken off the inherited
    /// backlog for the same reason `anticipated_guilt` is — what waited on
    /// the owner as the run began, not what the run left. The corpus kind
    /// (`intervention_rate`) reads `Deferred` here; the surfaces read it.
    ///
    /// `None` when no reading was taken — a row from before the field, or a
    /// charter that did not load — and `Some([])` when the charter carries
    /// no sensor: absent and empty are different facts, on
    /// `RunStats::delivered`'s shape. A row whose readings this binary
    /// cannot parse (a kind or state a later one wrote) loads as `None`
    /// rather than failing the record: a closed enum written to an
    /// append-only store is a wire format.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::reading::lenient"
    )]
    pub charter: Option<Vec<crate::reading::LineReading>>,
    /// The items the start's depths were counted from, held for `finish`
    /// to difference id by id ([`crate::backlog::Flow`]). **Never
    /// recorded** — the record carries what was computed from it — so a
    /// snapshot loaded back from a session file has none, and nothing
    /// reconstructing a run can difference against today's stores.
    #[serde(skip)]
    pub start_items: Option<Inventory>,
}

impl Homeostat {
    /// Sample the conditions at the start of a run.
    pub fn at_start() -> Homeostat {
        let survey = Backlog::survey();
        // The charter is loaded here rather than handed in: it is global
        // and read-only by construction (`charter.rs`), exactly as the
        // backlog's stores are, and the reading is a fact about the machine
        // — what waits on the owner against the owner's own numbers — that
        // holds whether or not this run's prompt carried the charter. A
        // charter that does not load records `None`: unknown, not
        // sensorless.
        let charter = crate::charter::Charter::default_path()
            .ok()
            .and_then(|p| crate::charter::Charter::load(&p).ok())
            .map(|c| {
                let mut readings = crate::reading::read_lines(
                    &c,
                    &crate::reading::Sources {
                        backlog: &survey.backlog,
                        requests_on_owner: survey.requests_on_owner.clone(),
                        corpus: crate::reading::CorpusRate::NotScanned,
                        items: Some(&survey.items),
                    },
                    Utc::now(),
                );
                // S5: a line saturated over the recorded runs is withheld
                // from this run's consumers. Streamed newest first and
                // stopped as soon as each over-setpoint line is decided,
                // and not read at all when no line is over — the corpus
                // kind's full scan stays a surface's cost, not a run's.
                // The store the runs record into (`Session::default_dir`,
                // which honours `MECHA_SESSION_DIR`), since it is their
                // history being read.
                if let Ok(sessions) = crate::session::Session::default_dir() {
                    crate::reading::withdraw_saturated(
                        &mut readings,
                        &c,
                        crate::reading::recorded_readings(&sessions),
                    );
                }
                readings
            });
        Homeostat {
            load_avg_1m: load_avg_1m(),
            mem_available_kb: mem_available_kb(),
            backlog: Some(survey.backlog),
            charter,
            start_items: Some(survey.items),
            ..Homeostat::default()
        }
    }

    /// The charter readings a run's in-run consumers read — every line but
    /// the withdrawn ones (`reading::in_run`). The one door
    /// `Decision::assess`'s input comes through, so a consumer added later
    /// cannot reach a saturated line by reading `charter` directly without
    /// someone deciding to.
    pub fn in_run_readings(&self) -> Option<Vec<crate::reading::LineReading>> {
        crate::reading::in_run(self.charter.as_deref())
    }

    /// Complete the snapshot at the end of a run: difference the backlog, and
    /// take the context pressure off the run's own size series.
    ///
    /// The tracker is the source rather than `RunOutcome::usage` for the
    /// reason `peak_prompt_tokens` gives — the usage total is a sum across
    /// turns, and the quantity wanted is a maximum over them. It is passed in
    /// rather than read from anywhere, because the series exists only in the
    /// loop's memory and is deliberately never stored: what is worth keeping
    /// is the one number below, not a per-turn trace of every run.
    pub fn finish(mut self, pressure: &ContextTracker, window: Option<u64>) -> Homeostat {
        // Zero means the run never got a response — no request was ever
        // priced — which is an absence and not a measurement of nought.
        self.peak_prompt_tokens = (pressure.peak_tokens() > 0).then(|| pressure.peak_tokens());
        self.peak_context_pressure = pressure.peak_pressure(window);
        if let Some(before) = &self.backlog {
            // Guilt is computed from what this run *inherited*, not what it
            // leaves behind — the same distinction `backlog_delta` already
            // makes ("the level alone cannot separate a run's own output
            // from what it inherited"). Reading it off `after` instead would
            // score a trigger that staged three replies overnight as
            // maximally guilty for doing exactly its job: those drafts are
            // seconds old and this run's own output, not neglected debt.
            let survey = Backlog::survey();
            let after = survey.backlog;
            let now = Utc::now();
            // The level is what the run inherited, and it keeps its own
            // field so the corpus mean over it stays one quantity. The
            // delta is what the run did about it, and only relief moves
            // the second reading — the comment above is still the rule,
            // and `guilt::with_backlogs` keeps it: a run that added to the
            // queue reads the level it inherited, not a maximum for doing
            // its job, and relief is the owner-facing share of what was
            // waiting (one seam derives both numbers from this same pair of
            // reads — found on review, when the numerator spanned five
            // stores and the denominator three).
            let fold = crate::guilt::with_backlogs(before, &after, self.peak_context_pressure, now);
            self.anticipated_guilt = fold.level;
            self.guilt_after_relief = fold.after_relief;
            let mut delta = fold.delta;
            // The per-item delta, from the same two reads: which ids the
            // run's window added and cleared, per store and per line. Only
            // where the start's items were held — a snapshot rebuilt from a
            // record has none, and differencing it against today's stores
            // would measure the afternoon.
            if let Some(before) = &self.start_items {
                delta.flow = Some(Inventory::flows(before, &survey.items));
                if let Some(readings) = self.charter.as_mut() {
                    crate::reading::with_deltas(readings, before, &survey.items);
                }
            }
            self.backlog_delta = Some(delta);
        }
        self
    }
}

fn load_avg_1m() -> Option<f32> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn mem_available_kb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("MemAvailable:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorded snapshot has to survive a round trip through a session
    /// file, because replay reads it back rather than re-sampling — a run
    /// reconstructed against today's machine state is measuring the afternoon.
    #[test]
    fn a_snapshot_round_trips_and_an_older_record_without_one_still_loads() {
        let h = Homeostat {
            load_avg_1m: Some(0.56),
            mem_available_kb: Some(21_000_000),
            backlog: Some(Backlog::default()),
            backlog_delta: Some(BacklogDelta::default()),
            peak_prompt_tokens: Some(18_008),
            peak_context_pressure: Some(0.0687),
            anticipated_guilt: Some(0.0),
            guilt_after_relief: Some(0.0),
            charter: Some(vec![crate::reading::LineReading {
                line: "waits".into(),
                kind: crate::charter::SensorKind::OutboxAge,
                setpoint: "24h".into(),
                reading: crate::reading::Reading::Nothing,
                items: None,
                delta: None,
                withdrawn: false,
            }]),
            start_items: None,
        };
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<Homeostat>(&json).unwrap(), h);

        // A reading this binary cannot parse — a kind a later one added —
        // costs the readings, never the row: the rest of the snapshot loads
        // and `charter` reads as unknown.
        let later = json.replace("\"outbox_age\"", "\"board_overdue\"");
        assert_ne!(later, json);
        let loaded: Homeostat = serde_json::from_str(&later).unwrap();
        assert_eq!(loaded.charter, None);
        assert_eq!(loaded.anticipated_guilt, Some(0.0));

        // Every field defaults, so a record written before this existed loads
        // as "nothing was sampled" rather than failing.
        let empty: Homeostat = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Homeostat::default());
    }

    /// The peak, and the two ways it can be absent.
    #[test]
    fn the_recorded_pressure_is_the_runs_high_water_mark() {
        let mut pressure = ContextTracker::new();
        pressure.observe(4_000, 4_000);
        pressure.observe(30_000, 30_000);
        // A compaction brought it back down. The row must still say the run
        // reached 30,000 — that is the pressure it ran under, and the last
        // reading is the one moment it was not under it.
        pressure.observe(6_000, 6_000);

        let h = Homeostat::default().finish(&pressure, Some(60_000));
        assert_eq!(h.peak_prompt_tokens, Some(30_000));
        assert_eq!(h.peak_context_pressure, Some(0.5));

        // No declared window: the count is still a fact, the fraction is not
        // knowable, and neither is reported as zero.
        let no_window = Homeostat::default().finish(&pressure, None);
        assert_eq!(no_window.peak_prompt_tokens, Some(30_000));
        assert_eq!(no_window.peak_context_pressure, None);

        // A run that never got a response priced nothing at all.
        let never = Homeostat::default().finish(&ContextTracker::new(), Some(60_000));
        assert_eq!(never.peak_prompt_tokens, None, "absent, not zero");
        assert_eq!(never.peak_context_pressure, None);
    }

    /// S5 through the real sampler, over a fixture home: one stale draft and
    /// two fresh ones, and ten recorded runs that read the line past its
    /// setpoint. The start reads the level saturated, the per-item form
    /// beside it, and withdraws the line from in-run consumers; a draft
    /// staged and one sent inside the run are the run's delta, which the
    /// net count reads as nothing.
    #[test]
    fn a_saturated_line_is_withdrawn_at_start_and_the_runs_delta_is_per_item() {
        use crate::reading::{Reading, SATURATED_AFTER_RUNS};
        use crate::session::{Record, RunStats, Session, SessionMeta};
        let home = crate::work::tests::HomeGuard::new();
        std::fs::write(
            home.dir().join("charter.toml"),
            "[[line]]\nid = \"replies\"\ntext = \"Answer people.\"\n\
             [line.sensor]\nkind = \"outbox_age\"\nsetpoint = \"24h\"\n",
        )
        .unwrap();
        let outbox = crate::outbox::OutboxStore::open(home.dir().join("outbox")).unwrap();
        let stale = outbox
            .stage_by_harness("mail_send", serde_json::json!({}))
            .unwrap();
        let path = home.dir().join("outbox").join(format!("{}.json", stale.id));
        let mut raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        raw["created_at"] =
            serde_json::json!((Utc::now() - chrono::Duration::days(5)).to_rfc3339());
        std::fs::write(&path, raw.to_string()).unwrap();
        let fresh = outbox
            .stage_by_harness("mail_send", serde_json::json!({}))
            .unwrap();
        outbox
            .stage_by_harness("mail_send", serde_json::json!({}))
            .unwrap();
        for i in 0..SATURATED_AFTER_RUNS {
            let s = Session::create(
                &home.dir().join("sessions"),
                SessionMeta {
                    id: format!("20260901T00{i:02}00-r"),
                    created_at: Utc::now() - chrono::Duration::hours(20 - i as i64),
                    provider: "local".into(),
                    model: "m".into(),
                    workspace: std::path::PathBuf::from("/tmp"),
                    title: None,
                    kind: None,
                },
            )
            .unwrap();
            s.append(&Record::Outcome(RunStats {
                homeostat: Some(Homeostat {
                    charter: Some(vec![crate::reading::LineReading {
                        line: "replies".into(),
                        kind: crate::charter::SensorKind::OutboxAge,
                        setpoint: "24h".into(),
                        reading: Reading::Observed {
                            value: crate::reading::Observed::Seconds(200_000),
                            over: true,
                            excess: 0.5,
                        },
                        items: None,
                        delta: None,
                        withdrawn: false,
                    }]),
                    ..Default::default()
                }),
                ..Default::default()
            }))
            .unwrap();
        }

        let h = Homeostat::at_start();
        let line = &h.charter.as_ref().unwrap()[0];
        assert_eq!(line.reading.over(), Some(true), "the level: saturated");
        let items = line.items.clone().unwrap();
        assert_eq!((items.waiting, items.over), (3, 1));
        assert_eq!(items.stale, vec![stale.id.clone()]);
        assert!(line.withdrawn, "saturated over ten runs and still over");
        assert_eq!(h.in_run_readings(), Some(Vec::new()));

        // Inside the run: one draft staged, one sent.
        outbox
            .stage_by_harness("mail_send", serde_json::json!({}))
            .unwrap();
        outbox.resolve(&fresh.id, "sent", None).unwrap();
        let done = h.finish(&ContextTracker::new(), None);
        let delta = done.backlog_delta.unwrap();
        assert_eq!(delta.outbox, Some(0), "the net count reads nothing");
        let moved = crate::backlog::Flow {
            added: 1,
            cleared: 1,
        };
        assert_eq!(delta.flow.unwrap().outbox, Some(moved));
        let line = &done.charter.unwrap()[0];
        assert_eq!(line.delta, Some(moved));
        assert!(line.withdrawn, "the record keeps what the run was shown");
    }

    /// The sensors degrade rather than panic where /proc is absent or shaped
    /// differently. On this machine they read; the assertion that matters is
    /// that a miss is `None`.
    #[test]
    fn a_sensor_that_cannot_be_read_is_absent_rather_than_zero() {
        if let Some(load) = load_avg_1m() {
            assert!(load >= 0.0);
        }
        if let Some(kb) = mem_available_kb() {
            assert!(kb > 0, "MemAvailable parsed as zero would be a parse bug");
        }
    }
}

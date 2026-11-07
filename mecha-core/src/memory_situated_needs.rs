//! Private host assessment for situated interpretation. Never a prompt payload.

use crate::reading::{LineReading, Reading};
use chrono::{DateTime, Utc};
use std::time::Duration;

/// Timestamped private readings. Deliberately has no serialization interface.
#[derive(Clone, Debug)]
pub struct NeedsInput {
    sampled_at: DateTime<Utc>,
    max_age: Duration,
    readings: Option<Vec<LineReading>>,
}

/// Freshness is independent of each reading's meaning and withdrawal status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    Missing,
    Stale,
    Fresh,
}

/// Owner-audit data only. Do not add Serialize or pass this to generation.
#[derive(Clone, Debug, PartialEq)]
pub struct NeedsAssessment {
    pub freshness: Freshness,
    pub lines: Vec<LineReading>,
}

impl NeedsInput {
    pub fn new(
        sampled_at: DateTime<Utc>,
        max_age: Duration,
        readings: Option<Vec<LineReading>>,
    ) -> Self {
        Self {
            sampled_at,
            max_age,
            readings,
        }
    }

    pub fn assess(&self, now: DateTime<Utc>) -> anyhow::Result<NeedsAssessment> {
        anyhow::ensure!(!self.max_age.is_zero(), "invalid needs freshness threshold");
        let age = now
            .signed_duration_since(self.sampled_at)
            .to_std()
            .map_err(|_| anyhow::anyhow!("needs observation is in the future"))?;
        // Reject unrepresentable thresholds rather than accepting any timestamp.
        chrono::Duration::from_std(self.max_age)
            .map_err(|_| anyhow::anyhow!("invalid needs freshness threshold"))?;
        let Some(lines) = &self.readings else {
            return Ok(NeedsAssessment {
                freshness: Freshness::Missing,
                lines: vec![],
            });
        };
        for line in lines {
            if let Reading::Observed { value, excess, .. } = &line.reading {
                anyhow::ensure!(
                    excess.is_finite() && (0.0..1.0).contains(excess),
                    "invalid needs observation"
                );
                if let crate::reading::Observed::Rate(rate) = value {
                    anyhow::ensure!(
                        rate.is_finite() && (0.0..=1.0).contains(rate),
                        "invalid needs observation"
                    );
                }
            }
        }
        Ok(NeedsAssessment {
            freshness: if age > self.max_age {
                Freshness::Stale
            } else {
                Freshness::Fresh
            },
            lines: lines.clone(),
        })
    }
}

/// Opaque host context identity, independent of private readings and thresholds.
/// A host retains this identity for one context; it is not a hash of sensor data.
pub fn new_context_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{charter::SensorKind, reading::Observed};

    fn line(reading: Reading, withdrawn: bool) -> LineReading {
        LineReading {
            line: "PRIVATE_NEEDS_CANARY".into(),
            kind: SensorKind::OutboxWaiting,
            setpoint: "PRIVATE_THRESHOLD_CANARY".into(),
            reading,
            items: None,
            delta: None,
            withdrawn,
        }
    }
    fn at() -> DateTime<Utc> {
        "2026-11-07T12:00:00Z".parse().unwrap()
    }
    #[test]
    fn missing_stale_and_empty_are_distinct() {
        assert_eq!(
            NeedsInput::new(at(), Duration::from_secs(60), None)
                .assess(at())
                .unwrap()
                .freshness,
            Freshness::Missing
        );
        let empty = NeedsInput::new(at(), Duration::from_secs(60), Some(vec![]));
        assert_eq!(empty.assess(at()).unwrap().freshness, Freshness::Fresh);
        assert_eq!(
            empty
                .assess(at() + chrono::Duration::seconds(60))
                .unwrap()
                .freshness,
            Freshness::Fresh
        );
        assert_eq!(
            empty
                .assess(at() + chrono::Duration::seconds(61))
                .unwrap()
                .freshness,
            Freshness::Stale
        );
        assert!(empty.assess(at() - chrono::Duration::seconds(1)).is_err());
        assert!(NeedsInput::new(at(), Duration::ZERO, None)
            .assess(at())
            .is_err());
    }
    #[test]
    fn readings_and_withdrawal_remain_separate_without_a_scalar() {
        let readings = vec![
            Reading::Unread,
            Reading::Deferred,
            Reading::Nothing,
            Reading::Sparse { runs: 2 },
            Reading::Observed {
                value: Observed::Count(0),
                over: false,
                excess: 0.0,
            },
        ];
        let input = NeedsInput::new(
            at(),
            Duration::from_secs(60),
            Some(
                readings
                    .iter()
                    .cloned()
                    .map(|r| line(r, false))
                    .chain([line(readings[4].clone(), true)])
                    .collect(),
            ),
        );
        let result = input.assess(at()).unwrap();
        for (actual, expected) in result.lines.iter().zip(&readings) {
            assert_eq!(&actual.reading, expected);
            assert!(!actual.withdrawn);
        }
        assert!(result.lines[5].withdrawn);
        assert_eq!(result.lines[5].reading, readings[4]);
        let stale = input.assess(at() + chrono::Duration::seconds(61)).unwrap();
        assert_eq!(
            stale.lines, result.lines,
            "freshness never erases withdrawal or observed zero"
        );
    }
    #[test]
    fn invalid_observation_is_refused_and_context_identity_is_random() {
        for excess in [f32::NAN, -0.1, 1.0] {
            let input = NeedsInput::new(
                at(),
                Duration::from_secs(60),
                Some(vec![line(
                    Reading::Observed {
                        value: Observed::Count(0),
                        over: false,
                        excess,
                    },
                    false,
                )]),
            );
            assert!(input.assess(at()).is_err());
        }
        for rate in [f64::NAN, -0.1, 1.1] {
            let input = NeedsInput::new(
                at(),
                Duration::from_secs(60),
                Some(vec![line(
                    Reading::Observed {
                        value: Observed::Rate(rate),
                        over: false,
                        excess: 0.0,
                    },
                    false,
                )]),
            );
            assert!(input.assess(at()).is_err());
        }
        assert!(NeedsInput::new(at(), Duration::MAX, None)
            .assess(at())
            .is_err());
        let first = new_context_id();
        let second = new_context_id();
        assert_ne!(first, second);
        assert!(uuid::Uuid::parse_str(&first).is_ok());
    }
}

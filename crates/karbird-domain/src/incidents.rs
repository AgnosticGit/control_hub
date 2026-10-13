//! Pure incident decision rules. Storage and scheduling live outside this module.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
    ErrorRate,
    P95Latency,
}

impl MetricKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "error_rate" => Some(Self::ErrorRate),
            "p95_latency" => Some(Self::P95Latency),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RuleThreshold {
    pub kind: MetricKind,
    /// Percent for error rate, milliseconds for p95 latency.
    pub threshold: f64,
    pub min_requests: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Observation {
    pub requests: u64,
    pub errors: u64,
    pub p95_ns: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Bad,
    Good,
    Unknown,
}

pub fn classify(rule: RuleThreshold, observation: Observation) -> Verdict {
    if observation.requests < rule.min_requests || observation.requests == 0 {
        return Verdict::Unknown;
    }
    let value = match rule.kind {
        MetricKind::ErrorRate => observation.errors as f64 * 100.0 / observation.requests as f64,
        MetricKind::P95Latency => observation.p95_ns / 1_000_000.0,
    };
    if !value.is_finite() {
        Verdict::Unknown
    } else if value > rule.threshold {
        Verdict::Bad
    } else {
        Verdict::Good
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Streak {
    pub bad: i32,
    pub good: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    None,
    Open,
    Recover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub streak: Streak,
    pub transition: Transition,
}

pub fn advance(previous: Streak, verdict: Verdict, has_open_incident: bool) -> Decision {
    match verdict {
        Verdict::Unknown => Decision { streak: Streak::default(), transition: Transition::None },
        Verdict::Bad => {
            let bad = previous.bad.saturating_add(1).min(2);
            Decision {
                streak: Streak { bad, good: 0 },
                transition: if bad == 2 && !has_open_incident { Transition::Open } else { Transition::None },
            }
        }
        Verdict::Good => {
            let good = previous.good.saturating_add(1).min(2);
            Decision {
                streak: Streak { bad: 0, good },
                transition: if good == 2 && has_open_incident { Transition::Recover } else { Transition::None },
            }
        }
    }
}

/// Consecutive evidence must come from adjacent closed minutes. A delayed evaluator
/// starts a new streak instead of counting an old observation twice.
pub fn consecutive_streak(previous: Streak, previous_end_ns: i64, next_end_ns: i64) -> Streak {
    if next_end_ns.checked_sub(previous_end_ns) == Some(60_000_000_000) { previous } else { Streak::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_rate_requires_enough_requests_and_strict_threshold() {
        let rule = RuleThreshold { kind: MetricKind::ErrorRate, threshold: 20.0, min_requests: 20 };
        assert_eq!(classify(rule, Observation { requests: 19, errors: 19, p95_ns: 0.0 }), Verdict::Unknown);
        assert_eq!(classify(rule, Observation { requests: 20, errors: 4, p95_ns: 0.0 }), Verdict::Good);
        assert_eq!(classify(rule, Observation { requests: 20, errors: 5, p95_ns: 0.0 }), Verdict::Bad);
    }

    #[test]
    fn p95_ignores_invalid_or_missing_data() {
        let rule = RuleThreshold { kind: MetricKind::P95Latency, threshold: 250.0, min_requests: 10 };
        assert_eq!(classify(rule, Observation { requests: 10, errors: 0, p95_ns: f64::NAN }), Verdict::Unknown);
        assert_eq!(classify(rule, Observation { requests: 9, errors: 0, p95_ns: 300_000_000.0 }), Verdict::Unknown);
        assert_eq!(classify(rule, Observation { requests: 10, errors: 0, p95_ns: 250_000_000.0 }), Verdict::Good);
        assert_eq!(classify(rule, Observation { requests: 10, errors: 0, p95_ns: 250_000_001.0 }), Verdict::Bad);
    }

    #[test]
    fn opens_after_two_bad_windows_and_recovers_after_two_good_windows() {
        let mut streak = Streak::default();
        let a = advance(streak, Verdict::Bad, false);
        assert_eq!(a.transition, Transition::None);
        streak = a.streak;
        let a = advance(streak, Verdict::Bad, false);
        assert_eq!(a.transition, Transition::Open);
        streak = a.streak;
        let a = advance(streak, Verdict::Good, true);
        assert_eq!(a.transition, Transition::None);
        streak = a.streak;
        let a = advance(streak, Verdict::Good, true);
        assert_eq!(a.transition, Transition::Recover);
    }

    #[test]
    fn unknown_resets_streak_without_recovering_open_incident() {
        let a = advance(Streak { bad: 1, good: 1 }, Verdict::Unknown, true);
        assert_eq!(a.streak, Streak::default());
        assert_eq!(a.transition, Transition::None);
        assert_eq!(advance(a.streak, Verdict::Good, true).transition, Transition::None);
    }

    #[test]
    fn skipped_minute_breaks_consecutive_streak() {
        let streak = Streak { bad: 1, good: 0 };
        assert_eq!(consecutive_streak(streak, 60_000_000_000, 120_000_000_000), streak);
        assert_eq!(consecutive_streak(streak, 60_000_000_000, 180_000_000_000), Streak::default());
    }
}

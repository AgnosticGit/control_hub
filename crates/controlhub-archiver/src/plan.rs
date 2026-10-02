//! Which archive windows to process next. Pure: the caller passes the clock and the state.

/// Archive windows are aligned to multiples of the window length (UTC).
pub fn align_down(t: i64, window: i64) -> i64 {
    t.div_euclid(window).saturating_mul(window)
}

#[derive(Debug, Clone, Copy)]
pub struct PlanInput {
    pub now: i64,
    pub window: i64,
    /// A window is archived only after it has been closed this long (late telemetry).
    pub min_age: i64,
    /// ClickHouse retention (table TTL).
    pub hot_retention: i64,
    /// Windows starting closer than this to the TTL edge may already have lost rows.
    pub safety: i64,
    pub earliest_data: Option<i64>,
    /// End of the newest finished window; `None` before the first run.
    pub last_done_end: Option<i64>,
    pub max_windows: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Windows that expired from ClickHouse before they could be archived.
    pub lost: Vec<(i64, i64)>,
    pub archive: Vec<(i64, i64)>,
}

/// First window start that is still complete in ClickHouse.
pub fn first_safe_start(i: &PlanInput) -> i64 {
    let edge = i.now.saturating_sub(i.hot_retention).saturating_add(i.safety);
    let aligned = align_down(edge, i.window);
    if aligned < edge { aligned.saturating_add(i.window) } else { aligned }
}

/// End of the newest closed window.
pub fn closed_until(i: &PlanInput) -> i64 {
    align_down(i.now.saturating_sub(i.min_age), i.window)
}

pub fn plan(i: &PlanInput) -> Plan {
    let mut out = Plan::default();
    if i.window <= 0 {
        return out;
    }
    let safe = first_safe_start(i);
    let until = closed_until(i);
    // Before the first run there was no archiver, so older data is not reported as lost.
    let mut start = match (i.last_done_end, i.earliest_data) {
        (Some(end), _) => end,
        (None, Some(t)) => align_down(t, i.window).max(safe),
        (None, None) => return out,
    };
    while start < until && out.archive.len() < i.max_windows && out.lost.len() < 10_000 {
        let end = start.saturating_add(i.window);
        if start < safe {
            out.lost.push((start, end));
        } else {
            out.archive.push((start, end));
        }
        start = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const H: i64 = 3_600_000_000_000;
    const DAY: i64 = 24 * H;

    fn input(now: i64) -> PlanInput {
        PlanInput {
            now,
            window: H,
            min_age: H,
            hot_retention: 14 * DAY,
            safety: H,
            earliest_data: None,
            last_done_end: None,
            max_windows: 1_000,
        }
    }

    #[test]
    fn nothing_to_do_without_data() {
        assert_eq!(plan(&input(100 * DAY)), Plan::default());
    }

    #[test]
    fn first_run_starts_at_the_oldest_data_but_not_before_the_safe_edge() {
        let now = 100 * DAY + 30 * 60_000_000_000;
        let p = plan(&PlanInput { earliest_data: Some(now - 3 * H), ..input(now) });
        assert!(p.lost.is_empty());
        // now = 100d 00:30 → closed until 99d 23:00; data from 99d 21:30 → window 99d 21:00.
        assert_eq!(p.archive, vec![(100 * DAY - 3 * H, 100 * DAY - 2 * H), (100 * DAY - 2 * H, 100 * DAY - H)]);

        let p = plan(&PlanInput { earliest_data: Some(now - 30 * DAY), ..input(now) });
        assert!(p.lost.is_empty(), "data that predates the archiver is not 'lost'");
        assert_eq!(p.archive.first().map(|w| w.0), Some(first_safe_start(&input(now))));
    }

    #[test]
    fn continues_after_the_last_window_and_reports_expired_gaps_as_lost() {
        let now = 100 * DAY;
        let p = plan(&PlanInput { last_done_end: Some(now - 3 * H), ..input(now) });
        assert_eq!(p.archive, vec![(now - 3 * H, now - 2 * H), (now - 2 * H, now - H)]);

        let p = plan(&PlanInput { last_done_end: Some(now - 20 * DAY), ..input(now) });
        let safe = first_safe_start(&input(now));
        assert_eq!(p.lost.first().map(|w| w.0), Some(now - 20 * DAY));
        assert_eq!(p.lost.last().map(|w| w.1), Some(safe));
        assert_eq!(p.archive.first().map(|w| w.0), Some(safe));
    }

    #[test]
    fn respects_min_age_and_the_per_tick_cap() {
        let now = 100 * DAY + H / 2;
        let p = plan(&PlanInput { last_done_end: Some(100 * DAY - H), ..input(now) });
        assert!(p.archive.is_empty(), "the last hour is not closed long enough yet");
        let p = plan(&PlanInput { last_done_end: Some(now - 5 * DAY), max_windows: 3, ..input(now) });
        assert_eq!(p.archive.len(), 3);
        assert!(plan(&PlanInput { window: 0, ..input(now) }).archive.is_empty());
    }

    proptest! {
        #[test]
        fn windows_are_aligned_contiguous_closed_and_safe(
            now in 30 * DAY..3_000 * DAY,
            back in 0i64..40 * DAY,
            data_back in 0i64..40 * DAY,
            first_run in any::<bool>(),
            window_h in 1i64..6,
        ) {
            let window = window_h * H;
            let i = PlanInput {
                window,
                last_done_end: (!first_run).then(|| align_down(now - back, window)),
                earliest_data: Some(now - data_back),
                ..input(now)
            };
            let p = plan(&i);
            let all: Vec<_> = p.lost.iter().chain(p.archive.iter()).copied().collect();
            for w in all.windows(2) {
                prop_assert_eq!(w[0].1, w[1].0, "contiguous");
            }
            for (s, e) in &all {
                prop_assert_eq!(e - s, window);
                prop_assert_eq!(s.rem_euclid(window), 0);
                prop_assert!(*e <= now - i.min_age, "only closed windows");
            }
            for (s, _) in &p.archive {
                prop_assert!(*s >= now - i.hot_retention + i.safety, "never archive a window ClickHouse may have trimmed");
            }
            if first_run {
                prop_assert!(p.lost.is_empty());
            }
        }
    }
}

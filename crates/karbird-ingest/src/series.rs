//! Bounds metric cardinality per environment: at most `max_per_env` distinct series per hourly
//! window. The window is per ingest instance, so with N ingest replicas the effective limit is up
//! to N times higher (ADR-0018).

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

pub const DEFAULT_MAX_SERIES_PER_ENV: usize = 10_000;
const WINDOW: Duration = Duration::from_secs(3600);

#[derive(Debug)]
struct Window {
    started: Instant,
    series: HashSet<u64>,
}

#[derive(Debug)]
pub struct SeriesLimiter {
    max_per_env: usize,
    window: Duration,
    envs: Mutex<HashMap<String, Window>>,
}

impl SeriesLimiter {
    pub fn new(max_per_env: usize) -> Self {
        Self::with_window(max_per_env, WINDOW)
    }

    pub fn with_window(max_per_env: usize, window: Duration) -> Self {
        Self { max_per_env, window, envs: Mutex::new(HashMap::new()) }
    }

    pub fn admit(&self, env_key: &str, series: u64) -> bool {
        self.admit_at(env_key, series, Instant::now())
    }

    /// Known series are always admitted; a new one only while the environment is under its limit.
    pub fn admit_at(&self, env_key: &str, series: u64, now: Instant) -> bool {
        let mut envs = self.envs.lock().unwrap_or_else(PoisonError::into_inner);
        let w = envs.entry(env_key.to_owned()).or_insert_with(|| Window { started: now, series: HashSet::new() });
        if now.saturating_duration_since(w.started) >= self.window {
            w.started = now;
            w.series.clear();
        }
        if w.series.contains(&series) {
            return true;
        }
        if w.series.len() >= self.max_per_env {
            return false;
        }
        w.series.insert(series);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_series_pass_and_new_ones_stop_at_the_limit() {
        let l = SeriesLimiter::new(2);
        let t = Instant::now();
        assert!(l.admit_at("e", 1, t));
        assert!(l.admit_at("e", 2, t));
        assert!(!l.admit_at("e", 3, t));
        assert!(l.admit_at("e", 1, t), "a known series is never refused");
        assert!(l.admit_at("e", 2, t));
    }

    #[test]
    fn environments_are_isolated() {
        let l = SeriesLimiter::new(1);
        let t = Instant::now();
        assert!(l.admit_at("tenant-a/p/e", 1, t));
        assert!(!l.admit_at("tenant-a/p/e", 2, t));
        assert!(l.admit_at("tenant-b/p/e", 2, t), "another tenant has its own budget");
    }

    #[test]
    fn the_window_resets_after_it_elapses() {
        let l = SeriesLimiter::with_window(1, Duration::from_secs(10));
        let t = Instant::now();
        assert!(l.admit_at("e", 1, t));
        assert!(!l.admit_at("e", 2, t + Duration::from_secs(9)));
        assert!(l.admit_at("e", 2, t + Duration::from_secs(10)));
        assert!(!l.admit_at("e", 1, t + Duration::from_secs(11)), "the old series is forgotten with the window");
    }

    #[test]
    fn zero_limit_refuses_everything() {
        let l = SeriesLimiter::new(0);
        assert!(!l.admit("e", 1));
    }
}

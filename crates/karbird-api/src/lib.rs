//! Reusable API logic, including pure route transformations used by benchmarks.

pub mod auth;
pub mod bootstrap;
pub mod error;
pub mod live_consumer;
pub mod live_hub;
pub mod live_metrics;
pub mod mail;
pub mod openapi;
pub mod permissions;
pub mod routes;
pub mod state;
pub mod system_permissions;
pub mod test_fixtures;
pub mod topology;

/// Advance the shared live cursor monotonically even if the wall clock stalls or moves back.
pub fn advance_live_cursor(previous: &mut i64, now: i64) -> i64 {
    *previous = now.max(previous.saturating_add(1));
    *previous
}

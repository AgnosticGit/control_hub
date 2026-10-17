//! Storage adapters. All telemetry queries take an explicit [`Scope`] and bind
//! it as a parameter; there is no API for arbitrary SQL.

pub mod actions;
pub mod admin;
pub mod archive;
pub mod ch;
pub mod control;
pub mod incidents;
pub mod pg;
pub mod retention;
pub mod secrets;
pub mod topology;

pub use karbird_domain::Scope;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("postgres: {0}")]
    Postgres(#[from] sqlx::Error),
    #[error("postgres migration: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("clickhouse: {0}")]
    ClickHouse(#[from] clickhouse::error::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T, E = StorageError> = std::result::Result<T, E>;

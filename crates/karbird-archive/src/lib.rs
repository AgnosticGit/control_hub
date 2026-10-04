//! karbird archive format (ADR-0015). Pure: no I/O, no clock.
//!
//! - [`template`]: lossless split of a log body into a template and typed variables.
//! - [`segment`]: columnar, zstd-compressed segment of one service's logs for one time window.
//! - [`dict`]: per-service zstd dictionaries that grow with the service's archived volume.
//! - [`index`]: per-segment search index (template IDs, max severity, trace bloom filter).

pub mod dict;
pub mod index;
pub mod segment;
pub mod template;
mod varint;

pub use dict::Dictionary;
pub use segment::{Encoded, LogRecord, SegmentHeader, SegmentStats, decode, encode, read_header};
pub use template::{Parsed, Var, parse, render, template_id};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ArchiveError {
    #[error("corrupt archive data: {0}")]
    Corrupt(&'static str),
    #[error("archive dictionary mismatch: segment needs version {needed}, got {got}")]
    DictionaryMismatch { needed: u32, got: u32 },
    #[error("compression: {0}")]
    Compression(String),
    #[error("template does not match its variables")]
    TemplateMismatch,
}

pub type Result<T, E = ArchiveError> = std::result::Result<T, E>;

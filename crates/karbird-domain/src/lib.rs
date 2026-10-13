//! Pure domain model for karbird.
//!
//! This crate must stay free of I/O, async runtimes and native-only
//! dependencies: it is compiled both natively and to `wasm32-unknown-unknown`.

pub mod ids;
pub mod incidents;
pub mod model;
pub mod redaction;
pub mod time;

pub use ids::*;
pub use model::*;

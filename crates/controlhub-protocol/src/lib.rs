//! Wire contracts shared by ingest and processor.

#[allow(clippy::all, missing_debug_implementations)]
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/controlhub.v1.rs"));
}

/// Official OTLP protobuf types and tonic services.
pub use opentelemetry_proto::tonic as otlp;

/// Current envelope schema version written by ingest.
pub const ENVELOPE_SCHEMA_VERSION: u32 = 1;

/// Broker topic carrying [`v1::IngestEnvelope`] records.
pub const TELEMETRY_TOPIC: &str = "controlhub.telemetry.v1";

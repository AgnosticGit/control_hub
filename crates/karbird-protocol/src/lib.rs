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

#[cfg(test)]
mod tests {
    use super::otlp::collector::trace::v1::ExportTraceServiceRequest;
    use super::otlp::trace::v1::{ResourceSpans, ScopeSpans, Span};
    use super::v1::{CustomEvent, CustomEventBatch, IngestEnvelope, IngestScope, ingest_envelope::Payload};
    use super::*;
    use prost::Message;

    fn envelope(payload: Option<Payload>) -> IngestEnvelope {
        IngestEnvelope {
            schema_version: ENVELOPE_SCHEMA_VERSION,
            batch_id: "0190-batch".into(),
            scope: Some(IngestScope {
                tenant_id: "t1".into(),
                project_id: "p1".into(),
                environment_id: "prod".into(),
                token_id: "tok-1".into(),
            }),
            received_time_unix_nano: 1_790_000_000_123_456_789,
            source_id: "ingest-1".into(),
            payload,
        }
    }

    #[test]
    fn constants_are_stable() {
        assert_eq!(ENVELOPE_SCHEMA_VERSION, 1);
        assert_eq!(TELEMETRY_TOPIC, "controlhub.telemetry.v1");
    }

    #[test]
    fn envelope_roundtrips_every_payload_variant() {
        let traces = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans {
                    spans: vec![Span { trace_id: vec![1; 16], span_id: vec![2; 8], ..Default::default() }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let mut event = CustomEvent {
            event_id: "e1".into(),
            event_type: "deployment.started".into(),
            source_sequence: Some(0),
            ..Default::default()
        };
        event.attributes.insert("k".into(), "v".into());
        for payload in [
            Some(Payload::OtlpTraces(traces.encode_to_vec())),
            Some(Payload::OtlpLogs(vec![])),
            Some(Payload::CustomEvents(CustomEventBatch { events: vec![event] })),
            None,
        ] {
            let env = envelope(payload);
            let decoded = IngestEnvelope::decode(env.encode_to_vec().as_slice()).unwrap();
            assert_eq!(decoded, env);
        }
    }

    #[test]
    fn nested_otlp_payload_survives_the_envelope() {
        let traces = ExportTraceServiceRequest {
            resource_spans: vec![ResourceSpans {
                scope_spans: vec![ScopeSpans {
                    spans: vec![Span { name: "GET /".into(), ..Default::default() }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let env = envelope(Some(Payload::OtlpTraces(traces.encode_to_vec())));
        let decoded = IngestEnvelope::decode(env.encode_to_vec().as_slice()).unwrap();
        let Some(Payload::OtlpTraces(bytes)) = decoded.payload else { panic!("wrong payload variant") };
        assert_eq!(ExportTraceServiceRequest::decode(bytes.as_slice()).unwrap(), traces);
    }

    #[test]
    fn optional_sequence_distinguishes_zero_from_absent() {
        for seq in [None, Some(0), Some(u64::MAX)] {
            let ev = CustomEvent { source_sequence: seq, ..Default::default() };
            assert_eq!(CustomEvent::decode(ev.encode_to_vec().as_slice()).unwrap().source_sequence, seq);
        }
    }

    #[test]
    fn empty_bytes_decode_to_defaults() {
        let env = IngestEnvelope::decode(&[][..]).unwrap();
        assert_eq!(env.schema_version, 0, "processors must treat version 0 as unsupported");
        assert!(env.scope.is_none());
        assert!(env.payload.is_none());
    }

    #[test]
    fn malformed_bytes_are_an_error_not_a_panic() {
        // Truncated length-delimited field and an invalid wire type.
        assert!(IngestEnvelope::decode(&[0x12, 0x05, b'a'][..]).is_err());
        assert!(IngestEnvelope::decode(&[0x0f][..]).is_err());
        let mut bytes = envelope(Some(Payload::OtlpLogs(vec![1, 2, 3]))).encode_to_vec();
        bytes.truncate(bytes.len() - 1);
        assert!(IngestEnvelope::decode(bytes.as_slice()).is_err());
    }

    #[test]
    fn unknown_fields_are_skipped_for_forward_compatibility() {
        let mut bytes = envelope(None).encode_to_vec();
        // Field 99, varint wire type, value 1.
        bytes.extend_from_slice(&[0x98, 0x06, 0x01]);
        let decoded = IngestEnvelope::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded, envelope(None));
    }
}

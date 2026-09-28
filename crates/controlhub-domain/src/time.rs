//! Nanosecond timestamps.
//!
//! Epoch nanoseconds exceed `Number.MAX_SAFE_INTEGER`, so in JSON they are
//! always encoded as decimal strings. Protobuf uses `int64`/`fixed64`, and the
//! WASM engine uses `i64` (JS `BigInt`) internally.

use serde::{Deserialize, Deserializer, Serializer};

pub type UnixNanos = i64;

pub const NANOS_PER_MILLI: i64 = 1_000_000;
pub const NANOS_PER_SEC: i64 = 1_000_000_000;

/// `#[serde(with = "controlhub_domain::time::nanos_string")]` for `i64` fields.
pub mod nanos_string {
    use super::*;

    pub fn serialize<S: Serializer>(value: &i64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Str(String),
            Int(i64),
        }
        match Repr::deserialize(d)? {
            Repr::Str(s) => {
                s.parse::<i64>().map_err(|_| serde::de::Error::custom("expected decimal nanosecond string"))
            }
            // Accept small integers (e.g. relative offsets); large epoch values
            // from JS numbers would already have lost precision upstream.
            Repr::Int(i) => Ok(i),
        }
    }
}

/// Same as [`nanos_string`] for `Option<i64>`.
pub mod opt_nanos_string {
    use super::*;

    pub fn serialize<S: Serializer>(value: &Option<i64>, s: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(v) => s.serialize_some(&v.to_string()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        let v: Option<String> = Option::deserialize(d)?;
        v.map(|s| s.parse::<i64>().map_err(|_| serde::de::Error::custom("expected decimal nanosecond string")))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct T {
        #[serde(with = "super::nanos_string")]
        t: i64,
    }

    #[test]
    fn roundtrip_exceeds_js_safe_integer() {
        let v = T { t: 1_790_000_000_123_456_789 };
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"t":"1790000000123456789"}"#);
        assert_eq!(serde_json::from_str::<T>(&json).unwrap(), v);
    }
}

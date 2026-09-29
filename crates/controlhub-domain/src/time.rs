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

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct O {
        #[serde(default, with = "super::opt_nanos_string")]
        t: Option<i64>,
    }

    #[test]
    fn nanos_string_accepts_small_integers_and_rejects_garbage() {
        assert_eq!(serde_json::from_str::<T>(r#"{"t":42}"#).unwrap(), T { t: 42 });
        assert_eq!(serde_json::from_str::<T>(r#"{"t":"-5"}"#).unwrap(), T { t: -5 });
        for bad in [r#"{"t":"abc"}"#, r#"{"t":"1.5"}"#, r#"{"t":""}"#, r#"{"t":null}"#, r#"{"t":1.5}"#, "{}"] {
            assert!(serde_json::from_str::<T>(bad).is_err(), "{bad}");
        }
        // Overflowing i64 must be an error, not a wrap-around.
        assert!(serde_json::from_str::<T>(r#"{"t":"9223372036854775808"}"#).is_err());
        assert_eq!(serde_json::from_str::<T>(r#"{"t":"9223372036854775807"}"#).unwrap(), T { t: i64::MAX });
    }

    #[test]
    fn opt_nanos_string_roundtrip() {
        let some = O { t: Some(1_790_000_000_123_456_789) };
        let json = serde_json::to_string(&some).unwrap();
        assert_eq!(json, r#"{"t":"1790000000123456789"}"#);
        assert_eq!(serde_json::from_str::<O>(&json).unwrap(), some);
        assert_eq!(serde_json::to_string(&O { t: None }).unwrap(), r#"{"t":null}"#);
        assert_eq!(serde_json::from_str::<O>(r#"{"t":null}"#).unwrap(), O { t: None });
        assert_eq!(serde_json::from_str::<O>("{}").unwrap(), O { t: None });
        assert!(serde_json::from_str::<O>(r#"{"t":"x"}"#).is_err());
        // Unlike `nanos_string`, the optional variant only accepts strings.
        assert!(serde_json::from_str::<O>(r#"{"t":5}"#).is_err());
    }

    #[test]
    fn constants() {
        assert_eq!(super::NANOS_PER_SEC, 1_000 * super::NANOS_PER_MILLI);
    }

    proptest::proptest! {
        #[test]
        fn nanos_string_roundtrips_any_i64(t in proptest::prelude::any::<i64>()) {
            let v = T { t };
            let json = serde_json::to_string(&v).unwrap();
            proptest::prop_assert_eq!(serde_json::from_str::<T>(&json).unwrap(), v);
        }
    }
}

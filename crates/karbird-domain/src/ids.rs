//! Strongly typed identifiers.
//!
//! A service *name* is never a global identifier: service and entity IDs are
//! derived from the full `(tenant, project, environment, ...)` scope, so two
//! services called `api` in different projects or environments never merge.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    #[error("{kind} must not be empty")]
    Empty { kind: &'static str },
    #[error("{kind} exceeds {max} bytes")]
    TooLong { kind: &'static str, max: usize },
    #[error("{kind} contains unsupported characters")]
    InvalidChars { kind: &'static str },
    #[error("{kind} must be {expected} lowercase hex characters")]
    InvalidHex { kind: &'static str, expected: usize },
}

const MAX_ID_LEN: usize = 128;

fn validate_token(kind: &'static str, value: &str) -> Result<(), IdError> {
    if value.is_empty() {
        return Err(IdError::Empty { kind });
    }
    if value.len() > MAX_ID_LEN {
        return Err(IdError::TooLong { kind, max: MAX_ID_LEN });
    }
    if !value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':')) {
        return Err(IdError::InvalidChars { kind });
    }
    Ok(())
}

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident, $kind:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                validate_token($kind, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// Tenant (organisation) identifier. Always taken from credentials, never from payloads.
    TenantId, "tenant_id");
string_id!(ProjectId, "project_id");
string_id!(EnvironmentId, "environment_id");
string_id!(
    /// Logical service identifier, derived from scope + service identity.
    ServiceId, "service_id");
string_id!(
    /// Concrete running instance (process / container / pod).
    InstanceId, "instance_id");
string_id!(
    /// Generic graph entity (service, resource, external dependency, deployment...).
    EntityId, "entity_id");
string_id!(EventId, "event_id");
string_id!(ActionId, "action_id");
string_id!(ActorId, "actor_id");

macro_rules! hex_id {
    ($name:ident, $kind:literal, $len:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub const HEX_LEN: usize = $len;

            pub fn parse(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                if value.len() != $len
                    || !value.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                {
                    return Err(IdError::InvalidHex {
                        kind: $kind,
                        expected: $len,
                    });
                }
                Ok(Self(value))
            }

            /// Builds from raw bytes (e.g. OTLP). Returns `None` for wrong length or all-zero IDs,
            /// which OTLP defines as invalid.
            pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
                if bytes.len() * 2 != $len || bytes.iter().all(|b| *b == 0) {
                    return None;
                }
                Some(Self(hex::encode(bytes)))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

hex_id!(TraceId, "trace_id", 32);
hex_id!(SpanId, "span_id", 16);

/// The full scope every stored record belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scope {
    pub tenant_id: TenantId,
    pub project_id: ProjectId,
    pub environment_id: EnvironmentId,
}

fn scoped_hash(prefix: &str, scope: &Scope, parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in [scope.tenant_id.as_str(), scope.project_id.as_str(), scope.environment_id.as_str()]
        .into_iter()
        .chain(parts.iter().copied())
    {
        // Length-prefix every component so ("ab","c") != ("a","bc").
        h.update((p.len() as u64).to_le_bytes());
        h.update(p.as_bytes());
    }
    let digest = h.finalize();
    format!("{prefix}_{}", hex::encode(&digest[..12]))
}

impl ServiceId {
    /// Derive the logical service ID from scope + OTel `service.namespace` / `service.name`.
    pub fn derive(scope: &Scope, namespace: &str, name: &str) -> Self {
        Self(scoped_hash("svc", scope, &["service", namespace, name]))
    }
}

impl EntityId {
    /// ID for a logical service entity in the graph (same value space as [`ServiceId`]).
    pub fn for_service(service: &ServiceId) -> Self {
        Self(service.0.clone())
    }

    /// ID for an infrastructure resource or external dependency, e.g.
    /// `("database", "postgresql", "orders-db")`.
    pub fn derive_resource(scope: &Scope, kind: &str, system: &str, name: &str) -> Self {
        Self(scoped_hash("res", scope, &[kind, system, name]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(t: &str, p: &str, e: &str) -> Scope {
        Scope {
            tenant_id: TenantId::parse(t).unwrap(),
            project_id: ProjectId::parse(p).unwrap(),
            environment_id: EnvironmentId::parse(e).unwrap(),
        }
    }

    #[test]
    fn same_name_different_scope_never_collides() {
        let a = ServiceId::derive(&scope("t1", "p1", "prod"), "", "api");
        let b = ServiceId::derive(&scope("t1", "p1", "staging"), "", "api");
        let c = ServiceId::derive(&scope("t1", "p2", "prod"), "", "api");
        let d = ServiceId::derive(&scope("t2", "p1", "prod"), "", "api");
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
        assert_eq!(a, ServiceId::derive(&scope("t1", "p1", "prod"), "", "api"));
    }

    #[test]
    fn component_boundaries_are_unambiguous() {
        let s = scope("t", "p", "e");
        assert_ne!(ServiceId::derive(&s, "ab", "c"), ServiceId::derive(&s, "a", "bc"));
    }

    #[test]
    fn trace_ids_validate() {
        assert!(TraceId::parse("0123456789abcdef0123456789abcdef").is_ok());
        assert!(TraceId::parse("0123456789ABCDEF0123456789ABCDEF").is_err());
        assert!(TraceId::from_bytes(&[0u8; 16]).is_none());
        assert!(SpanId::from_bytes(&[1u8; 8]).is_some());
    }

    #[test]
    fn string_ids_reject_injection() {
        assert!(TenantId::parse("a b").is_err());
        assert!(TenantId::parse("").is_err());
        assert!(TenantId::parse("x'; drop table").is_err());
    }

    #[test]
    fn string_id_errors_are_specific() {
        assert_eq!(ProjectId::parse(""), Err(IdError::Empty { kind: "project_id" }));
        assert_eq!(
            EnvironmentId::parse("a".repeat(MAX_ID_LEN + 1)),
            Err(IdError::TooLong { kind: "environment_id", max: MAX_ID_LEN })
        );
        assert!(EnvironmentId::parse("a".repeat(MAX_ID_LEN)).is_ok());
        assert_eq!(EventId::parse("a/b"), Err(IdError::InvalidChars { kind: "event_id" }));
        assert_eq!(IdError::Empty { kind: "tenant_id" }.to_string(), "tenant_id must not be empty");
    }

    #[test]
    fn string_ids_allow_documented_punctuation_only() {
        assert!(TenantId::parse("Acme-1_prod.eu:blue").is_ok());
        for bad in ["a\0b", "a\nb", "ü", "a;b", "a\"b", "a%b", "a\\b", "a\u{202e}b", " a"] {
            assert!(TenantId::parse(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn string_ids_deserialize_through_validation() {
        let ok: TenantId = serde_json::from_str("\"t1\"").unwrap();
        assert_eq!(ok.as_str(), "t1");
        assert_eq!(serde_json::to_string(&ok).unwrap(), "\"t1\"");
        assert!(serde_json::from_str::<TenantId>("\"bad id\"").is_err());
        assert!(serde_json::from_str::<TenantId>("\"\"").is_err());
        assert!(serde_json::from_str::<TraceId>("\"ABCDEF0123456789abcdef0123456789\"").is_err());
        let scope: Scope = serde_json::from_str(r#"{"tenant_id":"t","project_id":"p","environment_id":"e"}"#).unwrap();
        assert_eq!(scope, self::scope("t", "p", "e"));
        assert!(serde_json::from_str::<Scope>(r#"{"tenant_id":"t x","project_id":"p","environment_id":"e"}"#).is_err());
    }

    #[test]
    fn hex_ids_validate_length_and_case() {
        assert_eq!(SpanId::HEX_LEN, 16);
        assert_eq!(TraceId::HEX_LEN, 32);
        assert!(SpanId::parse("0123456789abcdef").is_ok());
        assert_eq!(SpanId::parse("0123456789abcde"), Err(IdError::InvalidHex { kind: "span_id", expected: 16 }));
        assert!(SpanId::parse("0123456789abcdef0").is_err());
        assert!(SpanId::parse("0123456789abcdeg").is_err());
        assert!(SpanId::parse("").is_err());
        // Multibyte chars whose byte length matches must still be rejected.
        assert_eq!("аааааааа".len(), 16);
        assert!(SpanId::parse("аааааааа").is_err());
    }

    #[test]
    fn hex_ids_from_bytes() {
        assert_eq!(SpanId::from_bytes(&[0xab; 8]).unwrap().as_str(), "abababababababab");
        assert!(SpanId::from_bytes(&[1; 7]).is_none());
        assert!(SpanId::from_bytes(&[1; 16]).is_none());
        assert!(SpanId::from_bytes(&[]).is_none());
        assert!(SpanId::from_bytes(&[0; 8]).is_none());
        let mut almost_zero = [0u8; 16];
        almost_zero[15] = 1;
        assert_eq!(TraceId::from_bytes(&almost_zero).unwrap().as_str(), format!("{}01", "0".repeat(30)));
    }

    #[test]
    fn derived_ids_have_stable_shape() {
        let s = scope("t", "p", "e");
        let svc = ServiceId::derive(&s, "ns", "api");
        assert!(svc.as_str().starts_with("svc_"));
        assert_eq!(svc.as_str().len(), 4 + 24);
        // Derived IDs are themselves valid IDs (they round-trip through parse).
        assert_eq!(ServiceId::parse(svc.to_string()).unwrap(), svc);
        assert_eq!(EntityId::for_service(&svc).as_str(), svc.as_str());
        let res = EntityId::derive_resource(&s, "database", "postgresql", "orders");
        assert!(res.as_str().starts_with("res_"));
        assert_ne!(res, EntityId::derive_resource(&s, "database", "postgresql", "orders2"));
        assert_ne!(res, EntityId::derive_resource(&scope("t2", "p", "e"), "database", "postgresql", "orders"));
        // Namespace is part of the identity.
        assert_ne!(svc, ServiceId::derive(&s, "", "api"));
    }

    mod props {
        use super::super::*;
        use super::scope;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn valid_string_ids_round_trip(v in "[A-Za-z0-9._:-]{1,128}") {
                let id = TenantId::parse(v.clone()).unwrap();
                prop_assert_eq!(id.as_str(), v.as_str());
                prop_assert_eq!(id.to_string(), v.clone());
                let json = serde_json::to_string(&id).unwrap();
                prop_assert_eq!(serde_json::from_str::<TenantId>(&json).unwrap(), id.clone());
                prop_assert_eq!(String::from(id), v);
            }

            #[test]
            fn arbitrary_strings_parse_iff_valid(v in any::<String>()) {
                let valid = !v.is_empty()
                    && v.len() <= 128
                    && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'));
                prop_assert_eq!(ProjectId::parse(v).is_ok(), valid);
            }

            #[test]
            fn hex_ids_round_trip_bytes(bytes in prop::array::uniform16(any::<u8>())) {
                match TraceId::from_bytes(&bytes) {
                    None => prop_assert!(bytes.iter().all(|b| *b == 0)),
                    Some(id) => {
                        prop_assert_eq!(hex::decode(id.as_str()).unwrap(), bytes.to_vec());
                        prop_assert_eq!(TraceId::parse(id.to_string()).unwrap(), id);
                    }
                }
            }

            #[test]
            fn service_ids_are_deterministic_and_scope_sensitive(
                t in "[a-z0-9]{1,8}", p in "[a-z0-9]{1,8}", e in "[a-z0-9]{1,8}",
                ns in ".{0,16}", name in ".{0,16}", other in "[a-z0-9]{1,8}",
            ) {
                let s = scope(&t, &p, &e);
                let a = ServiceId::derive(&s, &ns, &name);
                prop_assert_eq!(&a, &ServiceId::derive(&s, &ns, &name));
                if other != t {
                    prop_assert_ne!(&a, &ServiceId::derive(&scope(&other, &p, &e), &ns, &name));
                }
                if other != e {
                    prop_assert_ne!(&a, &ServiceId::derive(&scope(&t, &p, &other), &ns, &name));
                }
            }
        }
    }
}

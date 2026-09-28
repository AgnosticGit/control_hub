//! Server-side redaction applied at ingestion as a second line of defence.
//! SDKs must already avoid collecting sensitive data at the source.

use crate::model::limits;

/// Attribute keys (lowercased substrings) whose values are never stored.
const SENSITIVE_KEY_FRAGMENTS: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "private_key",
    "credential",
    "session",
    "x-csrf",
];

/// Attribute keys that carry request/response payloads or full statements and are
/// dropped by default.
const DROPPED_KEYS: &[&str] = &["http.request.body", "http.response.body", "db.statement", "db.query.text"];

pub const REDACTED: &str = "[REDACTED]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeDecision {
    Keep,
    Redact,
    Drop,
}

pub fn classify_key(key: &str) -> AttributeDecision {
    let lower = key.to_ascii_lowercase();
    if DROPPED_KEYS.iter().any(|k| *k == lower) {
        return AttributeDecision::Drop;
    }
    // Header attributes: http.request.header.<name>
    if SENSITIVE_KEY_FRAGMENTS.iter().any(|f| lower.contains(f)) {
        return AttributeDecision::Redact;
    }
    AttributeDecision::Keep
}

/// Truncate on a UTF-8 boundary.
pub fn truncate(value: &str, max: usize) -> &str {
    if value.len() <= max {
        return value;
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Sanitize a single string attribute value: truncation + masking of obvious
/// bearer credentials that leaked into free text.
pub fn sanitize_value(value: &str) -> String {
    let v = truncate(value, limits::MAX_ATTRIBUTE_VALUE_LEN);
    mask_bearer(v)
}

/// Mask `Bearer <token>` / `Basic <token>` fragments in free text (log bodies, messages).
pub fn mask_bearer(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let lower = rest.to_ascii_lowercase();
        let hit =
            ["bearer ", "basic "].iter().filter_map(|p| lower.find(p).map(|i| (i, p.len()))).min_by_key(|(i, _)| *i);
        match hit {
            Some((i, plen)) => {
                out.push_str(&rest[..i + plen]);
                let after = &rest[i + plen..];
                let tok_len =
                    after.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',').unwrap_or(after.len());
                if tok_len > 0 {
                    out.push_str(REDACTED);
                }
                rest = &after[tok_len..];
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_sensitive_keys() {
        assert_eq!(classify_key("http.request.header.authorization"), AttributeDecision::Redact);
        assert_eq!(classify_key("Cookie"), AttributeDecision::Redact);
        assert_eq!(classify_key("db.statement"), AttributeDecision::Drop);
        assert_eq!(classify_key("http.route"), AttributeDecision::Keep);
    }

    #[test]
    fn masks_bearer_tokens() {
        assert_eq!(
            mask_bearer("auth failed for Bearer abc.def-123 at gateway"),
            "auth failed for Bearer [REDACTED] at gateway"
        );
        assert_eq!(mask_bearer("no secrets here"), "no secrets here");
    }

    #[test]
    fn truncates_on_char_boundary() {
        let s = "ааа"; // 2 bytes per char
        assert_eq!(truncate(s, 3), "а");
    }
}

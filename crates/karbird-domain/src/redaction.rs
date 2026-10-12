//! Server-side redaction applied at ingestion as a second line of defence.
//! SDKs must already avoid collecting sensitive data at the source.

use std::borrow::Cow;

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
    let lower = if key.bytes().any(|byte| byte.is_ascii_uppercase()) {
        Cow::Owned(key.to_ascii_lowercase())
    } else {
        Cow::Borrowed(key)
    };
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

/// Attribute keys holding a URL (or its query) whose query string can carry tokens or PII.
const URL_KEYS: &[&str] = &["url.full", "url.original", "http.url", "http.target"];
const QUERY_KEYS: &[&str] = &["url.query", "http.query"];

/// Sanitize a string attribute value given its key: `sanitize_value`, plus the query string of
/// URL attributes replaced by `[REDACTED]` (the path and fragment are kept). Returns the value
/// and whether a query was removed.
pub fn sanitize_attribute(key: &str, value: &str) -> (String, bool) {
    let mut sanitized = value.to_owned();
    let removed_query = sanitize_attribute_in_place(key, &mut sanitized);
    (sanitized, removed_query)
}

/// First ASCII-case-insensitive `scheme` followed by a space or tab.
fn find_scheme(text: &str, scheme: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let scheme = scheme.as_bytes();
    bytes.windows(scheme.len()).enumerate().find_map(|(i, candidate)| {
        let matches = candidate.iter().zip(scheme).all(|(a, b)| a.eq_ignore_ascii_case(b));
        (matches && matches!(bytes.get(i + scheme.len()), Some(b' ' | b'\t'))).then_some((i, scheme.len()))
    })
}

fn find_next_scheme(text: &str) -> Option<(usize, usize)> {
    ["bearer", "basic"].iter().filter_map(|scheme| find_scheme(text, scheme)).min_by_key(|(i, _)| *i)
}

/// Mask `Bearer <token>` / `Basic <token>` fragments in free text (log bodies, messages).
pub fn mask_bearer(text: &str) -> String {
    mask_bearer_if_needed(text).unwrap_or_else(|| text.to_owned())
}

fn mask_bearer_if_needed(text: &str) -> Option<String> {
    let mut hit = Some(find_next_scheme(text)?);
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        match hit {
            Some((i, plen)) => {
                let after = &rest[i + plen..];
                let ws = after.len() - after.trim_start_matches([' ', '\t']).len();
                out.push_str(&rest[..i + plen + ws]);
                let after = &after[ws..];
                let tok_len =
                    after.find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',').unwrap_or(after.len());
                if tok_len > 0 {
                    out.push_str(REDACTED);
                }
                rest = &after[tok_len..];
                hit = find_next_scheme(rest);
            }
            None => {
                out.push_str(rest);
                return Some(out);
            }
        }
    }
}

/// Sanitize an owned value without allocating a replacement when it needs no masking or truncation.
pub fn sanitize_value_in_place(value: &mut String) {
    sanitize_value_in_place_with_limit(value, limits::MAX_ATTRIBUTE_VALUE_LEN);
}

/// Sanitize an owned string with the caller's existing length bound.
pub fn sanitize_value_in_place_with_limit(value: &mut String, max_len: usize) {
    let length = truncate(value, max_len).len();
    if length < value.len() {
        value.truncate(length);
    }
    if let Some(masked) = mask_bearer_if_needed(value) {
        *value = masked;
    }
}

/// Sanitize one owned string attribute in place; returns whether a URL query was removed.
pub fn sanitize_attribute_in_place(key: &str, value: &mut String) -> bool {
    sanitize_value_in_place(value);
    if QUERY_KEYS.iter().any(|candidate| key.eq_ignore_ascii_case(candidate)) {
        if value.is_empty() {
            return false;
        }
        value.clear();
        value.push_str(REDACTED);
        return true;
    }
    if URL_KEYS.iter().any(|candidate| key.eq_ignore_ascii_case(candidate))
        && let Some(query) = value.find('?')
    {
        let end = value[query..].find('#').map_or(value.len(), |offset| query + offset);
        if end > query + 1 {
            value.replace_range(query + 1..end, REDACTED);
            return true;
        }
    }
    false
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

    #[test]
    fn url_query_strings_are_redacted() {
        let (v, hit) = sanitize_attribute("url.full", "https://a.example/orders/7?token=abc&email=x@y.z#top");
        assert_eq!(v, "https://a.example/orders/7?[REDACTED]#top");
        assert!(hit);
        assert_eq!(sanitize_attribute("HTTP.TARGET", "/login?password=p").0, "/login?[REDACTED]");
        assert_eq!(sanitize_attribute("url.query", "api_key=k").0, REDACTED);
        // No query, an empty query, and unrelated keys are left alone.
        assert_eq!(sanitize_attribute("url.full", "https://a.example/x"), ("https://a.example/x".to_owned(), false));
        assert_eq!(sanitize_attribute("url.full", "https://a.example/x?").0, "https://a.example/x?");
        assert_eq!(sanitize_attribute("url.query", ""), (String::new(), false));
        assert_eq!(sanitize_attribute("url.path", "/a?b=c").0, "/a?b=c");
    }

    #[test]
    fn in_place_attribute_sanitizing_matches_the_owned_contract() {
        for (key, value) in [
            ("http.request.method", "GET"),
            ("HTTP.TARGET", "/login?password=p#form"),
            ("url.query", "api_key=k"),
            ("message", "request failed with Bearer synthetic-token"),
            ("url.full", "https://api.example/orders/7?token=abc#top"),
        ] {
            let (expected, expected_removed) = sanitize_attribute(key, value);
            let mut actual = value.to_owned();
            let removed = sanitize_attribute_in_place(key, &mut actual);
            assert_eq!((actual, removed), (expected, expected_removed), "{key}");
        }
    }

    #[test]
    fn classification_is_case_insensitive() {
        for key in [
            "AUTHORIZATION",
            "Http.Request.Header.Authorization",
            "http.request.header.proxy-authorization",
            "http.response.header.Set-Cookie",
            "COOKIE",
            "user.Password",
            "db.passwd",
            "client_secret",
            "X-Auth-Token",
            "refresh_token",
            "API_KEY",
            "x-api-key",
            "stripe.apiKey",
            "tls.private_key",
            "aws.credentials",
            "session.id",
            "http.request.header.x-csrf-token",
        ] {
            assert_eq!(classify_key(key), AttributeDecision::Redact, "{key} must be redacted");
        }
    }

    #[test]
    fn bodies_and_full_statements_are_dropped_case_insensitively() {
        for key in [
            "http.request.body",
            "HTTP.REQUEST.BODY",
            "http.response.body",
            "db.statement",
            "DB.Statement",
            "db.query.text",
        ] {
            assert_eq!(classify_key(key), AttributeDecision::Drop, "{key} must be dropped");
        }
    }

    #[test]
    fn ordinary_keys_are_kept() {
        for key in [
            "http.route",
            "http.request.method",
            "http.response.status_code",
            "service.name",
            "db.system",
            "db.name",
            "http.request.body.size",
            "http.response.body.size",
            "",
            "ключ",
        ] {
            assert_eq!(classify_key(key), AttributeDecision::Keep, "{key} must be kept");
        }
    }

    #[test]
    fn drop_takes_precedence_and_is_exact_match() {
        // Exact dropped keys never fall through to Redact, and prefixes of them are not dropped.
        assert_eq!(classify_key("db.statement.token"), AttributeDecision::Redact);
        assert_eq!(classify_key("db.statements"), AttributeDecision::Keep);
    }

    #[test]
    fn truncate_edge_cases() {
        assert_eq!(truncate("", 0), "");
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abc", 10), "abc");
        // 4-byte emoji: any cut inside it falls back to the previous boundary.
        let s = "a😀b";
        assert_eq!(truncate(s, 1), "a");
        assert_eq!(truncate(s, 2), "a");
        assert_eq!(truncate(s, 4), "a");
        assert_eq!(truncate(s, 5), "a😀");
    }

    #[test]
    fn sanitize_value_limits_length_and_masks() {
        let huge = "x".repeat(limits::MAX_ATTRIBUTE_VALUE_LEN * 3);
        assert_eq!(sanitize_value(&huge).len(), limits::MAX_ATTRIBUTE_VALUE_LEN);
        assert_eq!(sanitize_value("Bearer abc"), "Bearer [REDACTED]");
        let multibyte = "я".repeat(limits::MAX_ATTRIBUTE_VALUE_LEN);
        let out = sanitize_value(&multibyte);
        assert!(out.len() <= limits::MAX_ATTRIBUTE_VALUE_LEN);
        assert!(out.chars().all(|c| c == 'я'));
    }

    #[test]
    fn masks_basic_and_mixed_case_schemes() {
        assert_eq!(mask_bearer("Authorization: Basic dXNlcjpwYXNz"), "Authorization: Basic [REDACTED]");
        assert_eq!(mask_bearer("BEARER abc"), "BEARER [REDACTED]");
        assert_eq!(mask_bearer("bEaReR abc"), "bEaReR [REDACTED]");
    }

    #[test]
    fn masks_multiple_tokens_and_stops_at_delimiters() {
        assert_eq!(
            mask_bearer(r#"a=Bearer t1, b="Bearer t2" c='basic t3'"#),
            r#"a=Bearer [REDACTED], b="Bearer [REDACTED]" c='basic [REDACTED]'"#
        );
        assert_eq!(mask_bearer("Bearer t1\nBearer t2"), "Bearer [REDACTED]\nBearer [REDACTED]");
    }

    #[test]
    fn masks_tokens_after_extra_whitespace() {
        // Regression: `Bearer  <token>` (double space) and tab separators used to leak the token.
        assert_eq!(mask_bearer("Bearer  secret"), "Bearer  [REDACTED]");
        assert_eq!(mask_bearer("Bearer\tsecret"), "Bearer\t[REDACTED]");
        assert_eq!(mask_bearer("Basic \t secret tail"), "Basic \t [REDACTED] tail");
    }

    #[test]
    fn mask_bearer_edge_cases() {
        assert_eq!(mask_bearer(""), "");
        assert_eq!(mask_bearer("Bearer "), "Bearer ");
        assert_eq!(mask_bearer("Bearer"), "Bearer");
        assert_eq!(mask_bearer("bearerless text"), "bearerless text");
        assert_eq!(mask_bearer("Bearer \"quoted\""), "Bearer \"quoted\"");
        assert_eq!(mask_bearer("Привет Bearer токен!"), "Привет Bearer [REDACTED]");
        assert_eq!(mask_bearer("x bearerx Bearer tok"), "x bearerx Bearer [REDACTED]");
    }

    mod props {
        use super::super::*;
        use proptest::prelude::*;

        const SECRET_FRAGMENTS: &[&str] = &[
            "authorization",
            "cookie",
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

        fn case_mix(s: &str, mask: u64) -> String {
            s.chars()
                .enumerate()
                .map(|(i, c)| if (mask >> (i % 64)) & 1 == 1 { c.to_ascii_uppercase() } else { c })
                .collect()
        }

        proptest! {
            #[test]
            fn any_key_containing_a_secret_fragment_is_never_kept(
                prefix in "[a-zA-Z0-9._-]{0,20}",
                suffix in "[a-zA-Z0-9._-]{0,20}",
                idx in 0usize..13,
                mask in any::<u64>(),
            ) {
                let key = format!("{prefix}{}{suffix}", case_mix(SECRET_FRAGMENTS[idx], mask));
                prop_assert_ne!(classify_key(&key), AttributeDecision::Keep);
            }

            #[test]
            fn masked_output_never_contains_the_token(
                scheme in prop::sample::select(vec!["Bearer", "bearer", "BASIC", "Basic"]),
                sep in "[ \t]{1,3}",
                token in "[A-Za-z0-9._~+/=-]{1,64}",
                before in "[ac-z ]{0,20}",
                after in "( [a-z]{0,20})?",
            ) {
                let text = format!("{before}{scheme}{sep}ZZ{token}{after}");
                let out = mask_bearer(&text);
                let needle = format!("ZZ{token}");
                prop_assert!(!out.contains(&needle), "{out}");
                prop_assert!(out.contains(REDACTED));
            }

            #[test]
            fn mask_bearer_is_identity_without_schemes(text in "[^bB]*") {
                prop_assert_eq!(mask_bearer(&text), text);
            }

            #[test]
            fn truncate_is_a_valid_prefix(s in any::<String>(), max in 0usize..64) {
                let t = truncate(&s, max);
                prop_assert!(t.len() <= max);
                prop_assert!(s.starts_with(t));
                // At most a partial UTF-8 sequence (<= 3 bytes) is cut back.
                prop_assert!(s.len() <= max || t.len() + 3 >= max);
            }

            #[test]
            fn sanitize_value_is_bounded(s in any::<String>()) {
                // Every masked token grows by at most REDACTED.len() bytes.
                let out = sanitize_value(&s);
                prop_assert!(out.len() <= limits::MAX_ATTRIBUTE_VALUE_LEN + REDACTED.len() * (s.len() / 7 + 1));
            }

            #[test]
            fn in_place_value_sanitizing_matches_the_borrowed_input_contract(s in any::<String>()) {
                let expected = sanitize_value(&s);
                let mut actual = s;
                sanitize_value_in_place(&mut actual);
                prop_assert_eq!(actual, expected);
            }
        }
    }
}

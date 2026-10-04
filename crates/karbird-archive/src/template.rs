//! Lossless log templates.
//!
//! A body is split into words (runs of alphanumerics and `_`) and the text between them. A word
//! that contains a digit, or a long all-hex word, is a variable; everything else stays in the
//! template. `order 81723 created for user u_4f2a in 12 ms` becomes the template
//! `order \u{1} created for user \u{2} in \u{1} ms` with the variables `81723`, `"u_4f2a"`, `12`.
//! A service has a bounded number of templates, so the cost of storing them is spread over all
//! of its lines (ADR-0015). [`render`] restores the exact original text.

use sha2::{Digest, Sha256};

use crate::{ArchiveError, Result};

/// Marks an integer variable in a template.
pub const INT_VAR: char = '\u{1}';
/// Marks a string variable in a template.
pub const STR_VAR: char = '\u{2}';

/// Longest digit run stored as an integer; longer ones are strings (they may not fit an i64).
const MAX_INT_DIGITS: usize = 18;
/// All-hex words at least this long (hashes, IDs without digits) are variables, not template text.
const MIN_HEX_WORD: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Var {
    Int(i64),
    Str(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub template: String,
    pub vars: Vec<Var>,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_variable(word: &str) -> bool {
    word.bytes().any(|b| b.is_ascii_digit())
        || (word.len() >= MIN_HEX_WORD && word.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Canonical decimal integers only (`0`, `12`, not `012` or `+1`), so rendering gives back the
/// same text.
fn as_int(word: &str) -> Option<i64> {
    let b = word.as_bytes();
    let canonical = word == "0"
        || (b.len() <= MAX_INT_DIGITS
            && b.first().is_some_and(|c| (b'1'..=b'9').contains(c))
            && b.iter().all(u8::is_ascii_digit));
    if canonical { word.parse().ok() } else { None }
}

fn push_word(word: &str, template: &mut String, vars: &mut Vec<Var>) {
    if !is_variable(word) {
        template.push_str(word);
    } else if let Some(i) = as_int(word) {
        template.push(INT_VAR);
        vars.push(Var::Int(i));
    } else {
        template.push(STR_VAR);
        vars.push(Var::Str(word.to_owned()));
    }
}

pub fn parse(body: &str) -> Parsed {
    // A body that contains the marker characters itself is kept whole, so templates stay unambiguous.
    if body.contains([INT_VAR, STR_VAR]) {
        return Parsed { template: STR_VAR.to_string(), vars: vec![Var::Str(body.to_owned())] };
    }
    let mut template = String::with_capacity(body.len());
    let mut vars = Vec::new();
    let mut word_start: Option<usize> = None;
    for (i, c) in body.char_indices() {
        match (is_word_char(c), word_start) {
            (true, None) => word_start = Some(i),
            (false, Some(start)) => {
                push_word(&body[start..i], &mut template, &mut vars);
                word_start = None;
                template.push(c);
            }
            (false, None) => template.push(c),
            (true, Some(_)) => {}
        }
    }
    if let Some(start) = word_start {
        push_word(&body[start..], &mut template, &mut vars);
    }
    Parsed { template, vars }
}

/// The original body from a template and its variables.
pub fn render(template: &str, vars: &[Var]) -> Result<String> {
    let mut out = String::with_capacity(template.len() + vars.len() * 8);
    let mut it = vars.iter();
    for c in template.chars() {
        if c == INT_VAR {
            let Some(Var::Int(i)) = it.next() else { return Err(ArchiveError::TemplateMismatch) };
            out.push_str(&i.to_string());
        } else if c == STR_VAR {
            let Some(Var::Str(s)) = it.next() else { return Err(ArchiveError::TemplateMismatch) };
            out.push_str(s);
        } else {
            out.push(c);
        }
    }
    if it.next().is_some() {
        return Err(ArchiveError::TemplateMismatch);
    }
    Ok(out)
}

/// Stable 64-bit ID of a template (the same template has the same ID in every service).
pub fn template_id(template: &str) -> u64 {
    let digest = Sha256::digest(template.as_bytes());
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(first)
}

/// Variable slots of a template, in order: `true` for integer slots.
pub fn slots(template: &str) -> impl Iterator<Item = bool> + '_ {
    template.chars().filter(|c| *c == INT_VAR || *c == STR_VAR).map(|c| c == INT_VAR)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use proptest::prelude::*;

    fn t(s: &str) -> String {
        s.replace("{i}", &INT_VAR.to_string()).replace("{s}", &STR_VAR.to_string())
    }

    #[test]
    fn lines_of_one_kind_share_a_template() {
        let a = parse("order 81723 created for user u_4f2a in 12 ms");
        let b = parse("order 81724 created for user u_9c11 in 9 ms");
        assert_eq!(a.template, t("order {i} created for user {s} in {i} ms"));
        assert_eq!(a.template, b.template);
        assert_eq!(a.vars, vec![Var::Int(81723), Var::Str("u_4f2a".into()), Var::Int(12)]);
        assert_eq!(template_id(&a.template), template_id(&b.template));
        assert_ne!(template_id(&a.template), template_id(&parse("payment failed for order 81725").template));
    }

    #[test]
    fn punctuation_splits_words_so_routes_and_ids_stay_structured() {
        let p = parse("GET /api/orders/81723 200 in 3 ms");
        assert_eq!(p.template, t("GET /api/orders/{i} {i} in {i} ms"));
        let p = parse("retry 1/3 for sku SKU-1042 at 10.0.0.12:8080");
        assert_eq!(p.template, t("retry {i}/{i} for sku SKU-{i} at {i}.{i}.{i}.{i}:{i}"));
        let p = parse("trace deadbeefcafe ok, cafe ok");
        assert_eq!(p.template, t("trace {s} ok, cafe ok"), "long hex words are variables, short words are text");
    }

    #[test]
    fn non_canonical_numbers_are_strings_so_rendering_is_exact() {
        for (body, var) in [("code 007", "007"), ("big 1234567890123456789", "1234567890123456789")] {
            let p = parse(body);
            assert_eq!(p.vars, vec![Var::Str(var.into())], "{body}");
            assert_eq!(render(&p.template, &p.vars).unwrap(), body);
        }
        assert_eq!(parse("zero 0").vars, vec![Var::Int(0)]);
        assert_eq!(parse("neg -5").template, t("neg -{i}"), "the sign stays in the template");
    }

    #[test]
    fn bodies_containing_marker_characters_are_kept_whole() {
        let body = format!("weird {INT_VAR} 12");
        let p = parse(&body);
        assert_eq!(p.template, STR_VAR.to_string());
        assert_eq!(render(&p.template, &p.vars).unwrap(), body);
    }

    #[test]
    fn edge_cases() {
        assert_eq!(parse(""), Parsed { template: String::new(), vars: vec![] });
        assert_eq!(parse("plain words only").vars, vec![]);
        let p = parse("ключ=значение42 ✓ 5");
        assert_eq!(render(&p.template, &p.vars).unwrap(), "ключ=значение42 ✓ 5");
    }

    #[test]
    fn render_rejects_mismatched_variables() {
        let tpl = t("a {i} b {s}");
        assert!(render(&tpl, &[Var::Int(1)]).is_err(), "too few");
        assert!(render(&tpl, &[Var::Str("x".into()), Var::Str("y".into())]).is_err(), "wrong type");
        assert!(render(&tpl, &[Var::Int(1), Var::Str("y".into()), Var::Int(2)]).is_err(), "too many");
        assert_eq!(slots(&tpl).collect::<Vec<_>>(), vec![true, false]);
    }

    proptest! {
        #[test]
        fn parse_then_render_is_lossless(body in "\\PC*") {
            let p = parse(&body);
            prop_assert_eq!(render(&p.template, &p.vars).unwrap(), body);
            prop_assert_eq!(slots(&p.template).count(), p.vars.len());
        }

        #[test]
        fn lossless_for_log_like_text(words in prop::collection::vec("[a-z]{1,6}|[0-9]{1,20}|0[0-9]{1,3}|[a-f0-9]{8,16}|[ /:=.,-]", 0..40)) {
            let body: String = words.concat();
            let p = parse(&body);
            prop_assert_eq!(render(&p.template, &p.vars).unwrap(), body);
        }

        #[test]
        fn arbitrary_text_including_control_characters(body in any::<String>()) {
            let p = parse(&body);
            prop_assert_eq!(render(&p.template, &p.vars).unwrap(), body);
        }
    }
}

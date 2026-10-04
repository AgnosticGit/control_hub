//! Per-service zstd dictionaries.
//!
//! A dictionary is a raw-content zstd dictionary: the service's known templates and frequent
//! string values, encoded exactly as they appear in segment columns. Every segment repeats its
//! template and string tables; with the dictionary those tables compress to back-references.
//! Each archived segment can contribute new entries to the next version, so the dictionary grows
//! with the service's history and older entries are dropped first once it reaches its size cap.

use std::collections::HashSet;

use crate::varint::{Reader, Writer};
use crate::{ArchiveError, Result};

const MAGIC: &[u8] = b"CHDICT1\n";
/// zstd uses at most the last ~128 KiB of a raw-content dictionary efficiently.
pub const MAX_DICT_BYTES: usize = 112 * 1024;
/// Longer values are rarely repeated verbatim; they would only push useful entries out.
const MAX_ENTRY_BYTES: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dictionary {
    version: u32,
    entries: Vec<String>,
    bytes: Vec<u8>,
}

fn encode(entries: &[String]) -> Vec<u8> {
    let mut w = Writer { buf: MAGIC.to_vec() };
    for e in entries {
        w.str(e);
    }
    w.buf
}

impl Dictionary {
    /// Versions start at 1; 0 means "no dictionary" in segment headers.
    pub fn new(version: u32, entries: Vec<String>) -> Self {
        let bytes = encode(&entries);
        Self { version: version.max(1), entries, bytes }
    }

    pub fn from_bytes(version: u32, bytes: &[u8]) -> Result<Self> {
        let body = bytes.strip_prefix(MAGIC).ok_or(ArchiveError::Corrupt("not a dictionary"))?;
        if version == 0 || bytes.len() > 2 * MAX_DICT_BYTES {
            return Err(ArchiveError::Corrupt("invalid dictionary"));
        }
        let mut r = Reader::new(body);
        let mut entries = Vec::new();
        while !r.is_empty() {
            entries.push(r.str()?.to_owned());
        }
        Ok(Self { version, entries, bytes: bytes.to_vec() })
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The next dictionary version after archiving a segment with these template and string
    /// usage counts, or `None` when the segment taught nothing new. Templates always qualify;
    /// strings only when they repeat. The most frequent entries go last, where zstd references
    /// them most cheaply.
    pub fn evolve(
        prev: Option<&Dictionary>,
        templates: &[(String, u64)],
        strings: &[(String, u64)],
        max_bytes: usize,
    ) -> Option<Dictionary> {
        let mut seen: HashSet<&str> = prev.map(|d| d.entries.iter().map(String::as_str).collect()).unwrap_or_default();
        let mut fresh: Vec<(&str, u64)> = templates
            .iter()
            .map(|(t, n)| (t.as_str(), *n))
            .chain(strings.iter().filter(|(_, n)| *n >= 2).map(|(s, n)| (s.as_str(), *n)))
            .filter(|(e, _)| !e.is_empty() && e.len() <= MAX_ENTRY_BYTES && seen.insert(e))
            .collect();
        if fresh.is_empty() {
            return None;
        }
        fresh.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
        let mut entries: Vec<String> = prev.map(|d| d.entries.clone()).unwrap_or_default();
        entries.extend(fresh.into_iter().map(|(e, _)| e.to_owned()));
        let mut size: usize = MAGIC.len() + entries.iter().map(|e| e.len() + 2).sum::<usize>();
        let mut drop = 0;
        while size > max_bytes && drop < entries.len() {
            size -= entries[drop].len() + 2;
            drop += 1;
        }
        entries.drain(..drop);
        Some(Dictionary::new(prev.map_or(1, |d| d.version + 1), entries))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn counts(items: &[(&str, u64)]) -> Vec<(String, u64)> {
        items.iter().map(|(s, n)| ((*s).to_owned(), *n)).collect()
    }

    #[test]
    fn bytes_roundtrip_and_reject_garbage() {
        let d = Dictionary::new(3, vec!["order \u{1} created".into(), "u_4f2a".into()]);
        let back = Dictionary::from_bytes(3, d.as_bytes()).unwrap();
        assert_eq!(back, d);
        assert!(Dictionary::from_bytes(1, b"nope").is_err());
        assert!(Dictionary::from_bytes(0, d.as_bytes()).is_err(), "version 0 means no dictionary");
        let mut truncated = d.as_bytes().to_vec();
        truncated.pop();
        assert!(Dictionary::from_bytes(3, &truncated).is_err());
    }

    #[test]
    fn evolve_adds_templates_and_repeated_strings_only() {
        let d =
            Dictionary::evolve(None, &counts(&[("tpl a", 5)]), &counts(&[("once", 1), ("twice", 2)]), MAX_DICT_BYTES)
                .unwrap();
        assert_eq!(d.version(), 1);
        assert_eq!(d.entries(), ["twice", "tpl a"], "most frequent last");
        assert!(
            Dictionary::evolve(Some(&d), &counts(&[("tpl a", 9)]), &counts(&[("twice", 4)]), MAX_DICT_BYTES).is_none(),
            "nothing new: no new version"
        );
        let next = Dictionary::evolve(Some(&d), &counts(&[("tpl b", 1)]), &[], MAX_DICT_BYTES).unwrap();
        assert_eq!(next.version(), 2);
        assert_eq!(next.entries(), ["twice", "tpl a", "tpl b"], "history is kept");
    }

    #[test]
    fn evolve_drops_the_oldest_entries_at_the_size_cap_and_skips_huge_values() {
        let mut d: Option<Dictionary> = None;
        for i in 0..50 {
            let tpl = format!("template number {i:03} {}", "x".repeat(40));
            d = Dictionary::evolve(d.as_ref(), &counts(&[(&tpl, 1)]), &[], 1024).or(d);
        }
        let d = d.unwrap();
        assert!(d.as_bytes().len() <= 1024, "{}", d.as_bytes().len());
        assert!(d.entries().last().unwrap().contains("049"), "newest kept");
        assert!(!d.entries().iter().any(|e| e.contains("number 000")), "oldest dropped");
        let huge = "y".repeat(MAX_ENTRY_BYTES + 1);
        assert!(Dictionary::evolve(None, &counts(&[(&huge, 10)]), &[], MAX_DICT_BYTES).is_none());
    }
}

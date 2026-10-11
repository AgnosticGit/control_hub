//! Columnar log segments.
//!
//! One segment holds one service's logs for one time window. Rows are sorted by template, then
//! time, so lines of one kind sit next to each other. Each field goes into its own column with an
//! encoding that suits it, and every column is compressed with zstd (optionally with the
//! service's [`Dictionary`]):
//!
//! | column | encoding |
//! |---|---|
//! | templates, strings, resource sets | per-segment tables, referenced by index |
//! | template per row | table index |
//! | time | delta to the previous row |
//! | observed / ingested time | delta to the row's time |
//! | integer variables | delta to the previous value of the same template slot |
//! | string variables, attribute keys/values | string table index |
//! | log / trace / span IDs | raw bytes when hex, else text |
//!
//! Layout: `CHSEG\x01`, header varints (dictionary version, rows, min/max time, column count),
//! then per column its decoded length and the zstd frame. Input to [`decode`] is untrusted.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::dict::Dictionary;
use crate::template::{self, Var};
use crate::varint::{Reader, Writer};
use crate::{ArchiveError, Result};

const MAGIC: &[u8] = b"CHSEG\x01";
const MAX_COLUMN_BYTES: usize = 512 * 1024 * 1024;
const MAX_HEX_ID: usize = 128;

const TEMPLATES: usize = 0;
const STRINGS: usize = 1;
const RESOURCES: usize = 2;
const ROW_TEMPLATE: usize = 3;
const TIME: usize = 4;
const OBSERVED: usize = 5;
const INGESTED: usize = 6;
const SEVERITY: usize = 7;
const INTS: usize = 8;
const STRS: usize = 9;
const IDS: usize = 10;
const META: usize = 11;
const COLUMNS: usize = 12;

/// One archived log line: everything ClickHouse stores except the scope and service ID, which
/// are the same for the whole segment and live in the catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogRecord {
    pub service_name: String,
    pub service_instance_id: String,
    pub log_id: String,
    pub time: i64,
    pub observed_time: i64,
    pub severity_number: u8,
    pub severity_text: String,
    pub body: String,
    pub trace_id: String,
    pub span_id: String,
    pub attributes: Vec<(String, String)>,
    pub resource_attributes: Vec<(String, String)>,
    pub batch_id: String,
    pub ingested_at: i64,
}

impl LogRecord {
    /// Rough size of the record as plain text, to report how much the archive saves.
    fn source_bytes(&self) -> u64 {
        let pairs = |v: &[(String, String)]| v.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>();
        let text = self.service_name.len()
            + self.service_instance_id.len()
            + self.log_id.len()
            + self.severity_text.len()
            + self.body.len()
            + self.trace_id.len()
            + self.span_id.len()
            + pairs(&self.attributes)
            + pairs(&self.resource_attributes)
            + self.batch_id.len();
        (text + 3 * 8 + 1) as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentHeader {
    /// 0: compressed without a dictionary.
    pub dict_version: u32,
    pub rows: u64,
    pub min_time: i64,
    pub max_time: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentStats {
    pub rows: u64,
    pub templates: u64,
    pub min_time: i64,
    pub max_time: i64,
    /// Approximate plain-text size of the records.
    pub source_bytes: u64,
    /// Columns before zstd.
    pub column_bytes: u64,
    /// The segment as stored.
    pub stored_bytes: u64,
}

#[derive(Debug)]
pub struct Encoded {
    pub bytes: Vec<u8>,
    pub stats: SegmentStats,
    /// How often each template and string occurred, for [`Dictionary::evolve`].
    pub template_counts: Vec<(String, u64)>,
    pub string_counts: Vec<(String, u64)>,
}

#[derive(Debug, Default)]
struct Interner {
    ids: HashMap<String, u64>,
    list: Vec<String>,
    counts: Vec<u64>,
}

impl Interner {
    fn id(&mut self, s: &str) -> u64 {
        if let Some(&i) = self.ids.get(s) {
            if let Some(c) = self.counts.get_mut(i as usize) {
                *c += 1;
            }
            return i;
        }
        let i = self.list.len() as u64;
        self.ids.insert(s.to_owned(), i);
        self.list.push(s.to_owned());
        self.counts.push(1);
        i
    }

    fn write(&self, w: &mut Writer) {
        w.uvar(self.list.len() as u64);
        for s in &self.list {
            w.str(s);
        }
    }

    fn counts(self) -> Vec<(String, u64)> {
        self.list.into_iter().zip(self.counts).collect()
    }
}

fn is_lower_hex(s: &str) -> bool {
    s.len().is_multiple_of(2)
        && s.len() <= MAX_HEX_ID
        && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn put_id(w: &mut Writer, s: &str) {
    if s.is_empty() {
        w.u8(0);
    } else if is_lower_hex(s) {
        w.u8(1);
        let bytes = s.as_bytes();
        w.uvar((bytes.len() / 2) as u64);
        let (pairs, _) = bytes.as_chunks::<2>();
        for pair in pairs {
            let high = if pair[0].is_ascii_digit() { pair[0] - b'0' } else { pair[0] - b'a' + 10 };
            let low = if pair[1].is_ascii_digit() { pair[1] - b'0' } else { pair[1] - b'a' + 10 };
            w.u8((high << 4) | low);
        }
    } else {
        w.u8(2);
        w.str(s);
    }
}

fn get_id(r: &mut Reader<'_>) -> Result<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    match r.u8()? {
        0 => Ok(String::new()),
        1 => Ok(r
            .bytes()?
            .iter()
            .flat_map(|b| [HEX[usize::from(b >> 4)] as char, HEX[usize::from(b & 0xf)] as char])
            .collect()),
        2 => Ok(r.str()?.to_owned()),
        _ => Err(ArchiveError::Corrupt("unknown id encoding")),
    }
}

fn zstd_err(e: std::io::Error) -> ArchiveError {
    ArchiveError::Compression(e.to_string())
}

/// Encodes `records` (any order) into a segment. `level` is the zstd level (1..=22).
pub fn encode(records: &[LogRecord], dict: Option<&Dictionary>, level: i32) -> Result<Encoded> {
    let parsed: Vec<template::Parsed> = records.iter().map(|r| template::parse(&r.body)).collect();
    let mut order: Vec<usize> = (0..records.len()).collect();
    order.sort_by(|&a, &b| {
        parsed[a]
            .template
            .cmp(&parsed[b].template)
            .then(records[a].time.cmp(&records[b].time))
            .then_with(|| records[a].log_id.cmp(&records[b].log_id))
    });

    let mut cols: Vec<Writer> = (0..COLUMNS).map(|_| Writer::default()).collect();
    let mut templates = Interner::default();
    let mut strings = Interner::default();
    let mut resource_sets: HashMap<&[(String, String)], u64> = HashMap::new();
    let mut resources = Writer::default();
    let mut last_int: HashMap<(u64, usize), i64> = HashMap::new();
    let mut prev_time = 0i64;
    let (mut min_time, mut max_time) = (i64::MAX, i64::MIN);
    let mut source_bytes = 0u64;

    for &i in &order {
        let (r, p) = (&records[i], &parsed[i]);
        min_time = min_time.min(r.time);
        max_time = max_time.max(r.time);
        source_bytes += r.source_bytes();
        let t = templates.id(&p.template);
        cols[ROW_TEMPLATE].uvar(t);
        cols[TIME].ivar(r.time.wrapping_sub(prev_time));
        prev_time = r.time;
        cols[OBSERVED].ivar(r.observed_time.wrapping_sub(r.time));
        cols[INGESTED].ivar(r.ingested_at.wrapping_sub(r.time));
        cols[SEVERITY].u8(r.severity_number);
        cols[SEVERITY].uvar(strings.id(&r.severity_text));
        let mut slot = 0;
        for v in &p.vars {
            match v {
                Var::Int(x) => {
                    let last = last_int.insert((t, slot), *x).unwrap_or(0);
                    cols[INTS].ivar(x.wrapping_sub(last));
                    slot += 1;
                }
                Var::Str(s) => cols[STRS].uvar(strings.id(s)),
            }
        }
        put_id(&mut cols[IDS], &r.log_id);
        put_id(&mut cols[IDS], &r.trace_id);
        put_id(&mut cols[IDS], &r.span_id);
        let next_set = resource_sets.len() as u64;
        let set = *resource_sets.entry(&r.resource_attributes).or_insert_with(|| {
            resources.uvar(r.resource_attributes.len() as u64);
            for (k, v) in &r.resource_attributes {
                resources.uvar(strings.id(k));
                resources.uvar(strings.id(v));
            }
            next_set
        });
        let meta = &mut cols[META];
        meta.uvar(strings.id(&r.service_name));
        meta.uvar(strings.id(&r.service_instance_id));
        meta.uvar(strings.id(&r.batch_id));
        meta.uvar(set);
        meta.uvar(r.attributes.len() as u64);
        for (k, v) in &r.attributes {
            meta.uvar(strings.id(k));
            meta.uvar(strings.id(v));
        }
    }
    templates.write(&mut cols[TEMPLATES]);
    strings.write(&mut cols[STRINGS]);
    cols[RESOURCES].uvar(resource_sets.len() as u64);
    cols[RESOURCES].buf.extend_from_slice(&resources.buf);

    if records.is_empty() {
        (min_time, max_time) = (0, 0);
    }
    let dict_version = dict.map_or(0, Dictionary::version);
    let mut out = Writer { buf: MAGIC.to_vec() };
    out.uvar(u64::from(dict_version));
    out.uvar(records.len() as u64);
    out.ivar(min_time);
    out.ivar(max_time);
    out.uvar(COLUMNS as u64);
    let mut compressor = match dict {
        Some(d) => zstd::bulk::Compressor::with_dictionary(level, d.as_bytes()),
        None => zstd::bulk::Compressor::new(level),
    }
    .map_err(zstd_err)?;
    let mut column_bytes = 0u64;
    for col in &cols {
        let frame = compressor.compress(&col.buf).map_err(zstd_err)?;
        column_bytes += col.buf.len() as u64;
        out.uvar(col.buf.len() as u64);
        out.bytes(&frame);
    }
    let stats = SegmentStats {
        rows: records.len() as u64,
        templates: templates.list.len() as u64,
        min_time,
        max_time,
        source_bytes,
        column_bytes,
        stored_bytes: out.buf.len() as u64,
    };
    Ok(Encoded { bytes: out.buf, stats, template_counts: templates.counts(), string_counts: strings.counts() })
}

fn header(bytes: &[u8]) -> Result<(SegmentHeader, Reader<'_>)> {
    let body = bytes.strip_prefix(MAGIC).ok_or(ArchiveError::Corrupt("not a segment"))?;
    let mut r = Reader::new(body);
    let dict_version = u32::try_from(r.uvar()?).map_err(|_| ArchiveError::Corrupt("dictionary version"))?;
    let rows = r.uvar()?;
    let min_time = r.ivar()?;
    let max_time = r.ivar()?;
    if r.uvar()? != COLUMNS as u64 {
        return Err(ArchiveError::Corrupt("unexpected column count"));
    }
    Ok((SegmentHeader { dict_version, rows, min_time, max_time }, r))
}

/// The header alone, e.g. to find out which dictionary version [`decode`] needs.
pub fn read_header(bytes: &[u8]) -> Result<SegmentHeader> {
    header(bytes).map(|(h, _)| h)
}

fn lookup(list: &[String], i: u64) -> Result<&String> {
    usize::try_from(i).ok().and_then(|i| list.get(i)).ok_or(ArchiveError::Corrupt("index out of range"))
}

fn table(r: &mut Reader<'_>) -> Result<Vec<String>> {
    let n = r.count()?;
    (0..n).map(|_| r.str().map(str::to_owned)).collect()
}

fn pairs(r: &mut Reader<'_>, strings: &[String]) -> Result<Vec<(String, String)>> {
    let n = r.count()?;
    (0..n).map(|_| Ok((lookup(strings, r.uvar()?)?.clone(), lookup(strings, r.uvar()?)?.clone()))).collect()
}

/// Decodes a segment. Rows come back sorted by template and time, not by time alone.
pub fn decode(bytes: &[u8], dict: Option<&Dictionary>) -> Result<Vec<LogRecord>> {
    let (h, mut r) = header(bytes)?;
    let got = dict.map_or(0, Dictionary::version);
    if h.dict_version != 0 && h.dict_version != got {
        return Err(ArchiveError::DictionaryMismatch { needed: h.dict_version, got });
    }
    let mut decompressor = match dict.filter(|_| h.dict_version != 0) {
        Some(d) => zstd::bulk::Decompressor::with_dictionary(d.as_bytes()),
        None => zstd::bulk::Decompressor::new(),
    }
    .map_err(zstd_err)?;
    let mut raw = Vec::with_capacity(COLUMNS);
    for _ in 0..COLUMNS {
        let len = usize::try_from(r.uvar()?)
            .ok()
            .filter(|l| *l <= MAX_COLUMN_BYTES)
            .ok_or(ArchiveError::Corrupt("column too large"))?;
        let frame = r.bytes()?;
        let col = decompressor.decompress(frame, len).map_err(zstd_err)?;
        if col.len() != len {
            return Err(ArchiveError::Corrupt("column length mismatch"));
        }
        raw.push(col);
    }
    if !r.is_empty() {
        return Err(ArchiveError::Corrupt("trailing data"));
    }
    let mut cols: Vec<Reader<'_>> = raw.iter().map(|c| Reader::new(c)).collect();
    let templates = table(&mut cols[TEMPLATES])?;
    let strings = table(&mut cols[STRINGS])?;
    let n_sets = cols[RESOURCES].count()?;
    let resource_sets: Vec<Vec<(String, String)>> =
        (0..n_sets).map(|_| pairs(&mut cols[RESOURCES], &strings)).collect::<Result<_>>()?;
    let rows = usize::try_from(h.rows)
        .ok()
        .filter(|n| *n <= cols[ROW_TEMPLATE].remaining())
        .ok_or(ArchiveError::Corrupt("row count exceeds data"))?;

    let template_slots: Vec<Vec<bool>> = templates.iter().map(|tpl| template::slots(tpl).collect()).collect();
    let mut last_int: Vec<Vec<i64>> =
        template_slots.iter().map(|slots| vec![0; slots.iter().filter(|is_int| **is_int).count()]).collect();
    let mut out = Vec::with_capacity(rows);
    let mut vars = Vec::new();
    let mut prev_time = 0i64;
    for _ in 0..rows {
        let t = cols[ROW_TEMPLATE].uvar()?;
        let template_index = usize::try_from(t)
            .ok()
            .filter(|index| *index < templates.len())
            .ok_or(ArchiveError::Corrupt("index out of range"))?;
        let tpl = &templates[template_index];
        let time = prev_time.wrapping_add(cols[TIME].ivar()?);
        prev_time = time;
        let observed_time = time.wrapping_add(cols[OBSERVED].ivar()?);
        let ingested_at = time.wrapping_add(cols[INGESTED].ivar()?);
        let severity_number = cols[SEVERITY].u8()?;
        let severity_text = lookup(&strings, cols[SEVERITY].uvar()?)?.clone();
        vars.clear();
        let mut slot = 0;
        let last_values =
            last_int.get_mut(template_index).ok_or(ArchiveError::Corrupt("template index out of range"))?;
        for &is_int in &template_slots[template_index] {
            if is_int {
                let last = last_values.get_mut(slot).ok_or(ArchiveError::Corrupt("integer slot out of range"))?;
                let v = last.wrapping_add(cols[INTS].ivar()?);
                *last = v;
                vars.push(Var::Int(v));
                slot += 1;
            } else {
                vars.push(Var::Str(lookup(&strings, cols[STRS].uvar()?)?.clone()));
            }
        }
        let body = template::render(tpl, &vars)?;
        let log_id = get_id(&mut cols[IDS])?;
        let trace_id = get_id(&mut cols[IDS])?;
        let span_id = get_id(&mut cols[IDS])?;
        let meta = &mut cols[META];
        let service_name = lookup(&strings, meta.uvar()?)?.clone();
        let service_instance_id = lookup(&strings, meta.uvar()?)?.clone();
        let batch_id = lookup(&strings, meta.uvar()?)?.clone();
        let set = meta.uvar()?;
        let resource_attributes = usize::try_from(set)
            .ok()
            .and_then(|i| resource_sets.get(i))
            .ok_or(ArchiveError::Corrupt("resource set out of range"))?
            .clone();
        let attributes = pairs(meta, &strings)?;
        out.push(LogRecord {
            service_name,
            service_instance_id,
            log_id,
            time,
            observed_time,
            severity_number,
            severity_text,
            body,
            trace_id,
            span_id,
            attributes,
            resource_attributes,
            batch_id,
            ingested_at,
        });
    }
    if cols.iter().any(|c| !c.is_empty()) {
        return Err(ArchiveError::Corrupt("trailing column data"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use proptest::prelude::*;

    const BASE: i64 = 1_790_000_000_000_000_000;

    /// A service's log stream: a handful of line kinds with changing IDs, users and durations.
    fn service_logs(n: usize, seed: u64) -> Vec<LogRecord> {
        let users = ["u_4f2a", "u_9c11", "u_77b0", "u_0d3e", "u_51aa"];
        (0..n)
            .map(|i| {
                let k = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(seed);
                let order = 80_000 + i as u64;
                let (severity, text, body) = match k % 10 {
                    0..=5 => (
                        9,
                        "INFO",
                        format!("order {order} created for user {} in {} ms", users[(k % 5) as usize], 5 + k % 20),
                    ),
                    6 | 7 => (9, "INFO", format!("GET /api/orders/{order} 200 in {} ms", 1 + k % 9)),
                    8 => (
                        13,
                        "WARN",
                        format!("inventory reservation retry {}/3 for sku SKU-{}", 1 + k % 3, 1000 + k % 40),
                    ),
                    _ => (17, "ERROR", format!("payment failed for order {order}: provider timeout after 5000 ms")),
                };
                let time = BASE + i as i64 * 37_000_000 + (k % 1000) as i64;
                LogRecord {
                    service_name: "orders".into(),
                    service_instance_id: format!("orders-{}", k % 2),
                    log_id: format!("{:032x}", k.wrapping_mul(0x9e37_79b9_7f4a_7c15)),
                    time,
                    observed_time: time + 120_000,
                    severity_number: severity,
                    severity_text: text.into(),
                    body,
                    trace_id: format!("{:032x}", k.rotate_left(17)),
                    span_id: format!("{:016x}", k.rotate_left(29)),
                    attributes: vec![
                        ("http.route".into(), "/api/orders/:id".into()),
                        ("user.id".into(), users[(k % 5) as usize].into()),
                    ],
                    resource_attributes: vec![
                        ("service.name".into(), "orders".into()),
                        ("service.version".into(), "1.4.2".into()),
                    ],
                    batch_id: format!("b-{}", i / 50),
                    ingested_at: time + 900_000_000,
                }
            })
            .collect()
    }

    fn sorted(mut v: Vec<LogRecord>) -> Vec<LogRecord> {
        v.sort();
        v
    }

    #[test]
    fn roundtrip_is_lossless() {
        let logs = service_logs(500, 7);
        let enc = encode(&logs, None, 3).unwrap();
        assert_eq!(enc.stats.rows, 500);
        assert_eq!(enc.stats.templates, 4);
        assert_eq!(
            (enc.stats.min_time, enc.stats.max_time),
            (logs.iter().map(|l| l.time).min().unwrap(), logs.iter().map(|l| l.time).max().unwrap())
        );
        assert_eq!(enc.stats.stored_bytes, enc.bytes.len() as u64);
        assert_eq!(sorted(decode(&enc.bytes, None).unwrap()), sorted(logs));
        let h = read_header(&enc.bytes).unwrap();
        assert_eq!((h.rows, h.dict_version), (500, 0));
    }

    #[test]
    fn empty_segment_roundtrips() {
        let enc = encode(&[], None, 3).unwrap();
        assert!(decode(&enc.bytes, None).unwrap().is_empty());
        assert_eq!(enc.stats.rows, 0);
    }

    #[test]
    fn unusual_values_survive() {
        let rec = LogRecord {
            log_id: "NOT-HEX".into(),
            trace_id: "ABCDEF".into(),
            span_id: "abc".into(),
            time: i64::MIN,
            observed_time: i64::MAX,
            ingested_at: 0,
            body: format!("max {} min -{} weird \u{1}", i64::MAX, i64::MAX),
            severity_number: 255,
            ..LogRecord::default()
        };
        let other = LogRecord { time: i64::MAX, body: "x 1".into(), ..LogRecord::default() };
        let logs = vec![rec, other];
        let enc = encode(&logs, None, 1).unwrap();
        assert_eq!(sorted(decode(&enc.bytes, None).unwrap()), sorted(logs));
    }

    #[test]
    fn bytes_per_line_fall_as_the_service_logs_more() {
        let per_row = |n: usize| {
            let enc = encode(&service_logs(n, 1), None, 19).unwrap();
            enc.stats.stored_bytes as f64 / n as f64
        };
        let (small, medium, large) = (per_row(50), per_row(1_000), per_row(20_000));
        assert!(medium < small * 0.7, "50 rows: {small:.1} B/row, 1000 rows: {medium:.1} B/row");
        assert!(large < medium, "1000 rows: {medium:.1} B/row, 20000 rows: {large:.1} B/row");
        let enc = encode(&service_logs(20_000, 1), None, 19).unwrap();
        assert!(enc.stats.source_bytes > 10 * enc.stats.stored_bytes, "{:?}", enc.stats);
    }

    #[test]
    fn a_service_dictionary_shrinks_small_segments() {
        let first = encode(&service_logs(2_000, 1), None, 19).unwrap();
        let dict = Dictionary::evolve(None, &first.template_counts, &first.string_counts, crate::dict::MAX_DICT_BYTES)
            .unwrap();
        let next = service_logs(40, 99);
        let plain = encode(&next, None, 19).unwrap();
        let with_dict = encode(&next, Some(&dict), 19).unwrap();
        assert!(
            with_dict.stats.stored_bytes < plain.stats.stored_bytes,
            "dict {} vs plain {}",
            with_dict.stats.stored_bytes,
            plain.stats.stored_bytes
        );
        assert_eq!(read_header(&with_dict.bytes).unwrap().dict_version, dict.version());
        assert_eq!(sorted(decode(&with_dict.bytes, Some(&dict)).unwrap()), sorted(next));
    }

    #[test]
    fn decoding_needs_the_right_dictionary() {
        let dict = Dictionary::new(2, vec!["order \u{1} created".into()]);
        let enc = encode(&service_logs(10, 3), Some(&dict), 3).unwrap();
        assert_eq!(decode(&enc.bytes, None), Err(ArchiveError::DictionaryMismatch { needed: 2, got: 0 }));
        let other = Dictionary::new(3, vec![]);
        assert_eq!(decode(&enc.bytes, Some(&other)), Err(ArchiveError::DictionaryMismatch { needed: 2, got: 3 }));
        let plain = encode(&service_logs(10, 3), None, 3).unwrap();
        assert!(decode(&plain.bytes, Some(&dict)).is_ok(), "a dictionary is ignored for segments without one");
    }

    #[test]
    fn corrupt_segments_are_errors_not_panics() {
        let enc = encode(&service_logs(50, 5), None, 3).unwrap();
        assert!(decode(b"garbage", None).is_err());
        assert!(decode(&enc.bytes[..enc.bytes.len() - 1], None).is_err(), "truncated");
        let mut extra = enc.bytes.clone();
        extra.push(0);
        assert!(decode(&extra, None).is_err(), "trailing bytes");
        // A header that claims a gigantic column is refused before allocating it.
        let mut w = Writer { buf: MAGIC.to_vec() };
        for v in [0, 1, 0, 0, COLUMNS as u64, u64::MAX >> 1] {
            w.uvar(v);
        }
        assert!(decode(&w.buf, None).is_err());
    }

    fn arb_record() -> impl Strategy<Value = LogRecord> {
        let kv = prop::collection::vec(("[a-z.]{1,8}", ".{0,12}"), 0..4);
        (
            ("[a-z]{0,6}", "[a-z0-9-]{0,8}", "[0-9a-f]{0,32}|[A-Z]{0,4}", any::<i64>(), any::<i64>(), any::<u8>()),
            (".{0,60}", "[0-9a-f]{0,32}", "[0-9a-f]{0,16}", kv.clone(), kv, "[a-z0-9-]{0,6}", any::<i64>()),
        )
            .prop_map(
                |((name, inst, id, time, observed, sev), (body, trace, span, attrs, res, batch, ingested))| LogRecord {
                    service_name: name,
                    service_instance_id: inst,
                    log_id: id,
                    time,
                    observed_time: observed,
                    severity_number: sev,
                    severity_text: if sev >= 17 { "ERROR".into() } else { "INFO".into() },
                    body,
                    trace_id: trace,
                    span_id: span,
                    attributes: attrs,
                    resource_attributes: res,
                    batch_id: batch,
                    ingested_at: ingested,
                },
            )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn arbitrary_records_roundtrip(logs in prop::collection::vec(arb_record(), 0..40), level in 1i32..6) {
            let enc = encode(&logs, None, level).unwrap();
            prop_assert_eq!(sorted(decode(&enc.bytes, None).unwrap()), sorted(logs));
        }

        #[test]
        fn mutated_segments_never_panic(pos in any::<prop::sample::Index>(), byte in any::<u8>(), cut in any::<prop::sample::Index>()) {
            let enc = encode(&service_logs(30, 11), None, 3).unwrap();
            let mut b = enc.bytes.clone();
            let i = pos.index(b.len());
            b[i] = byte;
            let _ = decode(&b, None);
            let _ = decode(&enc.bytes[..cut.index(enc.bytes.len())], None);
        }
    }
}

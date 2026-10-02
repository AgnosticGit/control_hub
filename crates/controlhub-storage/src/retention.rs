//! How long ClickHouse keeps logs, spans and events (the table TTLs).
//!
//! The days are configured once, in `CONTROLHUB_{LOGS,SPANS,EVENTS}_RETENTION_DAYS`.
//! `controlhub-api bootstrap` applies them to the tables; every service that depends on them
//! compares them with the real TTL at startup and refuses to run when they differ, so the
//! configuration and ClickHouse can never silently drift apart.

pub const MIN_DAYS: u32 = 2;
pub const MAX_DAYS: u32 = 3650;
pub const DEFAULT_LOGS_DAYS: u32 = 14;
pub const DEFAULT_SPANS_DAYS: u32 = 14;
pub const DEFAULT_EVENTS_DAYS: u32 = 30;

/// The ClickHouse tables with a retention TTL. Table and column names come only from here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    Logs,
    Spans,
    Events,
}

impl Table {
    pub const ALL: [Table; 3] = [Table::Logs, Table::Spans, Table::Events];

    pub fn name(self) -> &'static str {
        match self {
            Table::Logs => "logs",
            Table::Spans => "spans",
            Table::Events => "events",
        }
    }

    /// The column the TTL counts from.
    pub fn time_column(self) -> &'static str {
        match self {
            Table::Logs => "time",
            Table::Spans => "start_time",
            Table::Events => "event_time",
        }
    }

    pub fn env_var(self) -> &'static str {
        match self {
            Table::Logs => "CONTROLHUB_LOGS_RETENTION_DAYS",
            Table::Spans => "CONTROLHUB_SPANS_RETENTION_DAYS",
            Table::Events => "CONTROLHUB_EVENTS_RETENTION_DAYS",
        }
    }
}

/// Days of a `TTL toDateTime(<column>) + toIntervalDay(N)` clause, as ClickHouse prints it in
/// `system.tables.engine_full`. Any other TTL (several clauses, moves, `DELETE WHERE`, another
/// unit) is `None`: the caller must not guess what it means.
pub fn parse_ttl_days(engine_full: &str, column: &str) -> Option<u32> {
    if engine_full.matches(" TTL ").count() != 1 {
        return None;
    }
    let prefix = format!(" TTL toDateTime({column}) + toIntervalDay(");
    let rest = &engine_full[engine_full.find(&prefix)? + prefix.len()..];
    let (digits, after) = rest.split_once(')')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !(after.is_empty() || after.starts_with(" SETTINGS ")) {
        return None;
    }
    digits.parse().ok()
}

/// The configured retention against the table's real TTL; the error tells the operator what to run.
pub fn check(table: Table, configured: u32, actual: Option<u32>) -> Result<(), String> {
    match actual {
        Some(days) if days == configured => Ok(()),
        Some(days) => Err(format!(
            "{} is {configured} days but ClickHouse keeps {} for {days} days; run `just bootstrap` to apply it \
             (a decrease that deletes data needs `just bootstrap --shrink-retention`)",
            table.env_var(),
            table.name(),
        )),
        None => Err(format!(
            "ClickHouse table `{}` has a TTL ControlHub does not recognise (expected `TTL toDateTime({}) + INTERVAL N DAY`); \
             run `just bootstrap` to set it from {}",
            table.name(),
            table.time_column(),
            table.env_var(),
        )),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Keep,
    Apply,
    Refuse(String),
}

/// What bootstrap does with a table: `rows_lost` is how many rows are older than `wanted` days
/// (only looked at when shrinking). Lengthening never loses data and applies at once; shortening
/// that would delete rows needs an explicit `allow_shrink`.
pub fn decide(table: Table, current: u32, wanted: u32, rows_lost: u64, allow_shrink: bool) -> Decision {
    if wanted == current {
        Decision::Keep
    } else if wanted > current || rows_lost == 0 || allow_shrink {
        Decision::Apply
    } else {
        Decision::Refuse(format!(
            "{} = {wanted} shortens {} retention from {current} days: {rows_lost} rows older than {wanted} days \
             would be deleted permanently. Run `just bootstrap --shrink-retention` to confirm, or set it back to {current}",
            table.env_var(),
            table.name(),
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const LOGS: &str = "ReplacingMergeTree(ingested_at) PARTITION BY toYYYYMMDD(time) ORDER BY (tenant_id, project_id, environment_id, service_id, time, log_id) TTL toDateTime(time) + toIntervalDay(14) SETTINGS index_granularity = 8192";

    #[test]
    fn parses_the_ttl_clickhouse_prints() {
        assert_eq!(parse_ttl_days(LOGS, "time"), Some(14));
        assert_eq!(parse_ttl_days(&LOGS.replace("(14)", "(3650)"), "time"), Some(3650));
        let events = "MergeTree PARTITION BY toYYYYMMDD(event_time) ORDER BY (tenant_id, event_time) TTL toDateTime(event_time) + toIntervalDay(30)";
        assert_eq!(parse_ttl_days(events, "event_time"), Some(30), "no SETTINGS after the TTL");
    }

    #[test]
    fn anything_but_a_plain_day_ttl_on_the_column_is_unknown() {
        for (engine, why) in [
            (LOGS.replace(" TTL toDateTime(time) + toIntervalDay(14)", ""), "no TTL"),
            (LOGS.replace("toIntervalDay(14)", "toIntervalHour(14)"), "other unit"),
            (LOGS.replace("toIntervalDay(14)", "toIntervalDay(-1)"), "negative"),
            (LOGS.replace("toIntervalDay(14)", "toIntervalDay(+14)"), "sign"),
            (LOGS.replace("toIntervalDay(14)", "toIntervalDay()"), "empty"),
            (LOGS.replace("toIntervalDay(14)", "toIntervalDay(99999999999)"), "overflow"),
            (LOGS.replace("toIntervalDay(14)", "toIntervalDay(14) TO VOLUME 'cold'"), "move"),
            (LOGS.replace("toIntervalDay(14)", "toIntervalDay(14) DELETE WHERE level = 1"), "delete where"),
            (LOGS.replace("toIntervalDay(14)", "toIntervalDay(14), toDateTime(time) + toIntervalDay(1)"), "two"),
            (LOGS.replace("toDateTime(time)", "toDateTime(ingested_at)"), "other column"),
            (format!("{LOGS} TTL toDateTime(time) + toIntervalDay(1)"), "second clause"),
        ] {
            assert_eq!(parse_ttl_days(&engine, "time"), None, "{why}: {engine}");
        }
        assert_eq!(parse_ttl_days(LOGS, "start_time"), None, "the column must match");
    }

    #[test]
    fn a_mismatch_names_the_variable_both_values_and_the_fix() {
        assert_eq!(check(Table::Logs, 14, Some(14)), Ok(()));
        let e = check(Table::Spans, 7, Some(14)).unwrap_err();
        for part in ["CONTROLHUB_SPANS_RETENTION_DAYS is 7 days", "keeps spans for 14 days", "just bootstrap"] {
            assert!(e.contains(part), "{part}: {e}");
        }
        let e = check(Table::Events, 30, None).unwrap_err();
        assert!(e.contains("`events`") && e.contains("event_time") && e.contains("just bootstrap"), "{e}");
    }

    #[test]
    fn longer_applies_shorter_needs_confirmation_only_when_rows_are_lost() {
        assert_eq!(decide(Table::Logs, 14, 14, 99, false), Decision::Keep);
        assert_eq!(decide(Table::Logs, 14, 30, 99, false), Decision::Apply, "lengthening never deletes");
        assert_eq!(decide(Table::Logs, 14, 7, 0, false), Decision::Apply, "nothing older: nothing lost");
        assert_eq!(decide(Table::Logs, 14, 7, 5, true), Decision::Apply, "confirmed");
        let Decision::Refuse(msg) = decide(Table::Events, 30, 7, 5, false) else { panic!("must refuse") };
        for part in ["CONTROLHUB_EVENTS_RETENTION_DAYS = 7", "from 30 days", "5 rows", "--shrink-retention"] {
            assert!(msg.contains(part), "{part}: {msg}");
        }
    }

    #[test]
    fn names_come_from_a_fixed_list() {
        let cols: Vec<_> = Table::ALL.iter().map(|t| (t.name(), t.time_column())).collect();
        assert_eq!(cols, [("logs", "time"), ("spans", "start_time"), ("events", "event_time")]);
        for t in Table::ALL {
            assert!(t.env_var().starts_with("CONTROLHUB_") && t.env_var().ends_with("_RETENTION_DAYS"));
        }
    }
}

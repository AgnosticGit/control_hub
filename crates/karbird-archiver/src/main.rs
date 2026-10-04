//! controlhub-archiver: moves closed log windows from ClickHouse into compressed segments in
//! S3-compatible object storage (ADR-0015). It only reads ClickHouse; the table TTL deletes.
//!
//! A window is recorded as done only after every service's segment is stored, verified and
//! catalogued, so a crash means the window is archived again. Windows that left ClickHouse
//! before they could be archived are recorded as `lost` and counted, never skipped silently.

mod job;
mod plan;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use clap::{Args as ClapArgs, Parser, Subcommand};
use controlhub_runtime::{backoff, init_tracing, load_local_env, shutdown_signal};
use controlhub_storage::archive::{ArchiveObjects, ObjectConfig};
use controlhub_storage::ch::{ChConfig, ChStore};
use controlhub_storage::pg::PgStore;
use controlhub_storage::retention::{self, Table};
use tokio::sync::watch;

use crate::job::{Ctx, Metrics, Summary, archive_window, parse_scope};
use crate::plan::{PlanInput, align_down, first_safe_start, plan};

const NS: i64 = 1_000_000_000;
const RECHECK_BATCH: i64 = 24;
const GC_BATCH: i64 = 500;

#[derive(Debug, Parser)]
#[command(name = "controlhub-archiver")]
struct Cli {
    #[command(flatten)]
    common: Common,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, ClapArgs)]
struct Common {
    #[arg(long, env = "CONTROLHUB_ARCHIVER_DATABASE_URL")]
    database_url: String,
    #[arg(long, env = "CONTROLHUB_CLICKHOUSE_URL", default_value = "http://127.0.0.1:18123")]
    clickhouse_url: String,
    #[arg(long, env = "CONTROLHUB_CLICKHOUSE_DATABASE", default_value = "controlhub")]
    clickhouse_database: String,
    #[arg(long, env = "CONTROLHUB_ARCHIVER_CLICKHOUSE_USER")]
    clickhouse_user: String,
    #[arg(long, env = "CONTROLHUB_ARCHIVER_CLICKHOUSE_PASSWORD")]
    clickhouse_password: String,
    #[arg(long, env = "CONTROLHUB_ARCHIVE_S3_ENDPOINT", default_value = "http://127.0.0.1:19000")]
    s3_endpoint: String,
    #[arg(long, env = "CONTROLHUB_ARCHIVE_S3_BUCKET", default_value = "controlhub-archive")]
    s3_bucket: String,
    #[arg(long, env = "CONTROLHUB_ARCHIVE_S3_REGION", default_value = "us-east-1")]
    s3_region: String,
    #[arg(long, env = "CONTROLHUB_ARCHIVER_S3_ACCESS_KEY")]
    s3_access_key: String,
    #[arg(long, env = "CONTROLHUB_ARCHIVER_S3_SECRET_KEY")]
    s3_secret_key: String,
    /// Archive window length.
    #[arg(long, env = "CONTROLHUB_ARCHIVE_WINDOW_SECS", default_value_t = 3600, value_parser = clap::value_parser!(u32).range(60..=86_400))]
    window_secs: u32,
    #[arg(long, env = "CONTROLHUB_ARCHIVE_ZSTD_LEVEL", default_value_t = 19, value_parser = clap::value_parser!(i32).range(1..=22))]
    zstd_level: i32,
    /// ClickHouse log retention; must equal the `logs` table TTL (`just bootstrap` applies it).
    #[arg(long, env = "CONTROLHUB_LOGS_RETENTION_DAYS", default_value_t = retention::DEFAULT_LOGS_DAYS, value_parser = clap::value_parser!(u32).range(i64::from(retention::MIN_DAYS)..=i64::from(retention::MAX_DAYS)))]
    logs_retention_days: u32,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Archive continuously (the service mode).
    Run(RunArgs),
    /// Archive the closed windows of one time range once, print a JSON summary and exit.
    /// Does not touch the window bookkeeping of `run`.
    Once(OnceArgs),
}

#[derive(Debug, ClapArgs)]
struct RunArgs {
    #[arg(long, env = "CONTROLHUB_ARCHIVER_ADMIN_ADDR", default_value = "127.0.0.1:9466")]
    admin_addr: SocketAddr,
    #[arg(long, env = "CONTROLHUB_ARCHIVE_INTERVAL_SECS", default_value_t = 60)]
    interval_secs: u64,
    /// A window is archived only after it has been closed this long.
    #[arg(long, env = "CONTROLHUB_ARCHIVE_MIN_AGE_SECS", default_value_t = 3600)]
    min_age_secs: u32,
    /// How long archived segments are kept.
    #[arg(long, env = "CONTROLHUB_ARCHIVE_RETENTION_DAYS", default_value_t = 365)]
    retention_days: u32,
    /// Windows are counted again this long after they closed; late rows trigger a re-archive.
    #[arg(long, env = "CONTROLHUB_ARCHIVE_RECHECK_AFTER_SECS", default_value_t = 86_400)]
    recheck_after_secs: u32,
    #[arg(long, env = "CONTROLHUB_ARCHIVE_MAX_WINDOWS_PER_TICK", default_value_t = 24)]
    max_windows: usize,
}

#[derive(Debug, ClapArgs)]
struct OnceArgs {
    #[arg(long, requires_all = ["project", "environment"])]
    tenant: Option<String>,
    #[arg(long, requires = "tenant")]
    project: Option<String>,
    #[arg(long, requires = "tenant")]
    environment: Option<String>,
    #[arg(long)]
    from_ns: i64,
    #[arg(long)]
    to_ns: i64,
    #[arg(long, default_value_t = 3600)]
    min_age_secs: u32,
}

fn now_ns() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
}

/// Windows of `[from, to)` that are closed for at least `min_age` at `now`.
fn once_windows(from: i64, to: i64, now: i64, window: i64, min_age: i64) -> Vec<(i64, i64)> {
    let until = to.min(align_down(now.saturating_sub(min_age), window));
    let mut out = Vec::new();
    let mut start = align_down(from, window);
    while start.saturating_add(window) <= until && out.len() < 10_000 {
        out.push((start, start + window));
        start += window;
    }
    out
}

async fn connect(c: &Common) -> anyhow::Result<Ctx> {
    let pg = PgStore::connect(&c.database_url, 4).await?;
    let ch = ChStore::new(&ChConfig {
        url: c.clickhouse_url.clone(),
        database: c.clickhouse_database.clone(),
        user: c.clickhouse_user.clone(),
        password: c.clickhouse_password.clone(),
    });
    ch.ping().await?;
    // Planning assumes this is when ClickHouse deletes logs; a wrong value loses windows.
    ch.verify_retention(Table::Logs, c.logs_retention_days).await?;
    let objects = ArchiveObjects::s3(&ObjectConfig {
        endpoint: c.s3_endpoint.clone(),
        bucket: c.s3_bucket.clone(),
        region: c.s3_region.clone(),
        access_key: c.s3_access_key.clone(),
        secret_key: c.s3_secret_key.clone(),
    })?;
    Ok(Ctx { pg, ch, objects, zstd_level: c.zstd_level, metrics: Arc::new(Metrics::default()) })
}

/// Deletes segments past retention and the archive of deleted projects.
async fn gc(ctx: &Ctx, retention_edge: i64) -> anyhow::Result<()> {
    for seg in ctx.pg.expired_archive_segments(retention_edge, GC_BATCH).await? {
        ctx.objects.delete(&seg.object_key).await?;
        ctx.pg.delete_archive_segment(&seg).await?;
        ctx.metrics.gc_segments_deleted.fetch_add(1, Ordering::Relaxed);
    }
    for (key, ..) in ctx.pg.orphaned_archive_dictionaries(GC_BATCH).await? {
        ctx.objects.delete(&key).await?;
        ctx.pg.delete_archive_dictionary(&key).await?;
    }
    Ok(())
}

async fn tick(ctx: &Ctx, window: i64, hot_retention_days: u32, a: &RunArgs) -> anyhow::Result<()> {
    let now = now_ns();
    let day = 86_400 * NS;
    gc(ctx, now - i64::from(a.retention_days) * day).await?;

    let last_done_end = ctx.pg.last_archive_window_end().await?;
    let earliest_data = match last_done_end {
        Some(_) => None,
        None => ctx.ch.earliest_log_time().await?,
    };
    let input = PlanInput {
        now,
        window,
        min_age: i64::from(a.min_age_secs) * NS,
        hot_retention: i64::from(hot_retention_days) * day,
        safety: window,
        earliest_data,
        last_done_end,
        max_windows: a.max_windows,
    };
    let p = plan(&input);
    for &(start, end) in &p.lost {
        ctx.pg.record_archive_window(start, end, false, 0, 0).await?;
        ctx.metrics.windows_lost.fetch_add(1, Ordering::Relaxed);
        tracing::error!(start, end, "log window expired from ClickHouse before it was archived");
    }
    for &(start, end) in &p.archive {
        let s = archive_window(ctx, start, end, None).await?;
        ctx.pg.record_archive_window(start, end, true, i64::try_from(s.rows)?, i32::try_from(s.segments)?).await?;
        ctx.metrics.windows_archived.fetch_add(1, Ordering::Relaxed);
        ctx.metrics.last_window_end.store(u64::try_from(end).unwrap_or(0), Ordering::Relaxed);
        tracing::info!(start, rows = s.rows, segments = s.segments, stored = s.stored_bytes, "window archived");
    }

    let safe = first_safe_start(&input);
    let recheck_edge = now - i64::from(a.recheck_after_secs) * NS;
    for w in ctx.pg.archive_windows_to_recheck(recheck_edge, RECHECK_BATCH).await? {
        let rows = i64::try_from(ctx.ch.count_logs(w.window_start_ns, w.window_end_ns, None).await?)?;
        // The archive never shrinks: only more rows than archived (late data) re-archive it,
        // and only while ClickHouse still holds the whole window.
        if rows > w.rows && w.window_start_ns >= safe {
            let s = archive_window(ctx, w.window_start_ns, w.window_end_ns, None).await?;
            ctx.pg
                .mark_archive_window_rechecked(
                    w.window_start_ns,
                    i64::try_from(s.rows)?,
                    Some(i32::try_from(s.segments)?),
                )
                .await?;
            ctx.metrics.windows_rearchived.fetch_add(1, Ordering::Relaxed);
            tracing::info!(
                start = w.window_start_ns,
                before = w.rows,
                after = s.rows,
                "window re-archived with late rows"
            );
        } else {
            ctx.pg.mark_archive_window_rechecked(w.window_start_ns, w.rows, None).await?;
        }
    }
    Ok(())
}

async fn run(ctx: Ctx, window: i64, hot_retention_days: u32, a: RunArgs) -> anyhow::Result<()> {
    let (stop_tx, stop_rx) = watch::channel(false);
    let m = ctx.metrics.clone();
    let admin = axum::Router::new().route(
        "/metrics",
        axum::routing::get(move || {
            let m = m.clone();
            async move { m.render() }
        }),
    );
    let listener = tokio::net::TcpListener::bind(a.admin_addr).await?;
    let mut admin_stop = stop_rx.clone();
    let admin_task = tokio::spawn(
        axum::serve(listener, admin)
            .with_graceful_shutdown(async move {
                let _ = admin_stop.wait_for(|v| *v).await;
            })
            .into_future(),
    );
    tracing::info!(admin = %a.admin_addr, "controlhub-archiver running");

    let worker = {
        let mut stop = stop_rx.clone();
        tokio::spawn(async move {
            let mut failures = 0u32;
            loop {
                let pause = match tick(&ctx, window, hot_retention_days, &a).await {
                    Ok(()) => {
                        failures = 0;
                        Duration::from_secs(a.interval_secs.max(1))
                    }
                    Err(e) => {
                        ctx.metrics.failures.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(error = %e, failures, "archive tick failed; will retry");
                        failures = failures.saturating_add(1);
                        backoff(failures, Duration::from_secs(1), Duration::from_secs(a.interval_secs.max(1)))
                    }
                };
                tokio::select! {
                    _ = tokio::time::sleep(pause) => {}
                    _ = stop.wait_for(|v| *v) => break,
                }
            }
        })
    };

    shutdown_signal().await;
    let _ = stop_tx.send(true);
    let _ = tokio::time::timeout(Duration::from_secs(60), worker).await;
    let _ = admin_task.await;
    tracing::info!("controlhub-archiver stopped");
    Ok(())
}

async fn once(ctx: Ctx, window: i64, a: OnceArgs) -> anyhow::Result<Summary> {
    let scope = match (&a.tenant, &a.project, &a.environment) {
        (Some(t), Some(p), Some(e)) => Some(parse_scope(t, p, e)?),
        _ => None,
    };
    let mut total = Summary::default();
    for (start, end) in once_windows(a.from_ns, a.to_ns, now_ns(), window, i64::from(a.min_age_secs) * NS) {
        let s = archive_window(&ctx, start, end, scope.as_ref()).await?;
        total.windows += s.windows;
        total.segments += s.segments;
        total.rows += s.rows;
        total.source_bytes += s.source_bytes;
        total.stored_bytes += s.stored_bytes;
    }
    Ok(total)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_local_env();
    init_tracing("controlhub-archiver");
    let cli = Cli::parse();
    let window = i64::from(cli.common.window_secs) * NS;
    let ctx = connect(&cli.common).await?;
    match cli.cmd {
        Cmd::Run(a) => run(ctx, window, cli.common.logs_retention_days, a).await,
        Cmd::Once(a) => {
            let summary = once(ctx, window, a).await?;
            println!("{}", serde_json::to_string(&summary)?);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use clap::CommandFactory;

    const H: i64 = 3_600 * NS;

    fn base() -> Vec<&'static str> {
        vec![
            "controlhub-archiver",
            "--database-url",
            "postgres://x",
            "--clickhouse-user",
            "u",
            "--clickhouse-password",
            "p",
            "--s3-access-key",
            "a",
            "--s3-secret-key",
            "s",
        ]
    }

    #[test]
    fn cli_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn run_has_safe_defaults() {
        let mut args = base();
        args.push("run");
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!((cli.common.window_secs, cli.common.zstd_level), (3600, 19));
        if std::env::var_os("CONTROLHUB_LOGS_RETENTION_DAYS").is_none() {
            assert_eq!(cli.common.logs_retention_days, 14);
        }
        let Cmd::Run(r) = cli.cmd else { panic!("run expected") };
        assert_eq!(r.admin_addr.to_string(), "127.0.0.1:9466");
        assert_eq!((r.min_age_secs, r.retention_days), (3600, 365));
    }

    #[test]
    fn log_retention_is_bounded() {
        for bad in ["0", "1", "3651", "x"] {
            let mut args = base();
            args.extend(["--logs-retention-days", bad, "run"]);
            assert!(Cli::try_parse_from(args).is_err(), "{bad}");
        }
        let mut args = base();
        args.extend(["--logs-retention-days", "30", "once", "--from-ns", "0", "--to-ns", "1"]);
        assert_eq!(Cli::try_parse_from(args).unwrap().common.logs_retention_days, 30);
    }

    #[test]
    fn once_needs_a_full_scope_or_none() {
        let mut args = base();
        args.extend(["once", "--from-ns", "0", "--to-ns", "1", "--tenant", "t"]);
        assert!(Cli::try_parse_from(args).is_err(), "a tenant without project and environment is rejected");
        let mut args = base();
        args.extend([
            "once",
            "--from-ns",
            "0",
            "--to-ns",
            "1",
            "--tenant",
            "t",
            "--project",
            "p",
            "--environment",
            "e",
        ]);
        assert!(Cli::try_parse_from(args).is_ok());
        let mut args = base();
        args.extend(["--window-secs", "5", "once", "--from-ns", "0", "--to-ns", "1"]);
        assert!(Cli::try_parse_from(args).is_err(), "windows shorter than a minute are rejected");
    }

    #[test]
    fn once_takes_only_closed_aligned_windows() {
        let now = 100 * H + H / 2;
        assert_eq!(
            once_windows(97 * H + 10, 100 * H, now, H, 0),
            vec![(97 * H, 98 * H), (98 * H, 99 * H), (99 * H, 100 * H)]
        );
        assert_eq!(once_windows(97 * H, 100 * H, now, H, H), vec![(97 * H, 98 * H), (98 * H, 99 * H)], "min age");
        assert_eq!(
            once_windows(97 * H, 99 * H - 1, now, H, 0),
            vec![(97 * H, 98 * H)],
            "a partial last window is skipped"
        );
        assert!(once_windows(100 * H, 101 * H, now, H, 0).is_empty(), "the open window is never archived");
    }
}

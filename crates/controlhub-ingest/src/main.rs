//! controlhub-ingest: accepts telemetry and writes it durably to the broker.
//! It has no access to ClickHouse and no infrastructure-control permissions.

mod auth;
mod broker;
mod events;
mod grpc;
mod http;
mod metrics;
mod pipeline;
mod sanitize;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use controlhub_protocol::TELEMETRY_TOPIC;
use controlhub_protocol::otlp::collector::logs::v1::logs_service_server::LogsServiceServer;
use controlhub_protocol::otlp::collector::trace::v1::trace_service_server::TraceServiceServer;
use controlhub_runtime::{connect_broker, ensure_topic, init_tracing, load_local_env, shutdown_signal};
use controlhub_storage::pg::PgStore;
use tokio::sync::watch;

#[derive(Debug, Parser)]
#[command(name = "controlhub-ingest", about = "ControlHub telemetry ingestion")]
struct Args {
    #[arg(long, env = "CONTROLHUB_INGEST_HTTP_ADDR", default_value = "127.0.0.1:4318")]
    http_addr: SocketAddr,
    #[arg(long, env = "CONTROLHUB_INGEST_GRPC_ADDR", default_value = "127.0.0.1:4317")]
    grpc_addr: SocketAddr,
    /// Loopback-only admin endpoint for self-metrics.
    #[arg(long, env = "CONTROLHUB_INGEST_ADMIN_ADDR", default_value = "127.0.0.1:9464")]
    admin_addr: SocketAddr,
    #[arg(long, env = "CONTROLHUB_INGEST_DATABASE_URL")]
    database_url: String,
    #[arg(long, env = "CONTROLHUB_KAFKA_BROKERS", default_value = "127.0.0.1:19092")]
    brokers: String,
    #[arg(long, env = "CONTROLHUB_TOPIC_PARTITIONS", default_value_t = 4)]
    partitions: i32,
    #[arg(long, env = "CONTROLHUB_INGEST_MAX_CONCURRENCY", default_value_t = 512)]
    max_concurrency: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_local_env();
    init_tracing("controlhub-ingest");
    let args = Args::parse();

    let pg = PgStore::connect(&args.database_url, 8).await?;
    let kafka = connect_broker(&args.brokers, Duration::from_secs(60)).await?;
    let partitions = ensure_topic(&kafka, TELEMETRY_TOPIC, args.partitions).await?;
    let metrics = Arc::new(metrics::Metrics::default());
    let (publisher, publisher_tasks) = broker::Publisher::start(
        kafka.clone(),
        TELEMETRY_TOPIC,
        partitions,
        broker::PublisherConfig::default(),
        metrics.clone(),
    )
    .await?;

    let state = pipeline::AppState {
        auth: auth::Authenticator::new(pg),
        publisher,
        metrics: metrics.clone(),
        source_id: format!("ingest-{}", uuid::Uuid::now_v7()),
    };

    let (stop_tx, stop_rx) = watch::channel(false);
    let wait_stop = |mut rx: watch::Receiver<bool>| async move {
        let _ = rx.wait_for(|v| *v).await;
    };

    let http_listener = tokio::net::TcpListener::bind(args.http_addr).await?;
    let http_app = http::router(state.clone(), args.max_concurrency);
    let http_task = tokio::spawn(
        axum::serve(http_listener, http_app).with_graceful_shutdown(wait_stop(stop_rx.clone())).into_future(),
    );

    let grpc_svc = grpc::OtlpGrpc { state: state.clone() };
    let grpc_task = tokio::spawn(
        tonic::transport::Server::builder()
            .concurrency_limit_per_connection(256)
            .timeout(Duration::from_secs(20))
            .add_service(
                TraceServiceServer::new(grpc_svc.clone())
                    .max_decoding_message_size(http::MAX_DECOMPRESSED_BODY)
                    .accept_compressed(tonic::codec::CompressionEncoding::Gzip),
            )
            .add_service(
                LogsServiceServer::new(grpc_svc)
                    .max_decoding_message_size(http::MAX_DECOMPRESSED_BODY)
                    .accept_compressed(tonic::codec::CompressionEncoding::Gzip),
            )
            .serve_with_shutdown(args.grpc_addr, wait_stop(stop_rx.clone())),
    );

    let admin_metrics = metrics.clone();
    let admin_app = axum::Router::new().route(
        "/metrics",
        axum::routing::get(move || {
            let m = admin_metrics.clone();
            async move { m.render() }
        }),
    );
    let admin_listener = tokio::net::TcpListener::bind(args.admin_addr).await?;
    let admin_task = tokio::spawn(
        axum::serve(admin_listener, admin_app).with_graceful_shutdown(wait_stop(stop_rx.clone())).into_future(),
    );

    tracing::info!(http = %args.http_addr, grpc = %args.grpc_addr, admin = %args.admin_addr, partitions, "controlhub-ingest listening");
    shutdown_signal().await;
    let _ = stop_tx.send(true);
    let _ = http_task.await;
    let _ = grpc_task.await;
    let _ = admin_task.await;
    // Drop the last publisher handle so partition workers drain their queues and exit.
    drop(state);
    for t in publisher_tasks {
        let _ = tokio::time::timeout(Duration::from_secs(20), t).await;
    }
    tracing::info!("controlhub-ingest stopped");
    Ok(())
}

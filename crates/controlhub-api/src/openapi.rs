use utoipa::OpenApi;

use crate::routes;

#[derive(OpenApi)]
#[openapi(
    info(title = "ControlHub API", version = "0.1.0",
        description = "Versioned ControlHub REST API. Nanosecond timestamps are decimal strings."),
    paths(
        routes::health,
        routes::ready,
        routes::audit,
        routes::session::login,
        routes::session::session,
        routes::session::logout,
        routes::session::projects,
        routes::telemetry::services,
        routes::telemetry::topology,
        routes::telemetry::traces,
        routes::telemetry::trace_detail,
        routes::telemetry::logs,
        routes::telemetry::events,
        routes::telemetry::timeline,
        routes::telemetry::coverage,
    ),
    tags(
        (name = "session"), (name = "projects"), (name = "telemetry"),
        (name = "timeline"), (name = "audit"), (name = "health")
    )
)]
pub struct ApiDoc;

pub fn spec_json() -> anyhow::Result<String> {
    Ok(ApiDoc::openapi().to_pretty_json()?)
}

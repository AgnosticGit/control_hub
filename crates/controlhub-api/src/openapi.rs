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
        routes::signup::register,
        routes::session::session,
        routes::session::logout,
        routes::session::projects,
        routes::roles::catalog,
        routes::roles::list_roles,
        routes::roles::create_role,
        routes::roles::update_role,
        routes::roles::delete_role,
        routes::roles::list_members,
        routes::roles::update_member,
        routes::roles::remove_member,
        routes::invites::list,
        routes::invites::create,
        routes::invites::revoke,
        routes::invites::preview,
        routes::invites::accept,
        routes::account::update,
        routes::account::change_password,
        routes::account::sessions,
        routes::account::revoke_session,
        routes::account::revoke_other_sessions,
        routes::account::create_project,
        routes::account::rename_project,
        routes::account::leave_project,
        routes::telemetry::services,
        routes::telemetry::topology,
        routes::telemetry::traces,
        routes::telemetry::trace_detail,
        routes::telemetry::logs,
        routes::telemetry::events,
        routes::telemetry::timeline,
        routes::telemetry::coverage,
        routes::live::live,
        routes::tokens::list,
        routes::tokens::create,
        routes::tokens::revoke,
        routes::tokens::rotate,
        routes::actions::catalog,
        routes::actions::list,
        routes::actions::detail,
        routes::actions::propose,
        routes::actions::approve,
        routes::actions::reject,
        routes::actions::cancel,
    ),
    tags(
        (name = "session"), (name = "account"), (name = "projects"), (name = "telemetry"),
        (name = "timeline"), (name = "audit"), (name = "tokens"), (name = "actions"), (name = "members"), (name = "health")
    ),
    modifiers(&BearerAuth)
)]
pub struct ApiDoc;

/// Documents both credentials: the browser session cookie and read-only bearer API tokens.
struct BearerAuth;

impl utoipa::Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{ApiKey, ApiKeyValue, HttpAuthScheme, HttpBuilder, SecurityScheme};
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "api_token",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some("Read-only API token (cha_…) bound to one environment; GET only."))
                    .build(),
            ),
        );
        components.add_security_scheme(
            "session",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new("controlhub_session"))),
        );
    }
}

pub fn spec_json() -> anyhow::Result<String> {
    Ok(ApiDoc::openapi().to_pretty_json()?)
}

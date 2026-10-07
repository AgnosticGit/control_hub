//! Self-service sign-up with email and password. Creates an account and starts
//! a browser session; the first-run wizard creates its organisation and project.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use karbird_storage::secrets::hash_password;
use serde::Deserialize;
use tokio::sync::Mutex;
use utoipa::ToSchema;

use crate::auth::check_origin;
use crate::error::{ApiError, ApiResult};
use crate::routes::session::start_session;
use crate::state::AppState;

pub const MIN_PASSWORD_CHARS: usize = 12;
pub const MAX_PASSWORD_BYTES: usize = 1024;
const MAX_NAME_CHARS: usize = 100;
/// Defaults used only by the production bootstrap owner. Self-service
/// registrations complete first-run onboarding instead.
pub(crate) const DEFAULT_PROJECT: &str = "My project";
pub(crate) const DEFAULT_ENVIRONMENT: &str = "production";

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterRequest {
    pub email: String,
    /// At least 12 characters.
    pub password: String,
    /// Shown in the UI; defaults to the part of the email before `@`.
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct Registration {
    email: String,
    password: String,
    display_name: String,
}

/// Global sliding-window limit on sign-up attempts. Global rather than per client because
/// behind the web proxy every request comes from the same address; it also bounds how fast
/// anyone can probe which emails are registered.
#[derive(Debug, Clone, Default)]
pub struct SignupThrottle {
    inner: Arc<Mutex<VecDeque<Instant>>>,
}

impl SignupThrottle {
    const WINDOW: Duration = Duration::from_secs(60 * 60);

    pub async fn take(&self, per_hour: u32) -> Result<(), ApiError> {
        let mut q = self.inner.lock().await;
        while q.front().is_some_and(|t| t.elapsed() >= Self::WINDOW) {
            q.pop_front();
        }
        if q.len() >= per_hour as usize {
            return Err(ApiError::TooManyRequests);
        }
        q.push_back(Instant::now());
        Ok(())
    }
}

pub(crate) fn normalise_email(raw: &str) -> Result<String, ApiError> {
    let bad = || ApiError::BadRequest("enter a valid email address".into());
    let email = raw.trim().to_lowercase();
    if email.len() > 254 {
        return Err(bad());
    }
    let (local, domain) = email.split_once('@').ok_or_else(bad)?;
    let local_ok = !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local.bytes().all(|b| b.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&b));
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    if local_ok && domain_ok { Ok(email) } else { Err(bad()) }
}

fn clean_name(raw: Option<&str>, field: &str, default: &str) -> Result<String, ApiError> {
    let name = raw.map(str::trim).filter(|s| !s.is_empty()).unwrap_or(default);
    if name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(format!("{field} must be at most {MAX_NAME_CHARS} printable characters")));
    }
    Ok(name.to_owned())
}

/// A name the user must give (profile, new project): trimmed, 1-100 printable characters.
pub(crate) fn required_name(raw: &str, field: &str) -> Result<String, ApiError> {
    if raw.trim().is_empty() {
        return Err(ApiError::BadRequest(format!("{field} must not be empty")));
    }
    clean_name(Some(raw), field, "")
}

/// Rules for a new password (sign-up and password change) of the account with `email`.
pub(crate) fn check_new_password(email: &str, password: &str) -> Result<(), ApiError> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(ApiError::BadRequest(format!("password must be at least {MIN_PASSWORD_CHARS} characters")));
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(ApiError::BadRequest(format!("password must be at most {MAX_PASSWORD_BYTES} bytes")));
    }
    if password.trim().to_lowercase() == email {
        return Err(ApiError::BadRequest("password must not be your email".into()));
    }
    Ok(())
}

fn validate(req: RegisterRequest) -> Result<Registration, ApiError> {
    let email = normalise_email(&req.email)?;
    check_new_password(&email, &req.password)?;
    let local = email.split('@').next().unwrap_or_default().to_owned();
    Ok(Registration {
        display_name: clean_name(req.display_name.as_deref(), "display name", &local)?,
        password: req.password,
        email,
    })
}

#[utoipa::path(post, path = "/api/v1/session/register", tag = "session", request_body = RegisterRequest,
    responses(
        (status = 201, description = "Account created and signed in", body = crate::routes::session::SessionResponse),
        (status = 400, body = crate::error::ErrorBody),
        (status = 403, description = "Origin not allowed or sign-up disabled", body = crate::error::ErrorBody),
        (status = 409, description = "Email already registered", body = crate::error::ErrorBody),
        (status = 429, body = crate::error::ErrorBody)))]
pub async fn register(
    State(state): State<AppState>,
    request_headers: HeaderMap,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<Response> {
    check_origin(&state, &request_headers)?;
    if !state.config.signup.open {
        return Err(ApiError::Forbidden("sign-up is disabled on this server"));
    }
    let r = validate(req)?;
    state.signup_throttle.take(state.config.signup.per_hour).await?;
    let password = r.password;
    let hash = tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;
    let user_id = state
        .pg
        .create_unconfigured_account(&r.email, &r.display_name, &hash)
        .await?
        .ok_or(ApiError::Conflict("an account with this email already exists"))?;
    let _ = state
        .pg
        .audit(karbird_storage::pg::NewAuditEntry {
            tenant_id: None,
            project_id: None,
            actor_id: &user_id.to_string(),
            actor_kind: "user",
            action: "user.register",
            target: Some(&user_id.to_string()),
            reason: None,
            before: None,
            after: None,
            outcome: "succeeded",
            request_id: None,
        })
        .await;
    let (headers, body) = start_session(&state, user_id, r.email).await?;
    Ok((StatusCode::CREATED, headers, Json(body)).into_response())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn req(email: &str, password: &str) -> RegisterRequest {
        RegisterRequest { email: email.into(), password: password.into(), display_name: None }
    }

    #[test]
    fn emails_are_trimmed_lowercased_and_checked() {
        assert_eq!(normalise_email("  Alice.B+tag@Example.COM ").unwrap(), "alice.b+tag@example.com");
        assert_eq!(normalise_email("a@sub.example.io").unwrap(), "a@sub.example.io");
        for bad in [
            "",
            "   ",
            "alice",
            "@example.com",
            "alice@",
            "alice@localhost",
            "alice@@example.com",
            "a@b@example.com",
            "al ice@example.com",
            "alice@exa mple.com",
            "alice@-example.com",
            "alice@example-.com",
            "alice@example..com",
            ".alice@example.com",
            "alice.@example.com",
            "al..ice@example.com",
            "alice\n@example.com",
            "alice@example.com\r\nBcc: x@y.z",
            "<script>@example.com",
            "\"quoted\"@example.com",
            "алиса@example.com",
        ] {
            assert!(normalise_email(bad).is_err(), "{bad:?} must be rejected");
        }
        let long_local = format!("{}@example.com", "a".repeat(65));
        let long_total = format!("a@{}.com", "b".repeat(250));
        assert!(normalise_email(&long_local).is_err());
        assert!(normalise_email(&long_total).is_err());
    }

    #[test]
    fn passwords_need_twelve_characters_and_are_bounded() {
        assert!(
            matches!(validate(req("a@example.com", "short")), Err(ApiError::BadRequest(m)) if m.contains("at least 12"))
        );
        assert!(validate(req("a@example.com", &"x".repeat(11))).is_err());
        assert!(validate(req("a@example.com", &"x".repeat(12))).is_ok());
        // Twelve characters, not twelve bytes.
        assert!(validate(req("a@example.com", &"пароль".repeat(2))).is_ok());
        assert!(validate(req("a@example.com", &"x".repeat(MAX_PASSWORD_BYTES))).is_ok());
        assert!(
            matches!(validate(req("a@example.com", &"x".repeat(MAX_PASSWORD_BYTES + 1))), Err(ApiError::BadRequest(m)) if m.contains("at most"))
        );
    }

    #[test]
    fn the_email_is_not_a_password() {
        assert!(validate(req("alice@example.com", "Alice@Example.com")).is_err());
        assert!(validate(req("alice@example.com", " alice@example.com ")).is_err());
    }

    #[test]
    fn names_default_and_are_bounded() {
        let r = validate(req(" Bob@Example.com", "correct horse battery")).unwrap();
        assert_eq!(
            r,
            Registration {
                email: "bob@example.com".into(),
                password: "correct horse battery".into(),
                display_name: "bob".into(),
            }
        );
        let mut named = req("bob@example.com", "correct horse battery");
        named.display_name = Some("  Bob Smith ".into());
        let r = validate(named).unwrap();
        assert_eq!(r.display_name.as_str(), "Bob Smith");

        for bad in ["x".repeat(MAX_NAME_CHARS + 1), "evil\u{0}name".into(), "line\nbreak".into()] {
            let mut r = req("bob@example.com", "correct horse battery");
            r.display_name = Some(bad.clone());
            assert!(validate(r).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn required_names_must_not_be_blank() {
        assert_eq!(required_name("  Shop ", "name").unwrap(), "Shop");
        for bad in ["", "   ", "a\u{0}b", &"x".repeat(MAX_NAME_CHARS + 1)] {
            assert!(matches!(required_name(bad, "name"), Err(ApiError::BadRequest(_))), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn throttle_allows_the_configured_number_per_hour() {
        let t = SignupThrottle::default();
        for _ in 0..3 {
            t.take(3).await.unwrap();
        }
        assert!(matches!(t.take(3).await, Err(ApiError::TooManyRequests)));
        assert!(matches!(SignupThrottle::default().take(0).await, Err(ApiError::TooManyRequests)), "0 = closed");
    }
}

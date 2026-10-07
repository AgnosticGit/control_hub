use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use base64::Engine;
use karbird_storage::secrets::{generate_token, hash_password, hash_token};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use tokio::sync::Mutex;

use crate::error::{ApiError, ApiResult};
use crate::routes::session::start_session;
use crate::routes::signup::normalise_email;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct GoogleOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
}

#[derive(Debug, Clone)]
struct OAuthAttempt {
    verifier: String,
    next: String,
    expires_at: OffsetDateTime,
}

#[derive(Debug, Clone, Default)]
pub struct OAuthStateStore {
    attempts: Arc<Mutex<HashMap<Vec<u8>, OAuthAttempt>>>,
}

impl OAuthStateStore {
    async fn insert(&self, state: &str, attempt: OAuthAttempt) {
        let mut attempts = self.attempts.lock().await;
        attempts.retain(|_, value| value.expires_at > OffsetDateTime::now_utc());
        attempts.insert(hash_token(state), attempt);
    }

    async fn take(&self, state: &str) -> Option<OAuthAttempt> {
        let attempt = self.attempts.lock().await.remove(&hash_token(state))?;
        (attempt.expires_at > OffsetDateTime::now_utc()).then_some(attempt)
    }
}

#[derive(Debug, Deserialize)]
pub struct StartQuery {
    pub next: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    id_token: String,
}

#[derive(Debug, Deserialize)]
struct TokenInfo {
    sub: String,
    email: String,
    email_verified: String,
    aud: String,
    iss: String,
    exp: String,
}

#[derive(Serialize)]
struct TokenRequest<'a> {
    code: &'a str,
    client_id: &'a str,
    client_secret: &'a str,
    redirect_uri: &'a str,
    grant_type: &'static str,
    code_verifier: &'a str,
}

fn safe_next(value: Option<&str>) -> String {
    value
        .filter(|next| next.starts_with("/app") && !next.starts_with("//") && !next.contains(['\r', '\n']))
        .unwrap_or("/app")
        .to_owned()
}

fn verify_token_info(info: &TokenInfo, config: &GoogleOAuthConfig) -> ApiResult<()> {
    let issuer_ok = matches!(info.iss.as_str(), "accounts.google.com" | "https://accounts.google.com");
    let expires = info.exp.parse::<i64>().unwrap_or_default();
    if !issuer_ok
        || info.aud != config.client_id
        || info.email_verified != "true"
        || expires <= OffsetDateTime::now_utc().unix_timestamp()
        || info.sub.is_empty()
    {
        return Err(ApiError::Unauthenticated);
    }
    Ok(())
}

#[utoipa::path(get, path = "/api/v1/auth/google/start", tag = "session", operation_id = "start_google_oauth",
    params(("next" = Option<String>, Query)), responses((status = 307, description = "Redirect to Google"), (status = 404)))]
pub async fn start(State(state): State<AppState>, Query(query): Query<StartQuery>) -> ApiResult<Redirect> {
    let config = state.google_oauth.as_ref().ok_or(ApiError::NotFound)?;
    let state_token = generate_token("kos_")?;
    let verifier = generate_token("kov_")?;
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    state
        .oauth_states
        .insert(
            &state_token,
            OAuthAttempt {
                verifier,
                next: safe_next(query.next.as_deref()),
                expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(10),
            },
        )
        .await;
    let mut url = reqwest::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    url.query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &config.redirect_url)
        .append_pair("response_type", "code")
        .append_pair("scope", "openid email profile")
        .append_pair("state", &state_token)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("prompt", "select_account");
    Ok(Redirect::temporary(url.as_str()))
}

#[utoipa::path(get, path = "/api/v1/auth/google/callback", tag = "session", operation_id = "finish_google_oauth",
    params(("code" = Option<String>, Query), ("state" = Option<String>, Query), ("error" = Option<String>, Query)),
    responses((status = 303, description = "Session started"), (status = 401, body = crate::error::ErrorBody)))]
pub async fn callback(State(state): State<AppState>, Query(query): Query<CallbackQuery>) -> ApiResult<Response> {
    if query.error.is_some() {
        return Ok(Redirect::temporary("/app/login?oauth=cancelled").into_response());
    }
    let config = state.google_oauth.as_ref().ok_or(ApiError::NotFound)?;
    let state_token = query.state.as_deref().ok_or(ApiError::Unauthenticated)?;
    let attempt = state.oauth_states.take(state_token).await.ok_or(ApiError::Unauthenticated)?;
    let code = query.code.as_deref().ok_or(ApiError::Unauthenticated)?;
    let token = state
        .oauth_client
        .post("https://oauth2.googleapis.com/token")
        .form(&TokenRequest {
            code,
            client_id: &config.client_id,
            client_secret: &config.client_secret,
            redirect_uri: &config.redirect_url,
            grant_type: "authorization_code",
            code_verifier: &attempt.verifier,
        })
        .send()
        .await
        .map_err(|_| ApiError::Unauthenticated)?;
    if !token.status().is_success() {
        return Err(ApiError::Unauthenticated);
    }
    let token: TokenResponse = token.json().await.map_err(|_| ApiError::Unauthenticated)?;
    // Google's tokeninfo endpoint performs signature, issuer and expiry verification. We still
    // enforce audience, issuer and verified email locally before linking any identity.
    let info = state
        .oauth_client
        .get("https://oauth2.googleapis.com/tokeninfo")
        .query(&[("id_token", &token.id_token)])
        .send()
        .await
        .map_err(|_| ApiError::Unauthenticated)?;
    if !info.status().is_success() {
        return Err(ApiError::Unauthenticated);
    }
    let info: TokenInfo = info.json().await.map_err(|_| ApiError::Unauthenticated)?;
    verify_token_info(&info, config)?;
    let email = normalise_email(&info.email)?;
    let user = match state.pg.user_for_oauth_identity("google", &info.sub).await? {
        Some(user) => user,
        None => {
            let user_id = match state.pg.user_id_by_email(&email).await? {
                Some(user_id) => user_id,
                None => {
                    let password = generate_token("google_only_")?;
                    let hash = tokio::task::spawn_blocking(move || hash_password(&password))
                        .await
                        .map_err(|error| ApiError::Internal(error.to_string()))??;
                    let display_name = email.split('@').next().unwrap_or("user");
                    state
                        .pg
                        .create_unconfigured_account(&email, display_name, &hash)
                        .await?
                        .ok_or(ApiError::Conflict("an account with this email already exists"))?
                }
            };
            if !state.pg.link_oauth_identity("google", &info.sub, user_id, &email).await? {
                return Err(ApiError::Conflict("this Google account is already linked"));
            }
            state.pg.find_user_by_email(&email).await?.ok_or(ApiError::Unauthenticated)?
        }
    };
    let (cookies, _) = start_session(&state, user.id, user.email).await?;
    let mut headers = HeaderMap::new();
    for value in cookies.get_all(header::SET_COOKIE) {
        headers.append(header::SET_COOKIE, value.clone());
    }
    headers.insert(header::LOCATION, attempt.next.parse().map_err(|_| ApiError::Internal("invalid redirect".into()))?);
    Ok((StatusCode::SEE_OTHER, headers).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> GoogleOAuthConfig {
        GoogleOAuthConfig {
            client_id: "client".into(),
            client_secret: "secret".into(),
            redirect_url: "https://karbird.com/callback".into(),
        }
    }

    #[test]
    fn google_oauth_accepts_only_safe_local_destinations() {
        assert_eq!(safe_next(Some("/app/p/e")), "/app/p/e");
        for unsafe_next in ["https://evil.test", "//evil.test", "/admin", "/app\r\nLocation:https://evil.test"] {
            assert_eq!(safe_next(Some(unsafe_next)), "/app");
        }
    }

    #[test]
    fn google_oauth_requires_verified_current_claims_for_this_client() {
        let valid = TokenInfo {
            sub: "subject".into(),
            email: "a@example.test".into(),
            email_verified: "true".into(),
            aud: "client".into(),
            iss: "https://accounts.google.com".into(),
            exp: (OffsetDateTime::now_utc().unix_timestamp() + 60).to_string(),
        };
        assert!(verify_token_info(&valid, &config()).is_ok());
        assert!(verify_token_info(&TokenInfo { email_verified: "false".into(), ..valid }, &config()).is_err());
    }

    #[tokio::test]
    async fn google_oauth_state_is_one_time_and_expires() {
        let store = OAuthStateStore::default();
        store
            .insert(
                "state",
                OAuthAttempt {
                    verifier: "v".into(),
                    next: "/app".into(),
                    expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(1),
                },
            )
            .await;
        assert!(store.take("state").await.is_some());
        assert!(store.take("state").await.is_none());
        store
            .insert(
                "expired",
                OAuthAttempt {
                    verifier: "v".into(),
                    next: "/app".into(),
                    expires_at: OffsetDateTime::now_utc() - time::Duration::seconds(1),
                },
            )
            .await;
        assert!(store.take("expired").await.is_none());
    }
}

//! Typed API errors with safe, non-leaking responses.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("authentication required")]
    Unauthenticated,
    #[error("forbidden: {0}")]
    Forbidden(&'static str),
    #[error("not found")]
    NotFound,
    #[error("invalid request: {0}")]
    BadRequest(String),
    #[error("too many requests")]
    TooManyRequests,
    #[error("conflict: {0}")]
    Conflict(&'static str),
    #[error("gone: {0}")]
    Gone(&'static str),
    #[error("denied by policy: {0}")]
    PolicyDenied(String),
    #[error("unavailable: {0}")]
    Unavailable(&'static str),
    #[error("storage unavailable")]
    Storage(#[from] karbird_storage::StorageError),
    #[error("internal error")]
    Internal(String),
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    /// Stable machine-readable code.
    pub code: String,
    pub message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ApiError::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthenticated"),
            ApiError::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            ApiError::TooManyRequests => (StatusCode::TOO_MANY_REQUESTS, "too_many_requests"),
            ApiError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            ApiError::Gone(_) => (StatusCode::GONE, "gone"),
            ApiError::PolicyDenied(_) => (StatusCode::FORBIDDEN, "policy_denied"),
            ApiError::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            ApiError::Storage(e) => {
                tracing::error!(error = %e, "storage error");
                (StatusCode::SERVICE_UNAVAILABLE, "storage_unavailable")
            }
            ApiError::Internal(e) => {
                tracing::error!(error = %e, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal")
            }
        };
        let body = ErrorBody { code: code.to_owned(), message: self.to_string() };
        (status, Json(body)).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    async fn parts(e: ApiError) -> (StatusCode, serde_json::Value) {
        let res = e.into_response();
        let status = res.status();
        assert_eq!(res.headers().get("content-type").unwrap(), "application/json");
        let body = axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn every_variant_maps_to_a_stable_status_and_code() {
        let cases = [
            (ApiError::Unauthenticated, StatusCode::UNAUTHORIZED, "unauthenticated"),
            (ApiError::Forbidden("x"), StatusCode::FORBIDDEN, "forbidden"),
            (ApiError::NotFound, StatusCode::NOT_FOUND, "not_found"),
            (ApiError::BadRequest("x".into()), StatusCode::BAD_REQUEST, "bad_request"),
            (ApiError::TooManyRequests, StatusCode::TOO_MANY_REQUESTS, "too_many_requests"),
            (ApiError::Conflict("x"), StatusCode::CONFLICT, "conflict"),
            (ApiError::Gone("x"), StatusCode::GONE, "gone"),
            (ApiError::PolicyDenied("x".into()), StatusCode::FORBIDDEN, "policy_denied"),
            (ApiError::Unavailable("x"), StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            (
                ApiError::Storage(karbird_storage::StorageError::Other("x".into())),
                StatusCode::SERVICE_UNAVAILABLE,
                "storage_unavailable",
            ),
            (ApiError::Internal("x".into()), StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        ];
        for (e, status, code) in cases {
            let (s, body) = parts(e).await;
            assert_eq!((s, body["code"].as_str().unwrap()), (status, code));
            assert_eq!(body.as_object().unwrap().len(), 2, "only code and message: {body}");
        }
    }

    #[tokio::test]
    async fn storage_and_internal_details_are_never_returned() {
        let secret = "postgres://karbird_api:hunter2@db:5432 connection refused";
        let (_, body) = parts(ApiError::Storage(karbird_storage::StorageError::Other(secret.into()))).await;
        assert_eq!(body["message"], "storage unavailable");
        let (_, body) = parts(ApiError::Internal(format!("panic at src/x.rs:1 {secret}"))).await;
        assert_eq!(body["message"], "internal error");
    }

    #[tokio::test]
    async fn client_errors_explain_themselves() {
        let (_, body) = parts(ApiError::BadRequest("from must be before to".into())).await;
        assert_eq!(body["message"], "invalid request: from must be before to");
        let (_, body) = parts(ApiError::Forbidden("invalid CSRF token")).await;
        assert_eq!(body["message"], "forbidden: invalid CSRF token");
        let (_, body) = parts(ApiError::PolicyDenied("rate limit: 30 plan(s)".into())).await;
        assert_eq!(body["message"], "denied by policy: rate limit: 30 plan(s)");
        let (_, body) = parts(ApiError::NotFound).await;
        assert_eq!(body["message"], "not found");
    }

    #[test]
    fn storage_errors_convert_via_question_mark() {
        fn f() -> ApiResult<()> {
            Err(karbird_storage::StorageError::Other("down".into()))?
        }
        assert!(matches!(f(), Err(ApiError::Storage(_))));
    }
}

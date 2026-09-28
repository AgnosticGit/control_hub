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
    #[error("storage unavailable")]
    Storage(#[from] controlhub_storage::StorageError),
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

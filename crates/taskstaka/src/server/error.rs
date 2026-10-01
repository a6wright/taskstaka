use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use taskstaka_core::StoreError;

use crate::client::{ApiErrorBody, ApiErrorInner};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn internal(e: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", msg)
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        use StatusCode as S;
        let (status, code) = match &e {
            StoreError::NotFound(_) => (S::NOT_FOUND, "not_found"),
            StoreError::Ambiguous { .. } => (S::CONFLICT, "ambiguous_id"),
            StoreError::HasOpenDescendants { .. } => (S::CONFLICT, "has_open_descendants"),
            StoreError::HasDescendants { .. } => (S::CONFLICT, "has_descendants"),
            StoreError::InvalidTransition { .. } => (S::CONFLICT, "invalid_transition"),
            StoreError::Cycle { .. } => (S::CONFLICT, "cycle"),
            StoreError::Validation(_) => (S::UNPROCESSABLE_ENTITY, "validation"),
            StoreError::Locked(_) => (S::SERVICE_UNAVAILABLE, "locked"),
            _ => (S::INTERNAL_SERVER_ERROR, "store"),
        };
        let details = match &e {
            StoreError::Ambiguous { candidates, .. } => {
                Some(serde_json::json!({ "candidates": candidates }))
            }
            _ => None,
        };
        ApiError {
            status,
            code,
            message: e.to_string(),
            details,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ApiErrorBody {
            error: ApiErrorInner {
                code: self.code.to_string(),
                message: self.message,
                details: self.details,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

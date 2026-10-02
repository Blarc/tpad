use std::{fmt, io};

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug)]
pub enum AppError {
    Config(&'static str),
    InvalidPath,
    InvalidName,
    NotFound,
    Conflict(&'static str),
    TooLarge,
    Unsupported(&'static str),
    Forbidden,
    BadRequest(&'static str),
    Io(io::Error),
}

impl AppError {
    pub fn from_io(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::AlreadyExists => {
                Self::Conflict("An entry with that name already exists")
            }
            io::ErrorKind::PermissionDenied => Self::Forbidden,
            _ => Self::Io(error),
        }
    }

    fn response_parts(&self) -> (StatusCode, &'static str, &'static str) {
        match self {
            Self::Config(message) => (StatusCode::INTERNAL_SERVER_ERROR, "configuration", message),
            Self::InvalidPath => (
                StatusCode::BAD_REQUEST,
                "invalid_path",
                "The path is invalid",
            ),
            Self::InvalidName => (
                StatusCode::BAD_REQUEST,
                "invalid_name",
                "The name is invalid",
            ),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "The entry was not found",
            ),
            Self::Conflict(message) => (StatusCode::CONFLICT, "conflict", message),
            Self::TooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "too_large",
                "The file exceeds the configured size limit",
            ),
            Self::Unsupported(message) => {
                (StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported", message)
            }
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "The operation is not allowed",
            ),
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, "bad_request", message),
            Self::Io(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "io_error",
                "The filesystem operation failed",
            ),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "filesystem error: {error}"),
            _ => write!(formatter, "{}", self.response_parts().2),
        }
    }
}

impl std::error::Error for AppError {}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = self.response_parts();
        if let Self::Io(error) = &self {
            tracing::error!(%error, "filesystem request failed");
        }
        (
            status,
            Json(ErrorEnvelope {
                error: ErrorBody { code, message },
            }),
        )
            .into_response()
    }
}

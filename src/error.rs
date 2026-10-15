use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

/// Gateway error classification, mirroring the legacy Go `gateway.ErrorKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Unauthorized,
    BadRequest,
    RateLimited,
    Unavailable,
    BadGateway,
    Internal,
}

impl ErrorKind {
    pub fn status(self) -> StatusCode {
        match self {
            ErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrorKind::BadRequest => StatusCode::BAD_REQUEST,
            ErrorKind::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorKind::BadGateway => StatusCode::BAD_GATEWAY,
            ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Unauthorized => "unauthorized",
            ErrorKind::BadRequest => "bad_request",
            ErrorKind::RateLimited => "rate_limited",
            ErrorKind::Unavailable => "unavailable",
            ErrorKind::BadGateway => "bad_gateway",
            ErrorKind::Internal => "internal",
        }
    }
}

/// Gateway error carrying a classification and human-readable message.
#[derive(Debug)]
pub struct GatewayError {
    pub kind: ErrorKind,
    pub message: String,
}

impl GatewayError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unauthorized, msg)
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::BadRequest, msg)
    }

    pub fn rate_limited(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::RateLimited, msg)
    }

    pub fn unavailable(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unavailable, msg)
    }

    pub fn bad_gateway(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::BadGateway, msg)
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, msg)
    }
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gateway error ({}): {}", self.kind.as_str(), self.message)
    }
}

impl std::error::Error for GatewayError {}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let status = self.kind.status();
        let body = json!({
            "error": {
                "message": self.message,
                "type": self.kind.as_str(),
                "code": self.kind.as_str(),
            }
        });
        (status, Json(body)).into_response()
    }
}

/// Classify an upstream error into a gateway error kind.
/// 401 -> unauthorized (retryable), 429 -> rate_limited (retryable),
/// other 4xx -> bad_request (not retryable), 5xx/network -> bad_gateway (retryable).
pub fn classify_upstream(status: Option<u16>, detail: &str) -> GatewayError {
    match status {
        Some(401) | Some(403) => GatewayError::unauthorized(format!("upstream auth failed: {}", detail)),
        Some(429) => GatewayError::rate_limited(format!("upstream rate limited: {}", detail)),
        Some(s) if (400..500).contains(&s) => {
            GatewayError::bad_request(format!("upstream rejected request ({}): {}", s, detail))
        }
        Some(s) => GatewayError::bad_gateway(format!("upstream error ({}): {}", s, detail)),
        None => GatewayError::bad_gateway(format!("upstream network error: {}", detail)),
    }
}

/// Whether an error is worth retrying on another account.
pub fn retryable(err: &GatewayError) -> bool {
    matches!(
        err.kind,
        ErrorKind::RateLimited | ErrorKind::Unavailable | ErrorKind::BadGateway | ErrorKind::Unauthorized
    )
}

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
    /// Optional status override (A2: 403 for authenticated non-admin keys).
    pub status_override: Option<StatusCode>,
}

impl GatewayError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status_override: None,
        }
    }

    /// Override the HTTP status while keeping the error envelope format.
    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status_override = Some(status);
        self
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
        let status = self.status_override.unwrap_or_else(|| self.kind.status());
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

/// A1: panic 捕获层的统一响应。message 固定，不回显 panic 文本、源码路径或行号。
pub fn panic_response(_panic: Box<dyn std::any::Any + Send>) -> Response {
    // 尽力从请求上下文取信息；CatchPanicLayer 不提供 path/method，
    // 由 TraceLayer 的 ERROR 行补充路径信息。
    let trace_id = format!("{:016x}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ (d.as_secs() & 0xffffffff))
        .unwrap_or(0));
    tracing::error!(
        trace_id = %trace_id,
        panic_message = "request handler panicked",
        "handler panic caught"
    );
    let body = json!({
        "error": {
            "message": "internal gateway error",
            "type": "internal",
            "code": "internal",
        }
    });
    (StatusCode::INTERNAL_SERVER_ERROR, Json(body)).into_response()
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

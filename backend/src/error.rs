use axum::Json;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use serde::Serialize;
use serde_json::{Value, json};

/// The error codes defined by the OCI Distribution Specification. HTTP status
/// mappings mirror the reference Go implementation (`registry/api/errcode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    Unknown,
    Unsupported,
    Unauthorized,
    Denied,
    Unavailable,
    TooManyRequests,
    DigestInvalid,
    SizeInvalid,
    RangeInvalid,
    NameInvalid,
    TagInvalid,
    NameUnknown,
    ManifestUnknown,
    ManifestInvalid,
    ManifestUnverified,
    ManifestBlobUnknown,
    BlobUnknown,
    BlobUploadUnknown,
    BlobUploadInvalid,
    PaginationNumberInvalid,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Unknown => "UNKNOWN",
            ErrorCode::Unsupported => "UNSUPPORTED",
            ErrorCode::Unauthorized => "UNAUTHORIZED",
            ErrorCode::Denied => "DENIED",
            ErrorCode::Unavailable => "UNAVAILABLE",
            ErrorCode::TooManyRequests => "TOOMANYREQUESTS",
            ErrorCode::DigestInvalid => "DIGEST_INVALID",
            ErrorCode::SizeInvalid => "SIZE_INVALID",
            ErrorCode::RangeInvalid => "RANGE_INVALID",
            ErrorCode::NameInvalid => "NAME_INVALID",
            ErrorCode::TagInvalid => "TAG_INVALID",
            ErrorCode::NameUnknown => "NAME_UNKNOWN",
            ErrorCode::ManifestUnknown => "MANIFEST_UNKNOWN",
            ErrorCode::ManifestInvalid => "MANIFEST_INVALID",
            ErrorCode::ManifestUnverified => "MANIFEST_UNVERIFIED",
            ErrorCode::ManifestBlobUnknown => "MANIFEST_BLOB_UNKNOWN",
            ErrorCode::BlobUnknown => "BLOB_UNKNOWN",
            ErrorCode::BlobUploadUnknown => "BLOB_UPLOAD_UNKNOWN",
            ErrorCode::BlobUploadInvalid => "BLOB_UPLOAD_INVALID",
            ErrorCode::PaginationNumberInvalid => "PAGINATION_NUMBER_INVALID",
        }
    }

    pub fn default_message(self) -> &'static str {
        match self {
            ErrorCode::Unknown => "unknown error",
            ErrorCode::Unsupported => "the operation is unsupported",
            ErrorCode::Unauthorized => "authentication required",
            ErrorCode::Denied => "requested access to the resource is denied",
            ErrorCode::Unavailable => "service unavailable",
            ErrorCode::TooManyRequests => "too many requests",
            ErrorCode::DigestInvalid => "provided digest did not match uploaded content",
            ErrorCode::SizeInvalid => "provided length did not match content length",
            ErrorCode::RangeInvalid => "invalid content range",
            ErrorCode::NameInvalid => "invalid repository name",
            ErrorCode::TagInvalid => "manifest tag did not match URI",
            ErrorCode::NameUnknown => "repository name not known to registry",
            ErrorCode::ManifestUnknown => "manifest unknown",
            ErrorCode::ManifestInvalid => "manifest invalid",
            ErrorCode::ManifestUnverified => "manifest failed signature verification",
            ErrorCode::ManifestBlobUnknown => {
                "manifest references a manifest or blob unknown to registry"
            }
            ErrorCode::BlobUnknown => "blob unknown to registry",
            ErrorCode::BlobUploadUnknown => "blob upload unknown to registry",
            ErrorCode::BlobUploadInvalid => "blob upload invalid",
            ErrorCode::PaginationNumberInvalid => "query parameter `n` is invalid",
        }
    }

    pub fn status(self) -> StatusCode {
        match self {
            ErrorCode::Unknown => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Unsupported => StatusCode::METHOD_NOT_ALLOWED,
            ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrorCode::Denied => StatusCode::FORBIDDEN,
            ErrorCode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RangeInvalid => StatusCode::RANGE_NOT_SATISFIABLE,
            ErrorCode::NameUnknown
            | ErrorCode::ManifestUnknown
            | ErrorCode::BlobUnknown
            | ErrorCode::BlobUploadUnknown
            | ErrorCode::BlobUploadInvalid => StatusCode::NOT_FOUND,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

#[derive(Debug, Serialize)]
struct OciErrorBody {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<Value>,
}

#[derive(Debug, Serialize)]
struct OciErrorEnvelope {
    errors: Vec<OciErrorBody>,
}

/// An error rendered in the OCI error response format:
/// `{"errors":[{"code":"...","message":"...","detail":...}]}`.
#[derive(Debug)]
pub struct RegistryError {
    pub code: ErrorCode,
    pub message: String,
    pub detail: Option<Value>,
    pub status_override: Option<StatusCode>,
}

impl RegistryError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
            status_override: None,
        }
    }

    pub fn code(code: ErrorCode) -> Self {
        Self::new(code, code.default_message())
    }

    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn with_status(mut self, status: StatusCode) -> Self {
        self.status_override = Some(status);
        self
    }

    pub fn status(&self) -> StatusCode {
        self.status_override.unwrap_or_else(|| self.code.status())
    }

    pub fn blob_unknown(digest: &str) -> Self {
        Self::code(ErrorCode::BlobUnknown).with_detail(json!({ "digest": digest }))
    }

    pub fn manifest_unknown(reference: &str) -> Self {
        Self::code(ErrorCode::ManifestUnknown).with_detail(json!({ "reference": reference }))
    }

    pub fn name_unknown(name: &str) -> Self {
        Self::code(ErrorCode::NameUnknown).with_detail(json!({ "name": name }))
    }

    pub fn name_invalid(name: &str) -> Self {
        Self::code(ErrorCode::NameInvalid).with_detail(json!({ "name": name }))
    }

    pub fn digest_invalid(digest: &str) -> Self {
        Self::code(ErrorCode::DigestInvalid).with_detail(json!({ "digest": digest }))
    }

    pub fn manifest_invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ManifestInvalid, message)
    }

    pub fn tag_invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::TagInvalid, message)
    }

    pub fn size_invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::SizeInvalid, message)
    }

    pub fn range_invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::RangeInvalid, message)
    }

    pub fn upload_unknown(uuid: &str) -> Self {
        Self::code(ErrorCode::BlobUploadUnknown).with_detail(json!({ "uuid": uuid }))
    }

    pub fn upload_invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BlobUploadInvalid, message)
    }

    pub fn pagination_invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::PaginationNumberInvalid, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unauthorized, message)
    }

    pub fn denied(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Denied, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unsupported, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unknown, message)
    }
}

impl IntoResponse for RegistryError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = OciErrorEnvelope {
            errors: vec![OciErrorBody {
                code: self.code.as_str(),
                message: self.message,
                detail: self.detail,
            }],
        };
        (status, Json(body)).into_response()
    }
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for RegistryError {}

impl From<sqlx::Error> for RegistryError {
    fn from(err: sqlx::Error) -> Self {
        tracing::error!(error = %err, "database error");
        RegistryError::internal("database error")
    }
}

impl From<std::io::Error> for RegistryError {
    fn from(err: std::io::Error) -> Self {
        tracing::error!(error = %err, "storage error");
        RegistryError::internal("storage error")
    }
}

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct ApiErrorEnvelope {
    error: ApiErrorBody,
}

/// Error type for the control-plane (frontend) API. Serialized as
/// `{"error":{"code":"...","message":"..."}}`.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    pub fn too_many_requests(message: impl Into<String>) -> Self {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ApiErrorEnvelope {
            error: ApiErrorBody {
                code: self.code,
                message: self.message,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        tracing::error!(error = %err, "database error");
        ApiError::internal("database error")
    }
}

impl From<std::io::Error> for ApiError {
    fn from(err: std::io::Error) -> Self {
        tracing::error!(error = %err, "io error");
        ApiError::internal("storage error")
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        tracing::error!(error = ?err, "internal error");
        ApiError::internal("internal error")
    }
}

impl From<RegistryError> for ApiError {
    fn from(err: RegistryError) -> Self {
        ApiError::new(err.status(), err.code.as_str().to_lowercase(), err.message)
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
pub type RegistryResult<T> = Result<T, RegistryError>;

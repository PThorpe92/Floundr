use axum::{
    http::{header::WARNING, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

pub static MAX_BYTES: usize = 4096;
pub static REFERENCE_REGEX: &str = "[a-zA-Z0-9_][a-zA-Z0-9._-]{0,127}";
pub static NAMESPACE_REGEX: &str =
    r"[a-z0-9]+((\.|_|__|-+)[a-z0-9]+)*(\/[a-z0-9]+((\.|_|__|-+)[a-z0-9]+)*)*";

/// Error responses
/// Format spec: 787 - 809
///
/// {
///     "errors": [
///         {
///             "code": "<error identifier, see below>",
///             "message": "<message describing condition>",
///             "detail": "<unstructured>"
///         },
///         ...
///     ]
/// }
///
#[derive(Serialize, Debug, Clone)]
pub struct ErrorResponse<T>
where
    T: Serialize + std::fmt::Debug + Clone,
{
    pub code: Code,
    pub message: String,
    pub detail: T,
}
impl<T> ErrorResponse<T>
where
    T: std::fmt::Debug + Serialize + Clone,
{
    pub fn from_code(code: &Code, detail: T) -> Self {
        Self {
            code: code.clone(),
            message: code.description().to_string(),
            detail,
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub enum Code {
    BlobUnknown,
    BlobUploadInvalid,
    BlobUploadUnknown,
    DigestInvalid,
    ManifestBlobUnknown,
    ManifestInvalid,
    ManifestUnknown,
    NameInvalid,
    NameUnknown,
    SizeInvalid,
    Unauthorized,
    Denied,
    Unsupported,
    TooManyRequests,
}

impl Code {
    pub fn from_code_id(code_id: &str) -> Option<Code> {
        match code_id {
            "code-1" => Some(Code::BlobUnknown),
            "code-2" => Some(Code::BlobUploadInvalid),
            "code-3" => Some(Code::BlobUploadUnknown),
            "code-4" => Some(Code::DigestInvalid),
            "code-5" => Some(Code::ManifestBlobUnknown),
            "code-6" => Some(Code::ManifestInvalid),
            "code-7" => Some(Code::ManifestUnknown),
            "code-8" => Some(Code::NameInvalid),
            "code-9" => Some(Code::NameUnknown),
            "code-10" => Some(Code::SizeInvalid),
            "code-11" => Some(Code::Unauthorized),
            "code-12" => Some(Code::Denied),
            "code-13" => Some(Code::Unsupported),
            "code-14" => Some(Code::TooManyRequests),
            _ => None,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Code::BlobUnknown => "blob unknown to registry",
            Code::BlobUploadInvalid => "blob upload invalid",
            Code::BlobUploadUnknown => "blob upload unknown to registry",
            Code::DigestInvalid => "provided digest did not match uploaded content",
            Code::ManifestBlobUnknown => {
                "manifest references a manifest or blob unknown to registry"
            }
            Code::ManifestInvalid => "manifest invalid",
            Code::ManifestUnknown => "manifest unknown to registry",
            Code::NameInvalid => "invalid repository name",
            Code::NameUnknown => "repository name not known to registry",
            Code::SizeInvalid => "provided length did not match content length",
            Code::Unauthorized => "authentication required",
            Code::Denied => "requested access to the resource is denied",
            Code::Unsupported => "the operation is unsupported",
            Code::TooManyRequests => "too many requests",
        }
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            Code::BlobUnknown => StatusCode::NOT_FOUND,
            Code::BlobUploadInvalid => StatusCode::BAD_REQUEST,
            Code::BlobUploadUnknown => StatusCode::NOT_FOUND,
            Code::DigestInvalid => StatusCode::BAD_REQUEST,
            Code::ManifestBlobUnknown => StatusCode::NOT_FOUND,
            Code::ManifestInvalid => StatusCode::BAD_REQUEST,
            Code::ManifestUnknown => StatusCode::NOT_FOUND,
            Code::NameInvalid => StatusCode::BAD_REQUEST,
            Code::NameUnknown => StatusCode::NOT_FOUND,
            Code::SizeInvalid => StatusCode::BAD_REQUEST,
            Code::Unauthorized => StatusCode::UNAUTHORIZED,
            Code::Denied => StatusCode::FORBIDDEN,
            Code::Unsupported => StatusCode::NOT_IMPLEMENTED,
            Code::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
        }
    }
    /// If included, `Warning` headers MUST specify a `warn-code` of `299` and a `warn-agent` of `-`, and MUST NOT specify a `warn-date` value.
    /// A registry MUST NOT send more than 4096 bytes of warning data from all headers combined.
    /// Example warning headers:
    /// ```
    /// Warning: 299 - "Your auth token will expire in 30 seconds."
    /// Warning: 299 - "This registry endpoint is deprecated and will be removed soon."
    /// Warning: 299 - "This image is deprecated and will be removed soon."
    /// ```
    /// If a client receives `Warning` response headers, it SHOULD report the warnings to the user in an unobtrusive way.
    /// Clients SHOULD deduplicate warnings from multiple associated responses.
    pub fn append_warning_header(&self, map: &mut HeaderMap) {
        map.insert(
            WARNING,
            HeaderValue::from_str(&format!("299 - {}", self.description()))
                .unwrap_or(HeaderValue::from_static(self.description())),
        );
    }
}

impl<T> IntoResponse for ErrorResponse<T>
where
    T: Serialize + std::fmt::Debug + Clone,
{
    fn into_response(self) -> Response {
        let body = Json(self.clone());
        let status = self.code.status_code();
        let mut response = (status, body).into_response();
        self.code.append_warning_header(response.headers_mut());
        response
    }
}

/// Determine whether an incoming request is a browser navigation or an API/fetch call.
///
/// Detection precedence (per spec §Q4):
/// 1. `Sec-Fetch-Mode: navigate` → Navigation
/// 2. `Sec-Fetch-Mode` present with any other value → ApiFetch
/// 3. `Accept` contains `text/html` → Navigation
/// 4. Default (including `Accept: */*`) → ApiFetch
use cli_framework::axum::http::HeaderMap;

#[derive(Debug, PartialEq, Eq)]
pub enum RequestType {
    Navigation,
    ApiFetch,
}

pub fn detect(headers: &HeaderMap) -> RequestType {
    if let Some(sfm) = headers.get("sec-fetch-mode") {
        return if sfm.as_bytes() == b"navigate" {
            RequestType::Navigation
        } else {
            RequestType::ApiFetch
        };
    }

    if let Some(accept) = headers.get("accept") {
        if let Ok(s) = accept.to_str() {
            if s.contains("text/html") {
                return RequestType::Navigation;
            }
        }
    }

    RequestType::ApiFetch
}

/// Validate a `return_to` path before encoding it in the auth-state cookie.
///
/// Accepts path-only URLs starting with `/`. Rejects:
/// - Protocol-relative (`//example.com`)
/// - Backslash tricks (`\evil`)
/// - URL-encoded variants of the above
/// - Control characters (CR, LF, NUL)
/// - Absolute URLs (callers should only pass paths)
pub fn validate_return_to(return_to: &str) -> Result<String, String> {
    if return_to.contains('\r') || return_to.contains('\n') || return_to.contains('\0') {
        return Err("return_to contains control characters".to_string());
    }

    // Reject backslash before the path
    if return_to.starts_with('\\') {
        return Err("return_to starts with backslash".to_string());
    }

    // Reject protocol-relative and absolute URLs
    if return_to.starts_with("//") {
        return Err("return_to is protocol-relative".to_string());
    }

    // Reject URL-encoded forms of the above
    let lower = return_to.to_lowercase();
    if lower.contains("%2f%2f") || lower.contains("%5c") {
        return Err("return_to contains URL-encoded traversal".to_string());
    }

    // Must start with /
    if !return_to.starts_with('/') {
        return Err("return_to must be a path starting with /".to_string());
    }

    Ok(return_to.to_string())
}

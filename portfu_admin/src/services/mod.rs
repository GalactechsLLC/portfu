use http::{header, HeaderValue, StatusCode};
use hyper::body::Bytes;
use portfu::pfcore::services::body::BodyType;
use portfu::pfcore::{IntoStreamBody, ServiceData};

#[cfg(feature = "admin_ui")]
pub mod editor;
#[cfg(feature = "admin_ui")]
pub mod themes;
#[cfg(feature = "admin_ui")]
pub mod users;

pub fn send_internal_error<S: AsRef<str>>(mut data: ServiceData, error: S) -> ServiceData {
    *data.response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    data.response.set_body(BodyType::Stream(
        Bytes::from(error.as_ref().to_string()).stream_body(),
    ));
    data
}

pub fn sanitize_relative_redirect_target(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() || !trimmed.starts_with('/') || trimmed.starts_with("//") {
        return None;
    }
    if trimmed.contains('\\') || trimmed.chars().any(char::is_control) {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn redirect_to_url<S: AsRef<str>>(mut data: ServiceData, url: S) -> ServiceData {
    *data.response.status_mut() = StatusCode::FOUND;
    data.response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(url.as_ref()).unwrap_or(HeaderValue::from_static("/")),
    );
    data
}

#[cfg(test)]
mod tests {
    use super::sanitize_relative_redirect_target;

    #[test]
    fn sanitize_relative_redirect_target_accepts_app_relative_paths() {
        assert_eq!(
            sanitize_relative_redirect_target("/sites/123?tab=content#hero"),
            Some("/sites/123?tab=content#hero".to_string())
        );
    }

    #[test]
    fn sanitize_relative_redirect_target_rejects_external_or_malformed_targets() {
        assert_eq!(
            sanitize_relative_redirect_target("https://evil.example/login"),
            None
        );
        assert_eq!(sanitize_relative_redirect_target("//evil.example"), None);
        assert_eq!(sanitize_relative_redirect_target("/\\evil"), None);
        assert_eq!(sanitize_relative_redirect_target("/safe\nbad"), None);
    }
}

use std::fmt::{Debug, Display, Formatter};

use crate::service::response::ResponseError;
use http::StatusCode;

#[derive(Debug)]
pub enum PortfuError {
    Parsing(String),
    BadRequest(String),
    Unauthorized(String),
    Forbidden(String),
    PayloadTooLarge(String),
    RequestTimeout(String),
    Internal(String),
    Io(std::io::Error),
}

impl Display for PortfuError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PortfuError::Parsing(msg) => write!(f, "{}", msg),
            PortfuError::BadRequest(msg) => write!(f, "{}", msg),
            PortfuError::Unauthorized(msg) => write!(f, "{}", msg),
            PortfuError::Forbidden(msg) => write!(f, "{}", msg),
            PortfuError::PayloadTooLarge(msg) => write!(f, "{}", msg),
            PortfuError::RequestTimeout(msg) => write!(f, "{}", msg),
            PortfuError::Internal(msg) => write!(f, "{}", msg),
            PortfuError::Io(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for PortfuError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PortfuError::Parsing(_) => None,
            PortfuError::BadRequest(_) => None,
            PortfuError::Unauthorized(_) => None,
            PortfuError::Forbidden(_) => None,
            PortfuError::PayloadTooLarge(_) => None,
            PortfuError::RequestTimeout(_) => None,
            PortfuError::Internal(_) => None,
            PortfuError::Io(e) => Some(e),
        }
    }
}

impl ResponseError for PortfuError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::Parsing(_) | Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Self::RequestTimeout(_) => StatusCode::REQUEST_TIMEOUT,
            Self::Internal(_) | Self::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PortfuError;
    use crate::service::response::IntoResponse;
    use http::StatusCode;

    #[test]
    fn response_errors_map_to_specific_http_statuses() {
        let cases = [
            (PortfuError::Parsing("json".into()), StatusCode::BAD_REQUEST),
            (
                PortfuError::Unauthorized("missing".into()),
                StatusCode::UNAUTHORIZED,
            ),
            (
                PortfuError::Forbidden("wrong trust".into()),
                StatusCode::FORBIDDEN,
            ),
            (
                PortfuError::PayloadTooLarge("large".into()),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
            (
                PortfuError::RequestTimeout("slow".into()),
                StatusCode::REQUEST_TIMEOUT,
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.into_response().status(), expected);
        }
    }
}

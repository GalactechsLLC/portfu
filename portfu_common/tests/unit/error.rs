use crate::error::PortfuError;
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

#[test]
fn internal_error_details_are_not_exposed_to_clients() {
    let response: http::Response<_> =
        PortfuError::Internal("database password appeared here".into())
            .into_response()
            .into();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .unwrap(),
        "21"
    );
}

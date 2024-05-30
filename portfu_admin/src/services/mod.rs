use std::io::Error;
use http::{header, HeaderValue, StatusCode};
use hyper::body::Bytes;
use portfu::pfcore::{IntoStreamBody, ServiceData};

pub mod users;
pub mod editor;
pub mod themes;

pub fn send_internal_error(
    mut data: ServiceData,
    error: String,
) -> Result<ServiceData, (ServiceData, Error)> {
    *data.response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    *data.response.body_mut() = Bytes::from(error).stream_body();
    Ok(data)
}

pub fn redirect_to_url(
    mut data: ServiceData,
    url: String,
) -> Result<ServiceData, (ServiceData, Error)> {
    *data.response.status_mut() = StatusCode::FOUND;
    data.response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(&url).unwrap_or(HeaderValue::from_static("/")),
    );
    Ok(data)
}

use http::{header, HeaderValue, StatusCode};
use hyper::body::Bytes;
use pfcore::{IntoStreamBody, ServiceData};
use std::io::Error;

pub mod oauth_login;

pub fn send_internal_error(mut data: ServiceData, error: String) -> Result<ServiceData, Error> {
    *data.response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    *data.response.body_mut() = Bytes::from(error).stream_body();
    Ok(data)
}

pub fn redirect_to_url(mut data: ServiceData, url: String) -> Result<ServiceData, Error> {
    *data.response.status_mut() = StatusCode::FOUND;
    data.response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(&url).unwrap_or(HeaderValue::from_static("/")),
    );
    Ok(data)
}

use std::io::Error;
use http::{header, HeaderValue, Response, StatusCode};
use http_body_util::Full;
use hyper::body::Bytes;

pub mod oauth_login;


pub fn send_internal_error(
    response: &mut Response<Full<Bytes>>,
    error: String,
) -> Result<(), Error> {
    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    *response.body_mut() = Full::new(Bytes::from(error));
    Ok(())
}

pub fn redirect_to_url(response: &mut Response<Full<Bytes>>, url: String) -> Result<(), Error> {
    *response.status_mut() = StatusCode::FOUND;
    response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(&url).unwrap_or(HeaderValue::from_static("/")),
    );
    Ok(())
}
use http::{header, HeaderValue, StatusCode};
use hyper::body::Bytes;
use portfu::pfcore::service::BodyType;
use std::io::Error;
use portfu::pfcore::{IntoStreamBody, ServiceData};

pub mod editor;
pub mod themes;
pub mod users;

pub fn send_internal_error<S: AsRef<str>>(
    mut data: ServiceData,
    error: S,
) -> Result<ServiceData, (ServiceData, Error)> {
    *data.response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    data.response.set_body(BodyType::Stream(
        Bytes::from(error.as_ref().to_string()).stream_body(),
    ));
    Ok(data)
}

pub fn redirect_to_url<S: AsRef<str>>(
    mut data: ServiceData,
    url: S,
) -> Result<ServiceData, (ServiceData, Error)> {
    *data.response.status_mut() = StatusCode::FOUND;
    data.response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(url.as_ref()).unwrap_or(HeaderValue::from_static("/")),
    );
    Ok(data)
}

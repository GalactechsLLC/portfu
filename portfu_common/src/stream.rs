use crate::service::StreamingBody;
use http_body_util::{BodyExt, BodyStream, Full, StreamBody};
use hyper::body::{Bytes, Incoming};

pub trait IntoStreamBody {
    type Data;
    type Error;
    fn stream_body(self) -> StreamingBody;
}

impl IntoStreamBody for Bytes {
    type Data = Bytes;
    type Error = &'static str;
    fn stream_body(self) -> StreamingBody {
        StreamBody::new(BodyStream::new(Box::pin(
            Full::new(self).map_err(|_| "Failed to Convert Bytes into ServiceBody"),
        )))
    }
}

impl IntoStreamBody for String {
    type Data = Bytes;
    type Error = &'static str;
    fn stream_body(self) -> StreamingBody {
        StreamBody::new(BodyStream::new(Box::pin(
            Full::new(Bytes::from(self)).map_err(|_| "Failed to Convert Bytes into ServiceBody"),
        )))
    }
}

impl IntoStreamBody for &str {
    type Data = Bytes;
    type Error = &'static str;
    fn stream_body(self) -> StreamingBody {
        StreamBody::new(BodyStream::new(Box::pin(
            Full::new(Bytes::from(self.to_string()))
                .map_err(|_| "Failed to Convert Bytes into ServiceBody"),
        )))
    }
}

impl IntoStreamBody for Vec<u8> {
    type Data = Bytes;
    type Error = &'static str;
    fn stream_body(self) -> StreamingBody {
        Bytes::from(self).stream_body()
    }
}

impl IntoStreamBody for Full<Bytes> {
    type Data = Bytes;
    type Error = &'static str;

    fn stream_body(self) -> StreamingBody {
        StreamBody::new(BodyStream::new(Box::pin(
            self.map_err(|_| "Failed to Convert Bytes into ServiceBody"),
        )))
    }
}

impl IntoStreamBody for Incoming {
    type Data = Bytes;
    type Error = &'static str;

    fn stream_body(self) -> StreamingBody {
        StreamBody::new(BodyStream::new(Box::pin(
            self.map_err(|_| "Failed to Convert Incoming into ServiceBody"),
        )))
    }
}

#[cfg(test)]
#[path = "../tests/unit/stream.rs"]
mod tests;

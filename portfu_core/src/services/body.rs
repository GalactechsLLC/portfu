use crate::StreamingBody;
use futures_util::TryStreamExt;
use http_body::{Body, Frame};
use http_body_util::{BodyStream, Full, StreamBody};
use hyper::body::Bytes;
use std::pin::Pin;
use std::task::{Context, Poll};

pub enum RefBodyType<'a> {
    Stream(&'a mut StreamingBody),
    Sized(&'a mut Full<Bytes>),
    Empty,
}

pub enum BodyType {
    Stream(StreamingBody),
    Sized(Full<Bytes>),
    Empty,
}

pub trait MutBody {
    fn consume(&mut self) -> BodyType;
    fn set_body(&mut self, body: BodyType);
}

impl Body for BodyType {
    type Data = Bytes;
    type Error = String;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            BodyType::Stream(s) => Pin::new(s)
                .poll_frame(cx)
                .map_err(|e| format!("Failed to Read from Stream Body: {e:?}")),
            BodyType::Sized(s) => Pin::new(s).poll_frame(cx).map_err(|_| {
                String::new() //Should Never Happen, e in infallible
            }),
            BodyType::Empty => Poll::Ready(None),
        }
    }
}

impl From<BodyType> for reqwest::Body {
    fn from(value: BodyType) -> reqwest::Body {
        match value {
            BodyType::Stream(value) => {
                let body_stream = BodyStream::new(value);
                let body_stream = body_stream.map_ok(|d| d.into_data().unwrap());
                let body = StreamBody::new(body_stream);
                reqwest::Body::wrap_stream(body)
            }
            BodyType::Sized(value) => {
                let body_stream = BodyStream::new(value);
                let body_stream = body_stream.map_ok(|d| d.into_data().unwrap());
                let body = StreamBody::new(body_stream);
                reqwest::Body::wrap_stream(body)
            }
            BodyType::Empty => reqwest::Body::default(),
        }
    }
}

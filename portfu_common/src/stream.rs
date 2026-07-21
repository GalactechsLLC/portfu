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
mod tests {
    use super::IntoStreamBody;
    use http_body_util::{BodyExt, Full};
    use hyper::body::Bytes;

    #[tokio::test]
    async fn stream_body_conversions_preserve_payload_bytes() {
        assert_eq!(
            collect(Bytes::from_static(b"bytes").stream_body())
                .await
                .as_ref(),
            b"bytes"
        );
        assert_eq!(collect("str".stream_body()).await.as_ref(), b"str");
        assert_eq!(
            collect("string".to_string().stream_body()).await.as_ref(),
            b"string"
        );
        assert_eq!(
            collect(vec![1_u8, 2, 3].stream_body()).await.as_ref(),
            &[1, 2, 3]
        );
        assert_eq!(
            collect(Full::new(Bytes::from_static(b"full")).stream_body())
                .await
                .as_ref(),
            b"full"
        );
    }

    async fn collect(body: crate::service::StreamingBody) -> Bytes {
        BodyExt::collect(body)
            .await
            .expect("stream body should collect")
            .to_bytes()
    }
}

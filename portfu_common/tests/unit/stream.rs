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

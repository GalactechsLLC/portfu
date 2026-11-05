use crate::client::{SupportedBody, build_request, default_port, new_websocket, send_request};
use http::{HeaderMap, HeaderValue, Method, Uri};
use http_body::Body;
use http_body_util::{BodyExt, BodyStream, Empty, Full, StreamBody};
use hyper::body::Bytes;
use portfu_common::service::PinnedBody;

#[tokio::test]
async fn supported_body_variants_preserve_size_hints_and_payloads() {
    assert!(matches!(SupportedBody::from(()), SupportedBody::Empty(_)));

    let empty = SupportedBody::from(Empty::<Bytes>::new());
    assert_eq!(empty.size_hint().exact(), Some(0));
    assert_eq!(collect(empty).await.as_ref(), b"");

    let full = SupportedBody::from(Bytes::from_static(b"bytes"));
    assert_eq!(full.size_hint().exact(), Some(5));
    assert_eq!(collect(full).await.as_ref(), b"bytes");

    assert_eq!(
        collect(SupportedBody::from(vec![1_u8, 2, 3]))
            .await
            .as_ref(),
        &[1, 2, 3]
    );
    assert_eq!(collect(SupportedBody::from("str")).await.as_ref(), b"str");
    assert_eq!(
        collect(SupportedBody::from("string".to_string()))
            .await
            .as_ref(),
        b"string"
    );
    assert_eq!(
        collect(SupportedBody::from(Full::new(Bytes::from_static(b"full"))))
            .await
            .as_ref(),
        b"full"
    );

    let body: PinnedBody =
        Box::pin(Full::new(Bytes::from_static(b"stream")).map_err(|_| "stream should not fail"));
    let stream = StreamBody::new(BodyStream::new(body));
    assert_eq!(
        collect(SupportedBody::from(stream)).await.as_ref(),
        b"stream"
    );
}

#[test]
fn build_request_sets_host_path_headers_and_body() {
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", HeaderValue::from_static("secret"));
    let request = build_request(
        Method::POST,
        "/submit?ok=1",
        "example.test:8080",
        headers,
        SupportedBody::from("payload"),
    )
    .expect("request should build");

    assert_eq!(request.method(), Method::POST);
    assert_eq!(request.uri(), "/submit?ok=1");
    assert_eq!(
        request.headers().get(http::header::HOST).unwrap(),
        "example.test:8080"
    );
    assert_eq!(request.headers().get("x-api-key").unwrap(), "secret");
    assert_eq!(request.body().size_hint().exact(), Some(7));
}

#[tokio::test]
async fn client_validation_errors_are_reported_before_network_use() {
    let no_host: Uri = "/local-only".parse().expect("uri parse failed");
    let err = send_request(Method::GET, no_host, ()).await.unwrap_err();
    assert!(err.to_string().contains("uri has no host"));

    let err = match new_websocket("not a websocket url", None).await {
        Ok(_) => panic!("invalid websocket URL should fail"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("failed to build websocket request")
    );
}

#[test]
fn default_ports_follow_http_and_tls_conventions() {
    assert_eq!(default_port("http"), 80);
    assert_eq!(default_port("HTTP"), 80);
    assert_eq!(default_port("https"), 443);
    assert_eq!(default_port("wss"), 443);
}

async fn collect(body: SupportedBody) -> Bytes {
    BodyExt::collect(body)
        .await
        .expect("body should collect")
        .to_bytes()
}

use crate::stream::IntoStreamBody;
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

use crate::error::PortfuError;
use crate::router::route::Route;
use crate::service::{
    PinnedBody,
    request::{FromRequest, Request, RequestBody, RequestType},
};
use http_body_util::{BodyStream, StreamBody};
use hyper::body::Frame;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Poll};
use std::time::Duration;

struct CountedBody {
    frames: std::collections::VecDeque<Bytes>,
    polls: Arc<AtomicUsize>,
    stalled: bool,
}
impl http_body::Body for CountedBody {
    type Data = Bytes;
    type Error = &'static str;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, &'static str>>> {
        self.polls.fetch_add(1, Ordering::Relaxed);
        if self.stalled {
            Poll::Pending
        } else {
            Poll::Ready(self.frames.pop_front().map(|bytes| Ok(Frame::data(bytes))))
        }
    }
}
fn counted_request(frames: &[&'static [u8]], stalled: bool) -> (Request, Arc<AtomicUsize>) {
    let polls = Arc::new(AtomicUsize::new(0));
    let body: PinnedBody = Box::pin(CountedBody {
        frames: frames
            .iter()
            .map(|bytes| Bytes::from_static(bytes))
            .collect(),
        polls: polls.clone(),
        stalled,
    });
    let raw = http::Request::new(StreamBody::new(BodyStream::new(body)));
    (
        Request::new(RequestType::Stream(raw), Arc::new(Route::new("/".into()))),
        polls,
    )
}

#[tokio::test]
async fn streaming_extractor_is_lazy_and_enforces_limits_on_unknown_length_bodies() {
    let (mut request, polls) = counted_request(&[b"abc", b"def"], false);
    request.limit_body(4, Duration::from_secs(1)).unwrap();
    assert_eq!(polls.load(Ordering::Relaxed), 0);
    let mut body = <RequestBody as FromRequest<Request>>::try_from(&mut request)
        .await
        .unwrap();
    assert_eq!(polls.load(Ordering::Relaxed), 0);
    assert_eq!(
        body.frame().await.unwrap().unwrap().into_data().unwrap(),
        "abc"
    );
    assert!(matches!(
        body.frame().await.unwrap(),
        Err(PortfuError::PayloadTooLarge(_))
    ));
    assert!(body.frame().await.is_none());
    assert_eq!(polls.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn known_oversized_bodies_are_rejected_without_reading() {
    let (mut request, polls) = counted_request(&[b"abc"], false);
    request
        .headers_mut()
        .insert(http::header::CONTENT_LENGTH, "999999999".parse().unwrap());
    assert!(matches!(
        request.take_body(),
        Err(PortfuError::PayloadTooLarge(_))
    ));
    assert_eq!(polls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn buffered_extractors_have_a_default_cap_and_preserve_metadata_on_failure() {
    let (mut request, polls) = counted_request(&[b"abc", b"def"], false);
    request.insert(42_u64);
    request.limit_body(4, Duration::from_secs(1)).unwrap();
    assert!(matches!(
        request.consume_body_bytes().await,
        Err(PortfuError::PayloadTooLarge(_))
    ));
    assert_eq!(polls.load(Ordering::Relaxed), 2);
    assert_eq!(request.get::<u64>(), Some(&42));
    let raw = http::Request::new(Full::new(Bytes::from(vec![0; 1024 * 1024 + 1])));
    let mut request = Request::new(RequestType::Sized(raw), Arc::new(Route::new("/".into())));
    assert!(matches!(
        request.consume_body_bytes().await,
        Err(PortfuError::PayloadTooLarge(_))
    ));
}

#[tokio::test]
async fn stalled_streams_time_out_without_buffering() {
    let (mut request, _) = counted_request(&[], true);
    request.limit_body(4, Duration::from_millis(1)).unwrap();
    assert!(matches!(
        request.consume_body_bytes().await,
        Err(PortfuError::RequestTimeout(_))
    ));
}

#[cfg(feature = "rate-limit")]
#[tokio::test]
async fn rate_limiter_never_reads_a_stream_before_authentication() {
    use crate::router::middleware::Middleware;
    let (mut request, polls) = counted_request(&[b"abc", b"def"], false);
    crate::wrappers::rate_limits::RateLimiter::default()
        .request_size_limit(4)
        .before(&mut request)
        .await
        .unwrap();
    assert_eq!(polls.load(Ordering::Relaxed), 0);
    assert!(matches!(
        request.consume_body_bytes().await,
        Err(PortfuError::PayloadTooLarge(_))
    ));
}

#[tokio::test]
async fn raw_stream_access_cannot_bypass_middleware_limits() {
    let (mut request, polls) = counted_request(&[b"abc", b"def"], false);
    request.limit_body(4, Duration::from_secs(1)).unwrap();
    let RequestType::Stream(raw) = request.request_type() else {
        panic!("expected stream")
    };
    assert_eq!(
        raw.body_mut()
            .frame()
            .await
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap(),
        "abc"
    );
    assert_eq!(
        raw.body_mut().frame().await.unwrap().unwrap_err(),
        "portfu:payload-too-large"
    );
    assert_eq!(polls.load(Ordering::Relaxed), 2);
}

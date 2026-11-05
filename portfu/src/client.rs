use futures_util::FutureExt;
use http::{HeaderMap, Method, Request, Response, Uri};
use http_body::Body;
use http_body_util::{BodyStream, Empty, Full, StreamBody};
use hyper::body::{Bytes, Frame, Incoming, SizeHint};
use hyper_util::rt::tokio::TokioIo;
use portfu_common::service::PinnedBody;
use portfu_common::websocket::ClientWebSocket;
use rustls::RootCertStore;
use rustls::pki_types::ServerName;
use std::io::Error;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

pub enum SupportedBody {
    Empty(Empty<Bytes>),
    Full(Full<Bytes>),
    Stream(StreamBody<BodyStream<PinnedBody>>),
}

impl Body for SupportedBody {
    type Data = Bytes;
    type Error = &'static str;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.get_mut() {
            SupportedBody::Empty(b) => Pin::new(b)
                .poll_frame(cx)
                .map_err(|_| "failed to poll empty body"),
            SupportedBody::Full(b) => Pin::new(b)
                .poll_frame(cx)
                .map_err(|_| "failed to poll full body"),
            SupportedBody::Stream(b) => Pin::new(b)
                .poll_frame(cx)
                .map_err(|_| "failed to poll stream body"),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            SupportedBody::Empty(b) => Pin::new(b).is_end_stream(),
            SupportedBody::Full(b) => Pin::new(b).is_end_stream(),
            SupportedBody::Stream(b) => Pin::new(b).is_end_stream(),
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            SupportedBody::Empty(b) => Pin::new(b).size_hint(),
            SupportedBody::Full(b) => Pin::new(b).size_hint(),
            SupportedBody::Stream(b) => Body::size_hint(b),
        }
    }
}

impl From<()> for SupportedBody {
    fn from(_: ()) -> Self {
        Self::Empty(Empty::default())
    }
}

impl From<Bytes> for SupportedBody {
    fn from(value: Bytes) -> Self {
        Self::Full(Full::new(value))
    }
}

impl From<Vec<u8>> for SupportedBody {
    fn from(value: Vec<u8>) -> Self {
        Self::Full(Full::new(Bytes::from(value)))
    }
}

impl From<String> for SupportedBody {
    fn from(value: String) -> Self {
        Self::Full(Full::new(Bytes::from(value)))
    }
}

impl From<&str> for SupportedBody {
    fn from(value: &str) -> Self {
        Self::Full(Full::new(Bytes::from(value.to_string())))
    }
}

impl From<Full<Bytes>> for SupportedBody {
    fn from(value: Full<Bytes>) -> Self {
        Self::Full(value)
    }
}

impl From<Empty<Bytes>> for SupportedBody {
    fn from(value: Empty<Bytes>) -> Self {
        Self::Empty(value)
    }
}

impl From<StreamBody<BodyStream<PinnedBody>>> for SupportedBody {
    fn from(value: StreamBody<BodyStream<PinnedBody>>) -> Self {
        Self::Stream(value)
    }
}

pub async fn get<T: Into<SupportedBody>>(
    url: Uri,
    body: T,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    send_request(Method::GET, url, body).await
}

pub async fn post<T: Into<SupportedBody>>(
    url: Uri,
    body: T,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    send_request(Method::POST, url, body).await
}

pub async fn put<T: Into<SupportedBody>>(
    url: Uri,
    body: T,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    send_request(Method::PUT, url, body).await
}

pub async fn patch<T: Into<SupportedBody>>(
    url: Uri,
    body: T,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    send_request(Method::PATCH, url, body).await
}

pub async fn delete<T: Into<SupportedBody>>(
    url: Uri,
    body: T,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    send_request(Method::DELETE, url, body).await
}

pub async fn head<T: Into<SupportedBody>>(
    url: Uri,
    body: T,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    send_request(Method::HEAD, url, body).await
}

pub async fn options<T: Into<SupportedBody>>(
    url: Uri,
    body: T,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    send_request(Method::OPTIONS, url, body).await
}

pub async fn trace<T: Into<SupportedBody>>(
    url: Uri,
    body: T,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    send_request(Method::TRACE, url, body).await
}

pub async fn connect<T: Into<SupportedBody>>(
    url: Uri,
    body: T,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    send_request(Method::CONNECT, url, body).await
}

pub async fn send_request<T: Into<SupportedBody>>(
    method: Method,
    url: Uri,
    body: T,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    send_request_with_headers(method, url, HeaderMap::new(), body).await
}

pub async fn send_request_with_headers<T: Into<SupportedBody>>(
    method: Method,
    url: Uri,
    headers: HeaderMap,
    body: T,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    let body = body.into();
    let host = url
        .host()
        .ok_or_else(|| Error::other("uri has no host"))?
        .to_string();
    let scheme = url.scheme_str().unwrap_or("https");
    let port = url.port_u16().unwrap_or_else(|| default_port(scheme));
    let addr = format!("{host}:{port}");
    let path = url.path_and_query().map(|v| v.as_str()).unwrap_or("/");
    let host_header = if let Some(port) = url.port_u16() {
        format!("{host}:{port}")
    } else {
        host.clone()
    };
    let request = build_request(method, path, host_header.as_str(), headers, body)?;

    if scheme.eq_ignore_ascii_case("https") {
        let mut root_cert_store = RootCertStore::empty();
        root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_cert_store)
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(config));
        let dns_name = ServerName::try_from(host)
            .map_err(|e| Error::other(format!("invalid dns name: {e}")))?;
        let stream = TcpStream::connect(&addr).await?;
        let stream = connector.connect(dns_name, stream).await?;
        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        Ok(sender.send_request(request).await?)
    } else {
        let stream = TcpStream::connect(&addr).await?;
        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::spawn(async move {
            let _ = conn.await;
        });
        Ok(sender.send_request(request).await?)
    }
}

pub async fn new_websocket(
    url: &str,
    headers: Option<HeaderMap>,
) -> Result<ClientWebSocket, Error> {
    let mut request = url
        .into_client_request()
        .map_err(|e| Error::other(format!("failed to build websocket request: {e}")))?;
    if let Some(headers) = headers {
        request.headers_mut().extend(headers);
    }
    let (ws_stream, _response) = connect_async(request)
        .map(|result| result.map_err(|e| Error::other(format!("websocket connect failed: {e}"))))
        .await?;
    Ok(ClientWebSocket::new(ws_stream))
}

pub(crate) fn build_request(
    method: Method,
    path: &str,
    host: &str,
    headers: HeaderMap,
    body: SupportedBody,
) -> Result<Request<SupportedBody>, Box<dyn std::error::Error + Send + Sync>> {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(http::header::HOST, host)
        .body(body)?;
    req.headers_mut().extend(headers);
    Ok(req)
}

pub(crate) fn default_port(scheme: &str) -> u16 {
    if scheme.eq_ignore_ascii_case("http") {
        80
    } else {
        443
    }
}

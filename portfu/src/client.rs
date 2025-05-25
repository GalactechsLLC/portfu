use crate::prelude::{WebSocket, WebsocketConnection, WebsocketMsgStream};
use http::{HeaderMap, Method, Request, Response, Uri};
use http_body_util::{BodyStream, Empty, Full, StreamBody};
use hyper::body::{Body, Bytes, Frame, Incoming, SizeHint};
use log::{debug, error};
use pfcore::service::BodyType;
use pfcore::PinnedBody;
use rustls::client::ClientConfig;
use rustls::pki_types::ServerName;
use rustls::RootCertStore;
use std::io::{Error, ErrorKind};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

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
                .map_err(|_| "Failed to Poll Empty"),
            SupportedBody::Full(b) => Pin::new(b)
                .poll_frame(cx)
                .map_err(|_| "Failed to Poll Full"),
            SupportedBody::Stream(b) => Pin::new(b)
                .poll_frame(cx)
                .map_err(|_| "Failed to Poll Incoming"),
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
impl From<Empty<Bytes>> for SupportedBody {
    fn from(value: Empty<Bytes>) -> Self {
        SupportedBody::Empty(value)
    }
}
impl From<Full<Bytes>> for SupportedBody {
    fn from(value: Full<Bytes>) -> Self {
        SupportedBody::Full(value)
    }
}
impl From<StreamBody<BodyStream<PinnedBody>>> for SupportedBody {
    fn from(value: StreamBody<BodyStream<PinnedBody>>) -> Self {
        SupportedBody::Stream(value)
    }
}
impl From<BodyType> for SupportedBody {
    fn from(value: BodyType) -> Self {
        match value {
            BodyType::Stream(value) => SupportedBody::Stream(value),
            BodyType::Sized(value) => SupportedBody::Full(value),
            BodyType::Empty => SupportedBody::Empty(Empty::default()),
        }
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

pub async fn send_request<T: Into<SupportedBody>>(
    method: Method,
    url: Uri,
    body: T,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    let body = body.into();
    let host = url.host().expect("uri has no host").to_string();
    let port = url.port_u16().unwrap_or(80);
    let addr = format!("{}:{}", host, port);
    let mut root_cert_store = RootCertStore::empty();
    root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(root_cert_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let dnsname = ServerName::try_from(host)?;
    let inner_stream = TcpStream::connect(&addr).await?;
    let stream = connector.connect(dnsname, inner_stream).await?;
    let io = ::hyper_util::rt::tokio::TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::task::spawn(async move {
        if let Err(err) = conn.await {
            error!("Connection failed: {:?}", err);
        }
    });
    let path = url.path();
    let req = Request::builder().method(method).uri(path).body(body)?;
    Ok(sender.send_request(req).await?)
}

pub async fn new_websocket(url: &str, headers: Option<HeaderMap>) -> Result<WebSocket, Error> {
    debug!("Starting Websocket Connection to: {}", url);
    let mut request = url
        .into_client_request()
        .map_err(|e| Error::new(ErrorKind::Other, format!("{:?}", e)))?;
    if let Some(headers) = headers {
        request.headers_mut().extend(headers.into_iter())
    }
    let (ws_stream, response) = match connect_async(request).await {
        Ok(result) => result,
        Err(e) => return Err(Error::new(ErrorKind::Other, format!("{:?}", e))),
    };
    debug!("Connected with HTTP status: {}", response.status());
    Ok(WebSocket::new(
        WebsocketConnection::new(WebsocketMsgStream::Tls(ws_stream)),
        Arc::new(Uuid::new_v4()),
    ))
}

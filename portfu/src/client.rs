use http::{Method, Request, Response, Uri};
use http_body_util::{BodyStream, Empty, Full, StreamBody};
use hyper::body::{Body, Bytes, Frame, Incoming, SizeHint};
use log::error;
use pfcore::service::ConsumedBodyType;
use rustls::client::ClientConfig;
use rustls::pki_types::ServerName;
use rustls::RootCertStore;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

pub enum SupportedBody {
    Empty(Empty<Bytes>),
    Full(Full<Bytes>),
    Incoming(StreamBody<BodyStream<Incoming>>),
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
            SupportedBody::Incoming(b) => Pin::new(b)
                .poll_frame(cx)
                .map_err(|_| "Failed to Poll Incoming"),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            SupportedBody::Empty(b) => Pin::new(b).is_end_stream(),
            SupportedBody::Full(b) => Pin::new(b).is_end_stream(),
            SupportedBody::Incoming(b) => Pin::new(b).is_end_stream(),
        }
    }

    fn size_hint(&self) -> SizeHint {
        match self {
            SupportedBody::Empty(b) => Pin::new(b).size_hint(),
            SupportedBody::Full(b) => Pin::new(b).size_hint(),
            SupportedBody::Incoming(b) => Body::size_hint(b),
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
impl From<StreamBody<BodyStream<Incoming>>> for SupportedBody {
    fn from(value: StreamBody<BodyStream<Incoming>>) -> Self {
        SupportedBody::Incoming(value)
    }
}
impl From<ConsumedBodyType> for SupportedBody {
    fn from(value: ConsumedBodyType) -> Self {
        match value {
            ConsumedBodyType::Stream(value) => {
                let body_stream = BodyStream::new(value);
                let body = StreamBody::new(body_stream);
                SupportedBody::Incoming(body)
            }
            ConsumedBodyType::Sized(value) => SupportedBody::Full(value),
            ConsumedBodyType::Empty => SupportedBody::Empty(Empty::default()),
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

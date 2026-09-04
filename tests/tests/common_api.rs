use http::Method;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use portfu::prelude::{PreEscaped, Render, get, html, inventory, maud_http, post};
use portfu_common::auth::oauth::{
    OAUTH, OAuthIdentity, OAuthToken, SessionOAuthIdentity, SessionOAuthToken,
};
use portfu_common::error::PortfuError;
use portfu_common::router::filter::{self, FilterResult, traits::Filter as FilterTrait};
use portfu_common::router::middleware::{Middleware, MiddlewareResult};
use portfu_common::router::path::{Path, PathImpl, PathName};
use portfu_common::router::route::Route;
use portfu_common::server::builder::ServerBuilder;
use portfu_common::server::connection::ClientIdentity;
use portfu_common::service::State;
use portfu_common::service::builder::ServiceBuilder;
use portfu_common::service::group::ServiceGroup;
use portfu_common::service::request::{Body, FromRequest, Json, Query, Request, RequestType};
use portfu_common::service::response::{JsonResponse, Response, ResponseError, Serialized};
use portfu_common::service::traits::Service as ServiceTrait;
use portfu_common::wrappers::cors::Cors;
use portfu_common::wrappers::metrics::MetricsWrapper;
use portfu_common::wrappers::rate_limits::{RateLimit, RateLimiter};
use portfu_common::wrappers::sessions::{
    Session, SessionManager, SessionState, get_session_cookie_from_request,
    get_session_from_request,
};
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

#[derive(Clone)]
struct OkService;

impl ServiceTrait for OkService {
    fn name(&self) -> &str {
        "ok-service"
    }

    fn serve<'a>(
        &'a self,
        _data: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Response, PortfuError>> + 'a + Send>> {
        Box::pin(async move { Ok(Response::ok("ok")) })
    }
}

#[derive(Clone)]
struct Config {
    name: String,
}

#[derive(Clone)]
struct Client;

struct GroupStateService;

impl ServiceTrait for GroupStateService {
    fn name(&self) -> &str {
        "group-state-service"
    }

    fn serve<'a>(
        &'a self,
        request: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Response, PortfuError>> + 'a + Send>> {
        Box::pin(async move {
            let config = <State<Config> as FromRequest<Request>>::try_from(request).await?;
            let _client = <State<Client> as FromRequest<Request>>::try_from(request).await?;
            let connections =
                <State<RwLock<HashMap<String, usize>>> as FromRequest<Request>>::try_from(request)
                    .await?;
            assert_eq!(config.name, "julia-web-sdk");
            assert_eq!(connections.read().await.len(), 1);
            Ok(Response::ok("group state available"))
        })
    }
}

struct StaticFilter {
    name: &'static str,
    allow: bool,
}

impl FilterTrait for StaticFilter {
    fn name(&self) -> &str {
        self.name
    }

    fn filter<'a>(
        &'a self,
        _request: &'a Request,
    ) -> Pin<Box<dyn Future<Output = FilterResult> + 'a + Send + Sync>> {
        Box::pin(async move { self.allow.into() })
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct QueryPayload {
    name: String,
    count: u32,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct JsonPayload {
    id: u64,
    title: String,
}

#[derive(Debug, Serialize)]
struct MarkerJson {
    id: u64,
}

impl Serialized for MarkerJson {}

#[derive(Debug, Serialize)]
struct PlainJson {
    id: u64,
    name: String,
}

struct BrokenJson;

impl Serialize for BrokenJson {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(serde::ser::Error::custom("broken json"))
    }
}

#[derive(Debug)]
struct TeapotError;

impl Display for TeapotError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("short and stout")
    }
}

impl Error for TeapotError {}

impl ResponseError for TeapotError {
    fn status_code(&self) -> http::StatusCode {
        http::StatusCode::IM_A_TEAPOT
    }
}

#[get("/typed-json", name = "typed-json")]
async fn typed_json_endpoint() -> Result<Vec<PlainJson>, PortfuError> {
    Ok(vec![PlainJson {
        id: 7,
        name: "seven".to_string(),
    }])
}

#[get("/broken-json", name = "broken-json")]
async fn broken_json_endpoint() -> Result<BrokenJson, PortfuError> {
    Ok(BrokenJson)
}

#[get("/handler-error", name = "handler-error")]
async fn handler_error_endpoint() -> Result<String, TeapotError> {
    Err(TeapotError)
}

#[post("/extract-json", name = "extract-json")]
async fn extract_json_endpoint(payload: Json<JsonPayload>) -> Result<String, PortfuError> {
    Ok(payload.into_inner().title)
}

#[post(
    "/trusted-endpoint",
    name = "trusted-endpoint",
    client_trust = "internal-clients"
)]
async fn trusted_endpoint() -> Result<String, PortfuError> {
    Ok("trusted".to_string())
}

#[maud_http("/maud/", "/maud/index.html", name = "maud-index")]
#[derive(Clone, Debug, Default)]
struct MaudIndexPage;

impl Render for MaudIndexPage {
    fn render(&self) -> PreEscaped<String> {
        html! {
            h1 { "Hello from Maud" }
        }
    }
}

#[maud_http("/maud/users/{id}", "/maud/organizations/{id}", name = "maud-user")]
#[derive(Clone, Debug, Default)]
struct MaudUserPage {
    id: String,
}

impl Render for MaudUserPage {
    fn render(&self) -> PreEscaped<String> {
        html! {
            p { (self.id.as_str()) }
        }
    }
}

#[maud_http(
    "/maud/blocked",
    name = "maud-blocked",
    filter = Arc::new(StaticFilter {
        name: "maud-block",
        allow: false
    })
)]
#[derive(Clone, Debug, Default)]
struct MaudBlockedPage;

impl Render for MaudBlockedPage {
    fn render(&self) -> PreEscaped<String> {
        html! {
            p { "blocked" }
        }
    }
}

#[maud_http(
    "/maud/post",
    name = "maud-post",
    scope = "maud-scope",
    domain = "example.test",
    method = "POST",
    filter = Arc::new(StaticFilter {
        name: "maud-allow",
        allow: true
    }),
    wrap = Arc::new(AfterOnlyMiddleware)
)]
#[derive(Clone, Debug, Default)]
struct MaudPostPage;

impl Render for MaudPostPage {
    fn render(&self) -> PreEscaped<String> {
        html! {
            p { "posted" }
        }
    }
}

struct ItemId;

impl PathName for ItemId {
    const NAME: &'static str = "id";
}

struct MissingPath;

impl PathName for MissingPath {
    const NAME: &'static str = "missing";
}

#[derive(Debug)]
struct AppState {
    value: String,
}

struct AfterOnlyMiddleware;

impl Middleware for AfterOnlyMiddleware {
    fn name(&self) -> &str {
        "after-only"
    }

    fn before<'a>(
        &'a self,
        _data: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        Box::pin(async move { Ok(MiddlewareResult::Continue) })
    }

    fn after<'a>(
        &'a self,
        data: &'a mut Response,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        Box::pin(async move {
            data.headers_mut()
                .insert("x-after", http::HeaderValue::from_static("called"));
            Ok(MiddlewareResult::Continue)
        })
    }
}

#[test]
fn route_matches_and_extracts_variables() {
    let route = Route::new("/users/{id}/posts/{slug}".to_string());
    assert!(route.matches("/users/42/posts/hello-world"));
    assert_eq!(
        route
            .extract("/users/42/posts/hello-world", "id")
            .as_deref(),
        Some("42")
    );
    assert_eq!(
        route
            .extract("/users/42/posts/hello-world", "slug")
            .as_deref(),
        Some("hello-world")
    );
    assert!(!route.matches("/users/42/posts"));
}

#[test]
fn route_tail_wildcard_matches_nested_paths() {
    let route = Route::new("/assets/{file}*".to_string());
    assert!(route.matches("/assets/css/app.css"));
    assert!(route.matches("/assets/js/app.js"));
    assert_eq!(
        route.extract("/assets/css/app.css", "file").as_deref(),
        Some("css/app.css")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn path_and_state_extractors_cover_success_and_failure() {
    let mut request = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/items/42")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        Arc::new(Route::new("/items/{id}".to_string())),
    );
    let state = Arc::new(AppState {
        value: "ready".to_string(),
    });
    request.insert(state.clone());

    let id = <PathImpl<ItemId> as FromRequest<Request>>::try_from(&mut request)
        .await
        .expect("path id should extract");
    assert_eq!(id.name(), "id");
    assert_eq!(id.value(), "42");
    assert_eq!(id.to_string(), "42");

    let path = PathImpl::<ItemId>::new(id.value()).into_path();
    assert_eq!(path.value(), "42");
    assert_eq!(path.to_string(), "42");
    assert_eq!(Path::from(id).inner(), "42");
    assert_eq!(Path::new("manual").value(), "manual");

    let state_extractor = <State<AppState> as FromRequest<Request>>::try_from(&mut request)
        .await
        .expect("state should extract");
    assert_eq!(state_extractor.value, "ready");
    assert_eq!(state_extractor.as_ref().value, "ready");
    assert!(Arc::ptr_eq(&state_extractor.inner(), &state));

    let missing_path =
        match <PathImpl<MissingPath> as FromRequest<Request>>::try_from(&mut request).await {
            Ok(_) => panic!("missing path variable should fail"),
            Err(err) => err,
        };
    assert!(missing_path.to_string().contains("missing"));

    let missing_state = match <State<u64> as FromRequest<Request>>::try_from(&mut request).await {
        Ok(_) => panic!("missing state should fail"),
        Err(err) => err,
    };
    assert!(missing_state.to_string().contains("Failed to find State"));
}

#[tokio::test]
async fn service_group_state_is_available_to_registered_handlers() {
    let mut connections = HashMap::new();
    connections.insert("peer-1".to_string(), 1_usize);
    let server = ServerBuilder::new()
        .service_group(
            ServiceGroup::default()
                .shared_state(Config {
                    name: "julia-web-sdk".to_string(),
                })
                .shared_state(Client)
                .shared_state(RwLock::new(connections))
                .service(
                    ServiceBuilder::new("/sdk/state")
                        .handler(Arc::new(GroupStateService))
                        .build(),
                ),
        )
        .build();

    let default_scope = server.scoped_state.read().await;
    let config = default_scope
        .get("default")
        .and_then(|state| state.get::<Arc<Config>>())
        .expect("group config state should be registered")
        .clone();
    let client = default_scope
        .get("default")
        .and_then(|state| state.get::<Arc<Client>>())
        .expect("group client state should be registered")
        .clone();
    let connections = default_scope
        .get("default")
        .and_then(|state| state.get::<Arc<RwLock<HashMap<String, usize>>>>())
        .expect("group connection state should be registered")
        .clone();
    drop(default_scope);

    let mut request = basic_request();
    request.insert(config);
    request.insert(client);
    request.insert(connections);
    let response = server.services[0]
        .serve(&mut request)
        .await
        .expect("group state handler should succeed");
    assert_eq!(response.status(), http::StatusCode::OK);
}

#[test]
fn portfu_error_display_and_sources_are_specific() {
    let parsing = PortfuError::Parsing("bad input".to_string());
    assert_eq!(parsing.to_string(), "bad input");
    assert!(parsing.source().is_none());

    let internal = PortfuError::Internal("bad state".to_string());
    assert_eq!(internal.to_string(), "bad state");
    assert!(internal.source().is_none());

    let io = PortfuError::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "denied",
    ));
    assert_eq!(io.to_string(), "denied");
    assert!(io.source().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn request_extractors_and_host_parsing_work() {
    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/items?name=test&count=7")
        .header(http::header::HOST, "example.com:8080")
        .body(Full::new(Bytes::from(
            r#"{"id":99,"title":"sample"}"#.as_bytes().to_vec(),
        )))
        .expect("request build failed");
    let mut request = Request::new(
        RequestType::Sized(request),
        Arc::new(Route::new("/items".to_string())),
    );

    assert_eq!(request.host(), Some("example.com"));
    assert_eq!(request.method(), &Method::POST);
    assert_eq!(request.uri().path(), "/items");

    let query = <Query<QueryPayload> as FromRequest<Request>>::try_from(&mut request)
        .await
        .expect("query extraction failed");
    assert_eq!(
        query.into_inner(),
        QueryPayload {
            name: "test".to_string(),
            count: 7
        }
    );

    let json = <Json<JsonPayload> as FromRequest<Request>>::try_from(&mut request)
        .await
        .expect("json extraction failed");
    assert_eq!(
        json.into_inner(),
        JsonPayload {
            id: 99,
            title: "sample".to_string()
        }
    );

    let body = <Body as FromRequest<Request>>::try_from(&mut request)
        .await
        .expect("body extraction should not fail");
    assert!(body.into_bytes().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn request_body_mutation_handles_sized_consumed_and_empty_requests() {
    let mut sized = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::POST)
                .uri("/body")
                .body(Full::new(Bytes::from_static(b"original")))
                .expect("request build failed"),
        ),
        Arc::new(Route::new("/body".to_string())),
    );
    assert_eq!(sized.body_size_hint().exact(), Some(8));
    sized.set_body_bytes(Bytes::from_static(b"replacement"));
    assert_eq!(
        sized
            .consume_body_bytes()
            .await
            .expect("body read failed")
            .as_ref(),
        b"replacement"
    );
    assert_eq!(
        sized
            .consume_body_bytes()
            .await
            .expect("consumed body read should be empty")
            .as_ref(),
        b""
    );

    sized.set_body_bytes(Bytes::from_static(b"restored"));
    assert_eq!(
        sized
            .consume_body_bytes_limited(16, Duration::from_secs(1))
            .await
            .expect("limited body read failed")
            .as_ref(),
        b"restored"
    );

    sized.set_body_bytes(Bytes::from_static(b"too-large"));
    let err = sized
        .consume_body_bytes_limited(4, Duration::from_secs(1))
        .await
        .expect_err("oversized body should fail");
    assert!(err.to_string().contains("exceeded 4 bytes"));

    let mut empty = Request::new(
        RequestType::Empty(http::HeaderMap::new()),
        Arc::new(Route::new("/empty".to_string())),
    );
    assert_eq!(empty.method(), &Method::OPTIONS);
    assert_eq!(empty.uri(), &http::Uri::default());
    assert_eq!(empty.body_size_hint().exact(), Some(0));
    empty.headers_mut().insert(
        http::header::HOST,
        http::HeaderValue::from_static("empty.test"),
    );
    assert_eq!(
        empty.headers().get(http::header::HOST).unwrap(),
        "empty.test"
    );
    assert!(empty.shared_state_mut().is_none());
    empty.set_body_bytes(Bytes::new());
    assert_eq!(
        empty
            .consume_body_bytes()
            .await
            .expect("empty body read failed")
            .as_ref(),
        b""
    );
    empty.set_body_bytes(Bytes::from_static(b"created"));
    assert_eq!(
        empty
            .consume_body_bytes_limited(16, Duration::from_secs(1))
            .await
            .expect("body created from empty request should read")
            .as_ref(),
        b"created"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn request_extractors_report_parse_failures() {
    let mut bad_query = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/items?count=not-a-number")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        Arc::new(Route::new("/items".to_string())),
    );
    let query_err =
        match <Query<QueryPayload> as FromRequest<Request>>::try_from(&mut bad_query).await {
            Ok(_) => panic!("bad query should fail"),
            Err(err) => err,
        };
    assert!(query_err.to_string().contains("Failed to parse query"));

    let mut bad_json = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::POST)
                .uri("/items")
                .body(Full::new(Bytes::from_static(b"{not-json")))
                .expect("request build failed"),
        ),
        Arc::new(Route::new("/items".to_string())),
    );
    let json_err = match <Json<JsonPayload> as FromRequest<Request>>::try_from(&mut bad_json).await
    {
        Ok(_) => panic!("bad json should fail"),
        Err(err) => err,
    };
    assert!(json_err.to_string().contains("Failed to parse JSON body"));
}

#[tokio::test(flavor = "current_thread")]
async fn response_conversions_populate_status_headers_and_body() {
    let response: Response = "hello".into();
    let response: http::Response<_> = response.into();
    assert_eq!(response.status(), http::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );
    let content_length = response
        .headers()
        .get(http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok());
    assert_eq!(content_length, Some("5"));
    let body = BodyExt::collect(response.into_body())
        .await
        .expect("body collection failed")
        .to_bytes();
    assert_eq!(&body[..], b"hello");

    let empty: Response = ().into();
    let empty: http::Response<_> = empty.into();
    assert_eq!(empty.status(), http::StatusCode::OK);
    let body = BodyExt::collect(empty.into_body())
        .await
        .expect("empty body collection failed")
        .to_bytes();
    assert!(body.is_empty());

    let json: Response = JsonResponse::from(vec!["a", "b"]).into();
    let json: http::Response<_> = json.into();
    let body = BodyExt::collect(json.into_body())
        .await
        .expect("json body collection failed")
        .to_bytes();
    assert_eq!(&body[..], br#"["a","b"]"#);

    let marker: Response = MarkerJson { id: 7 }.into();
    let marker: http::Response<_> = marker.into();
    assert_eq!(
        marker
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
    let body = BodyExt::collect(marker.into_body())
        .await
        .expect("marker json body collection failed")
        .to_bytes();
    assert_eq!(&body[..], br#"{"id":7}"#);
}

#[tokio::test(flavor = "current_thread")]
async fn response_accessors_and_conversions_cover_empty_sized_and_consumed_shapes() {
    let mut response = Response::new();
    assert_eq!(response.status(), http::StatusCode::OK);
    *response.status_mut() = http::StatusCode::ACCEPTED;
    response
        .headers_mut()
        .insert("x-test", http::HeaderValue::from_static("yes"));
    assert_eq!(
        response
            .headers()
            .get("x-test")
            .and_then(|v| v.to_str().ok()),
        Some("yes")
    );
    assert_eq!(response.body_size_hint().exact(), Some(0));

    let empty_message = Response::from_status_and_message(http::StatusCode::NO_CONTENT, "");
    let empty_message: http::Response<_> = empty_message.into();
    assert_eq!(empty_message.status(), http::StatusCode::NO_CONTENT);
    assert_eq!(
        empty_message
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok()),
        Some("0")
    );

    let bytes: Response = Bytes::from_static(b"bytes").into();
    let bytes: http::Response<_> = bytes.into();
    assert_eq!(
        bytes
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/octet-stream")
    );
    let body = BodyExt::collect(bytes.into_body())
        .await
        .expect("bytes body collection failed")
        .to_bytes();
    assert_eq!(&body[..], b"bytes");

    let vec_response: Response = vec![1_u8, 2, 3].into();
    let vec_response: http::Response<_> = vec_response.into();
    let body = BodyExt::collect(vec_response.into_body())
        .await
        .expect("vec body collection failed")
        .to_bytes();
    assert_eq!(&body[..], &[1, 2, 3]);

    let slice_response: Response = (&b"slice"[..]).into();
    let slice_response: http::Response<_> = slice_response.into();
    let body = BodyExt::collect(slice_response.into_body())
        .await
        .expect("slice body collection failed")
        .to_bytes();
    assert_eq!(&body[..], b"slice");

    let sized: Response = http::Response::builder()
        .status(http::StatusCode::CREATED)
        .body(Full::new(Bytes::from_static(b"created")))
        .expect("response build failed")
        .into();
    let sized: http::Response<_> = sized.into();
    assert_eq!(sized.status(), http::StatusCode::CREATED);
    assert_eq!(
        sized
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok()),
        Some("7")
    );

    let middleware = AfterOnlyMiddleware;
    let request = basic_request();
    let mut response = Response::ok("ok");
    assert_eq!(middleware.name(), "after-only");
    assert!(matches!(
        middleware
            .after_with_request(&request, &mut response)
            .await
            .expect("middleware after failed"),
        MiddlewareResult::Continue
    ));
    assert_eq!(
        response
            .headers()
            .get("x-after")
            .and_then(|v| v.to_str().ok()),
        Some("called")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn filters_any_all_and_or_behave_as_expected() {
    let allow = Arc::new(StaticFilter {
        name: "allow",
        allow: true,
    });
    let block = Arc::new(StaticFilter {
        name: "block",
        allow: false,
    });
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/f")
        .body(Full::new(Bytes::new()))
        .expect("request build failed");
    let request = Request::new(
        RequestType::Sized(request),
        Arc::new(Route::new("/f".to_string())),
    );

    let any = filter::any("any".to_string(), &[allow.clone(), block.clone()]);
    assert!(any.filter(&request).await == FilterResult::Allow);

    let all = filter::all("all".to_string(), &[allow, block]);
    assert!(all.filter(&request).await == FilterResult::Block);

    let chained = filter::all(
        "chain".to_string(),
        &[Arc::new(StaticFilter {
            name: "lhs",
            allow: false,
        })],
    )
    .or(Arc::new(StaticFilter {
        name: "rhs",
        allow: true,
    }));
    assert!(chained.filter(&request).await == FilterResult::Allow);
}

#[tokio::test(flavor = "current_thread")]
async fn method_filters_name_and_match_expected_methods() {
    let get_request = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/method")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        Arc::new(Route::new("/method".to_string())),
    );
    let post_request = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::POST)
                .uri("/method")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        Arc::new(Route::new("/method".to_string())),
    );

    assert_eq!(filter::method::GET.name(), "GET");
    assert_eq!(
        filter::method::GET.filter(&get_request).await,
        FilterResult::Allow
    );
    assert_eq!(
        filter::method::GET.filter(&post_request).await,
        FilterResult::Block
    );
}

#[tokio::test(flavor = "current_thread")]
async fn session_and_oauth_extractors_read_request_session() {
    let mut request = request_with_session(Some(OAuthToken {
        access_token: "access".to_string(),
        refresh_token: None,
        token_type: "Bearer".to_string(),
        expires_in_seconds: Some(60),
        scopes: vec!["profile".to_string(), "email".to_string()],
    }))
    .await;

    let session = <SessionState as FromRequest<Request>>::try_from(&mut request)
        .await
        .expect("session extractor should succeed");
    assert_eq!(
        session.inner().read().await.id,
        request
            .get::<Arc<RwLock<Session>>>()
            .unwrap()
            .read()
            .await
            .id
    );

    let token = <OAuthToken as FromRequest<Request>>::try_from(&mut request)
        .await
        .expect("oauth token extractor should succeed");
    assert_eq!(token.access_token, "access");
    assert_eq!(
        token.scopes,
        vec!["profile".to_string(), "email".to_string()]
    );

    let identity = <OAuthIdentity as FromRequest<Request>>::try_from(&mut request)
        .await
        .expect("oauth identity extractor should succeed");
    assert_eq!(identity.subject, "user-1");
    assert_eq!(identity.email.as_deref(), Some("user@example.com"));
}

#[tokio::test(flavor = "current_thread")]
async fn session_manager_creates_reuses_and_expires_sessions() {
    let manager = SessionManager {
        session_duration: Duration::from_secs(60),
        secure: false,
    };
    let mut first = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/session")
                .header("x-real-ip", "203.0.113.10")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        Arc::new(Route::new("/session".to_string())),
    );

    assert!(matches!(
        manager
            .before(&mut first)
            .await
            .expect("session before failed"),
        MiddlewareResult::Continue
    ));
    let first_session = first
        .get::<Arc<RwLock<Session>>>()
        .expect("session should be attached")
        .clone();
    let mut response = Response::ok("ok");
    manager
        .after_with_request(&first, &mut response)
        .await
        .expect("session after failed");
    let set_cookie = response
        .headers()
        .get("set-cookie")
        .expect("set-cookie should be present")
        .clone();
    assert_eq!(
        response
            .headers()
            .get("session_id")
            .and_then(|v| v.to_str().ok()),
        set_cookie.to_str().ok()
    );

    let cookie_header = set_cookie.to_str().expect("cookie should be valid");
    let cookie_request = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/session")
                .header(http::header::COOKIE, cookie_header)
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        Arc::new(Route::new("/session".to_string())),
    );
    let cookie =
        get_session_cookie_from_request(&cookie_request).expect("session cookie should parse");
    let by_id = SessionManager::get_session_from_id(cookie.value_trimmed())
        .expect("session should be indexed by client id");
    assert!(Arc::ptr_eq(&first_session, &by_id));

    let mut second = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/session")
                .header("x-real-ip", "203.0.113.10")
                .header(http::header::COOKIE, cookie_header)
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        Arc::new(Route::new("/session".to_string())),
    );
    assert!(get_session_from_request(&second).await.is_some());
    manager
        .before(&mut second)
        .await
        .expect("session reuse before failed");
    let second_session = second
        .get::<Arc<RwLock<Session>>>()
        .expect("reused session should be attached")
        .clone();
    assert!(Arc::ptr_eq(&first_session, &second_session));

    first_session.write().await.last_update = std::time::Instant::now() - Duration::from_secs(120);
    manager.cleanup_expired().await;
    assert!(SessionManager::get_session_from_id(cookie.value_trimmed()).is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn auth_filters_lock_routes_against_session_and_oauth_state() {
    let anonymous = basic_request();
    assert_eq!(
        filter::auth::session().filter(&anonymous).await,
        FilterResult::Block
    );
    assert_eq!(
        filter::auth::oauth().filter(&anonymous).await,
        FilterResult::Block
    );

    let session_only = request_with_session(None).await;
    assert_eq!(
        filter::auth::session().filter(&session_only).await,
        FilterResult::Allow
    );
    assert_eq!(
        filter::auth::oauth().filter(&session_only).await,
        FilterResult::Block
    );

    let oauth_request = request_with_session(Some(OAuthToken {
        access_token: "access".to_string(),
        refresh_token: None,
        token_type: "Bearer".to_string(),
        expires_in_seconds: None,
        scopes: vec!["read".to_string(), "write".to_string()],
    }))
    .await;
    assert_eq!(
        filter::auth::oauth().filter(&oauth_request).await,
        FilterResult::Allow
    );
    assert_eq!(
        filter::auth::oauth_scope("read")
            .filter(&oauth_request)
            .await,
        FilterResult::Allow
    );
    assert_eq!(
        filter::auth::oauth_all_scopes(["read", "admin"])
            .filter(&oauth_request)
            .await,
        FilterResult::Block
    );
    assert_eq!(
        filter::auth::oauth_any_scope(["admin", "write"])
            .filter(&oauth_request)
            .await,
        FilterResult::Allow
    );
    assert_eq!(
        filter::auth::oauth_role("user")
            .filter(&oauth_request)
            .await,
        FilterResult::Allow
    );
    assert_eq!(
        filter::auth::oauth_role("admin")
            .filter(&oauth_request)
            .await,
        FilterResult::Block
    );
    assert_eq!(
        filter::auth::oauth_any_role(["admin", "user"])
            .filter(&oauth_request)
            .await,
        FilterResult::Allow
    );
    assert_eq!(
        filter::auth::oauth_all_roles(["user", "admin"])
            .filter(&oauth_request)
            .await,
        FilterResult::Block
    );
    assert_eq!(
        filter::auth::oauth_group("/engineering")
            .filter(&oauth_request)
            .await,
        FilterResult::Allow
    );
    assert_eq!(
        filter::auth::oauth_any_group(["/ops", "/engineering"])
            .filter(&oauth_request)
            .await,
        FilterResult::Allow
    );

    let oauth_without_role = request_with_session_and_roles(
        Some(OAuthToken {
            access_token: "access".to_string(),
            refresh_token: None,
            token_type: "Bearer".to_string(),
            expires_in_seconds: None,
            scopes: vec!["read".to_string()],
        }),
        Vec::<String>::new(),
    )
    .await;
    assert_eq!(
        filter::auth::oauth_role("user")
            .filter(&oauth_without_role)
            .await,
        FilterResult::Block
    );
}

#[tokio::test(flavor = "current_thread")]
async fn metrics_wrapper_records_requests_and_endpoint_returns_text() {
    let service = ServiceBuilder::new("/metrics-target")
        .name("metrics-target")
        .filter(filter::method::GET.clone())
        .wrap(Arc::new(MetricsWrapper))
        .handler(Arc::new(OkService))
        .build();
    let mut request = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/metrics-target")
                .body(Full::new(Bytes::from_static(b"abc")))
                .expect("request build failed"),
        ),
        Arc::new(Route::new("/metrics-target".to_string())),
    );

    let response = service.serve(&mut request).await.expect("service failed");
    assert_eq!(response.status(), http::StatusCode::OK);

    let metrics = ServerBuilder::new().enable_metrics().build();
    let endpoint = metrics
        .services
        .iter()
        .find(|service| service.name() == "metrics_endpoint")
        .expect("metrics endpoint should be registered");
    let mut metrics_request = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/metrics")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        endpoint.route(),
    );
    let metrics_response = endpoint
        .serve(&mut metrics_request)
        .await
        .expect("metrics endpoint failed");
    assert_eq!(
        metrics_response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/plain; version=0.0.4")
    );
    let metrics_response: http::Response<_> = metrics_response.into();
    let body = BodyExt::collect(metrics_response.into_body())
        .await
        .expect("metrics body collection failed")
        .to_bytes();
    let body = String::from_utf8(body.to_vec()).expect("metrics should be utf-8");
    assert!(body.contains("portfu_metrics_response_sizes_histogram"));
}

#[tokio::test(flavor = "current_thread")]
async fn endpoint_macro_serializes_plain_serialize_results_as_json() {
    let services = load_registered_services();
    let service = services
        .iter()
        .find(|service| service.name() == "typed-json")
        .expect("typed-json service should be registered");

    let mut request = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/typed-json")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        service.route(),
    );
    assert!(service.serves(&request).await);
    let response = service.serve(&mut request).await.expect("service failed");
    assert_eq!(response.status(), http::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("application/json")
    );

    let response: http::Response<_> = response.into();
    let body = BodyExt::collect(response.into_body())
        .await
        .expect("json body collection failed")
        .to_bytes();
    assert_eq!(&body[..], br#"[{"id":7,"name":"seven"}]"#);
}

#[tokio::test(flavor = "current_thread")]
async fn endpoint_macro_reports_json_serialization_failures() {
    let services = load_registered_services();
    let service = services
        .iter()
        .find(|service| service.name() == "broken-json")
        .expect("broken-json service should be registered");

    let mut request = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/broken-json")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        service.route(),
    );
    assert!(service.serves(&request).await);
    let response = service.serve(&mut request).await.expect("service failed");
    assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/plain; charset=utf-8")
    );

    let response: http::Response<_> = response.into();
    let body = BodyExt::collect(response.into_body())
        .await
        .expect("error body collection failed")
        .to_bytes();
    assert_eq!(&body[..], b"Failed to serialize JSON");
}

#[tokio::test(flavor = "current_thread")]
async fn endpoint_macro_maps_extractor_and_client_trust_errors() {
    let services = load_registered_services();
    let json_service = services
        .iter()
        .find(|service| service.name() == "extract-json")
        .expect("extract-json service should be registered");
    let mut malformed = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::POST)
                .uri("/extract-json")
                .body(Full::new(Bytes::from_static(b"{bad-json")))
                .unwrap(),
        ),
        json_service.route(),
    );
    let response = json_service.serve(&mut malformed).await.unwrap();
    assert_eq!(response.status(), http::StatusCode::BAD_REQUEST);

    let trusted_service = services
        .iter()
        .find(|service| service.name() == "trusted-endpoint")
        .expect("trusted endpoint should be registered");
    let make_request = || {
        Request::new(
            RequestType::Sized(
                http::Request::builder()
                    .method(Method::POST)
                    .uri("/trusted-endpoint")
                    .body(Full::new(Bytes::new()))
                    .unwrap(),
            ),
            trusted_service.route(),
        )
    };
    let mut missing = make_request();
    assert_eq!(
        trusted_service.serve(&mut missing).await.unwrap().status(),
        http::StatusCode::UNAUTHORIZED
    );

    let identity = |verified_by: &str| ClientIdentity {
        leaf_der: Arc::from([]),
        chain_der: Arc::from([]),
        sha256_fingerprint: [0; 32],
        verified_by: vec![verified_by.to_string()],
    };
    let mut wrong = make_request();
    wrong.insert(identity("public-clients"));
    assert_eq!(
        trusted_service.serve(&mut wrong).await.unwrap().status(),
        http::StatusCode::FORBIDDEN
    );
    let mut allowed = make_request();
    allowed.insert(identity("internal-clients"));
    assert_eq!(
        trusted_service.serve(&mut allowed).await.unwrap().status(),
        http::StatusCode::OK
    );
}

#[tokio::test(flavor = "current_thread")]
async fn endpoint_macro_maps_handler_errors_with_into_response() {
    let services = load_registered_services();
    let service = services
        .iter()
        .find(|service| service.name() == "handler-error")
        .expect("handler-error service should be registered");
    let mut request = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/handler-error")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        service.route(),
    );
    let response = service
        .serve(&mut request)
        .await
        .expect("handler error should become a response");
    assert_eq!(response.status(), http::StatusCode::IM_A_TEAPOT);
    let response: http::Response<_> = response.into();
    let body = BodyExt::collect(response.into_body())
        .await
        .expect("error body collection failed")
        .to_bytes();
    assert_eq!(&body[..], b"short and stout");
}

#[tokio::test(flavor = "current_thread")]
async fn maud_http_macro_registers_html_service() {
    let services = load_registered_services();
    let maud_services: Vec<_> = services
        .iter()
        .filter(|service| service.name() == "maud-index")
        .collect();
    assert_eq!(
        maud_services.len(),
        2,
        "maud aliases should each register a service"
    );

    for path in ["/maud/", "/maud/index.html"] {
        let mut matched = None;
        for service in &maud_services {
            let request = Request::new(
                RequestType::Sized(
                    http::Request::builder()
                        .method(Method::GET)
                        .uri(path)
                        .body(Full::new(Bytes::new()))
                        .expect("request build failed"),
                ),
                service.route(),
            );
            if service.serves(&request).await {
                matched = Some(*service);
                break;
            }
        }
        let service = matched.expect("maud path should be registered");

        let mut get_request = Request::new(
            RequestType::Sized(
                http::Request::builder()
                    .method(Method::GET)
                    .uri(path)
                    .body(Full::new(Bytes::new()))
                    .expect("request build failed"),
            ),
            service.route(),
        );
        let response = service
            .serve(&mut get_request)
            .await
            .expect("maud service failed");
        assert_eq!(response.status(), http::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/html; charset=utf-8")
        );
        let response: http::Response<_> = response.into();
        let body = BodyExt::collect(response.into_body())
            .await
            .expect("maud body collection failed")
            .to_bytes();
        assert_eq!(&body[..], b"<h1>Hello from Maud</h1>");
    }

    let mut matched_options = None;
    for service in &maud_services {
        let request = Request::new(
            RequestType::Sized(
                http::Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/maud/")
                    .body(Full::new(Bytes::new()))
                    .expect("request build failed"),
            ),
            service.route(),
        );
        if service.serves(&request).await {
            matched_options = Some(*service);
            break;
        }
    }
    let options_service = matched_options.expect("maud options path should be registered");
    let mut options_request = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::OPTIONS)
                .uri("/maud/")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        options_service.route(),
    );
    let options_response = options_service
        .serve(&mut options_request)
        .await
        .expect("maud options failed");
    assert_eq!(
        options_response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/html; charset=utf-8")
    );

    for service in maud_services {
        let post_request = Request::new(
            RequestType::Sized(
                http::Request::builder()
                    .method(Method::POST)
                    .uri("/maud/")
                    .body(Full::new(Bytes::new()))
                    .expect("request build failed"),
            ),
            service.route(),
        );
        assert!(!service.serves(&post_request).await);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn maud_http_macro_populates_path_fields() {
    let services = load_registered_services();
    let maud_services: Vec<_> = services
        .iter()
        .filter(|service| service.name() == "maud-user")
        .collect();
    assert_eq!(
        maud_services.len(),
        2,
        "maud path aliases should each register a service"
    );

    for (uri, expected_body) in [
        ("/maud/users/42", b"<p>42</p>".as_slice()),
        (
            "/maud/organizations/1?return_to=%2Forganizations",
            b"<p>1</p>".as_slice(),
        ),
    ] {
        let mut matched = None;
        for service in &maud_services {
            let request = Request::new(
                RequestType::Sized(
                    http::Request::builder()
                        .method(Method::GET)
                        .uri(uri)
                        .body(Full::new(Bytes::new()))
                        .expect("request build failed"),
                ),
                service.route(),
            );
            if service.serves(&request).await {
                matched = Some(*service);
                break;
            }
        }
        let service = matched.expect("maud path should be registered");

        let mut request = Request::new(
            RequestType::Sized(
                http::Request::builder()
                    .method(Method::GET)
                    .uri(uri)
                    .body(Full::new(Bytes::new()))
                    .expect("request build failed"),
            ),
            service.route(),
        );
        let response = service.serve(&mut request).await.expect("service failed");
        assert_eq!(response.status(), http::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/html; charset=utf-8")
        );
        let response: http::Response<_> = response.into();
        let body = BodyExt::collect(response.into_body())
            .await
            .expect("maud body collection failed")
            .to_bytes();
        assert_eq!(&body[..], expected_body);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn maud_http_macro_applies_endpoint_style_options() {
    let services = load_registered_services();
    let blocked = services
        .iter()
        .find(|service| service.name() == "maud-blocked")
        .expect("blocked maud service should be registered");
    let blocked_request = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/maud/blocked")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        blocked.route(),
    );
    assert!(!blocked.serves(&blocked_request).await);

    let service = services
        .iter()
        .find(|service| service.name() == "maud-post")
        .expect("post maud service should be registered");
    assert_eq!(service.scope(), "maud-scope");
    assert_eq!(service.domains(), &["example.test".to_string()]);

    let wrong_host = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::POST)
                .uri("/maud/post")
                .header(http::header::HOST, "other.test")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        service.route(),
    );
    assert!(!service.serves(&wrong_host).await);

    let unsupported_method = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::PUT)
                .uri("/maud/post")
                .header(http::header::HOST, "example.test")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        service.route(),
    );
    assert!(!service.serves(&unsupported_method).await);

    let mut request = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::POST)
                .uri("/maud/post")
                .header(http::header::HOST, "example.test")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        service.route(),
    );
    assert!(service.serves(&request).await);
    let response = service.serve(&mut request).await.expect("service failed");
    assert_eq!(response.status(), http::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok()),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(
        response
            .headers()
            .get("x-after")
            .and_then(|v| v.to_str().ok()),
        Some("called")
    );
    let response: http::Response<_> = response.into();
    let body = BodyExt::collect(response.into_body())
        .await
        .expect("maud body collection failed")
        .to_bytes();
    assert_eq!(&body[..], b"<p>posted</p>");
}

#[tokio::test(flavor = "current_thread")]
async fn service_builder_scope_domain_and_filters_are_enforced() {
    let service = ServiceBuilder::new("/scoped")
        .name("scoped-service")
        .scope("tenant-a")
        .domain("tenant-a.local")
        .filter(filter::method::GET.clone())
        .handler(Arc::new(OkService))
        .build();

    let matching = http::Request::builder()
        .method(Method::GET)
        .uri("/scoped")
        .header(http::header::HOST, "tenant-a.local")
        .body(Full::new(Bytes::new()))
        .expect("request build failed");
    let non_matching_domain = http::Request::builder()
        .method(Method::GET)
        .uri("/scoped")
        .header(http::header::HOST, "tenant-b.local")
        .body(Full::new(Bytes::new()))
        .expect("request build failed");
    let non_matching_method = http::Request::builder()
        .method(Method::POST)
        .uri("/scoped")
        .header(http::header::HOST, "tenant-a.local")
        .body(Full::new(Bytes::new()))
        .expect("request build failed");

    let matching = Request::new(
        RequestType::Sized(matching),
        Arc::new(Route::new("/scoped".to_string())),
    );
    let non_matching_domain = Request::new(
        RequestType::Sized(non_matching_domain),
        Arc::new(Route::new("/scoped".to_string())),
    );
    let non_matching_method = Request::new(
        RequestType::Sized(non_matching_method),
        Arc::new(Route::new("/scoped".to_string())),
    );

    assert_eq!(service.scope(), "tenant-a");
    assert_eq!(service.name(), "scoped-service");
    assert!(service.serves(&matching).await);
    assert!(!service.serves(&non_matching_domain).await);
    assert!(!service.serves(&non_matching_method).await);
}

#[tokio::test(flavor = "current_thread")]
async fn rate_limiter_enforces_request_windows_and_can_be_disabled() {
    let limiter = Arc::new(RateLimiter::with_global_limit(RateLimit::new(1, 1, 0)));
    let service = ServiceBuilder::new("/limited-window")
        .name("limited-window")
        .filter(filter::method::GET.clone())
        .wrap(limiter.clone())
        .handler(Arc::new(OkService))
        .build();

    let mut first = limited_request("/limited-window", "198.51.100.10");
    let response = service
        .serve(&mut first)
        .await
        .expect("first request failed");
    assert_eq!(response.status(), http::StatusCode::OK);

    let mut second = limited_request("/limited-window", "198.51.100.10");
    let response = service
        .serve(&mut second)
        .await
        .expect("second request failed");
    assert_eq!(response.status(), http::StatusCode::TOO_MANY_REQUESTS);

    limiter
        .enabled
        .store(false, std::sync::atomic::Ordering::Relaxed);
    let mut third = limited_request("/limited-window", "198.51.100.10");
    let response = service
        .serve(&mut third)
        .await
        .expect("disabled request failed");
    assert_eq!(response.status(), http::StatusCode::OK);
}

#[tokio::test(flavor = "current_thread")]
async fn rate_limiter_uses_path_specific_limits_and_socket_fallback_ip() {
    let limiter = Arc::new(RateLimiter::default().request_size_limit(0));
    limiter
        .set_path_limit("/tight", RateLimit::new(1, 1, 0))
        .await;
    let service = ServiceBuilder::new("/tight")
        .name("tight")
        .filter(filter::method::GET.clone())
        .wrap(limiter)
        .handler(Arc::new(OkService))
        .build();

    let mut first = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/tight")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        Arc::new(Route::new("/tight".to_string())),
    );
    first.insert(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 55)),
        8080,
    ));
    let response = service
        .serve(&mut first)
        .await
        .expect("first request failed");
    assert_eq!(response.status(), http::StatusCode::OK);

    let mut second = Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri("/tight")
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        Arc::new(Route::new("/tight".to_string())),
    );
    second.insert(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 55)),
        8080,
    ));
    let response = service
        .serve(&mut second)
        .await
        .expect("second request failed");
    assert_eq!(response.status(), http::StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test(flavor = "current_thread")]
async fn cors_wrapper_allows_all_origins_methods_and_headers() {
    let service = ServiceBuilder::new("/cors")
        .name("cors-allow-all")
        .filter(filter::method::OPTIONS.clone())
        .wrap(Arc::new(Cors::allow_all()))
        .handler(Arc::new(OkService))
        .build();

    let request = http::Request::builder()
        .method(Method::OPTIONS)
        .uri("/cors")
        .header(http::header::ORIGIN, "https://example.test")
        .body(Full::new(Bytes::new()))
        .expect("request build failed");
    let mut request = Request::new(
        RequestType::Sized(request),
        Arc::new(Route::new("/cors".to_string())),
    );

    assert!(service.serves(&request).await);
    let response = service.serve(&mut request).await.expect("service failed");

    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("*")
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-methods")
            .and_then(|v| v.to_str().ok()),
        Some("*")
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-headers")
            .and_then(|v| v.to_str().ok()),
        Some("*")
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-credentials")
            .and_then(|v| v.to_str().ok()),
        Some("false")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn cors_wrapper_reflects_only_allowed_origin_and_requested_headers() {
    let service = ServiceBuilder::new("/cors")
        .name("cors-allow-list")
        .filter(filter::method::GET.clone())
        .wrap(Arc::new(Cors::new(
            vec!["https://allowed.test".to_string()],
            vec!["GET".to_string(), "POST".to_string()],
            vec![
                http::HeaderName::from_static("content-type"),
                http::HeaderName::from_static("x-api-key"),
            ],
            true,
        )))
        .handler(Arc::new(OkService))
        .build();

    let allowed_request = http::Request::builder()
        .method(Method::GET)
        .uri("/cors")
        .header(http::header::ORIGIN, "https://allowed.test")
        .header(
            "access-control-request-headers",
            "X-Api-Key, X-Denied, Content-Type",
        )
        .body(Full::new(Bytes::new()))
        .expect("request build failed");
    let mut allowed_request = Request::new(
        RequestType::Sized(allowed_request),
        Arc::new(Route::new("/cors".to_string())),
    );
    let allowed_response = service
        .serve(&mut allowed_request)
        .await
        .expect("service failed");

    assert_eq!(
        allowed_response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("https://allowed.test")
    );
    assert_eq!(
        allowed_response
            .headers()
            .get("access-control-allow-methods")
            .and_then(|v| v.to_str().ok()),
        Some("GET,POST")
    );
    assert_eq!(
        allowed_response
            .headers()
            .get("access-control-allow-headers")
            .and_then(|v| v.to_str().ok()),
        Some("x-api-key,content-type")
    );
    assert_eq!(
        allowed_response
            .headers()
            .get("access-control-allow-credentials")
            .and_then(|v| v.to_str().ok()),
        Some("true")
    );

    let denied_request = http::Request::builder()
        .method(Method::GET)
        .uri("/cors")
        .header(http::header::ORIGIN, "https://denied.test")
        .body(Full::new(Bytes::new()))
        .expect("request build failed");
    let mut denied_request = Request::new(
        RequestType::Sized(denied_request),
        Arc::new(Route::new("/cors".to_string())),
    );
    let denied_response = service
        .serve(&mut denied_request)
        .await
        .expect("service failed");

    assert!(
        denied_response
            .headers()
            .get("access-control-allow-origin")
            .is_none()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn rate_limiter_rejects_payloads_over_size_limit() {
    let service = ServiceBuilder::new("/limited")
        .name("limited")
        .filter(filter::method::POST.clone())
        .wrap(Arc::new(RateLimiter::default().request_size_limit(4)))
        .handler(Arc::new(OkService))
        .build();

    let request = http::Request::builder()
        .method(Method::POST)
        .uri("/limited")
        .body(Full::new(Bytes::from_static(b"too-large")))
        .expect("request build failed");
    let mut request = Request::new(
        RequestType::Sized(request),
        Arc::new(Route::new("/limited".to_string())),
    );

    let response = service.serve(&mut request).await.expect("service failed");
    assert_eq!(response.status(), http::StatusCode::PAYLOAD_TOO_LARGE);
}

#[test]
fn server_builder_adds_wrapper_services_without_inventory() {
    let server = ServerBuilder::new()
        .enable_metrics()
        .finish_metrics()
        .enable_cors()
        .allow_all()
        .finish_cors()
        .enable_rate_limits()
        .request_size_limit(1024)
        .finish_rate_limits()
        .enable_oauth(portfu_common::auth::oauth::OAUTH::CUSTOM)
        .client_id("client")
        .client_secret("secret")
        .auth_url("https://example.com/auth")
        .token_url("https://example.com/token")
        .redirect_url("https://example.com/callback")
        .build();

    assert!(
        server
            .services
            .iter()
            .any(|service| service.name() == "metrics_endpoint")
    );
    assert!(
        server
            .services
            .iter()
            .any(|service| service.name() == "oauth_login")
    );
    assert!(
        server
            .services
            .iter()
            .any(|service| service.name() == "oauth_callback")
    );
    assert_eq!(server.middleware.len(), 4);

    let _metrics = MetricsWrapper;
}

#[tokio::test(flavor = "current_thread")]
async fn server_builder_configures_cors_and_sessions_as_default_wrappers() {
    let server = ServerBuilder::new()
        .enable_cors()
        .allowed_origin("https://allowed.test")
        .allowed_method("GET")
        .allowed_header(http::HeaderName::from_static("x-api-key"))
        .allow_credentials(true)
        .finish_cors()
        .enable_sessions()
        .duration(Duration::from_secs(30))
        .secure(false)
        .finish_sessions()
        .build();

    assert_eq!(server.middleware.len(), 2);
    assert_eq!(server.middleware[0].name(), "Cors Wrapper");
    assert_eq!(server.middleware[1].name(), "SessionManager");

    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/wrapped")
        .header(http::header::ORIGIN, "https://allowed.test")
        .header("access-control-request-headers", "X-Api-Key, X-Denied")
        .body(Full::new(Bytes::new()))
        .expect("request build failed");
    let mut request = Request::new(
        RequestType::Sized(request),
        Arc::new(Route::new("/wrapped".to_string())),
    );
    let mut response = Response::ok("ok");

    assert!(matches!(
        server.middleware[0]
            .after_with_request(&request, &mut response)
            .await
            .expect("cors after failed"),
        MiddlewareResult::Continue
    ));
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("https://allowed.test")
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-methods")
            .and_then(|v| v.to_str().ok()),
        Some("GET")
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-headers")
            .and_then(|v| v.to_str().ok()),
        Some("x-api-key")
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-credentials")
            .and_then(|v| v.to_str().ok()),
        Some("true")
    );

    assert!(matches!(
        server.middleware[1]
            .before(&mut request)
            .await
            .expect("session before failed"),
        MiddlewareResult::Continue
    ));
    server.middleware[1]
        .after_with_request(&request, &mut response)
        .await
        .expect("session after failed");
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .expect("set-cookie should be present");
    assert!(cookie.contains("HttpOnly"));
    assert!(!cookie.contains("Secure"));
}

fn basic_request() -> Request {
    let request = http::Request::builder()
        .method(Method::GET)
        .uri("/auth")
        .body(Full::new(Bytes::new()))
        .expect("request build failed");
    Request::new(
        RequestType::Sized(request),
        Arc::new(Route::new("/auth".to_string())),
    )
}

fn load_registered_services() -> Vec<portfu_common::service::Service> {
    let mut registry = portfu_common::server::ServiceRegistry::default();
    let services: Vec<_> = inventory::iter::<portfu_common::server::ServiceRegistration>
        .into_iter()
        .map(|reg| (reg.register)(&mut registry))
        .collect();
    registry.services.extend(services.clone());
    services
}

fn limited_request(path: &str, ip: &str) -> Request {
    Request::new(
        RequestType::Sized(
            http::Request::builder()
                .method(Method::GET)
                .uri(path)
                .header("cf-connecting-ip", ip)
                .body(Full::new(Bytes::new()))
                .expect("request build failed"),
        ),
        Arc::new(Route::new(path.to_string())),
    )
}

async fn request_with_session(token: Option<OAuthToken>) -> Request {
    request_with_session_and_roles(token, ["user"]).await
}

async fn request_with_session_and_roles<I, S>(token: Option<OAuthToken>, roles: I) -> Request
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut request = basic_request();
    let session = Arc::new(RwLock::new(Session::default()));
    if let Some(token) = token {
        let identity = OAuthIdentity {
            provider: OAUTH::CUSTOM,
            subject: "user-1".to_string(),
            username: Some("user".to_string()),
            email: Some("user@example.com".to_string()),
            roles: roles.into_iter().map(Into::into).collect(),
            groups: vec!["/engineering".to_string()],
            scopes: token.scopes.clone(),
            raw: serde_json::json!({"sub": "user-1"}),
        };
        let mut session = session.write().await;
        session.data.insert(SessionOAuthToken(token));
        session.data.insert(SessionOAuthIdentity(identity));
    }
    request.insert(session);
    request
}

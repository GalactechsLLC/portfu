use http::Method;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use portfu_common::auth::oauth::{
    OAUTH, OAuthIdentity, OAuthToken, SessionOAuthIdentity, SessionOAuthToken,
};
use portfu_common::error::PortfuError;
use portfu_common::router::filter::{self, FilterResult, traits::Filter as FilterTrait};
use portfu_common::router::route::Route;
use portfu_common::server::builder::ServerBuilder;
use portfu_common::service::builder::ServiceBuilder;
use portfu_common::service::request::{Body, FromRequest, Json, Query, Request, RequestType};
use portfu_common::service::response::{JsonResponse, Response, Serialized};
use portfu_common::service::traits::Service as ServiceTrait;
use portfu_common::wrappers::cors::Cors;
use portfu_common::wrappers::metrics::MetricsWrapper;
use portfu_common::wrappers::rate_limits::RateLimiter;
use portfu_common::wrappers::sessions::{Session, SessionState};
use serde::Deserialize;
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
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
    ) -> Pin<Box<dyn Future<Output = Result<Response, PortfuError>> + 'a + Send + Sync>> {
        Box::pin(async move { Ok(Response::ok("ok")) })
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
    assert_eq!(server.middleware.len(), 3);

    let _metrics = MetricsWrapper;
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

async fn request_with_session(token: Option<OAuthToken>) -> Request {
    let mut request = basic_request();
    let session = Arc::new(RwLock::new(Session::default()));
    if let Some(token) = token {
        let identity = OAuthIdentity {
            provider: OAUTH::CUSTOM,
            subject: "user-1".to_string(),
            username: Some("user".to_string()),
            email: Some("user@example.com".to_string()),
            role: Some("user".to_string()),
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

use http::Method;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use portfu_common::error::PortfuError;
use portfu_common::router::filter::{self, FilterResult, traits::Filter as FilterTrait};
use portfu_common::router::route::Route;
use portfu_common::service::builder::ServiceBuilder;
use portfu_common::service::request::{Body, FromRequest, Json, Query, Request, RequestType};
use portfu_common::service::response::{JsonResponse, Response, Serialized};
use portfu_common::service::traits::Service as ServiceTrait;
use serde::Deserialize;
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

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

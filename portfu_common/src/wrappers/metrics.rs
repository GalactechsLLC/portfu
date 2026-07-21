use crate::error::PortfuError;
use crate::router::filter;
use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::server::builder::ServerBuilder;
use crate::service::builder::ServiceBuilder;
use crate::service::request::Request;
use crate::service::response::Response;
use crate::service::traits::Service;
use http::HeaderValue;
use http::header::CONTENT_TYPE;
use log::error;
use once_cell::sync::Lazy;
use prometheus::{HistogramOpts, HistogramVec, Registry, TextEncoder};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

#[derive(Copy, Clone)]
struct RequestTracking {
    timer: Instant,
    request_size: f64,
}

pub static REGISTRY: Lazy<Registry> = Lazy::new(|| {
    let instance_id = Uuid::new_v4();
    Registry::new_custom(
        Some(String::from("portfu_metrics")),
        Some(std::collections::HashMap::from([(
            "instance".to_string(),
            instance_id.to_string(),
        )])),
    )
    .unwrap()
});

static RESPONSE_TIMES: Lazy<Arc<HistogramVec>> = Lazy::new(|| {
    Arc::new(
        HistogramVec::new(
            HistogramOpts::new("response_times_histogram", "Response Times"),
            &["method", "path", "status_code"],
        )
        .inspect(|g: &HistogramVec| {
            REGISTRY.register(Box::new(g.clone())).unwrap_or(());
        })
        .unwrap(),
    )
});

static REQUEST_SIZES: Lazy<Arc<HistogramVec>> = Lazy::new(|| {
    Arc::new(
        HistogramVec::new(
            HistogramOpts::new("request_sizes_histogram", "Request Sizes").buckets(size_buckets()),
            &["method", "path", "status_code"],
        )
        .inspect(|g: &HistogramVec| {
            REGISTRY.register(Box::new(g.clone())).unwrap_or(());
        })
        .unwrap(),
    )
});

static RESPONSE_SIZES: Lazy<Arc<HistogramVec>> = Lazy::new(|| {
    Arc::new(
        HistogramVec::new(
            HistogramOpts::new("response_sizes_histogram", "Response Sizes")
                .buckets(size_buckets()),
            &["method", "path", "status_code"],
        )
        .inspect(|g: &HistogramVec| {
            REGISTRY.register(Box::new(g.clone())).unwrap_or(());
        })
        .unwrap(),
    )
});

#[derive(Default)]
pub struct MetricsWrapper;

impl Middleware for MetricsWrapper {
    fn name(&self) -> &str {
        "MetricsWrapper"
    }

    fn before<'a>(
        &'a self,
        request: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        Box::pin(async move {
            request.insert(RequestTracking {
                timer: Instant::now(),
                request_size: observed_body_size(request.body_size_hint()),
            });
            Ok(MiddlewareResult::Continue)
        })
    }

    fn after_with_request<'a>(
        &'a self,
        request: &'a Request,
        response: &'a mut Response,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        Box::pin(async move {
            let status = response.status();
            let labels = [
                request.method().as_str(),
                request.uri().path(),
                status.as_str(),
            ];
            let tracking = request.get::<RequestTracking>().copied();
            let request_size = tracking
                .map(|tracking| tracking.request_size)
                .unwrap_or_else(|| observed_body_size(request.body_size_hint()));
            REQUEST_SIZES
                .with_label_values(&labels)
                .observe(request_size);
            RESPONSE_SIZES
                .with_label_values(&labels)
                .observe(observed_body_size(response.body_size_hint()));
            if let Some(tracking) = tracking {
                RESPONSE_TIMES
                    .with_label_values(&labels)
                    .observe(Instant::now().duration_since(tracking.timer).as_secs_f64());
            }
            Ok(MiddlewareResult::Continue)
        })
    }

    fn after<'a>(
        &'a self,
        response: &'a mut Response,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + 'a + Send + Sync>>
    {
        Box::pin(async move {
            RESPONSE_SIZES
                .with_label_values(&["UNKNOWN", "UNKNOWN", response.status().as_str()])
                .observe(observed_body_size(response.body_size_hint()));
            Ok(MiddlewareResult::Continue)
        })
    }
}

pub struct MetricsEndpoint;

impl Service for MetricsEndpoint {
    fn name(&self) -> &str {
        "MetricsEndpoint"
    }

    fn serve<'a>(
        &'a self,
        _request: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Response, PortfuError>> + 'a + Send>> {
        Box::pin(async move {
            let encoder = TextEncoder::new();
            match encoder.encode_to_string(&REGISTRY.gather()) {
                Ok(body) => {
                    let mut response = Response::ok(body);
                    response.headers_mut().insert(
                        CONTENT_TYPE,
                        HeaderValue::from_static("text/plain; version=0.0.4"),
                    );
                    Ok(response)
                }
                Err(e) => {
                    error!("Failed to gather metrics data: {e:?}");
                    Ok(Response::internal_error("Failed to gather metrics data"))
                }
            }
        })
    }
}

pub struct MetricsServerBuilder {
    builder: ServerBuilder,
    path: String,
    wrapper: MetricsWrapper,
}

impl MetricsServerBuilder {
    pub fn path<S: Into<String>>(mut self, path: S) -> Self {
        self.path = path.into();
        self
    }

    pub fn finish_metrics(self) -> ServerBuilder {
        let service = ServiceBuilder::new(self.path.as_str())
            .name("metrics_endpoint")
            .filter(filter::method::GET.clone())
            .handler(Arc::new(MetricsEndpoint))
            .build();
        self.builder.service(service).wrap(Arc::new(self.wrapper))
    }

    pub fn build(self) -> crate::server::Server {
        self.finish_metrics().build()
    }
}

impl ServerBuilder {
    pub fn enable_metrics(self) -> MetricsServerBuilder {
        MetricsServerBuilder {
            builder: self,
            path: "/metrics".to_string(),
            wrapper: MetricsWrapper,
        }
    }
}

fn observed_body_size(size_hint: hyper::body::SizeHint) -> f64 {
    if let Some(exact) = size_hint.exact() {
        exact as f64
    } else if let Some(upper) = size_hint.upper() {
        upper as f64
    } else if size_hint.lower() > 0 {
        size_hint.lower() as f64
    } else {
        -1.0
    }
}

fn size_buckets() -> Vec<f64> {
    vec![
        0f64,
        1024f64,
        16f64 * 1024f64,
        64f64 * 1024f64,
        128f64 * 1024f64,
        256f64 * 1024f64,
        512f64 * 1024f64,
        1024f64 * 1024f64,
        8f64 * 1024f64 * 1024f64,
        16f64 * 1024f64 * 1024f64,
    ]
}

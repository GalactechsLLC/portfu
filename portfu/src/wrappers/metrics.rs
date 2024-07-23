use crate::filters::method::GET;
use crate::prelude::ServiceData;
use async_trait::async_trait;
use http::header::CONTENT_TYPE;
use http::{Extensions, HeaderValue};
use http_body_util::{BodyExt, Full};
use hyper::body::Body;
use log::error;
use once_cell::sync::Lazy;
use pfcore::service::{BodyType, MutBody, ServiceBuilder};
use prometheus::{self, HistogramOpts, HistogramVec, Registry, TextEncoder};
use std::io::{Error, ErrorKind};
use std::sync::Arc;
use std::time::Instant;
use pfcore::{IntoStreamBody, ServiceHandler, ServiceRegister, ServiceRegistry, ServiceType};
use pfcore::wrappers::{WrapperFn, WrapperResult};
use uuid::Uuid;

#[derive(Copy, Clone)]
struct TrackingStruct {
    timer: Instant,
}

pub static REGISTRY: Lazy<Registry> = Lazy::new(|| {
    let instance_id = Uuid::new_v4();
    Registry::new_custom(
        Some(String::from("farm_gate")),
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
        .map(|g: HistogramVec| {
            REGISTRY.register(Box::new(g.clone())).unwrap_or(());
            g
        })
        .unwrap(),
    )
});

static REQUEST_SIZES: Lazy<Arc<HistogramVec>> = Lazy::new(|| {
    Arc::new(
        HistogramVec::new(
            HistogramOpts::new("request_sizes_histogram", "Request Sizes").buckets(vec![
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
            ]),
            &["method", "path", "status_code"],
        )
        .map(|g: HistogramVec| {
            REGISTRY.register(Box::new(g.clone())).unwrap_or(());
            g
        })
        .unwrap(),
    )
});

static RESPONSE_SIZES: Lazy<Arc<HistogramVec>> = Lazy::new(|| {
    Arc::new(
        HistogramVec::new(
            HistogramOpts::new("response_sizes_histogram", "Response Sizes").buckets(vec![
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
            ]),
            &["method", "path", "status_code"],
        )
        .map(|g: HistogramVec| {
            REGISTRY.register(Box::new(g.clone())).unwrap_or(());
            g
        })
        .unwrap(),
    )
});
pub struct MetricsEndpoint;
#[async_trait]
impl ServiceHandler for MetricsEndpoint {
    fn name(&self) -> &str {
        "MetricsWrapper"
    }

    async fn handle(
        &self,
        mut data: ServiceData,
    ) -> Result<pfcore::ServiceData, (ServiceData, Error)> {
        data.response.headers_mut().insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; version=0.0.4"),
        );
        let encoder = TextEncoder::new();
        match encoder.encode_to_string(&REGISTRY.gather()) {
            Ok(v) => {
                data.response.set_body(BodyType::Stream(v.stream_body()));
                Ok(data)
            }
            Err(e) => Err((
                data,
                Error::new(
                    ErrorKind::InvalidData,
                    format!("Failed to Gather Metrics Data: {e:?}"),
                ),
            )),
        }
    }

    fn service_type(&self) -> ServiceType {
        ServiceType::API
    }
}
impl ServiceRegister for MetricsEndpoint {
    fn register(self, service_registry: &mut ServiceRegistry, shared_state: Extensions) {
        let __resource = ServiceBuilder::new("/metrics")
            .name("metrics_endpoint")
            .extend_state(shared_state.clone())
            .filter(GET.clone())
            .handler(Arc::new(self))
            .build();
        service_registry.register(__resource);
    }
}
#[derive(Default)]
pub struct MetricsWrapper {}

#[async_trait]
impl WrapperFn for MetricsWrapper {
    fn name(&self) -> &str {
        "SessionWrapper"
    }

    async fn before(&self, data: &mut ServiceData) -> WrapperResult {
        data.request.insert(TrackingStruct {
            timer: Instant::now(),
        });
        let body = data.request.consume();
        let new_body = convert_to_fixed(body).await;
        match &new_body {
            BodyType::Stream(_) => REQUEST_SIZES
                .with_label_values(&[
                    data.request.request.method().as_str(),
                    data.request.request.uri().path(),
                    data.response.status().as_str(),
                ])
                .observe(-1f64),
            BodyType::Sized(s) => REQUEST_SIZES
                .with_label_values(&[
                    data.request.request.method().as_str(),
                    data.request.request.uri().path(),
                    data.response.status().as_str(),
                ])
                .observe(s.size_hint().exact().unwrap_or_default() as f64),
            BodyType::Empty => REQUEST_SIZES
                .with_label_values(&[
                    data.request.request.method().as_str(),
                    data.request.request.uri().path(),
                    data.response.status().as_str(),
                ])
                .observe(0f64),
        }
        data.request.set_body(new_body);
        WrapperResult::Continue
    }

    async fn after(&self, data: &mut ServiceData) -> WrapperResult {
        let body = data.response.consume();
        let new_body = convert_to_fixed(body).await;
        match &new_body {
            BodyType::Stream(_) => RESPONSE_SIZES
                .with_label_values(&[
                    data.request.request.method().as_str(),
                    data.request.request.uri().path(),
                    data.response.status().as_str(),
                ])
                .observe(-1f64),
            BodyType::Sized(s) => RESPONSE_SIZES
                .with_label_values(&[
                    data.request.request.method().as_str(),
                    data.request.request.uri().path(),
                    data.response.status().as_str(),
                ])
                .observe(s.size_hint().exact().unwrap_or_default() as f64),
            BodyType::Empty => RESPONSE_SIZES
                .with_label_values(&[
                    data.request.request.method().as_str(),
                    data.request.request.uri().path(),
                    data.response.status().as_str(),
                ])
                .observe(0f64),
        }
        data.response.set_body(new_body);
        if let Some(tracking_struct) = data.request.remove::<TrackingStruct>() {
            RESPONSE_TIMES
                .with_label_values(&[
                    data.request.request.method().as_str(),
                    data.request.request.uri().path(),
                    data.response.status().as_str(),
                ])
                .observe(
                    Instant::now()
                        .duration_since(tracking_struct.timer)
                        .as_secs_f64(),
                );
        }
        WrapperResult::Continue
    }
}

async fn convert_to_fixed(body: BodyType) -> BodyType {
    match body {
        BodyType::Stream(s) => match s.collect().await {
            Ok(body) => BodyType::Sized(Full::from(body.to_bytes())),
            Err(e) => {
                error!("Failed to consume body in metrics: {e:?}");
                BodyType::Empty
            }
        },
        BodyType::Sized(s) => BodyType::Sized(s),
        BodyType::Empty => BodyType::Empty,
    }
}

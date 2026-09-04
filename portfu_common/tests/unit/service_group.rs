use crate::error::PortfuError;
use crate::router::filter::{FilterResult, traits::Filter};
use crate::router::middleware::{Middleware, MiddlewareResult};
use crate::service::builder::ServiceBuilder;
use crate::service::group::ServiceGroup;
use crate::service::request::Request;
use crate::service::response::Response;
use std::pin::Pin;
use std::sync::Arc;

struct NamedFilter(&'static str);

impl Filter for NamedFilter {
    fn name(&self) -> &str {
        self.0
    }

    fn filter<'a>(
        &'a self,
        _request: &'a Request,
    ) -> Pin<Box<dyn Future<Output = FilterResult> + Send + Sync + 'a>> {
        Box::pin(async { FilterResult::Allow })
    }
}

struct NamedMiddleware(&'static str);

impl Middleware for NamedMiddleware {
    fn name(&self) -> &str {
        self.0
    }

    fn before<'a>(
        &'a self,
        _request: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + Send + Sync + 'a>>
    {
        Box::pin(async { Ok(MiddlewareResult::Continue) })
    }

    fn after<'a>(
        &'a self,
        _response: &'a mut Response,
    ) -> Pin<Box<dyn Future<Output = Result<MiddlewareResult, PortfuError>> + Send + Sync + 'a>>
    {
        Box::pin(async { Ok(MiddlewareResult::Continue) })
    }
}

fn service(path: &str) -> crate::service::Service {
    ServiceBuilder::new(path).build()
}

#[test]
fn configuration_applies_only_to_later_services() {
    let services = ServiceGroup::new()
        .service(service("/public"))
        .filter(Arc::new(NamedFilter("auth")))
        .wrap(Arc::new(NamedMiddleware("session")))
        .service(service("/private"))
        .into_iter()
        .collect::<Vec<_>>();

    assert_eq!(services[0].filters.len(), 0);
    assert_eq!(services[0].middleware.len(), 0);
    assert_eq!(services[1].filters[0].name(), "auth");
    assert_eq!(services[1].middleware[0].name(), "session");
}

#[test]
fn subgroup_inherits_parent_configuration_without_affecting_siblings() {
    let services = ServiceGroup::new()
        .filter(Arc::new(NamedFilter("parent")))
        .wrap(Arc::new(NamedMiddleware("parent")))
        .sub_group(
            ServiceGroup::new()
                .filter(Arc::new(NamedFilter("child")))
                .wrap(Arc::new(NamedMiddleware("child")))
                .service(ServiceBuilder::new("/nested").scope("tenant").build()),
        )
        .service(ServiceBuilder::new("/sibling").scope("other").build())
        .into_iter()
        .collect::<Vec<_>>();

    assert_eq!(services[0].filters.len(), 2);
    assert_eq!(services[0].filters[0].name(), "child");
    assert_eq!(services[0].filters[1].name(), "parent");
    assert_eq!(services[0].middleware[0].name(), "child");
    assert_eq!(services[0].middleware[1].name(), "parent");
    assert_eq!(services[0].scope(), "tenant");
    assert_eq!(services[1].filters.len(), 1);
    assert_eq!(services[1].filters[0].name(), "parent");
    assert_eq!(services[1].middleware[0].name(), "parent");
    assert_eq!(services[1].scope(), "other");
}

#[test]
fn shared_state_replaces_an_earlier_value_of_the_same_type() {
    let (state, _) = ServiceGroup::new()
        .shared_state("first".to_string())
        .shared_state("second".to_string())
        .into_parts();

    assert_eq!(
        state.get::<Arc<String>>().map(|value| value.as_str()),
        Some("second")
    );
}

#[test]
fn subgroup_state_is_retained_by_the_enclosing_group() {
    let (state, _) = ServiceGroup::new()
        .sub_group(ServiceGroup::new().shared_state(42_u32))
        .into_parts();

    assert_eq!(state.get::<Arc<u32>>().map(|value| **value), Some(42));
}

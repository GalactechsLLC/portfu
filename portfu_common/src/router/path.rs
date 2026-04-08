use crate::error::PortfuError;
use crate::service::request::{FromRequest, Request};
use std::fmt::{Display, Formatter};
use std::marker::PhantomData;
use std::pin::Pin;

pub struct Path(String);
impl Path {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn inner(self) -> String {
        self.0
    }
    pub fn value(&self) -> &str {
        self.0.as_str()
    }
}
impl Display for Path {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}
impl<N: PathName> From<PathImpl<N>> for Path {
    fn from(path_impl: PathImpl<N>) -> Self {
        Self(path_impl.0)
    }
}

pub trait PathName {
    const NAME: &'static str;
}
#[derive(Clone)]
pub struct PathImpl<N: PathName>(String, PhantomData<N>);
impl<N: PathName> PathImpl<N> {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into(), PhantomData)
    }
    pub fn name(&self) -> &'static str {
        N::NAME
    }
    pub fn value(&self) -> &str {
        self.0.as_str()
    }
    pub fn into_path(self) -> Path {
        Path(self.0)
    }
}
impl<N: PathName> Display for PathImpl<N> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}
impl<N: PathName> FromRequest<Request> for PathImpl<N> {
    type Error = PortfuError;
    fn try_from<'a>(
        request: &'a mut Request,
    ) -> Pin<Box<dyn Future<Output = Result<Self, Self::Error>> + 'a + Send + Sync>> {
        Box::pin(async {
            request
                .route()
                .extract(request.uri().path(), N::NAME)
                .map(PathImpl::new)
                .ok_or(PortfuError::Parsing(format!(
                    "Failed to parse path variable {} in path {}",
                    N::NAME,
                    request.uri().path()
                )))
        })
    }
}

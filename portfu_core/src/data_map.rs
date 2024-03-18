use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::net::SocketAddr;
use http::Response;
use http_body_util::Full;
use hyper::body::Bytes;
use crate::service::ServiceRequest;

pub struct DynMap {
    data_map: HashMap<TypeId, Box<dyn Any + Send + Sync>>
}
impl DynMap {
    #[inline]
    pub fn new() -> DynMap {
        DynMap {
            data_map: HashMap::default(),
        }
    }
    pub fn insert<T: Send + Sync + 'static>(&mut self, val: T) -> Option<T> {
        self.data_map
            .insert(TypeId::of::<T>(), Box::new(val))
            .and_then(boxed_to_owned)
    }
    pub fn contains<T: Send + Sync + 'static>(&self) -> bool {
            self.data_map.contains_key(&TypeId::of::<T>())
    }

    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.data_map
            .get(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_ref())
    }

    pub fn get_mut<T: Send + Sync + 'static>(&mut self) -> Option<&mut T> {
        self.data_map
            .get_mut(&TypeId::of::<T>())
            .and_then(|boxed| boxed.downcast_mut())
    }

    pub fn remove<T: Send + Sync + 'static>(&mut self) -> Option<T> {
        self.data_map.remove(&TypeId::of::<T>()).and_then(boxed_to_owned)
    }
    #[inline]
    pub fn clear(&mut self) {
        self.data_map.clear();
    }

    /// Extends self with the items from another `Extensions`.
    pub fn extend(&mut self, other: DynMap) {
        self.data_map.extend(other.data_map);
    }

}

fn boxed_to_owned<T: Send + Sync + 'static>(boxed: Box<dyn Any +Send + Sync>) -> Option<T> {
    boxed.downcast().ok().map(|boxed| *boxed)
}

pub struct DynArg<T: Any + Send + Sync + 'static> {
    name: String,
    value: PhantomData<T>
}
impl<T: Any + Send + Sync + 'static> DynArg<T> {
    pub fn new(name: String) -> Self {
        DynArg {
            name,
            value: PhantomData::default()
        }
    }

    pub fn name(&self) -> &str {
        let type_id = self.value.type_id();
        if TypeId::of::<SocketAddr>() == type_id {
            "address_id"
        } else if TypeId::of::<ServiceRequest>() == type_id  {
            "request"
        } else if TypeId::of::<Response<Full<Bytes>>>() == type_id {
            "response"
        } else {
            &self.name
        }
    }

    pub fn type_id(self) -> TypeId {
        self.value.type_id()
    }
}

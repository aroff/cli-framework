use std::fmt;
use std::sync::Arc;

use tower::Layer;

/// A boxed, cloneable `Layer<T, Service = T>`.
///
/// This is intentionally local to `cli-framework` so enabling `api-server` does not require
/// globally patching the `tower` crate (which could affect other features such as `mcp-server`).
#[derive(Clone)]
pub struct BoxCloneLayer<T> {
    f: Arc<dyn Fn(T) -> T + Send + Sync + 'static>,
}

impl<T> BoxCloneLayer<T> {
    pub fn new<L>(layer: L) -> Self
    where
        L: Layer<T, Service = T> + Clone + Send + Sync + 'static,
        T: 'static,
    {
        let f = Arc::new(move |svc: T| layer.clone().layer(svc));
        Self { f }
    }
}

impl<T> Layer<T> for BoxCloneLayer<T> {
    type Service = T;

    fn layer(&self, inner: T) -> Self::Service {
        (self.f)(inner)
    }
}

impl<T> fmt::Debug for BoxCloneLayer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoxCloneLayer").finish()
    }
}

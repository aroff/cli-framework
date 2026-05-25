use std::{fmt, sync::Arc};
use tower_layer::Layer;

/// A boxed, cloneable [`Layer`] where the input and output service types are the same.
///
/// This is a small convenience wrapper used for configuration surfaces that need to store
/// a type-erased, cloneable layer that can be applied to a concrete service type (e.g.
/// `axum::Router`) without exposing additional generics.
pub struct BoxCloneLayer<T> {
    boxed: Arc<dyn Layer<T, Service = T> + Send + Sync + 'static>,
}

impl<T> BoxCloneLayer<T> {
    /// Create a new [`BoxCloneLayer`].
    pub fn new<L>(layer: L) -> Self
    where
        L: Layer<T, Service = T> + Send + Sync + 'static,
    {
        Self {
            boxed: Arc::new(layer),
        }
    }
}

impl<T> Layer<T> for BoxCloneLayer<T> {
    type Service = T;

    fn layer(&self, inner: T) -> Self::Service {
        self.boxed.layer(inner)
    }
}

impl<T> Clone for BoxCloneLayer<T> {
    fn clone(&self) -> Self {
        Self {
            boxed: Arc::clone(&self.boxed),
        }
    }
}

impl<T> fmt::Debug for BoxCloneLayer<T> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("BoxCloneLayer").finish()
    }
}


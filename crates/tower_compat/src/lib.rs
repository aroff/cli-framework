pub use tower_real::*;

pub mod util {
    pub use tower_real::util::*;

    use axum::{body::Body, http::Request, response::Response};
    use std::convert::Infallible;
    use tower_layer::Layer;
    use tower_service::Service;

    /// Compatibility shim for specs that require `tower::util::BoxCloneLayer<In>`.
    ///
    /// This type is intentionally scoped to axum's `Request<Body>`/`Response`/`Infallible` shape.
    #[derive(Clone, Debug)]
    pub struct BoxCloneLayer<In> {
        inner: tower_real::util::BoxCloneSyncServiceLayer<In, Request<Body>, Response, Infallible>,
    }

    impl<In> BoxCloneLayer<In> {
        pub fn new<L>(layer: L) -> Self
        where
            L: Layer<In> + Send + Sync + 'static,
            L::Service: Service<Request<Body>, Response = Response, Error = Infallible>
                + Clone
                + Send
                + Sync
                + 'static,
            <L::Service as Service<Request<Body>>>::Future: Send + 'static,
        {
            Self {
                inner: tower_real::util::BoxCloneSyncServiceLayer::new(layer),
            }
        }
    }

    impl<In> Layer<In> for BoxCloneLayer<In> {
        type Service = tower_real::util::BoxCloneSyncService<Request<Body>, Response, Infallible>;

        fn layer(&self, inner: In) -> Self::Service {
            self.inner.layer(inner)
        }
    }
}

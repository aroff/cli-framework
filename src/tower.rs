//! Internal shim around the upstream `tower` crate.
//!
//! This exists to provide `tower::util::BoxCloneLayer<S>` which is referenced by the
//! `api-server` public API surface, while the upstream tower crate provides
//! `BoxCloneSyncServiceLayer`/`BoxCloneServiceLayer` rather than `BoxCloneLayer`.

pub use ::tower::*;

pub mod util {
    pub use ::tower::util::*;

    /// A cloneable, type-erased `Layer` for Axum routers.
    ///
    /// This is a shim alias used by `cli_framework::api::ApiServerBuilder::auth(...)`.
    pub type BoxCloneLayer<S> = ::tower::util::BoxCloneSyncServiceLayer<
        S,
        axum::http::Request<axum::body::Body>,
        axum::response::Response,
        std::convert::Infallible,
    >;
}

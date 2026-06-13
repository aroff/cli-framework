//! Generic MCP resource serving.
//!
//! A [`ResourceRegistry`] maps a resource URI to a provider closure that
//! produces the resource body on demand. Providers return a [`UiResource`]
//! carrying the MIME type, a text or blob payload, and an optional opaque
//! per-resource `_meta` value; that value is placed verbatim in the
//! `contents[]._meta` slot of the `resources/read` response. cli-framework
//! does not interpret the `_meta` value — the consumer owns its shape.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Body of a single MCP resource produced by a provider.
#[derive(Debug, Clone)]
pub struct UiResource {
    /// MIME type of the payload (e.g. `text/html`).
    pub mime_type: String,
    /// Either a UTF-8 text body or a base64-encoded blob.
    pub body: UiResourceBody,
    /// Optional opaque per-resource `_meta` passthrough. Surfaces verbatim as
    /// `contents[]._meta` in the `resources/read` reply. cli-framework does not
    /// inspect it; the consumer owns its shape.
    pub meta: Option<Value>,
}

/// Text vs. binary payload for a [`UiResource`].
#[derive(Debug, Clone)]
pub enum UiResourceBody {
    /// UTF-8 text content (the common case: a single-file HTML host shell).
    Text(String),
    /// Base64-encoded binary content.
    Blob(String),
}

impl UiResource {
    /// Construct a `text/html` resource with no `_meta`.
    pub fn html(body: impl Into<String>) -> Self {
        Self {
            mime_type: "text/html".to_string(),
            body: UiResourceBody::Text(body.into()),
            meta: None,
        }
    }

    /// Construct a text resource with an explicit MIME type.
    pub fn text(mime_type: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            mime_type: mime_type.into(),
            body: UiResourceBody::Text(body.into()),
            meta: None,
        }
    }

    /// Construct a blob resource (base64-encoded body) with an explicit MIME type.
    pub fn blob(mime_type: impl Into<String>, base64_body: impl Into<String>) -> Self {
        Self {
            mime_type: mime_type.into(),
            body: UiResourceBody::Blob(base64_body.into()),
            meta: None,
        }
    }

    /// Attach an opaque `_meta` value to this resource. Emitted verbatim at
    /// `contents[]._meta`; cli-framework never inspects it.
    pub fn with_meta(mut self, meta: Value) -> Self {
        self.meta = Some(meta);
        self
    }
}

/// A provider closure: given the requested URI, produce the resource body.
///
/// Returning `None` signals the URI is registered but currently unavailable;
/// `read_resource` maps that to a not-found error.
pub type ResourceProvider = Arc<dyn Fn(&str) -> Option<UiResource> + Send + Sync>;

/// Metadata for a registered resource as it appears in `resources/list`.
#[derive(Debug, Clone)]
pub struct ResourceListing {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

struct Entry {
    listing: ResourceListing,
    provider: ResourceProvider,
}

/// Registry of resources, held alongside the tool registry.
///
/// Two kinds of registration: **exact** (a fixed URI) and **prefix** (a provider
/// that serves every URI under a base, e.g. `ui://es/invoice/detail/` serving
/// `ui://es/invoice/detail/<id>`). On `read`, an exact match wins; otherwise the
/// **longest** matching prefix provider is invoked with the full requested URI,
/// so a single stateless provider can render per-record resources on demand.
#[derive(Default, Clone)]
pub struct ResourceRegistry {
    entries: HashMap<String, Arc<Entry>>,
    prefixes: Vec<(String, Arc<Entry>)>,
}

impl ResourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a resource at `uri`, served by `provider`.
    ///
    /// `name`/`description`/`mime_type` populate the `resources/list` entry.
    /// Re-registering the same URI replaces the prior entry.
    pub fn register<F>(
        &mut self,
        uri: impl Into<String>,
        name: impl Into<String>,
        description: Option<String>,
        mime_type: Option<String>,
        provider: F,
    ) -> &mut Self
    where
        F: Fn(&str) -> Option<UiResource> + Send + Sync + 'static,
    {
        let uri = uri.into();
        let listing = ResourceListing {
            uri: uri.clone(),
            name: name.into(),
            description,
            mime_type,
        };
        self.entries.insert(
            uri,
            Arc::new(Entry {
                listing,
                provider: Arc::new(provider),
            }),
        );
        self
    }

    /// Convenience: register a static text resource (no provider closure).
    pub fn register_static(
        &mut self,
        uri: impl Into<String>,
        name: impl Into<String>,
        resource: UiResource,
    ) -> &mut Self {
        let uri = uri.into();
        let mime_type = Some(resource.mime_type.clone());
        let res = resource;
        self.register(uri.clone(), name, None, mime_type, move |_| {
            Some(res.clone())
        })
    }

    /// Register a **prefix** provider: it serves every URI that starts with
    /// `prefix`, receiving the full requested URI so it can parse a trailing
    /// segment (e.g. a record id) and render on demand. The `listing` advertises
    /// the prefix itself as a template URI. Longest prefix wins on `read`.
    pub fn register_prefix<F>(
        &mut self,
        prefix: impl Into<String>,
        name: impl Into<String>,
        description: Option<String>,
        mime_type: Option<String>,
        provider: F,
    ) -> &mut Self
    where
        F: Fn(&str) -> Option<UiResource> + Send + Sync + 'static,
    {
        let prefix = prefix.into();
        let listing = ResourceListing {
            uri: prefix.clone(),
            name: name.into(),
            description,
            mime_type,
        };
        let entry = Arc::new(Entry {
            listing,
            provider: Arc::new(provider),
        });
        // Replace an existing provider for the same prefix, else append.
        if let Some(slot) = self.prefixes.iter_mut().find(|(p, _)| *p == prefix) {
            slot.1 = entry;
        } else {
            self.prefixes.push((prefix, entry));
        }
        self
    }

    /// All registered listings (exact + prefix templates), for `resources/list`.
    pub fn listings(&self) -> Vec<ResourceListing> {
        self.entries
            .values()
            .map(|e| e.listing.clone())
            .chain(self.prefixes.iter().map(|(_, e)| e.listing.clone()))
            .collect()
    }

    /// Resolve a URI to its current resource body. An exact registration wins;
    /// otherwise the longest matching prefix provider is invoked with `uri`.
    pub fn read(&self, uri: &str) -> Option<UiResource> {
        if let Some(entry) = self.entries.get(uri) {
            return (entry.provider)(uri);
        }
        let best = self
            .prefixes
            .iter()
            .filter(|(p, _)| uri.starts_with(p.as_str()))
            .max_by_key(|(p, _)| p.len())?;
        (best.1.provider)(uri)
    }

    /// Whether any resource (exact or prefix) is registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.prefixes.is_empty()
    }

    /// Number of registered resources (exact + prefix).
    pub fn len(&self) -> usize {
        self.entries.len() + self.prefixes.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_provider_serves_subpaths_with_full_uri() {
        let mut r = ResourceRegistry::new();
        r.register_prefix(
            "ui://es/invoice/detail/",
            "Invoice detail",
            None,
            Some("text/html".into()),
            |uri| {
                let id = uri.rsplit('/').next().unwrap_or_default().to_string();
                Some(UiResource::html(format!(
                    "<article data-id=\"{id}\"></article>"
                )))
            },
        );

        // a concrete id-bearing URI routes to the prefix provider, which sees the full URI
        let res = r.read("ui://es/invoice/detail/abc-123").expect("served");
        match res.body {
            UiResourceBody::Text(html) => assert!(html.contains("abc-123")),
            _ => panic!("expected text"),
        }
        // unrelated URI does not match
        assert!(r.read("ui://es/customer/detail/x").is_none());
    }

    #[test]
    fn exact_match_wins_over_prefix() {
        let mut r = ResourceRegistry::new();
        r.register_prefix("ui://es/", "catchall", None, None, |_| {
            Some(UiResource::html("prefix"))
        });
        r.register_static("ui://es/exact", "exact", UiResource::html("exact"));
        match r.read("ui://es/exact").unwrap().body {
            UiResourceBody::Text(t) => assert_eq!(t, "exact"),
            _ => panic!(),
        }
    }

    #[test]
    fn longest_prefix_wins() {
        let mut r = ResourceRegistry::new();
        r.register_prefix("ui://es/", "short", None, None, |_| {
            Some(UiResource::html("short"))
        });
        r.register_prefix("ui://es/invoice/", "long", None, None, |_| {
            Some(UiResource::html("long"))
        });
        match r.read("ui://es/invoice/detail/1").unwrap().body {
            UiResourceBody::Text(t) => assert_eq!(t, "long"),
            _ => panic!(),
        }
    }
}

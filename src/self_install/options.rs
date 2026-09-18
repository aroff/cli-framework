//! What an application declares about where its releases live, and how
//! its installs behave.

use serde::{Deserialize, Serialize};

/// Where release archives are published.
///
/// `self update` downloads from it; the install receipt records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ReleaseSource {
    /// GitHub releases of `owner/name`.
    Github {
        repo: String,
        /// GitHub Enterprise API base; `None` means `https://api.github.com`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_base: Option<String>,
    },
    /// A plain HTTPS mirror serving the same asset names plus `latest.json`.
    Http { base_url: String },
}

/// The application's self-install declaration, passed to
/// `AppBuilder::with_self_install`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfInstallOptions {
    pub(crate) source: ReleaseSource,
    pub(crate) tag_prefix: String,
    pub(crate) asset_template: String,
    pub(crate) completions: bool,
    pub(crate) public_key: Option<String>,
    pub(crate) update_notice: bool,
    pub(crate) apps_and_features: bool,
    pub(crate) publisher: Option<String>,
}

/// The default archive name: `<app>-<target>.tar.gz` or `.zip`.
pub const DEFAULT_ASSET_TEMPLATE: &str = "{app}-{target}.{ext}";

impl SelfInstallOptions {
    /// Releases published on GitHub under `owner/name`.
    pub fn github(repo: impl Into<String>) -> Self {
        Self::with_source(ReleaseSource::Github {
            repo: repo.into(),
            api_base: None,
        })
    }

    /// Releases served by an HTTPS mirror at `base_url`.
    pub fn http(base_url: impl Into<String>) -> Self {
        Self::with_source(ReleaseSource::Http {
            base_url: base_url.into(),
        })
    }

    fn with_source(source: ReleaseSource) -> Self {
        Self {
            source,
            tag_prefix: "v".to_string(),
            asset_template: DEFAULT_ASSET_TEMPLATE.to_string(),
            completions: false,
            public_key: None,
            update_notice: false,
            apps_and_features: true,
            publisher: None,
        }
    }

    /// GitHub Enterprise API base, for example `https://ghe.example.com/api/v3`.
    /// Ignored for an HTTP source.
    pub fn github_api_base(mut self, api_base: impl Into<String>) -> Self {
        if let ReleaseSource::Github {
            api_base: ref mut slot,
            ..
        } = self.source
        {
            *slot = Some(api_base.into());
        }
        self
    }

    /// Tag prefix before the version; `"v"` by default, `"myapp-v"` in a
    /// monorepo.
    pub fn tag_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.tag_prefix = prefix.into();
        self
    }

    /// Archive file name template. Placeholders: `{app}`, `{target}`,
    /// `{version}` and `{ext}` (`tar.gz` or `zip`).
    pub fn asset_template(mut self, template: impl Into<String>) -> Self {
        self.asset_template = template.into();
        self
    }

    /// Install bash and fish completion files during `self install`.
    pub fn completions(mut self, enabled: bool) -> Self {
        self.completions = enabled;
        self
    }

    /// A minisign public key (the `RW...` line of a `.pub` file). When set,
    /// `self update` and `--from` require `SHA256SUMS.minisig` beside
    /// `SHA256SUMS` and refuse a release it does not verify.
    pub fn public_key(mut self, key: Option<&str>) -> Self {
        self.public_key = key.map(str::to_string);
        self
    }

    /// After a successful command, print one stderr line when a newer
    /// release exists (checked at most once a day). Off by default; see
    /// ADR 0080 for when it stays silent.
    pub fn update_notice(mut self, enabled: bool) -> Self {
        self.update_notice = enabled;
        self
    }

    /// Register a Windows Apps & Features entry for user installs. On by
    /// default; no effect on other platforms.
    pub fn apps_and_features(mut self, enabled: bool) -> Self {
        self.apps_and_features = enabled;
        self
    }

    /// The publisher Apps & Features shows. Defaults to the GitHub owner, or
    /// the application name for an HTTP source.
    pub fn publisher(mut self, publisher: impl Into<String>) -> Self {
        self.publisher = Some(publisher.into());
        self
    }

    pub(crate) fn publisher_name(&self, app: &str) -> String {
        if let Some(p) = &self.publisher {
            return p.clone();
        }
        match &self.source {
            ReleaseSource::Github { repo, .. } => repo.split('/').next().unwrap_or(app).to_string(),
            ReleaseSource::Http { .. } => app.to_string(),
        }
    }

    pub fn update_notice_enabled(&self) -> bool {
        self.update_notice
    }

    pub fn source(&self) -> &ReleaseSource {
        &self.source
    }

    pub fn get_tag_prefix(&self) -> &str {
        &self.tag_prefix
    }

    pub fn get_asset_template(&self) -> &str {
        &self.asset_template
    }

    pub fn completions_enabled(&self) -> bool {
        self.completions
    }

    /// The archive name for `app` at `version` on `target`.
    pub fn asset_name(&self, app: &str, version: &str, target: &str) -> String {
        let ext = if target.contains("windows") {
            "zip"
        } else {
            "tar.gz"
        };
        self.asset_template
            .replace("{app}", app)
            .replace("{target}", target)
            .replace("{version}", version.trim_start_matches('v'))
            .replace("{ext}", ext)
    }

    /// Reject declarations that could never work, at build time rather than
    /// on a user's machine.
    pub fn validate(&self) -> Result<(), String> {
        match &self.source {
            ReleaseSource::Github { repo, api_base } => {
                let mut parts = repo.split('/');
                let valid = matches!(
                    (parts.next(), parts.next(), parts.next()),
                    (Some(o), Some(n), None) if !o.is_empty() && !n.is_empty()
                );
                if !valid {
                    return Err(format!(
                        "self-install GitHub repo {repo:?} must be \"owner/name\""
                    ));
                }
                if let Some(base) = api_base {
                    check_https(base)?;
                }
            }
            ReleaseSource::Http { base_url } => check_https(base_url)?,
        }
        if !self.asset_template.contains("{target}") {
            return Err(format!(
                "self-install asset template {:?} must contain {{target}}",
                self.asset_template
            ));
        }
        if let Some(key) = &self.public_key {
            super::archive::check_public_key(key)?;
        }
        Ok(())
    }
}

/// HTTPS only, with one exception: loopback `http://` so a local mirror can
/// serve tests. Nothing on loopback crosses a network.
pub(crate) fn check_https(url: &str) -> Result<(), String> {
    if url.starts_with("https://") || is_loopback_http(url) {
        Ok(())
    } else {
        Err(format!("self-install URL {url:?} must use https://"))
    }
}

fn is_loopback_http(url: &str) -> bool {
    ["http://127.0.0.1", "http://localhost", "http://[::1]"]
        .iter()
        .any(|p| {
            url.strip_prefix(p).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with(':') || rest.starts_with('/')
            })
        })
}

/// The Rust target triple of the running binary, as release archives name
/// it. Linux reports `musl` or `gnu` from how this binary was built.
pub fn current_target() -> String {
    let arch = std::env::consts::ARCH;
    let rest = if cfg!(target_os = "macos") {
        "apple-darwin"
    } else if cfg!(all(target_os = "windows", target_env = "msvc")) {
        "pc-windows-msvc"
    } else if cfg!(target_os = "windows") {
        "pc-windows-gnu"
    } else if cfg!(all(target_os = "linux", target_env = "musl")) {
        "unknown-linux-musl"
    } else if cfg!(target_os = "linux") {
        "unknown-linux-gnu"
    } else {
        std::env::consts::OS
    };
    format!("{arch}-{rest}")
}

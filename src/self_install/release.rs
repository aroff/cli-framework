//! Finding a release and downloading its assets.
//!
//! Two sources share one contract (ADR 0080):
//!
//! - **GitHub**: the releases API of `owner/name`. Tags are filtered by the
//!   app's tag prefix, so a monorepo whose releases interleave several apps
//!   still resolves correctly.
//! - **HTTP mirror**: `<base>/latest.json` holds `{"version": "1.4.2"}` and
//!   optionally `"latest": "1.5.0-rc.1"` for the `latest` channel; assets live
//!   at `<base>/<tag>/<asset>`, the layout the installer scripts use.
//!
//! Downloads are HTTPS only (loopback `http` is allowed for tests), refuse a
//! redirect to plain HTTP, and send a GitHub token only as an
//! `Authorization` header to the GitHub API host. reqwest drops the header
//! when a redirect leaves that host.

use super::archive::{SIGNATURE_FILE, SUMS_FILE};
use super::options::{check_https, ReleaseSource};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

const DEFAULT_GITHUB_API: &str = "https://api.github.com";

/// What `self update` was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    /// The newest release that is not a prerelease.
    Stable,
    /// The newest release, prereleases included.
    Latest,
    /// Exactly this version.
    Exact(semver::Version),
}

impl Channel {
    /// Parse `stable`, `latest` or a version (`1.4.2`, `v1.4.2`).
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim() {
            "stable" => Ok(Self::Stable),
            "latest" => Ok(Self::Latest),
            other => semver::Version::parse(other.trim_start_matches('v'))
                .map(Self::Exact)
                .map_err(|e| format!("{other:?} is not stable, latest or a version: {e}")),
        }
    }

    /// The name recorded in the receipt, for a channel that is one.
    pub fn name(&self) -> Option<&'static str> {
        match self {
            Self::Stable => Some("stable"),
            Self::Latest => Some("latest"),
            Self::Exact(_) => None,
        }
    }
}

/// A release picked for this target, with where to fetch its three files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRelease {
    pub version: semver::Version,
    pub tag: String,
    pub asset: String,
    pub asset_url: String,
    pub sums_url: String,
    /// `None` when the GitHub release carries no signature asset.
    pub signature_url: Option<String>,
    /// Whether the URLs are GitHub API asset URLs, which need the token and
    /// `Accept: application/octet-stream`.
    pub via_api: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    #[error("{0}")]
    Invalid(String),
    #[error("GET {url}: {message}")]
    Http { url: String, message: String },
    #[error("GET {url}: HTTP {status}")]
    Status { url: String, status: u16 },
    #[error("no release matches {wanted} (tag prefix {prefix:?})")]
    NotFound { wanted: String, prefix: String },
    #[error("release {tag} has no asset {asset}")]
    MissingAsset { tag: String, asset: String },
    #[error("cannot write {path}: {source}")]
    Write {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
}

/// One release as the GitHub API lists it.
#[derive(Debug, Clone, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubAsset {
    pub name: String,
    /// API URL; downloads with `Accept: application/octet-stream`.
    pub url: String,
    pub browser_download_url: String,
}

/// `latest.json` on an HTTP mirror.
#[derive(Debug, Clone, Deserialize)]
struct LatestJson {
    version: String,
    #[serde(default)]
    latest: Option<String>,
}

/// The version a tag names, if it carries `prefix`.
pub fn tag_version(tag: &str, prefix: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.strip_prefix(prefix)?).ok()
}

/// Pick the newest release for `channel` among `releases`. Drafts never
/// count; `stable` also skips releases flagged as prereleases and versions
/// with a prerelease part.
pub fn pick_release<'a>(
    releases: &'a [GithubRelease],
    prefix: &str,
    channel: &Channel,
) -> Option<(semver::Version, &'a GithubRelease)> {
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| tag_version(&r.tag_name, prefix).map(|v| (v, r)))
        .filter(|(v, r)| match channel {
            Channel::Stable => !r.prerelease && v.pre.is_empty(),
            Channel::Latest => true,
            Channel::Exact(want) => v == want,
        })
        .max_by(|a, b| a.0.cmp(&b.0))
}

/// The HTTP client every self-install download uses.
pub fn client() -> Result<reqwest::Client, ReleaseError> {
    build_client(Duration::from_secs(30), Duration::from_secs(600))
}

/// The same client with one overall timeout, for the update notice.
pub fn client_with_timeout(timeout: Duration) -> Result<reqwest::Client, ReleaseError> {
    build_client(timeout, timeout)
}

fn build_client(connect: Duration, total: Duration) -> Result<reqwest::Client, ReleaseError> {
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            attempt.error("too many redirects")
        } else if check_https(attempt.url().as_str()).is_err() {
            let url = attempt.url().to_string();
            attempt.error(format!("refusing a redirect to a non-https URL: {url}"))
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .redirect(policy)
        .connect_timeout(connect)
        .timeout(total)
        .user_agent(concat!(
            "cli-framework-self-install/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|e| ReleaseError::Invalid(format!("cannot build the HTTP client: {e}")))
}

/// Everything needed to talk to a source.
pub struct Fetcher {
    client: reqwest::Client,
    /// Sent only to the GitHub API host.
    token: Option<String>,
}

impl Fetcher {
    pub fn new(token: Option<String>) -> Result<Self, ReleaseError> {
        Ok(Self {
            client: client()?,
            token,
        })
    }

    /// A fetcher with a caller-supplied client, for a short notice timeout.
    pub fn with_client(client: reqwest::Client, token: Option<String>) -> Self {
        Self { client, token }
    }

    async fn get(
        &self,
        url: &str,
        accept: &str,
        auth: bool,
    ) -> Result<reqwest::Response, ReleaseError> {
        check_https(url).map_err(ReleaseError::Invalid)?;
        let mut req = self.client.get(url).header(reqwest::header::ACCEPT, accept);
        if auth {
            if let Some(token) = &self.token {
                req = req.bearer_auth(token);
            }
        }
        let http_err = |message: String| ReleaseError::Http {
            url: url.to_string(),
            message,
        };
        let resp = req.send().await.map_err(|e| http_err(error_chain(&e)))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ReleaseError::Status {
                url: url.to_string(),
                status: status.as_u16(),
            });
        }
        Ok(resp)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        auth: bool,
    ) -> Result<T, ReleaseError> {
        let resp = self.get(url, "application/vnd.github+json", auth).await?;
        resp.json::<T>().await.map_err(|e| ReleaseError::Http {
            url: url.to_string(),
            message: format!("unexpected response: {e}"),
        })
    }

    /// Resolve `channel` against `source`. `asset_for` names the archive
    /// for a version, since an asset template may contain `{version}`.
    pub async fn resolve(
        &self,
        source: &ReleaseSource,
        tag_prefix: &str,
        asset_for: &(dyn Fn(&semver::Version) -> String + Sync),
        channel: &Channel,
    ) -> Result<ResolvedRelease, ReleaseError> {
        match source {
            ReleaseSource::Github { repo, api_base } => {
                let api = api_base
                    .as_deref()
                    .unwrap_or(DEFAULT_GITHUB_API)
                    .trim_end_matches('/');
                let release = match channel {
                    Channel::Exact(v) => {
                        let tag = format!("{tag_prefix}{v}");
                        let url = format!("{api}/repos/{repo}/releases/tags/{tag}");
                        match self.get_json::<GithubRelease>(&url, true).await {
                            Ok(r) => r,
                            Err(ReleaseError::Status { status: 404, .. }) => {
                                return Err(ReleaseError::NotFound {
                                    wanted: v.to_string(),
                                    prefix: tag_prefix.to_string(),
                                })
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    _ => {
                        let url = format!("{api}/repos/{repo}/releases?per_page=100");
                        let all: Vec<GithubRelease> = self.get_json(&url, true).await?;
                        pick_release(&all, tag_prefix, channel)
                            .map(|(_, r)| r.clone())
                            .ok_or_else(|| ReleaseError::NotFound {
                                wanted: format!("{channel:?}").to_lowercase(),
                                prefix: tag_prefix.to_string(),
                            })?
                    }
                };
                let version = tag_version(&release.tag_name, tag_prefix).ok_or_else(|| {
                    ReleaseError::Invalid(format!(
                        "tag {} does not carry a version after {tag_prefix:?}",
                        release.tag_name
                    ))
                })?;
                let asset = asset_for(&version);
                github_release_urls(&release, version, &asset, self.token.is_some())
            }
            ReleaseSource::Http { base_url } => {
                let base = base_url.trim_end_matches('/');
                let version = match channel {
                    Channel::Exact(v) => v.clone(),
                    _ => {
                        let url = format!("{base}/latest.json");
                        let latest: LatestJson = self.get_json(&url, false).await?;
                        let raw = match channel {
                            Channel::Latest => latest.latest.unwrap_or(latest.version),
                            _ => latest.version,
                        };
                        semver::Version::parse(raw.trim_start_matches('v')).map_err(|e| {
                            ReleaseError::Invalid(format!(
                                "{url} names {raw:?}, which is not a version: {e}"
                            ))
                        })?
                    }
                };
                let tag = format!("{tag_prefix}{version}");
                let asset = asset_for(&version);
                Ok(ResolvedRelease {
                    asset_url: format!("{base}/{tag}/{asset}"),
                    sums_url: format!("{base}/{tag}/{SUMS_FILE}"),
                    signature_url: Some(format!("{base}/{tag}/{SIGNATURE_FILE}")),
                    asset,
                    via_api: false,
                    tag,
                    version,
                })
            }
        }
    }

    /// Only the newest version for `channel`, for the update notice.
    pub async fn newest_version(
        &self,
        source: &ReleaseSource,
        tag_prefix: &str,
        channel: &Channel,
    ) -> Result<semver::Version, ReleaseError> {
        match source {
            ReleaseSource::Github { repo, api_base } => {
                let api = api_base
                    .as_deref()
                    .unwrap_or(DEFAULT_GITHUB_API)
                    .trim_end_matches('/');
                let url = format!("{api}/repos/{repo}/releases?per_page=100");
                let all: Vec<GithubRelease> = self.get_json(&url, true).await?;
                pick_release(&all, tag_prefix, channel)
                    .map(|(v, _)| v)
                    .ok_or_else(|| ReleaseError::NotFound {
                        wanted: format!("{channel:?}").to_lowercase(),
                        prefix: tag_prefix.to_string(),
                    })
            }
            ReleaseSource::Http { .. } => self
                .resolve(source, tag_prefix, &|_| String::new(), channel)
                .await
                .map(|r| r.version),
        }
    }

    /// Download `url` to `dest`. `api` marks a GitHub API asset URL.
    pub async fn download(&self, url: &str, api: bool, dest: &Path) -> Result<(), ReleaseError> {
        let accept = if api {
            "application/octet-stream"
        } else {
            "*/*"
        };
        let mut resp = self.get(url, accept, api).await?;
        let write_err = |source| ReleaseError::Write {
            path: dest.to_path_buf(),
            source,
        };
        let mut file = std::fs::File::create(dest).map_err(write_err)?;
        while let Some(chunk) = resp.chunk().await.map_err(|e| ReleaseError::Http {
            url: url.to_string(),
            message: error_chain(&e),
        })? {
            std::io::Write::write_all(&mut file, &chunk).map_err(write_err)?;
        }
        Ok(())
    }

    /// Download a small text file, `None` on 404.
    pub async fn fetch_optional_text(
        &self,
        url: &str,
        api: bool,
    ) -> Result<Option<String>, ReleaseError> {
        let accept = if api {
            "application/octet-stream"
        } else {
            "*/*"
        };
        match self.get(url, accept, api).await {
            Ok(resp) => resp.text().await.map(Some).map_err(|e| ReleaseError::Http {
                url: url.to_string(),
                message: error_chain(&e),
            }),
            Err(ReleaseError::Status { status: 404, .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

fn github_release_urls(
    release: &GithubRelease,
    version: semver::Version,
    asset: &str,
    with_token: bool,
) -> Result<ResolvedRelease, ReleaseError> {
    let find = |name: &str| release.assets.iter().find(|a| a.name == name);
    let missing = |name: &str| ReleaseError::MissingAsset {
        tag: release.tag_name.clone(),
        asset: name.to_string(),
    };
    let archive = find(asset).ok_or_else(|| missing(asset))?;
    let sums = find(SUMS_FILE).ok_or_else(|| missing(SUMS_FILE))?;
    // With a token, go through the API: it is the only route to a private
    // repository's assets. Without one, the public download URL does not
    // count against the API rate limit.
    let pick = |a: &GithubAsset| {
        if with_token {
            a.url.clone()
        } else {
            a.browser_download_url.clone()
        }
    };
    Ok(ResolvedRelease {
        version,
        tag: release.tag_name.clone(),
        asset: asset.to_string(),
        asset_url: pick(archive),
        sums_url: pick(sums),
        signature_url: find(SIGNATURE_FILE).map(pick),
        via_api: with_token,
    })
}

/// A reqwest error with its causes. The URL is already in the message of
/// [`ReleaseError::Http`], and no header value ever appears.
fn error_chain(e: &reqwest::Error) -> String {
    let mut out = String::from(if e.is_timeout() {
        "timed out"
    } else if e.is_connect() {
        "cannot connect"
    } else if e.is_redirect() {
        "redirect refused"
    } else {
        "request failed"
    });
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        out.push_str(": ");
        out.push_str(&s.to_string());
        source = s.source();
    }
    out
}

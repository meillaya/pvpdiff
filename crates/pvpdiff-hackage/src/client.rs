//! Blocking Hackage client with a content cache.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use directories::ProjectDirs;
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::cache::{Artifact, ArtifactKind, Cache, CacheError, CacheHeaders, CacheMeta};
use crate::urls;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ATTEMPTS: usize = 3;

/// Failure to talk to Hackage.
#[derive(Debug, Error)]
pub enum HackageError {
    /// HTTP transport failure.
    #[error("Hackage request failed: {0}")]
    Request(#[from] reqwest::Error),
    /// Unexpected HTTP status.
    #[error("unexpected HTTP status {status} for {url}")]
    Status {
        /// Status code.
        status: u16,
        /// Requested URL.
        url: String,
    },
    /// The Hoogle database was not found.
    #[error("Hoogle database unavailable for {package}-{version}")]
    DocsUnavailable {
        /// Package name.
        package: String,
        /// Package version.
        version: String,
    },
    /// Cache failure.
    #[error("cache error: {0}")]
    Cache(#[from] CacheError),
    /// Invalid JSON in a Hackage response.
    #[error("invalid Hackage JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Offline mode and the artifact is missing.
    #[error("offline and not cached: {0}")]
    Offline(String),
    /// No platform cache directory is available.
    #[error("no platform cache directory is available")]
    NoCacheDirectory,
}

/// Client for Hackage requests used by pvpdiff.
#[derive(Debug, Clone)]
pub struct HackageClient {
    http: Client,
    cache: Cache,
    base: String,
    offline: bool,
    refresh: bool,
}

impl HackageClient {
    /// Create a client rooted at a cache directory.
    pub fn new(cache_dir: PathBuf, offline: bool, refresh: bool) -> Result<Self, HackageError> {
        Self::with_base(cache_dir, offline, refresh, urls::DEFAULT_BASE_URL)
    }

    /// Create a client with an explicit base URL, used by tests.
    pub fn with_base(
        cache_dir: PathBuf,
        offline: bool,
        refresh: bool,
        base: impl Into<String>,
    ) -> Result<Self, HackageError> {
        let http = Client::builder()
            .user_agent(crate::user_agent())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()?;
        Ok(Self {
            http,
            cache: Cache::new(cache_dir),
            base: base.into(),
            offline,
            refresh,
        })
    }

    /// Default cache directory for this platform.
    pub fn default_cache_dir() -> Result<PathBuf, HackageError> {
        ProjectDirs::from("dev", "pvpdiff", "pvpdiff")
            .map(|directories| directories.cache_dir().to_path_buf())
            .ok_or(HackageError::NoCacheDirectory)
    }

    /// Whether the client refuses network access.
    pub fn is_offline(&self) -> bool {
        self.offline
    }

    /// Cache root.
    pub fn cache_dir(&self) -> &Path {
        self.cache.root()
    }

    /// Remove the cache.
    pub fn clear_cache(&self) -> Result<(), HackageError> {
        self.cache.clear().map_err(HackageError::from)
    }

    /// Fetch the version map for a package.
    pub fn versions(&self, package: &str) -> Result<BTreeMap<String, String>, HackageError> {
        let url = urls::versions_url(&self.base, package);
        let response = self.get(&url, None)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(HackageError::Status { status: 404, url });
        }
        let response = self.check_status(response, &url)?;
        self.decode_json(response, &url)
    }

    /// Fetch a version's Hoogle database.
    pub fn fetch_docs(&self, package: &str, version: &str) -> Result<Artifact, HackageError> {
        let url = urls::docs_url(&self.base, package, version);
        match self.fetch(package, version, ArtifactKind::Docs, &url)? {
            Some(artifact) => Ok(artifact),
            None => Err(HackageError::DocsUnavailable {
                package: package.to_string(),
                version: version.to_string(),
            }),
        }
    }

    /// Fetch a version's Cabal file. A 404 returns `None` because the Hoogle
    /// analysis can proceed without Cabal metadata.
    pub fn fetch_cabal(
        &self,
        package: &str,
        version: &str,
    ) -> Result<Option<Artifact>, HackageError> {
        let url = urls::cabal_url(&self.base, package, version);
        self.fetch(package, version, ArtifactKind::Cabal, &url)
    }

    fn fetch(
        &self,
        package: &str,
        version: &str,
        kind: ArtifactKind,
        url: &str,
    ) -> Result<Option<Artifact>, HackageError> {
        let cached = self.cache.load(package, version, kind);
        let meta = self.cache.load_meta(package, version, kind);

        if self.offline {
            return cached
                .ok_or_else(|| HackageError::Offline(url.to_string()))
                .map(Some);
        }

        if let Some(cached) = &cached
            && !self.refresh
            && meta.as_ref().is_some_and(|meta| !meta.is_stale(kind))
        {
            return Ok(Some(cached.clone()));
        }

        let response = self.get(url, meta.as_ref())?;
        if response.status() == StatusCode::NOT_MODIFIED {
            return cached
                .ok_or_else(|| HackageError::Status {
                    status: 304,
                    url: url.to_string(),
                })
                .map(Some);
        }
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = self.check_status(response, url)?;
        let etag = header_string(&response, ETAG.as_str());
        let last_modified = header_string(&response, LAST_MODIFIED.as_str());
        let bytes = response.bytes()?.to_vec();
        let artifact = self.cache.store(
            package,
            version,
            kind,
            &bytes,
            url,
            CacheHeaders {
                etag,
                last_modified,
            },
        )?;
        Ok(Some(artifact))
    }

    fn get(&self, url: &str, meta: Option<&CacheMeta>) -> Result<Response, HackageError> {
        let mut attempt = 0usize;
        loop {
            let mut request = self.http.get(url);
            if let Some(meta) = meta {
                if let Some(etag) = &meta.etag {
                    request = request.header(IF_NONE_MATCH, etag);
                }
                if let Some(last_modified) = &meta.last_modified {
                    request = request.header(IF_MODIFIED_SINCE, last_modified);
                }
            }
            let response = request.send()?;
            let retryable = response.status() == StatusCode::TOO_MANY_REQUESTS
                || response.status().is_server_error();
            if !retryable || attempt + 1 >= MAX_ATTEMPTS {
                return Ok(response);
            }
            let delay = retry_after(&response)
                .unwrap_or_else(|| Duration::from_millis(250u64.saturating_mul(1 << attempt)));
            sleep(delay);
            attempt += 1;
        }
    }

    fn check_status(&self, response: Response, url: &str) -> Result<Response, HackageError> {
        if response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED {
            return Ok(response);
        }
        Err(HackageError::Status {
            status: response.status().as_u16(),
            url: url.to_string(),
        })
    }

    fn decode_json<T: DeserializeOwned>(
        &self,
        response: Response,
        url: &str,
    ) -> Result<T, HackageError> {
        let text = response.text()?;
        serde_json::from_str(&text).map_err(|error| {
            let _ = url;
            HackageError::Json(error)
        })
    }
}

fn header_string(response: &Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
}

fn retry_after(response: &Response) -> Option<Duration> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::MockServer;
    use httpmock::prelude::*;

    /// Check that the version map is decoded and that a missing package is an
    /// error.
    #[test]
    fn fetches_version_map() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/package/demo.json");
            then.status(200)
                .body("{\"1.0.0\":\"normal\",\"2.0.0\":\"normal\"}");
        });
        let directory = tempfile::tempdir().expect("tempdir");
        let client = HackageClient::with_base(
            directory.path().to_path_buf(),
            false,
            false,
            server.base_url(),
        )
        .expect("client");
        let versions = client.versions("demo").expect("versions");
        mock.assert();
        assert_eq!(versions.len(), 2);
    }

    /// Check that a Hoogle database is cached and that offline mode reads it.
    #[test]
    fn caches_docs_for_offline_use() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/package/demo-1.0.0/docs/demo.txt");
            then.status(200).body("database");
        });
        let directory = tempfile::tempdir().expect("tempdir");
        let online = HackageClient::with_base(
            directory.path().to_path_buf(),
            false,
            false,
            server.base_url(),
        )
        .expect("client");
        let artifact = online.fetch_docs("demo", "1.0.0").expect("docs");
        assert_eq!(artifact.bytes, b"database");
        mock.assert();

        let offline = HackageClient::with_base(
            directory.path().to_path_buf(),
            true,
            false,
            server.base_url(),
        )
        .expect("client");
        let cached = offline.fetch_docs("demo", "1.0.0").expect("cached docs");
        assert_eq!(cached.bytes, b"database");
    }

    /// Check that a missing Hoogle database becomes a typed error and a
    /// missing Cabal file is optional.
    #[test]
    fn distinguishes_missing_docs_and_cabal() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/package/demo-1.0.0/docs/demo.txt");
            then.status(404);
        });
        server.mock(|when, then| {
            when.method(GET).path("/package/demo-1.0.0/demo.cabal");
            then.status(404);
        });
        let directory = tempfile::tempdir().expect("tempdir");
        let client = HackageClient::with_base(
            directory.path().to_path_buf(),
            false,
            false,
            server.base_url(),
        )
        .expect("client");
        assert!(matches!(
            client.fetch_docs("demo", "1.0.0"),
            Err(HackageError::DocsUnavailable { .. })
        ));
        assert!(
            client
                .fetch_cabal("demo", "1.0.0")
                .expect("cabal")
                .is_none()
        );
    }
}

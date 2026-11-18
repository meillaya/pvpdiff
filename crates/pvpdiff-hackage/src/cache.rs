//! Local content cache with provenance metadata.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Cached artifact kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// Hoogle text database.
    Docs,
    /// Cabal file.
    Cabal,
}

impl ArtifactKind {
    /// File name inside one version directory.
    pub fn file_name(self, package: &str) -> String {
        match self {
            Self::Docs => format!("{package}.txt"),
            Self::Cabal => format!("{package}.cabal"),
        }
    }

    /// Whether the artifact is immutable and can be cached forever.
    pub fn immutable(self) -> bool {
        matches!(self, Self::Docs)
    }
}

/// Cached bytes and their provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// File contents.
    pub bytes: Vec<u8>,
    /// SHA-256 of the contents.
    pub sha256: String,
    /// URL the contents came from.
    pub url: String,
}

/// Optional HTTP validators for one fetch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheHeaders {
    /// Strong ETag.
    pub etag: Option<String>,
    /// Last-Modified header.
    pub last_modified: Option<String>,
}

/// Cache metadata stored next to an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheMeta {
    /// Source URL.
    pub url: String,
    /// Strong ETag, when the server sent one.
    pub etag: Option<String>,
    /// Last-Modified header, when present.
    pub last_modified: Option<String>,
    /// SHA-256 of the bytes.
    pub sha256: String,
    /// Unix timestamp of the fetch.
    pub fetched_at: u64,
}

impl CacheMeta {
    /// Whether a mutable artifact should be revalidated.
    pub fn is_stale(&self, kind: ArtifactKind) -> bool {
        if kind.immutable() {
            return false;
        }
        let now = now_unix();
        now.saturating_sub(self.fetched_at) > 7 * 24 * 60 * 60
    }
}

/// Failure to read or write the cache.
#[derive(Debug, Error)]
pub enum CacheError {
    /// Filesystem error.
    #[error("cache I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Metadata was not valid JSON.
    #[error("cache metadata is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

/// Content cache rooted at a directory.
#[derive(Debug, Clone)]
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    /// Create a cache rooted at `root`.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Cache root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn version_dir(&self, package: &str, version: &str) -> PathBuf {
        self.root.join("hackage").join(package).join(version)
    }

    /// Load an artifact, verifying its digest against the metadata.
    pub fn load(&self, package: &str, version: &str, kind: ArtifactKind) -> Option<Artifact> {
        let directory = self.version_dir(package, version);
        let file_name = kind.file_name(package);
        let bytes = fs::read(directory.join(&file_name)).ok()?;
        let meta_text =
            fs::read_to_string(directory.join(format!("{file_name}.meta.json"))).ok()?;
        let meta: CacheMeta = serde_json::from_str(&meta_text).ok()?;
        let sha256 = sha256_hex(&bytes);
        if sha256 != meta.sha256 {
            return None;
        }
        Some(Artifact {
            bytes,
            sha256,
            url: meta.url,
        })
    }

    /// Load only the metadata for an artifact.
    pub fn load_meta(&self, package: &str, version: &str, kind: ArtifactKind) -> Option<CacheMeta> {
        let directory = self.version_dir(package, version);
        let file_name = kind.file_name(package);
        let text = fs::read_to_string(directory.join(format!("{file_name}.meta.json"))).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Store an artifact and its metadata.
    pub fn store(
        &self,
        package: &str,
        version: &str,
        kind: ArtifactKind,
        bytes: &[u8],
        url: &str,
        headers: CacheHeaders,
    ) -> Result<Artifact, CacheError> {
        let directory = self.version_dir(package, version);
        fs::create_dir_all(&directory)?;
        let file_name = kind.file_name(package);
        let sha256 = sha256_hex(bytes);
        write_atomic(&directory.join(&file_name), bytes)?;
        let meta = CacheMeta {
            url: url.to_string(),
            etag: headers.etag,
            last_modified: headers.last_modified,
            sha256: sha256.clone(),
            fetched_at: now_unix(),
        };
        let meta_bytes = serde_json::to_vec_pretty(&meta)?;
        write_atomic(
            &directory.join(format!("{file_name}.meta.json")),
            &meta_bytes,
        )?;
        Ok(Artifact {
            bytes: bytes.to_vec(),
            sha256,
            url: url.to_string(),
        })
    }

    /// Remove every cached artifact.
    pub fn clear(&self) -> Result<(), CacheError> {
        if self.root.exists() {
            fs::remove_dir_all(&self.root)?;
        }
        Ok(())
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)
}

/// SHA-256 as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Check that an artifact round-trips through the cache and that a
    /// corrupted file is treated as missing.
    #[test]
    fn stores_and_verifies_artifacts() {
        let directory = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(directory.path().to_path_buf());
        let artifact = cache
            .store(
                "demo",
                "1.0.0",
                ArtifactKind::Docs,
                b"contents",
                "https://example.test/docs",
                CacheHeaders {
                    etag: Some("\"etag\"".to_string()),
                    last_modified: None,
                },
            )
            .expect("store");
        assert_eq!(artifact.sha256, sha256_hex(b"contents"));
        let loaded = cache
            .load("demo", "1.0.0", ArtifactKind::Docs)
            .expect("load");
        assert_eq!(loaded.bytes, b"contents");

        let path = directory.path().join("hackage/demo/1.0.0/demo.txt");
        fs::write(path, b"corrupted").expect("corrupt");
        assert!(cache.load("demo", "1.0.0", ArtifactKind::Docs).is_none());
    }

    /// Check that docs are immutable and cabal files can go stale.
    #[test]
    fn classifies_cache_staleness() {
        let meta = CacheMeta {
            url: "u".to_string(),
            etag: None,
            last_modified: None,
            sha256: "s".to_string(),
            fetched_at: 0,
        };
        assert!(!meta.is_stale(ArtifactKind::Docs));
        assert!(meta.is_stale(ArtifactKind::Cabal));
    }
}

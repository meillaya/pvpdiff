//! Hackage `01-index.tar.gz` synchronization and local storage.
//!
//! A cold sync downloads about 141 MB and streams roughly 362k tar entries.
//! The archive carries one copy of every package version plus one entry per
//! Cabal revision, so the parser keeps the last entry for a path and drops the
//! earlier copies. The result is a JSONL dependency database and a small
//! metadata file, both written atomically so an interrupted sync cannot leave
//! a half written database behind.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{ETAG, IF_NONE_MATCH};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use url::Url;

/// Default URL of the Hackage index archive.
pub const DEFAULT_INDEX_URL: &str = "https://hackage.haskell.org/01-index.tar.gz";

const INDEX_DIR: &str = "index";
const ARCHIVE_FILE: &str = "01-index.tar.gz";
const JSONL_FILE: &str = "index-v1.jsonl";
const META_FILE: &str = "index-meta.json";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// One dependency of one index row.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexDependency {
    /// Dependency package name.
    pub package: String,
    /// Cabal version range as written, such as `>=2.0 && <2.1` or `-any`.
    pub range: String,
    /// Component that declared the dependency. The index database stores
    /// library dependencies only for now.
    pub component: String,
}

/// One package version in the index database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexRow {
    /// Package name.
    pub package: String,
    /// Package version.
    pub version: String,
    /// Whether any Cabal conditional appears in the file.
    pub conditional: bool,
    /// Library dependencies, sorted by package, range, and component.
    pub dependencies: Vec<IndexDependency>,
    /// SHA-256 of the Cabal file bytes this row was parsed from.
    pub cabal_sha256: String,
}

/// Summary of a completed index sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IndexMeta {
    /// URL the archive was downloaded from.
    pub index_url: String,
    /// SHA-256 of the archive bytes.
    pub index_sha256: String,
    /// ETag returned by the server, when it sent one.
    pub etag: Option<String>,
    /// Number of distinct packages in the database.
    pub packages: usize,
    /// Number of package versions in the database.
    pub versions: usize,
    /// Number of Cabal files that could not be parsed into a row.
    pub parse_skips: usize,
}

/// Failure to sync or read the index database.
#[derive(Debug, Error)]
pub enum IndexError {
    /// The cache has no index database.
    #[error("index database is not cached; run `pvpdiff index sync` first")]
    Missing,
    /// Offline mode was requested and the cache has no index database.
    #[error("offline mode and the index database is not cached; run `pvpdiff index sync` first")]
    OfflineMissing,
    /// The HTTP request failed before a response arrived.
    #[error("index request failed: {0}")]
    Request(#[from] reqwest::Error),
    /// The server returned a status other than success or 304.
    #[error("unexpected HTTP status {status} for {url}")]
    Status {
        /// Status code.
        status: u16,
        /// Requested URL.
        url: String,
    },
    /// A local file operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A JSONL record is invalid.
    #[error("invalid index database at line {line}: {source}")]
    JsonLine {
        /// One based line number.
        line: usize,
        /// Decode error.
        source: serde_json::Error,
    },
    /// The metadata file is invalid.
    #[error("invalid index metadata: {source}")]
    MetaJson {
        /// Decode error.
        source: serde_json::Error,
    },
    /// A JSON value could not be encoded.
    #[error("index JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// The URL could not be parsed.
    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),
    /// The URL scheme is not supported for the index.
    #[error("unsupported index URL: {0}")]
    UnsupportedUrl(String),
    /// A version string passed to the blast-radius engine is invalid.
    #[error("invalid version: {0}")]
    InvalidVersion(String),
    /// A snapshot name contains characters that cannot appear in a URL.
    #[error("invalid snapshot name: {0}")]
    InvalidSnapshot(String),
    /// The snapshot server returned 404.
    #[error("snapshot not found: {snapshot}")]
    SnapshotNotFound {
        /// Snapshot name.
        snapshot: String,
    },
    /// The snapshot file cannot be parsed.
    #[error("cannot parse snapshot {snapshot}: {reason}")]
    Snapshot {
        /// Snapshot name.
        snapshot: String,
        /// Why parsing failed.
        reason: String,
    },
}

/// Synchronize the local index database with `01-index.tar.gz`.
///
/// A warm run sends a conditional GET and returns the cached metadata on 304.
/// Offline mode never touches the network. `refresh` skips the conditional GET
/// and downloads the archive again.
pub fn sync_index(
    cache_dir: &Path,
    index_url: Option<&str>,
    offline: bool,
    refresh: bool,
) -> Result<IndexMeta, IndexError> {
    let layout = Layout::new(cache_dir);

    // Offline sync only reports what is already cached, and a metadata file
    // without its database is not a usable cache.
    if offline {
        let meta = read_meta(cache_dir)?.ok_or(IndexError::OfflineMissing)?;
        if !layout.jsonl().is_file() {
            return Err(IndexError::OfflineMissing);
        }
        return Ok(meta);
    }

    let url = index_url.unwrap_or(DEFAULT_INDEX_URL).to_string();
    let cached = read_meta(cache_dir)?;

    // An ETag only identifies the URL that served it.
    let etag = if refresh {
        None
    } else {
        cached
            .as_ref()
            .filter(|meta| meta.index_url == url)
            .and_then(|meta| meta.etag.clone())
    };

    layout.ensure_dir()?;
    match download(&url, etag.as_deref(), &layout.archive())? {
        Download::NotModified => cached.ok_or_else(|| IndexError::Status {
            status: StatusCode::NOT_MODIFIED.as_u16(),
            url,
        }),
        Download::Fetched {
            sha256: index_sha256,
            etag,
        } => {
            let (rows, parse_skips) = parse_archive(&layout.archive())?;
            let packages = rows
                .iter()
                .map(|row| row.package.as_str())
                .collect::<BTreeSet<_>>()
                .len();
            let versions = rows.len();
            write_jsonl(&layout.jsonl(), &rows)?;
            let meta = IndexMeta {
                index_url: url,
                index_sha256,
                etag,
                packages,
                versions,
                parse_skips,
            };
            write_meta(&layout.meta(), &meta)?;
            Ok(meta)
        }
    }
}

/// Read the index database.
pub fn read_index(cache_dir: &Path) -> Result<Vec<IndexRow>, IndexError> {
    let path = Layout::new(cache_dir).jsonl();
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Err(IndexError::Missing),
        Err(error) => return Err(IndexError::Io(error)),
    };

    let mut rows = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let row = serde_json::from_str(&line).map_err(|source| IndexError::JsonLine {
            line: index + 1,
            source,
        })?;
        rows.push(row);
    }
    Ok(rows)
}

/// Read the metadata of the last sync, when one exists.
pub fn read_meta(cache_dir: &Path) -> Result<Option<IndexMeta>, IndexError> {
    let path = Layout::new(cache_dir).meta();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(IndexError::Io(error)),
    };
    let meta = serde_json::from_str(&text).map_err(|source| IndexError::MetaJson { source })?;
    Ok(Some(meta))
}

/// Hex encoded SHA-256 digest of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Cache paths under one cache root.
struct Layout {
    root: PathBuf,
}

impl Layout {
    fn new(cache_dir: &Path) -> Self {
        Self {
            root: cache_dir.join(INDEX_DIR),
        }
    }

    fn archive(&self) -> PathBuf {
        self.root.join(ARCHIVE_FILE)
    }

    fn jsonl(&self) -> PathBuf {
        self.root.join(JSONL_FILE)
    }

    fn meta(&self) -> PathBuf {
        self.root.join(META_FILE)
    }

    fn ensure_dir(&self) -> Result<(), IndexError> {
        fs::create_dir_all(&self.root)?;
        Ok(())
    }
}

/// Result of one index download attempt.
enum Download {
    /// The server answered 304 and the cached database stays current.
    NotModified,
    /// A new archive was written, with its digest and ETag.
    Fetched {
        sha256: String,
        etag: Option<String>,
    },
}

fn download(url: &str, etag: Option<&str>, archive: &Path) -> Result<Download, IndexError> {
    let parsed = Url::parse(url)?;
    match parsed.scheme() {
        // Local archives keep the tests offline and let a user sync from a
        // mirror snapshot without an HTTP server.
        "file" => {
            let source = parsed
                .to_file_path()
                .map_err(|()| IndexError::UnsupportedUrl(url.to_string()))?;
            let file = File::open(source)?;
            write_archive(archive, file)?;
            let sha256 = sha256_file(archive)?;
            Ok(Download::Fetched { sha256, etag: None })
        }
        "http" | "https" => download_http(url, etag, archive),
        _ => Err(IndexError::UnsupportedUrl(url.to_string())),
    }
}

fn download_http(url: &str, etag: Option<&str>, archive: &Path) -> Result<Download, IndexError> {
    let client = Client::builder()
        .user_agent(crate::user_agent())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(DOWNLOAD_TIMEOUT)
        .build()?;
    let mut request = client.get(url);
    if let Some(etag) = etag {
        request = request.header(IF_NONE_MATCH, etag);
    }
    let mut response = request.send()?;

    if response.status() == StatusCode::NOT_MODIFIED {
        return Ok(Download::NotModified);
    }
    if !response.status().is_success() {
        return Err(IndexError::Status {
            status: response.status().as_u16(),
            url: url.to_string(),
        });
    }

    let etag = header_text(&response, ETAG.as_str());
    write_archive(archive, &mut response)?;
    let sha256 = sha256_file(archive)?;
    Ok(Download::Fetched { sha256, etag })
}

fn header_text(response: &Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
}

fn write_archive(archive: &Path, source: impl Read) -> Result<(), IndexError> {
    let mut temp = tempfile::NamedTempFile::new_in(parent_dir(archive)?)?;
    let mut source = source;
    std::io::copy(&mut source, temp.as_file_mut())?;
    temp.as_file_mut().flush()?;
    temp.persist(archive)
        .map_err(|error| IndexError::Io(error.error))?;
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, IndexError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; HASH_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn parse_archive(archive: &Path) -> Result<(Vec<IndexRow>, usize), IndexError> {
    let file = File::open(archive)?;
    let decoder = GzDecoder::new(BufReader::new(file));
    let mut tar = tar::Archive::new(decoder);
    let mut rows: BTreeMap<(String, String), IndexRow> = BTreeMap::new();
    let mut parse_skips = 0usize;

    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.to_string_lossy().into_owned();
        let Some((package, version)) = cabal_path(&path) else {
            continue;
        };
        let mut source = Vec::new();
        entry.read_to_end(&mut source)?;
        let text = String::from_utf8_lossy(&source);
        let parsed = pvpdiff_parse::parse_cabal(&text);
        let Some(meta) = parsed.meta else {
            parse_skips += 1;
            continue;
        };
        let Some(row) = index_row(meta, &source) else {
            parse_skips += 1;
            continue;
        };
        // A duplicate cabal path is a revision, and the later entry is the one
        // Hackage serves today.
        rows.insert((package, version), row);
    }

    let mut rows: Vec<IndexRow> = rows.into_values().collect();
    rows.sort_by(|left, right| {
        (&left.package, &left.version).cmp(&(&right.package, &right.version))
    });
    Ok((rows, parse_skips))
}

/// Split `<package>/<version>/<package>.cabal` into its identity. Every other
/// archive entry is not a Cabal file and is ignored.
fn cabal_path(path: &str) -> Option<(String, String)> {
    let path = path.strip_prefix("./").unwrap_or(path);
    let mut parts = path.split('/');
    let package = parts.next()?;
    let version = parts.next()?;
    let file = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if package.is_empty() || version.is_empty() || !file.ends_with(".cabal") {
        return None;
    }
    Some((package.to_string(), version.to_string()))
}

fn index_row(meta: pvpdiff_parse::CabalMeta, cabal_bytes: &[u8]) -> Option<IndexRow> {
    let package = meta.name.trim();
    let version = meta.version.trim();
    if package.is_empty() || version.is_empty() {
        return None;
    }

    let mut dependencies: Vec<IndexDependency> = meta
        .libraries
        .iter()
        .flat_map(|library| library.dependencies.iter())
        .map(|dependency| IndexDependency {
            package: dependency.package.clone(),
            range: dependency.range.clone(),
            component: dependency.component.as_str().to_string(),
        })
        .collect();
    dependencies.sort();
    dependencies.dedup();

    Some(IndexRow {
        package: package.to_string(),
        version: version.to_string(),
        conditional: meta.conditional,
        dependencies,
        cabal_sha256: sha256_hex(cabal_bytes),
    })
}

fn write_jsonl(path: &Path, rows: &[IndexRow]) -> Result<(), IndexError> {
    let mut temp = tempfile::NamedTempFile::new_in(parent_dir(path)?)?;
    {
        let mut writer = BufWriter::new(temp.as_file_mut());
        for row in rows {
            serde_json::to_writer(&mut writer, row)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;
    }
    temp.persist(path)
        .map_err(|error| IndexError::Io(error.error))?;
    Ok(())
}

fn write_meta(path: &Path, meta: &IndexMeta) -> Result<(), IndexError> {
    let mut temp = tempfile::NamedTempFile::new_in(parent_dir(path)?)?;
    let text = serde_json::to_vec_pretty(meta)?;
    temp.as_file_mut().write_all(&text)?;
    temp.as_file_mut().write_all(b"\n")?;
    temp.as_file_mut().flush()?;
    temp.persist(path)
        .map_err(|error| IndexError::Io(error.error))?;
    Ok(())
}

fn parent_dir(path: &Path) -> Result<&Path, IndexError> {
    path.parent().ok_or_else(|| {
        IndexError::Io(std::io::Error::other(
            "index cache path has no parent directory",
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO_REVISION_ONE: &str = "\
cabal-version: 2.2
name: demo
version: 1.0.0

library
  exposed-modules: Demo
  build-depends:
    base >=4.14 && <5,
    text ^>=2.0
";

    const DEMO_REVISION_TWO: &str = "\
cabal-version: 2.2
name: demo
version: 1.0.0
x-revision: 2

library
  exposed-modules: Demo
  build-depends:
    base >=4.14 && <5,
    text >=2.1
";

    const OTHER: &str = "\
cabal-version: 2.2
name: other
version: 0.1.0

flag extra
  default: False

library
  exposed-modules: Other
  if flag(extra)
    build-depends: text >=2.0
  build-depends: base >=4.14
";

    const BROKEN: &str = "this file has no package fields\n";

    fn fixture_archive(root: &Path, entries: &[(&str, &str)]) -> PathBuf {
        let path = root.join("fixture.tar.gz");
        let file = File::create(&path).expect("create fixture archive");
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            builder
                .append_data(&mut header, name, contents.as_bytes())
                .expect("append fixture entry");
        }
        let encoder = builder.into_inner().expect("finish fixture tar");
        encoder.finish().expect("finish fixture gzip");
        path
    }

    fn file_url(path: &Path) -> String {
        Url::from_file_path(path)
            .expect("fixture path becomes a file URL")
            .to_string()
    }

    /// Check that a sync from a local archive writes both cache files, counts
    /// the rows and skips, and keeps the later entry of a duplicated path.
    #[test]
    fn syncs_fixture_and_keeps_last_revision() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let archive = fixture_archive(
            workspace.path(),
            &[
                ("demo/1.0.0/demo.cabal", DEMO_REVISION_ONE),
                ("demo/1.0.0/package.json", "{}"),
                ("other/0.1.0/other.cabal", OTHER),
                ("broken/0.1.0/broken.cabal", BROKEN),
                ("demo/1.0.0/demo.cabal", DEMO_REVISION_TWO),
            ],
        );
        let cache = workspace.path().join("cache");
        let url = file_url(&archive);

        let meta = sync_index(&cache, Some(&url), false, false).expect("sync");
        assert_eq!(meta.packages, 2);
        assert_eq!(meta.versions, 2);
        assert_eq!(meta.parse_skips, 1);
        assert_eq!(meta.etag, None);
        assert_eq!(meta.index_url, url);
        assert_eq!(
            meta.index_sha256,
            sha256_hex(&fs::read(&archive).expect("read fixture archive"))
        );
        assert!(cache.join(INDEX_DIR).join(ARCHIVE_FILE).is_file());

        let rows = read_index(&cache).expect("rows");
        assert_eq!(rows.len(), 2);

        // The duplicate demo path must carry the revision two digest and range.
        let demo = rows
            .iter()
            .find(|row| row.package == "demo")
            .expect("demo row");
        assert_eq!(demo.version, "1.0.0");
        assert!(!demo.conditional);
        assert_eq!(demo.cabal_sha256, sha256_hex(DEMO_REVISION_TWO.as_bytes()));
        assert!(
            demo.dependencies
                .iter()
                .all(|dep| dep.component == "library")
        );
        assert!(
            demo.dependencies
                .iter()
                .any(|dep| dep.package == "text" && dep.range == ">=2.1")
        );
        assert!(
            !demo
                .dependencies
                .iter()
                .any(|dep| dep.package == "text" && dep.range == "^>=2.0")
        );

        // A conditional in the file marks the row and its dependencies.
        let other = rows
            .iter()
            .find(|row| row.package == "other")
            .expect("other row");
        assert!(other.conditional);
        assert!(
            other
                .dependencies
                .iter()
                .any(|dep| dep.package == "text" && dep.range == ">=2.0")
        );

        let stored = read_meta(&cache).expect("meta option").expect("meta");
        assert_eq!(stored, meta);
    }

    /// Check that offline sync errors on an empty cache and returns the stored
    /// metadata once an online sync has populated the cache.
    #[test]
    fn offline_reads_existing_database() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let cache = workspace.path().join("cache");
        assert!(matches!(
            sync_index(&cache, None, true, false),
            Err(IndexError::OfflineMissing)
        ));

        let archive = fixture_archive(
            workspace.path(),
            &[("demo/1.0.0/demo.cabal", DEMO_REVISION_ONE)],
        );
        let online = sync_index(&cache, Some(&file_url(&archive)), false, false).expect("sync");
        let offline = sync_index(&cache, None, true, false).expect("offline meta");
        assert_eq!(offline, online);
    }

    /// Check that a missing database is a typed error, that a missing metadata
    /// file is not, and that a bad JSONL line reports its number.
    #[test]
    fn reports_missing_and_invalid_database() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let cache = workspace.path().join("cache");
        assert!(matches!(read_index(&cache), Err(IndexError::Missing)));
        assert_eq!(read_meta(&cache).expect("meta option"), None);

        fs::create_dir_all(cache.join(INDEX_DIR)).expect("create index dir");
        fs::write(
            cache.join(INDEX_DIR).join(JSONL_FILE),
            "{\"package\":\"demo\"}\n",
        )
        .expect("write bad line");
        match read_index(&cache) {
            Err(IndexError::JsonLine { line, .. }) => assert_eq!(line, 1),
            other => panic!("expected a line error, got {other:?}"),
        }
    }

    /// Check a real Hackage archive. Ignored by default because it downloads
    /// about 141 MB; run it with `--ignored` after a change to the sync path.
    #[test]
    #[ignore]
    fn syncs_live_index() {
        // Thresholds below the measured 2026-11 counts catch a truncated parse.
        const LIVE_MIN_PACKAGES: usize = 10_000;
        const LIVE_MIN_VERSIONS: usize = 100_000;

        let workspace = tempfile::tempdir().expect("tempdir");
        let cache = workspace.path().join("cache");
        let meta = sync_index(&cache, None, false, false).expect("live sync");
        println!(
            "live index: {} packages, {} versions, {} parse skips, sha {}",
            meta.packages, meta.versions, meta.parse_skips, meta.index_sha256
        );
        assert_eq!(meta.index_url, DEFAULT_INDEX_URL);
        assert!(!meta.index_sha256.is_empty());
        assert!(
            meta.packages > LIVE_MIN_PACKAGES,
            "packages: {}",
            meta.packages
        );
        assert!(
            meta.versions > LIVE_MIN_VERSIONS,
            "versions: {}",
            meta.versions
        );

        let rows = read_index(&cache).expect("live rows");
        assert_eq!(rows.len(), meta.versions);
        let text = rows
            .iter()
            .find(|row| row.package == "text")
            .expect("text row");
        assert!(
            text.dependencies
                .iter()
                .any(|dependency| dependency.package == "base")
        );
    }

    /// Check that only the three component package/version/cabal layout is
    /// accepted and that other archive entries are ignored.
    #[test]
    fn recognises_cabal_entry_paths() {
        assert_eq!(
            cabal_path("demo/1.0.0/demo.cabal"),
            Some(("demo".to_string(), "1.0.0".to_string()))
        );
        assert_eq!(
            cabal_path("./demo/1.0.0/demo.cabal"),
            Some(("demo".to_string(), "1.0.0".to_string()))
        );
        assert_eq!(cabal_path("demo/1.0.0/package.json"), None);
        assert_eq!(cabal_path("demo/preferred-versions"), None);
        assert_eq!(cabal_path("demo/1.0.0/extra/demo.cabal"), None);
    }
}

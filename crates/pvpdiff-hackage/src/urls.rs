//! Hackage URL layout.

/// Default Hackage base URL.
pub const DEFAULT_BASE_URL: &str = "https://hackage.haskell.org";

/// URL for the package version map.
pub fn versions_url(base: &str, package: &str) -> String {
    format!("{base}/package/{package}.json")
}

/// URL for one version's metadata.
pub fn version_metadata_url(base: &str, package: &str, version: &str) -> String {
    format!("{base}/package/{package}-{version}.json")
}

/// URL for one version's Hoogle database.
pub fn docs_url(base: &str, package: &str, version: &str) -> String {
    format!("{base}/package/{package}-{version}/docs/{package}.txt")
}

/// URL for one version's Cabal file.
pub fn cabal_url(base: &str, package: &str, version: &str) -> String {
    format!("{base}/package/{package}-{version}/{package}.cabal")
}

/// URL for the full Hackage index.
pub fn index_url(base: &str) -> String {
    format!("{base}/01-index.tar.gz")
}

/// URL for a Stackage snapshot's cabal.config.
pub fn stackage_config_url(snapshot: &str) -> String {
    format!("https://www.stackage.org/{snapshot}/cabal.config")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Check the exact URL shapes for the two P1 artifacts and the index.
    #[test]
    fn builds_documented_urls() {
        assert_eq!(
            docs_url(DEFAULT_BASE_URL, "text", "2.0.2"),
            "https://hackage.haskell.org/package/text-2.0.2/docs/text.txt"
        );
        assert_eq!(
            cabal_url(DEFAULT_BASE_URL, "text", "2.0.2"),
            "https://hackage.haskell.org/package/text-2.0.2/text.cabal"
        );
        assert_eq!(
            index_url(DEFAULT_BASE_URL),
            "https://hackage.haskell.org/01-index.tar.gz"
        );
    }
}

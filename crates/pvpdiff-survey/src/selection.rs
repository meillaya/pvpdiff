//! Candidate package selection and sharding.
//!
//! The index rows arrive in file order with one row per package version. A
//! candidate is a package name plus its parseable versions, newest first. The
//! survey selects its work in three steps: sort the packages by name, keep one
//! shard of a stable name hash, and finally apply the row limit. Sorting first
//! makes the shard and limit independent of the order the index file was
//! written in.

use std::collections::BTreeMap;

use pvpdiff_core::HaskellVersion;
use pvpdiff_index::IndexRow;

use crate::SurveyError;

/// The 64 bit FNV-1a offset basis.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// The 64 bit FNV-1a prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// One candidate package with its versions, newest first.
#[derive(Debug, Clone)]
pub(crate) struct PackageCandidate {
    /// Package name.
    pub(crate) name: String,
    /// Parseable versions, newest first.
    pub(crate) versions: Vec<CandidateVersion>,
}

/// One version from the index with its parsed form.
#[derive(Debug, Clone)]
pub(crate) struct CandidateVersion {
    /// Version text as written in the index.
    pub(crate) text: String,
    /// Parsed version, used for ordering and bump arithmetic.
    pub(crate) parsed: HaskellVersion,
}

/// Select the candidate packages for one run.
///
/// Versions that do not parse as Haskell versions are dropped before sorting;
/// they cannot produce a PVP bump and would make the order depend on the index
/// text. A package left without parseable versions is dropped as well.
pub(crate) fn select_candidates(
    rows: &[IndexRow],
    shard: Option<(usize, usize)>,
    limit: Option<usize>,
) -> Result<Vec<PackageCandidate>, SurveyError> {
    if let Some((index, count)) = shard
        && (count == 0 || index >= count)
    {
        return Err(SurveyError::InvalidShard { index, count });
    }

    // `limit == Some(0)` selects nothing, so the grouping below can be skipped.
    if limit == Some(0) {
        return Ok(Vec::new());
    }

    let mut grouped: BTreeMap<&str, BTreeMap<&str, HaskellVersion>> = BTreeMap::new();
    for row in rows {
        let Ok(parsed) = HaskellVersion::parse(&row.version) else {
            continue;
        };
        grouped
            .entry(row.package.as_str())
            .or_default()
            .insert(row.version.as_str(), parsed);
    }

    let mut candidates = Vec::new();
    for (package, versions) in grouped {
        // The shard decides on the package name only, so a package never moves
        // between shards when its version list changes.
        if let Some((index, count)) = shard
            && shard_index(package, count) != index
        {
            continue;
        }

        let mut versions: Vec<CandidateVersion> = versions
            .into_iter()
            .map(|(text, parsed)| CandidateVersion {
                text: text.to_string(),
                parsed,
            })
            .collect();
        versions.sort_by(|left, right| {
            right
                .parsed
                .cmp(&left.parsed)
                .then_with(|| right.text.cmp(&left.text))
        });
        candidates.push(PackageCandidate {
            name: package.to_string(),
            versions,
        });
        if let Some(limit) = limit
            && candidates.len() >= limit
        {
            break;
        }
    }
    Ok(candidates)
}

/// Shard index of a package name.
///
/// The hash is FNV-1a over the name bytes, so the same checkout and the same
/// shard count always place a package in the same shard.
pub(crate) fn shard_index(package: &str, count: usize) -> usize {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in package.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    (hash % count as u64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an index row with no dependencies; selection only reads the
    /// package and version fields.
    fn row(package: &str, version: &str) -> IndexRow {
        IndexRow {
            package: package.to_string(),
            version: version.to_string(),
            conditional: false,
            dependencies: Vec::new(),
            cabal_sha256: String::new(),
        }
    }

    /// Check that packages sort by name, versions sort numerically in
    /// descending order, and a version that does not parse is dropped.
    #[test]
    fn sorts_packages_and_versions() {
        let rows = [
            row("beta", "2.0.0"),
            row("alpha", "9.0.0"),
            row("alpha", "10.0.0"),
            row("gamma", "not-a-version"),
            row("beta", "1.0.0"),
            row("alpha", "2.0.0"),
            row("alpha", "10.0.0"),
        ];
        let candidates = select_candidates(&rows, None, None).expect("selection");
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
        assert_eq!(
            candidates[0]
                .versions
                .iter()
                .map(|version| version.text.as_str())
                .collect::<Vec<_>>(),
            vec!["10.0.0", "9.0.0", "2.0.0"]
        );
        assert_eq!(candidates[1].versions.len(), 2);
    }

    /// Check that the limit counts packages rather than versions, that a zero
    /// limit selects nothing, and that a shard keeps only its own packages.
    #[test]
    fn applies_shard_and_limit() {
        let rows = [
            row("alpha", "1.0.0"),
            row("beta", "1.0.0"),
            row("gamma", "1.0.0"),
            row("delta", "1.0.0"),
        ];
        let limited = select_candidates(&rows, None, Some(2)).expect("limit");
        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0].name, "alpha");
        assert_eq!(limited[1].name, "beta");
        assert!(
            select_candidates(&rows, None, Some(0))
                .expect("zero limit")
                .is_empty()
        );

        let shard = select_candidates(&rows, Some((1, 2)), None).expect("shard");
        assert!(
            shard
                .iter()
                .all(|candidate| { shard_index(&candidate.name, 2) == 1 })
        );
        let other = select_candidates(&rows, Some((0, 2)), None).expect("other shard");
        let names: Vec<&str> = shard
            .iter()
            .chain(other.iter())
            .map(|candidate| candidate.name.as_str())
            .collect();
        assert_eq!(names.len(), rows.len());

        // The limit applies after the shard, so a shard of one package plus a
        // limit of one returns that package.
        let sharded_limit = select_candidates(&rows, Some((1, 2)), Some(1)).expect("shard limit");
        assert_eq!(sharded_limit.len(), shard.len().min(1));
    }

    /// Check that a shard count of zero and an index outside the count are
    /// rejected instead of dividing by zero or selecting nothing.
    #[test]
    fn rejects_invalid_shards() {
        let rows = [row("alpha", "1.0.0")];
        assert!(matches!(
            select_candidates(&rows, Some((0, 0)), None),
            Err(SurveyError::InvalidShard { index: 0, count: 0 })
        ));
        assert!(matches!(
            select_candidates(&rows, Some((3, 3)), None),
            Err(SurveyError::InvalidShard { index: 3, count: 3 })
        ));
    }

    /// Check that the name hash is deterministic, stays below the count, and
    /// spreads a generated name set over every shard.
    #[test]
    fn hashes_names_into_shards() {
        const COUNT: usize = 4;
        let mut seen = [false; COUNT];
        for offset in 0..100 {
            let name = format!("pkg-{offset}");
            let index = shard_index(&name, COUNT);
            assert_eq!(index, shard_index(&name, COUNT));
            assert!(index < COUNT);
            seen[index] = true;
        }
        assert!(seen.iter().all(|hit| *hit));
    }
}

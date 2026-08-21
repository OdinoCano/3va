use sha2::Digest;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::ExtractLimits;

pub struct PackageFetcher {
    registry: String,
    _cache_dir: PathBuf,
    client: reqwest::Client,
}

impl PackageFetcher {
    pub fn new(registry: &str, cache_dir: PathBuf) -> Self {
        Self {
            registry: registry.to_string(),
            _cache_dir: cache_dir,
            client: reqwest::Client::new(),
        }
    }

    pub async fn fetch(&self, name: &str, version: &str) -> anyhow::Result<Vec<u8>> {
        let url = format!("{}/{}/-/{}-{}.tgz", self.registry, name, name, version);

        tracing::info!("Fetching {}@{} from {}", name, version, url);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to fetch {}@{}: {}",
                name,
                version,
                response.status()
            );
        }

        let bytes = response.bytes().await?.to_vec();

        Ok(bytes)
    }

    pub fn extract(&self, tarball: &[u8], dest: &PathBuf) -> anyhow::Result<()> {
        self.extract_with_limits(tarball, dest, ExtractLimits::default())
    }

    pub(crate) fn extract_with_limits(
        &self,
        tarball: &[u8],
        dest: &PathBuf,
        lim: ExtractLimits,
    ) -> anyhow::Result<()> {
        let decoder = flate2::read::GzDecoder::new(tarball);
        let mut archive = tar::Archive::new(decoder);

        std::fs::create_dir_all(dest)?;
        // Canonical dest is used to verify no entry escapes the package directory.
        let dest_canonical = std::fs::canonicalize(dest).unwrap_or_else(|_| dest.clone());
        let mut total_declared: u64 = 0;

        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?;

            // npm tarballs always have a "package/" prefix — strip it.
            let cleaned: std::path::PathBuf = path.iter().skip(1).collect();

            // Skip root directory entry ("package/" itself becomes empty after skip).
            if cleaned.as_os_str().is_empty() {
                continue;
            }

            // Security: reject path traversal and absolute paths before joining.
            let is_unsafe = cleaned.components().any(|c| {
                matches!(
                    c,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            });
            if is_unsafe {
                tracing::warn!("Skipping unsafe path in package tarball: {:?}", path);
                continue;
            }

            let out_path = dest.join(&cleaned);

            // Security: verify the resolved output path stays within dest.
            // Walk up to the first existing ancestor so canonicalize doesn't fail on
            // not-yet-created intermediate directories.
            let check_base = {
                let mut p = out_path.clone();
                loop {
                    if p.exists() {
                        break std::fs::canonicalize(&p).unwrap_or(dest_canonical.clone());
                    }
                    if !p.pop() {
                        break dest_canonical.clone();
                    }
                }
            };
            if !check_base.starts_with(&dest_canonical) {
                tracing::warn!("Skipping path that escapes package directory: {:?}", path);
                continue;
            }

            // Create parent directories before extracting.
            if let Some(parent) = out_path.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                tracing::warn!("Could not create parent dir for {:?}: {}", cleaned, e);
                continue;
            }

            // Handle entry types explicitly.
            match entry.header().entry_type() {
                tar::EntryType::Directory => {
                    // Directory entries: just ensure the dir exists.
                    if let Err(e) = std::fs::create_dir_all(&out_path) {
                        tracing::warn!("Could not create dir {:?}: {}", cleaned, e);
                    }
                    continue;
                }
                tar::EntryType::Symlink | tar::EntryType::Link => {
                    // Symlinks from untrusted packages are a supply-chain risk — always skip.
                    tracing::debug!("Skipping symlink in package: {:?}", cleaned);
                    continue;
                }
                _ => {}
            }

            // Enforce extraction caps BEFORE writing file content. Tar readers
            // consume exactly `header().size()` bytes per entry, so the
            // declared size is what unpack() will write. Unlike per-entry
            // extraction hiccups, a cap violation aborts the whole extract:
            // it indicates a malicious/broken tarball, not a flaky entry.
            let declared = entry.header().size()?;
            if declared > lim.max_file_bytes {
                anyhow::bail!(
                    "package extraction aborted: entry '{}' declares {} bytes, exceeding the per-file cap of {} bytes (possible decompression bomb)",
                    cleaned.display(),
                    declared,
                    lim.max_file_bytes
                );
            }
            total_declared = total_declared.saturating_add(declared);
            if total_declared > lim.max_total_bytes {
                anyhow::bail!(
                    "package extraction aborted: cumulative payload reached {} bytes, exceeding the total cap of {} bytes (possible decompression bomb)",
                    total_declared,
                    lim.max_total_bytes
                );
            }

            // Extract the regular file. Skip on error so one bad entry doesn't abort
            // the entire package (common with native-code packages like react-native).
            if let Err(e) = entry.unpack(&out_path) {
                tracing::warn!("Failed to extract {:?}: {}", cleaned, e);
            }
        }

        Ok(())
    }

    pub fn verify_hash(tarball: &[u8], expected: &str) -> bool {
        let mut hasher = sha2::Sha256::new();
        hasher.update(tarball);
        let result = hasher.finalize();
        let actual = hex::encode(result);
        actual == expected
    }
}

pub struct PackageCache {
    _cache_dir: PathBuf,
    metadata: HashMap<String, CacheEntry>,
    max_size: u64,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    path: PathBuf,
    size: u64,
    last_access: std::time::SystemTime,
}

impl PackageCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            _cache_dir: cache_dir,
            metadata: HashMap::new(),
            max_size: 1024 * 1024 * 1024,
        }
    }

    pub fn get(&self, name: &str, version: &str) -> Option<PathBuf> {
        let key = format!("{}@{}", name, version);
        self.metadata.get(&key).map(|e| e.path.clone())
    }

    pub fn put(&mut self, name: &str, version: &str, path: PathBuf) {
        let key = format!("{}@{}", name, version);
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        self.metadata.insert(
            key,
            CacheEntry {
                path,
                size,
                last_access: std::time::SystemTime::now(),
            },
        );
    }

    pub fn prune(&mut self) -> anyhow::Result<()> {
        let total_size: u64 = self.metadata.values().map(|e| e.size).sum();

        if total_size <= self.max_size {
            return Ok(());
        }

        let mut entries: Vec<_> = self
            .metadata
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        entries.sort_by_key(|a| a.1.last_access);

        let mut current_size = total_size;
        for (key, _entry) in entries {
            if current_size <= self.max_size / 2 {
                break;
            }

            if let Some(e) = self.metadata.get(&key) {
                current_size -= e.size;
                let _ = std::fs::remove_dir_all(&e.path);
            }
            self.metadata.remove(&key);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_cache(entries: &[(&str, &str, u64)]) -> PackageCache {
        let mut cache = PackageCache::new(PathBuf::from("/tmp/3va-cache-test"));
        for (name, version, size) in entries {
            let key = format!("{}@{}", name, version);
            cache.metadata.insert(
                key,
                CacheEntry {
                    path: PathBuf::from(format!("/tmp/3va-cache-test/{}-{}", name, version)),
                    size: *size,
                    last_access: std::time::SystemTime::now(),
                },
            );
            cache.max_size = 1024 * 1024; // 1 MB limit for tests
        }
        cache
    }

    #[test]
    fn cache_get_returns_none_when_empty() {
        let cache = PackageCache::new(PathBuf::from("/tmp/3va-test"));
        assert!(cache.get("lodash", "4.17.21").is_none());
    }

    #[test]
    fn cache_put_then_get_returns_path() {
        let mut cache = PackageCache::new(PathBuf::from("/tmp/3va-test"));
        let path = PathBuf::from("/tmp/3va-test/lodash-4.17.21");
        cache.put("lodash", "4.17.21", path.clone());
        assert_eq!(cache.get("lodash", "4.17.21"), Some(path));
    }

    #[test]
    fn cache_get_different_version_returns_none() {
        let mut cache = PackageCache::new(PathBuf::from("/tmp/3va-test"));
        cache.put("lodash", "4.17.21", PathBuf::from("/tmp/lodash-4"));
        assert!(cache.get("lodash", "4.17.0").is_none());
    }

    #[test]
    fn prune_noop_when_under_limit() {
        // 100 bytes total, limit 1 MB → nothing pruned
        let mut cache = fake_cache(&[("pkg-a", "1.0.0", 50), ("pkg-b", "1.0.0", 50)]);
        cache.max_size = 1024 * 1024;
        cache.prune().unwrap();
        assert_eq!(cache.metadata.len(), 2, "nothing should be pruned");
    }

    #[test]
    fn prune_removes_oldest_entries_when_over_limit() {
        let mut cache = fake_cache(&[
            ("big-a", "1.0.0", 600 * 1024),
            ("big-b", "1.0.0", 600 * 1024),
        ]);
        // Total 1200 KB > 1 MB limit → prune must remove at least one entry
        cache.prune().unwrap();
        // After pruning, total size should be at most max_size/2 = 512 KB
        let remaining: u64 = cache.metadata.values().map(|e| e.size).sum();
        assert!(
            remaining <= cache.max_size / 2,
            "remaining {remaining} should be ≤ {}",
            cache.max_size / 2
        );
    }

    #[test]
    fn prune_empty_cache_is_noop() {
        let mut cache = PackageCache::new(PathBuf::from("/tmp/3va-test"));
        cache.max_size = 0; // Would prune everything if there were entries
        cache.prune().unwrap();
        assert_eq!(cache.metadata.len(), 0);
    }

    // ── extraction cap tests ──────────────────────────────────────────────

    fn make_tgz(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(enc);
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, &data[..]).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn test_fetcher() -> PackageFetcher {
        PackageFetcher::new(
            "http://registry.invalid",
            PathBuf::from("/tmp/3va-fetcher-test"),
        )
    }

    fn tiny_caps(file: u64, total: u64) -> ExtractLimits {
        ExtractLimits {
            max_file_bytes: file,
            max_total_bytes: total,
        }
    }

    #[test]
    fn extract_aborts_when_entry_exceeds_per_file_cap() {
        let tgz = make_tgz(&[("package/big.bin", vec![0u8; 4096])]);
        let dest = tempfile::tempdir().unwrap();
        let err = test_fetcher()
            .extract_with_limits(&tgz, &dest.path().to_path_buf(), tiny_caps(1024, u64::MAX))
            .unwrap_err();
        assert!(
            err.to_string().contains("per-file cap"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn extract_aborts_when_cumulative_payload_exceeds_total_cap() {
        // Each file fits the per-file cap, but their sum busts the total cap.
        let tgz = make_tgz(&[
            ("package/a.bin", vec![0u8; 2048]),
            ("package/b.bin", vec![0u8; 2048]),
        ]);
        let dest = tempfile::tempdir().unwrap();
        let err = test_fetcher()
            .extract_with_limits(&tgz, &dest.path().to_path_buf(), tiny_caps(2048, 4095))
            .unwrap_err();
        assert!(
            err.to_string().contains("total cap"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn extract_succeeds_under_generous_caps() {
        let payload = b"hello extraction".to_vec();
        let tgz = make_tgz(&[("package/index.js", payload.clone())]);
        let dest = tempfile::tempdir().unwrap();
        test_fetcher()
            .extract_with_limits(
                &tgz,
                &dest.path().to_path_buf(),
                tiny_caps(u64::MAX, u64::MAX),
            )
            .unwrap();
        let written = std::fs::read(dest.path().join("index.js")).unwrap();
        assert_eq!(written, payload);
    }
}

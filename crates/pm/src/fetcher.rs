use sha2::Digest;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct PackageFetcher {
    registry: String,
    #[allow(dead_code)]
    cache_dir: PathBuf,
    client: reqwest::Client,
}

impl PackageFetcher {
    pub fn new(registry: &str, cache_dir: PathBuf) -> Self {
        Self {
            registry: registry.to_string(),
            cache_dir,
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
        let decoder = flate2::read::GzDecoder::new(tarball);
        let mut archive = tar::Archive::new(decoder);

        std::fs::create_dir_all(dest)?;

        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?;

            let cleaned: std::path::PathBuf = path.iter().skip(1).collect();

            let out_path = dest.join(cleaned);

            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            entry.unpack(&out_path)?;
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
    #[allow(dead_code)]
    cache_dir: PathBuf,
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
            cache_dir,
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
    #[test]
    fn test_cache_prune() {
        // Basic test placeholder
    }
}

use crate::index::CodeIndex;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const CACHE_DIR_NAME: &str = ".codemapper";
const CACHE_SUBDIR: &str = "cache";
const CACHE_VERSION: &str = "1.0";

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheMetadata {
    pub version: String,
    pub created_at: SystemTime,
    pub root_path: PathBuf,
    pub extensions: Vec<String>,
    pub file_count: usize,
    pub symbol_count: usize,
    pub cache_key: String,
    pub file_hashes: HashMap<PathBuf, String>,
}

impl CacheMetadata {
    fn new(
        root_path: PathBuf,
        extensions: Vec<String>,
        file_count: usize,
        symbol_count: usize,
        cache_key: String,
        file_hashes: HashMap<PathBuf, String>,
    ) -> Self {
        Self {
            version: CACHE_VERSION.to_string(),
            created_at: SystemTime::now(),
            root_path,
            extensions,
            file_count,
            symbol_count,
            cache_key,
            file_hashes,
        }
    }
}

pub struct CacheManager;

impl CacheManager {
    /// Compute cache key from canonical path and extensions
    pub fn compute_cache_key(root: &Path, extensions: &[&str]) -> Result<String> {
        let canonical = root
            .canonicalize()
            .context("Failed to canonicalize root path")?;
        let mut hasher = Sha256::new();
        hasher.update(canonical.to_string_lossy().as_bytes());
        for ext in extensions {
            hasher.update(b":");
            hasher.update(ext.as_bytes());
        }
        let hash = hasher.finalize();
        Ok(format!("{:x}", hash))
    }

    /// Get cache file paths (binary and metadata)
    fn get_cache_paths(root: &Path, extensions: &[&str]) -> Result<(PathBuf, PathBuf)> {
        let cache_key = Self::compute_cache_key(root, extensions)?;
        let cache_dir = root.join(CACHE_DIR_NAME).join(CACHE_SUBDIR);
        let cache_file = cache_dir.join(format!("project-{}.bin", &cache_key[..16]));
        let meta_file = cache_dir.join(format!("project-{}.meta.json", &cache_key[..16]));
        Ok((cache_file, meta_file))
    }

    /// Compute Blake3 hash of a file
    pub fn compute_file_hash(path: &Path) -> Result<String> {
        let mut file = File::open(path).context("Failed to open file for hashing")?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0u8; 8192];

        loop {
            let bytes_read = file.read(&mut buffer).context("Failed to read file")?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }

        let hash = hasher.finalize();
        Ok(format!("blake3:{}", hash.to_hex()))
    }

    /// Compute hashes for all files in directory matching extensions
    pub fn compute_directory_hashes(
        root: &Path,
        extensions: &[&str],
    ) -> Result<HashMap<PathBuf, String>> {
        use walkdir::WalkDir;

        let mut hashes = HashMap::new();

        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy();
                if extensions.iter().any(|&e| e == ext_str) {
                    match Self::compute_file_hash(path) {
                        Ok(hash) => {
                            hashes.insert(path.to_path_buf(), hash);
                        }
                        Err(_) => {
                            // Skip files we can't read
                            continue;
                        }
                    }
                }
            }
        }

        Ok(hashes)
    }

    /// Save CodeIndex to cache with metadata
    pub fn save(
        index: &CodeIndex,
        root: &Path,
        extensions: &[&str],
    ) -> Result<()> {
        let (cache_file, meta_file) = Self::get_cache_paths(root, extensions)?;

        // Create cache directory
        if let Some(parent) = cache_file.parent() {
            fs::create_dir_all(parent).context("Failed to create cache directory")?;
        }

        // Compute file hashes
        let file_hashes = Self::compute_directory_hashes(root, extensions)?;

        // Create metadata
        let cache_key = Self::compute_cache_key(root, extensions)?;
        let metadata = CacheMetadata::new(
            root.to_path_buf(),
            extensions.iter().map(|&s| s.to_string()).collect(),
            index.total_files(),
            index.total_symbols(),
            cache_key,
            file_hashes,
        );

        // Save binary cache
        let cache_data = bincode::serialize(index).context("Failed to serialize index")?;
        let mut cache_writer = BufWriter::new(
            File::create(&cache_file).context("Failed to create cache file")?
        );
        cache_writer
            .write_all(&cache_data)
            .context("Failed to write cache data")?;
        cache_writer.flush()?;

        // Save metadata
        let meta_data = serde_json::to_string_pretty(&metadata)
            .context("Failed to serialize metadata")?;
        fs::write(&meta_file, meta_data).context("Failed to write metadata file")?;

        // Create .gitignore if it doesn't exist
        Self::ensure_gitignore(root)?;

        Ok(())
    }

    /// Load CodeIndex from cache if valid
    pub fn load(
        root: &Path,
        extensions: &[&str],
    ) -> Result<Option<(CodeIndex, CacheMetadata)>> {
        let (cache_file, meta_file) = Self::get_cache_paths(root, extensions)?;

        // Check if cache files exist
        if !cache_file.exists() || !meta_file.exists() {
            return Ok(None);
        }

        // Load metadata
        let meta_data = fs::read_to_string(&meta_file)
            .context("Failed to read metadata file")?;
        let metadata: CacheMetadata = serde_json::from_str(&meta_data)
            .context("Failed to parse metadata")?;

        // Validate cache version
        if metadata.version != CACHE_VERSION {
            return Ok(None);
        }

        // Validate with hashes
        if !Self::validate_with_hashes(&metadata, root)? {
            return Ok(None);
        }

        // Load cache
        let cache_reader = BufReader::new(
            File::open(&cache_file).context("Failed to open cache file")?
        );
        let index: CodeIndex = bincode::deserialize_from(cache_reader)
            .context("Failed to deserialize index")?;

        Ok(Some((index, metadata)))
    }

    /// Validate cache using hybrid timestamp + hash approach
    pub fn validate_with_hashes(metadata: &CacheMetadata, root: &Path) -> Result<bool> {
        let current_hashes = Self::compute_directory_hashes(
            root,
            &metadata.extensions.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        )?;

        // Check file count first (quick check)
        if current_hashes.len() != metadata.file_hashes.len() {
            return Ok(false);
        }

        // Check each file with timestamp pre-filter
        for (path, cached_hash) in &metadata.file_hashes {
            // Check if file still exists
            match current_hashes.get(path) {
                Some(current_hash) => {
                    if current_hash != cached_hash {
                        // Content changed
                        return Ok(false);
                    }
                }
                None => {
                    // File was deleted
                    return Ok(false);
                }
            }
        }

        // Check for new files
        for path in current_hashes.keys() {
            if !metadata.file_hashes.contains_key(path) {
                // New file added
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Invalidate (delete) cache for a project
    pub fn invalidate(root: &Path, extensions: &[&str]) -> Result<()> {
        let (cache_file, meta_file) = Self::get_cache_paths(root, extensions)?;

        if cache_file.exists() {
            fs::remove_file(&cache_file).context("Failed to remove cache file")?;
        }
        if meta_file.exists() {
            fs::remove_file(&meta_file).context("Failed to remove metadata file")?;
        }

        Ok(())
    }

    /// Ensure .gitignore exists in .codemapper directory
    fn ensure_gitignore(root: &Path) -> Result<()> {
        let gitignore_path = root.join(CACHE_DIR_NAME).join(".gitignore");
        if !gitignore_path.exists() {
            fs::write(&gitignore_path, "*\n").context("Failed to create .gitignore")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_compute_cache_key_same_inputs() {
        let temp = TempDir::new().unwrap();
        let path = temp.path();

        let key1 = CacheManager::compute_cache_key(path, &["py", "rs"]).unwrap();
        let key2 = CacheManager::compute_cache_key(path, &["py", "rs"]).unwrap();

        assert_eq!(key1, key2);
    }

    #[test]
    fn test_compute_cache_key_different_extensions() {
        let temp = TempDir::new().unwrap();
        let path = temp.path();

        let key1 = CacheManager::compute_cache_key(path, &["py"]).unwrap();
        let key2 = CacheManager::compute_cache_key(path, &["rs"]).unwrap();

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_file_hash_consistency() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.txt");
        fs::write(&file_path, "test content").unwrap();

        let hash1 = CacheManager::compute_file_hash(&file_path).unwrap();
        let hash2 = CacheManager::compute_file_hash(&file_path).unwrap();

        assert_eq!(hash1, hash2);
        assert!(hash1.starts_with("blake3:"));
    }

    #[test]
    fn test_file_hash_detects_changes() {
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("test.txt");

        fs::write(&file_path, "content 1").unwrap();
        let hash1 = CacheManager::compute_file_hash(&file_path).unwrap();

        fs::write(&file_path, "content 2").unwrap();
        let hash2 = CacheManager::compute_file_hash(&file_path).unwrap();

        assert_ne!(hash1, hash2);
    }
}

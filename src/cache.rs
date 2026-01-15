use crate::index::CodeIndex;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const CACHE_DIR_NAME: &str = ".codemapper";
const CACHE_SUBDIR: &str = "cache";
const CACHE_VERSION: &str = "1.2";

#[derive(Debug)]
pub enum ValidationResult {
    Valid,
    Invalid,
    NeedsUpdate(Vec<FileChange>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileChangeKind {
    Modified,
    Added,
    Deleted,
}

#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: PathBuf,
    pub kind: FileChangeKind,
    pub size: Option<u64>,
    pub mtime: Option<SystemTime>,
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub hash: String,
    pub size: u64,
    pub mtime: SystemTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheMetadata {
    pub version: String,
    pub created_at: SystemTime,
    pub root_path: PathBuf,
    pub extensions: Vec<String>,
    pub file_count: usize,
    pub symbol_count: usize,
    pub cache_key: String,
    pub file_metadata: HashMap<PathBuf, FileMetadata>,
}

impl CacheMetadata {
    fn new(
        root_path: PathBuf,
        extensions: Vec<String>,
        file_count: usize,
        symbol_count: usize,
        cache_key: String,
        file_metadata: HashMap<PathBuf, FileMetadata>,
    ) -> Self {
        Self {
            version: CACHE_VERSION.to_string(),
            created_at: SystemTime::now(),
            root_path,
            extensions,
            file_count,
            symbol_count,
            cache_key,
            file_metadata,
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
    fn get_cache_paths(
        root: &Path,
        extensions: &[&str],
        cache_dir: Option<&Path>,
    ) -> Result<(PathBuf, PathBuf)> {
        let cache_key = Self::compute_cache_key(root, extensions)?;
        let base_dir = cache_dir
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| root.join(CACHE_DIR_NAME));
        let cache_dir_path = base_dir.join(CACHE_SUBDIR);
        let cache_file = cache_dir_path.join(format!("project-{}.bin", &cache_key[..16]));
        let meta_file = cache_dir_path.join(format!("project-{}.meta.json", &cache_key[..16]));
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

    /// Collect file metadata (hash, size, mtime) for all files in directory
    pub fn collect_file_metadata(
        root: &Path,
        extensions: &[&str],
    ) -> Result<HashMap<PathBuf, FileMetadata>> {
        use walkdir::WalkDir;

        const IGNORED_DIRS: &[&str] = &[
            ".codemapper",
            ".git",
            "node_modules",
            "__pycache__",
            "target",
            "dist",
            "build",
        ];

        let mut metadata_map = HashMap::new();

        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.file_type().is_dir() {
                    let dir_name = e.file_name().to_string_lossy();
                    !IGNORED_DIRS.contains(&dir_name.as_ref())
                } else {
                    true
                }
            })
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy();
                if extensions.iter().any(|&e| e == ext_str) {
                    match Self::compute_file_metadata_single(path) {
                        Ok(metadata) => {
                            metadata_map.insert(path.to_path_buf(), metadata);
                        }
                        Err(_) => {
                            // Skip files we can't read
                            continue;
                        }
                    }
                }
            }
        }

        Ok(metadata_map)
    }

    /// Compute metadata for a single file (hash, size, mtime)
    fn compute_file_metadata_single(path: &Path) -> Result<FileMetadata> {
        let fs_metadata = fs::metadata(path).context("Failed to read file metadata")?;
        let size = fs_metadata.len();
        let mtime = fs_metadata
            .modified()
            .context("Failed to get modification time")?;
        let hash = Self::compute_file_hash(path)?;

        Ok(FileMetadata { hash, size, mtime })
    }

    fn file_metadata_for_change(change: &FileChange) -> Result<FileMetadata> {
        let hash = match &change.hash {
            Some(hash) => hash.clone(),
            None => {
                if let FileChangeKind::Modified | FileChangeKind::Added = change.kind {
                    let path = &change.path;
                    Self::compute_file_metadata_single(path)?.hash
                } else {
                    return Err(anyhow::anyhow!("Deleted files cannot produce metadata"));
                }
            }
        };

        let size = change
            .size
            .or_else(|| {
                if let FileChangeKind::Modified | FileChangeKind::Added = change.kind {
                    fs::metadata(&change.path).map(|m| m.len()).ok()
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow::anyhow!("Missing size for change: {}", change.path.display()))?;

        let mtime = change
            .mtime
            .or_else(|| {
                if let FileChangeKind::Modified | FileChangeKind::Added = change.kind {
                    fs::metadata(&change.path).and_then(|m| m.modified()).ok()
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                anyhow::anyhow!("Missing mtime for change: {}", change.path.display())
            })?;

        Ok(FileMetadata { hash, size, mtime })
    }

    /// Collect file stats (size, mtime) without hashing - fast path for validation
    fn collect_file_stats(
        root: &Path,
        extensions: &[String],
    ) -> Result<HashMap<PathBuf, (u64, SystemTime)>> {
        use walkdir::WalkDir;

        const IGNORED_DIRS: &[&str] = &[
            ".codemapper",
            ".git",
            "node_modules",
            "__pycache__",
            "target",
            "dist",
            "build",
        ];

        let mut stats = HashMap::new();

        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.file_type().is_dir() {
                    let dir_name = e.file_name().to_string_lossy();
                    !IGNORED_DIRS.contains(&dir_name.as_ref())
                } else {
                    true
                }
            })
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy();
                if extensions.iter().any(|e| e == &ext_str) {
                    match fs::metadata(path) {
                        Ok(metadata) => {
                            let size = metadata.len();
                            let mtime = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                            stats.insert(path.to_path_buf(), (size, mtime));
                        }
                        Err(_) => continue,
                    }
                }
            }
        }

        Ok(stats)
    }

    /// Save CodeIndex to cache with metadata
    pub fn save(
        index: &CodeIndex,
        root: &Path,
        extensions: &[&str],
        cache_dir: Option<&Path>,
    ) -> Result<CacheMetadata> {
        Self::save_internal(index, root, extensions, None, None, cache_dir)
    }

    pub fn save_with_changes(
        index: &CodeIndex,
        root: &Path,
        extensions: &[&str],
        previous: &CacheMetadata,
        changes: &[FileChange],
        cache_dir: Option<&Path>,
    ) -> Result<CacheMetadata> {
        Self::save_internal(index, root, extensions, Some(previous), Some(changes), cache_dir)
    }

    fn save_internal(
        index: &CodeIndex,
        root: &Path,
        extensions: &[&str],
        previous: Option<&CacheMetadata>,
        changes: Option<&[FileChange]>,
        cache_dir: Option<&Path>,
    ) -> Result<CacheMetadata> {
        let (cache_file, meta_file) = Self::get_cache_paths(root, extensions, cache_dir)?;

        if let Some(parent) = cache_file.parent() {
            fs::create_dir_all(parent).context("Failed to create cache directory")?;
        }

        let mut file_metadata = match previous {
            Some(prev) => prev.file_metadata.clone(),
            None => Self::collect_file_metadata(root, extensions)?,
        };

        if let Some(changes) = changes {
            for change in changes {
                match change.kind {
                    FileChangeKind::Deleted => {
                        file_metadata.remove(&change.path);
                    }
                    FileChangeKind::Modified | FileChangeKind::Added => {
                        let entry = Self::file_metadata_for_change(change)?;
                        file_metadata.insert(change.path.clone(), entry);
                    }
                }
            }
        }

        Self::assert_metadata_consistency(index, &file_metadata)?;

        let cache_key = Self::compute_cache_key(root, extensions)?;
        let mut metadata = CacheMetadata::new(
            root.to_path_buf(),
            extensions.iter().map(|&s| s.to_string()).collect(),
            index.total_files(),
            index.total_symbols(),
            cache_key,
            file_metadata,
        );

        metadata.file_count = index.total_files();
        metadata.symbol_count = index.total_symbols();

        let cache_data = bincode::serialize(index).context("Failed to serialize index")?;
        let mut cache_writer =
            BufWriter::new(File::create(&cache_file).context("Failed to create cache file")?);
        cache_writer
            .write_all(&cache_data)
            .context("Failed to write cache data")?;
        cache_writer.flush()?;

        let meta_data =
            serde_json::to_string_pretty(&metadata).context("Failed to serialize metadata")?;
        fs::write(&meta_file, meta_data).context("Failed to write metadata file")?;

        Self::ensure_gitignore(root, cache_dir)?;

        Ok(metadata)
    }

    fn assert_metadata_consistency(
        index: &CodeIndex,
        file_metadata: &HashMap<PathBuf, FileMetadata>,
    ) -> Result<()> {
        let expected_files = index.total_files();
        if file_metadata.len() != expected_files {
            return Err(anyhow!(
                "file metadata count mismatch: expected {} entries, found {}",
                expected_files,
                file_metadata.len()
            ));
        }

        for file in index.files() {
            if !file_metadata.contains_key(&file.path) {
                return Err(anyhow!("missing metadata for {}", file.path.display()));
            }
        }

        for (path, metadata) in file_metadata {
            if metadata.hash.is_empty() {
                return Err(anyhow!("empty hash for {}", path.display()));
            }
        }

        Ok(())
    }

    /// Load CodeIndex from cache with validation
    /// Returns (index, metadata, changed_files)
    /// changed_files is empty if cache is valid, or contains files that need updating
    pub fn load(
        root: &Path,
        extensions: &[&str],
        cache_dir: Option<&Path>,
    ) -> Result<Option<(CodeIndex, CacheMetadata, Vec<FileChange>)>> {
        let (cache_file, meta_file) = Self::get_cache_paths(root, extensions, cache_dir)?;

        // Check if cache files exist
        if !cache_file.exists() || !meta_file.exists() {
            return Ok(None);
        }

        // Load metadata
        let meta_data = fs::read_to_string(&meta_file).context("Failed to read metadata file")?;
        let metadata: CacheMetadata =
            serde_json::from_str(&meta_data).context("Failed to parse metadata")?;

        // Validate cache version
        if metadata.version != CACHE_VERSION {
            return Ok(None);
        }

        // Validate with hashes
        let validation_result = Self::validate_with_hashes(&metadata, root)?;

        match validation_result {
            ValidationResult::Invalid => {
                // Too many changes, full rebuild needed
                return Ok(None);
            }
            ValidationResult::Valid => {
                // Load cache as-is
                let cache_reader =
                    BufReader::new(File::open(&cache_file).context("Failed to open cache file")?);
                let index: CodeIndex = bincode::deserialize_from(cache_reader)
                    .context("Failed to deserialize index")?;

                Ok(Some((index, metadata, Vec::new())))
            }
            ValidationResult::NeedsUpdate(changed_files) => {
                // Load cache but needs incremental update
                let cache_reader =
                    BufReader::new(File::open(&cache_file).context("Failed to open cache file")?);
                let index: CodeIndex = bincode::deserialize_from(cache_reader)
                    .context("Failed to deserialize index")?;

                Ok(Some((index, metadata, changed_files)))
            }
        }
    }

    /// Validate cache using size + mtime pre-filter (git's approach)
    /// Returns ValidationResult indicating if cache is valid, invalid, or needs incremental update
    pub fn validate_with_hashes(metadata: &CacheMetadata, root: &Path) -> Result<ValidationResult> {
        let current_stats = Self::collect_file_stats(root, &metadata.extensions)?;

        let mut changes = Vec::new();

        for path in metadata.file_metadata.keys() {
            if !current_stats.contains_key(path) {
                changes.push(FileChange {
                    path: path.clone(),
                    kind: FileChangeKind::Deleted,
                    size: None,
                    mtime: None,
                    hash: None,
                });
            }
        }

        for (path, (current_size, current_mtime)) in &current_stats {
            if let Some(cached) = metadata.file_metadata.get(path) {
                if current_size != &cached.size || current_mtime != &cached.mtime {
                    let hash = Self::compute_file_hash(path)?;
                    if hash != cached.hash {
                        changes.push(FileChange {
                            path: path.clone(),
                            kind: FileChangeKind::Modified,
                            size: Some(*current_size),
                            mtime: Some(*current_mtime),
                            hash: Some(hash),
                        });
                    }
                }
            } else {
                changes.push(FileChange {
                    path: path.clone(),
                    kind: FileChangeKind::Added,
                    size: Some(*current_size),
                    mtime: Some(*current_mtime),
                    hash: Some(Self::compute_file_hash(path)?),
                });
            }
        }

        if changes.is_empty() {
            return Ok(ValidationResult::Valid);
        }

        let total_changes = changes.len();
        let threshold = metadata.file_metadata.len().max(1000) / 10;
        if total_changes > threshold || total_changes > 100 {
            return Ok(ValidationResult::Invalid);
        }

        Ok(ValidationResult::NeedsUpdate(changes))
    }

    /// Invalidate (delete) cache for a project
    pub fn invalidate(root: &Path, extensions: &[&str], cache_dir: Option<&Path>) -> Result<()> {
        let (cache_file, meta_file) = Self::get_cache_paths(root, extensions, cache_dir)?;

        if cache_file.exists() {
            fs::remove_file(&cache_file).context("Failed to remove cache file")?;
        }
        if meta_file.exists() {
            fs::remove_file(&meta_file).context("Failed to remove metadata file")?;
        }

        Ok(())
    }

    /// Ensure .gitignore exists in .codemapper directory
    fn ensure_gitignore(root: &Path, cache_dir: Option<&Path>) -> Result<()> {
        let base_dir = cache_dir
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| root.join(CACHE_DIR_NAME));
        let gitignore_path = base_dir.join(".gitignore");
        if !gitignore_path.exists() {
            if let Some(parent) = gitignore_path.parent() {
                fs::create_dir_all(parent).ok();
            }
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

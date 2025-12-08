use crate::diff::{ChangeType, DiffResult, SymbolDiff};
use crate::git;
use crate::index::CodeIndex;
use crate::models::SymbolType;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const SNAPSHOTS_DIR: &str = ".codemapper/snapshots";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotSymbol {
    pub name: String,
    pub symbol_type: SymbolType,
    pub signature: Option<String>,
    pub file_path: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub name: String,
    pub timestamp: SystemTime,
    pub commit: Option<String>,
    pub symbols: Vec<SnapshotSymbol>,
    pub file_count: usize,
    pub symbol_count: usize,
}

impl Snapshot {
    pub fn new(name: String, commit: Option<String>, symbols: Vec<SnapshotSymbol>, file_count: usize) -> Self {
        let symbol_count = symbols.len();
        Self {
            name,
            timestamp: SystemTime::now(),
            commit,
            symbols,
            file_count,
            symbol_count,
        }
    }
}

pub fn save_snapshot(index: &CodeIndex, name: &str, root_path: &Path) -> Result<Snapshot> {
    let snapshots_dir = root_path.join(SNAPSHOTS_DIR);
    fs::create_dir_all(&snapshots_dir)
        .context("Failed to create snapshots directory")?;
    
    let commit = get_current_commit(root_path);
    
    let symbols: Vec<SnapshotSymbol> = index
        .all_symbols()
        .iter()
        .filter(|s| !s.name.is_empty())
        .map(|s| SnapshotSymbol {
            name: s.name.clone(),
            symbol_type: s.symbol_type,
            signature: s.signature.clone(),
            file_path: s.file_path.clone(),
            line_start: s.line_start,
            line_end: s.line_end,
        })
        .collect();
    
    let snapshot = Snapshot::new(
        name.to_string(),
        commit,
        symbols,
        index.total_files(),
    );
    
    let snapshot_path = snapshots_dir.join(format!("{}.json", name));
    let json = serde_json::to_string_pretty(&snapshot)
        .context("Failed to serialize snapshot")?;
    fs::write(&snapshot_path, json)
        .context("Failed to write snapshot file")?;
    
    ensure_gitignore(root_path)?;
    
    Ok(snapshot)
}

pub fn load_snapshot(name: &str, root_path: &Path) -> Result<Snapshot> {
    let snapshot_path = root_path.join(SNAPSHOTS_DIR).join(format!("{}.json", name));
    
    if !snapshot_path.exists() {
        anyhow::bail!("Snapshot '{}' not found at {}", name, snapshot_path.display());
    }
    
    let json = fs::read_to_string(&snapshot_path)
        .context("Failed to read snapshot file")?;
    let snapshot: Snapshot = serde_json::from_str(&json)
        .context("Failed to parse snapshot file")?;
    
    Ok(snapshot)
}

pub fn list_snapshots(root_path: &Path) -> Result<Vec<String>> {
    let snapshots_dir = root_path.join(SNAPSHOTS_DIR);
    
    if !snapshots_dir.exists() {
        return Ok(Vec::new());
    }
    
    let mut snapshots = Vec::new();
    
    for entry in fs::read_dir(&snapshots_dir).context("Failed to read snapshots directory")? {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();
        
        if path.extension().map(|e| e == "json").unwrap_or(false) {
            if let Some(stem) = path.file_stem() {
                snapshots.push(stem.to_string_lossy().to_string());
            }
        }
    }
    
    snapshots.sort();
    Ok(snapshots)
}

pub fn delete_snapshot(name: &str, root_path: &Path) -> Result<()> {
    let snapshot_path = root_path.join(SNAPSHOTS_DIR).join(format!("{}.json", name));
    
    if !snapshot_path.exists() {
        anyhow::bail!("Snapshot '{}' not found", name);
    }
    
    fs::remove_file(&snapshot_path)
        .context("Failed to delete snapshot file")?;
    
    Ok(())
}

pub fn compare_to_snapshot(index: &CodeIndex, snapshot: &Snapshot) -> DiffResult {
    let mut symbol_diffs = Vec::new();
    
    let old_map: HashMap<(&str, SymbolType, &Path), &SnapshotSymbol> = snapshot
        .symbols
        .iter()
        .map(|s| ((s.name.as_str(), s.symbol_type, s.file_path.as_path()), s))
        .collect();
    
    let current_symbols = index.all_symbols();
    let new_map: HashMap<(&str, SymbolType, &Path), _> = current_symbols
        .iter()
        .filter(|s| !s.name.is_empty())
        .map(|s| ((s.name.as_str(), s.symbol_type, s.file_path.as_path()), *s))
        .collect();
    
    for new_sym in current_symbols.iter().filter(|s| !s.name.is_empty()) {
        let key = (new_sym.name.as_str(), new_sym.symbol_type, new_sym.file_path.as_path());
        
        match old_map.get(&key) {
            None => {
                symbol_diffs.push(SymbolDiff {
                    name: new_sym.name.clone(),
                    symbol_type: new_sym.symbol_type,
                    change_type: ChangeType::Added,
                    file_path: new_sym.file_path.clone(),
                    old_lines: None,
                    new_lines: Some((new_sym.line_start, new_sym.line_end)),
                    old_signature: None,
                    new_signature: new_sym.signature.clone(),
                });
            }
            Some(old_sym) => {
                let sig_changed = old_sym.signature != new_sym.signature;
                let lines_changed = old_sym.line_start != new_sym.line_start
                    || old_sym.line_end != new_sym.line_end;
                let size_changed = (old_sym.line_end.saturating_sub(old_sym.line_start))
                    != (new_sym.line_end.saturating_sub(new_sym.line_start));
                
                if sig_changed {
                    symbol_diffs.push(SymbolDiff {
                        name: new_sym.name.clone(),
                        symbol_type: new_sym.symbol_type,
                        change_type: ChangeType::SignatureChanged,
                        file_path: new_sym.file_path.clone(),
                        old_lines: Some((old_sym.line_start, old_sym.line_end)),
                        new_lines: Some((new_sym.line_start, new_sym.line_end)),
                        old_signature: old_sym.signature.clone(),
                        new_signature: new_sym.signature.clone(),
                    });
                } else if lines_changed || size_changed {
                    symbol_diffs.push(SymbolDiff {
                        name: new_sym.name.clone(),
                        symbol_type: new_sym.symbol_type,
                        change_type: ChangeType::Modified,
                        file_path: new_sym.file_path.clone(),
                        old_lines: Some((old_sym.line_start, old_sym.line_end)),
                        new_lines: Some((new_sym.line_start, new_sym.line_end)),
                        old_signature: old_sym.signature.clone(),
                        new_signature: new_sym.signature.clone(),
                    });
                }
            }
        }
    }
    
    for old_sym in &snapshot.symbols {
        let key = (old_sym.name.as_str(), old_sym.symbol_type, old_sym.file_path.as_path());
        
        if !new_map.contains_key(&key) {
            symbol_diffs.push(SymbolDiff {
                name: old_sym.name.clone(),
                symbol_type: old_sym.symbol_type,
                change_type: ChangeType::Deleted,
                file_path: old_sym.file_path.clone(),
                old_lines: Some((old_sym.line_start, old_sym.line_end)),
                new_lines: None,
                old_signature: old_sym.signature.clone(),
                new_signature: None,
            });
        }
    }
    
    let commit = snapshot.commit.clone().unwrap_or_else(|| snapshot.name.clone());
    
    let files_analyzed = symbol_diffs
        .iter()
        .map(|d| d.file_path.clone())
        .collect::<std::collections::HashSet<_>>()
        .len();
    
    DiffResult {
        commit,
        symbols: symbol_diffs,
        files_analyzed,
    }
}

fn get_current_commit(path: &Path) -> Option<String> {
    if !git::is_git_repo(path) {
        return None;
    }
    
    git::resolve_commit(path, "HEAD")
        .map(|h| h[..8.min(h.len())].to_string())
        .ok()
}

fn ensure_gitignore(root_path: &Path) -> Result<()> {
    let codemapper_dir = root_path.join(".codemapper");
    let gitignore_path = codemapper_dir.join(".gitignore");
    
    if !gitignore_path.exists() {
        fs::write(&gitignore_path, "*\n")
            .context("Failed to create .gitignore")?;
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[test]
    fn test_list_snapshots_empty() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let result = list_snapshots(temp.path()).expect("Failed to list snapshots");
        assert!(result.is_empty());
    }
    
    #[test]
    fn test_snapshot_not_found() {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let result = load_snapshot("nonexistent", temp.path());
        assert!(result.is_err());
    }
}

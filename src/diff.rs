use crate::git;
use crate::indexer;
use crate::models::{Language, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeType {
    Added,
    Deleted,
    Modified,
    SignatureChanged,
}

impl ChangeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChangeType::Added => "ADDED",
            ChangeType::Deleted => "DELETED",
            ChangeType::Modified => "MODIFIED",
            ChangeType::SignatureChanged => "SIGNATURE_CHANGED",
        }
    }
    
    pub fn short(&self) -> &'static str {
        match self {
            ChangeType::Added => "+",
            ChangeType::Deleted => "-",
            ChangeType::Modified => "~",
            ChangeType::SignatureChanged => "!",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SymbolDiff {
    pub name: String,
    pub symbol_type: SymbolType,
    pub change_type: ChangeType,
    pub file_path: PathBuf,
    pub old_lines: Option<(usize, usize)>,
    pub new_lines: Option<(usize, usize)>,
    pub old_signature: Option<String>,
    pub new_signature: Option<String>,
}

#[derive(Debug)]
pub struct DiffResult {
    pub commit: String,
    pub symbols: Vec<SymbolDiff>,
    pub files_analyzed: usize,
}

pub fn compute_diff(
    repo_path: &Path,
    commit: &str,
    subpath: Option<&Path>,
    extensions: &[&str],
) -> Result<DiffResult> {
    if !git::is_git_repo(repo_path) {
        anyhow::bail!("Not a git repository: {}", repo_path.display());
    }
    
    let resolved_commit = git::resolve_commit(repo_path, commit)?;
    let repo_root = git::get_repo_root(repo_path)?;
    
    let changed_files = git::get_changed_files(repo_path, &resolved_commit, subpath)?;
    
    let all_changed: Vec<PathBuf> = changed_files.added.iter()
        .chain(changed_files.deleted.iter())
        .chain(changed_files.modified.iter())
        .cloned()
        .collect();
    
    let filtered_files: Vec<PathBuf> = all_changed.into_iter()
        .filter(|f| {
            if extensions.is_empty() {
                true
            } else {
                f.extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| extensions.contains(&ext))
                    .unwrap_or(false)
            }
        })
        .collect();
    
    let mut symbol_diffs = Vec::new();
    let files_analyzed = filtered_files.len();
    
    for file_path in &filtered_files {
        let language = indexer::detect_language(file_path);
        if language == Language::Unknown {
            continue;
        }
        
        let old_symbols = get_symbols_at_commit(&repo_root, file_path, &resolved_commit, language)?;
        let new_symbols = get_current_symbols(file_path, language)?;
        
        let diffs = compare_symbols(&old_symbols, &new_symbols, file_path);
        symbol_diffs.extend(diffs);
    }
    
    Ok(DiffResult {
        commit: resolved_commit,
        symbols: symbol_diffs,
        files_analyzed,
    })
}

fn get_symbols_at_commit(
    repo_root: &Path,
    file_path: &Path,
    commit: &str,
    language: Language,
) -> Result<Vec<Symbol>> {
    let content = match git::get_file_at_commit(repo_root, file_path, commit)? {
        Some(c) => c,
        None => return Ok(Vec::new()),
    };
    
    let file_info = indexer::index_file(file_path, &content, language, None)
        .context("Failed to parse file at commit")?;
    
    Ok(file_info.symbols)
}

fn get_current_symbols(file_path: &Path, language: Language) -> Result<Vec<Symbol>> {
    if !file_path.exists() {
        return Ok(Vec::new());
    }
    
    let content = std::fs::read_to_string(file_path)
        .context("Failed to read current file")?;
    
    let file_info = indexer::index_file(file_path, &content, language, None)
        .context("Failed to parse current file")?;
    
    Ok(file_info.symbols)
}

fn compare_symbols(old_symbols: &[Symbol], new_symbols: &[Symbol], file_path: &Path) -> Vec<SymbolDiff> {
    let mut diffs = Vec::new();
    
    let old_map: HashMap<(&str, SymbolType), &Symbol> = old_symbols.iter()
        .map(|s| ((s.name.as_str(), s.symbol_type), s))
        .collect();
    
    let new_map: HashMap<(&str, SymbolType), &Symbol> = new_symbols.iter()
        .map(|s| ((s.name.as_str(), s.symbol_type), s))
        .collect();
    
    for new_sym in new_symbols {
        let key = (new_sym.name.as_str(), new_sym.symbol_type);
        
        match old_map.get(&key) {
            None => {
                diffs.push(SymbolDiff {
                    name: new_sym.name.clone(),
                    symbol_type: new_sym.symbol_type,
                    change_type: ChangeType::Added,
                    file_path: file_path.to_path_buf(),
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
                let size_changed = (old_sym.line_end - old_sym.line_start) 
                    != (new_sym.line_end - new_sym.line_start);
                
                if sig_changed {
                    diffs.push(SymbolDiff {
                        name: new_sym.name.clone(),
                        symbol_type: new_sym.symbol_type,
                        change_type: ChangeType::SignatureChanged,
                        file_path: file_path.to_path_buf(),
                        old_lines: Some((old_sym.line_start, old_sym.line_end)),
                        new_lines: Some((new_sym.line_start, new_sym.line_end)),
                        old_signature: old_sym.signature.clone(),
                        new_signature: new_sym.signature.clone(),
                    });
                } else if lines_changed || size_changed {
                    diffs.push(SymbolDiff {
                        name: new_sym.name.clone(),
                        symbol_type: new_sym.symbol_type,
                        change_type: ChangeType::Modified,
                        file_path: file_path.to_path_buf(),
                        old_lines: Some((old_sym.line_start, old_sym.line_end)),
                        new_lines: Some((new_sym.line_start, new_sym.line_end)),
                        old_signature: old_sym.signature.clone(),
                        new_signature: new_sym.signature.clone(),
                    });
                }
            }
        }
    }
    
    for old_sym in old_symbols {
        let key = (old_sym.name.as_str(), old_sym.symbol_type);
        
        if !new_map.contains_key(&key) {
            diffs.push(SymbolDiff {
                name: old_sym.name.clone(),
                symbol_type: old_sym.symbol_type,
                change_type: ChangeType::Deleted,
                file_path: file_path.to_path_buf(),
                old_lines: Some((old_sym.line_start, old_sym.line_end)),
                new_lines: None,
                old_signature: old_sym.signature.clone(),
                new_signature: None,
            });
        }
    }
    
    diffs
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_change_type_str() {
        assert_eq!(ChangeType::Added.as_str(), "ADDED");
        assert_eq!(ChangeType::Deleted.as_str(), "DELETED");
        assert_eq!(ChangeType::Modified.as_str(), "MODIFIED");
        assert_eq!(ChangeType::SignatureChanged.as_str(), "SIGNATURE_CHANGED");
    }
}

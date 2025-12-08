use crate::git::{self, CommitInfo};
use crate::indexer;
use crate::models::{Language, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct BlameResult {
    pub symbol_name: String,
    pub symbol_type: SymbolType,
    pub last_commit: CommitInfo,
    pub old_signature: Option<String>,
    pub new_signature: Option<String>,
    pub current_lines: (usize, usize),
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub commit: CommitInfo,
    pub signature: Option<String>,
    pub lines: Option<(usize, usize)>,
    pub existed: bool,
}

pub fn blame_symbol(
    repo_path: &Path,
    file_path: &Path,
    symbol_name: &str,
) -> Result<BlameResult> {
    if !git::is_git_repo(repo_path) {
        anyhow::bail!("Not a git repository: {}", repo_path.display());
    }

    let canonical_file = std::fs::canonicalize(file_path)
        .context("Failed to resolve file path")?;
    
    if !canonical_file.exists() {
        anyhow::bail!("File does not exist: {}", file_path.display());
    }

    let language = indexer::detect_language(&canonical_file);
    if language == Language::Unknown {
        anyhow::bail!("Unknown or unsupported file type: {}", file_path.display());
    }

    let current_content = std::fs::read_to_string(&canonical_file)
        .context("Failed to read current file")?;
    let current_file_info = indexer::index_file(&canonical_file, &current_content, language, None)?;
    
    let current_symbol = current_file_info.symbols.iter()
        .find(|s| s.name == symbol_name)
        .ok_or_else(|| anyhow::anyhow!("Symbol '{}' not found in current file", symbol_name))?;

    let commits = git::get_commits_for_file(repo_path, &canonical_file, Some(100))?;
    
    if commits.is_empty() {
        anyhow::bail!("No git history found for file: {}", file_path.display());
    }

    let repo_root = git::get_repo_root(repo_path)?;
    
    let mut last_modifying_commit: Option<&CommitInfo> = None;
    let mut previous_signature: Option<String> = None;
    
    for i in 0..commits.len() {
        let commit = &commits[i];
        
        let symbol_at_commit = get_symbol_at_commit(
            &repo_root, 
            &canonical_file, 
            &commit.hash, 
            symbol_name, 
            language
        )?;
        
        if i == 0 {
            last_modifying_commit = Some(commit);
            continue;
        }
        
        let prev_commit = &commits[i - 1];
        let symbol_at_prev = get_symbol_at_commit(
            &repo_root,
            &canonical_file,
            &prev_commit.hash,
            symbol_name,
            language
        )?;
        
        let current_sig = symbol_at_prev.as_ref().and_then(|s| s.signature.clone());
        let prev_sig = symbol_at_commit.as_ref().and_then(|s| s.signature.clone());
        
        match (&symbol_at_prev, &symbol_at_commit) {
            (Some(_), None) => {
                last_modifying_commit = Some(&commits[i - 1]);
                previous_signature = None;
                break;
            }
            (Some(curr), Some(prev)) => {
                let curr_lines = curr.line_end - curr.line_start;
                let prev_lines = prev.line_end - prev.line_start;
                
                if current_sig != prev_sig || curr_lines != prev_lines {
                    last_modifying_commit = Some(&commits[i - 1]);
                    previous_signature = prev_sig;
                    break;
                }
            }
            _ => {}
        }
    }

    let last_commit = last_modifying_commit
        .cloned()
        .unwrap_or_else(|| commits.first().cloned().unwrap_or_else(|| CommitInfo {
            hash: "unknown".to_string(),
            short_hash: "unknown".to_string(),
            author: "unknown".to_string(),
            date: "unknown".to_string(),
            message: "unknown".to_string(),
        }));

    Ok(BlameResult {
        symbol_name: current_symbol.name.clone(),
        symbol_type: current_symbol.symbol_type,
        last_commit,
        old_signature: previous_signature,
        new_signature: current_symbol.signature.clone(),
        current_lines: (current_symbol.line_start, current_symbol.line_end),
    })
}

pub fn history_symbol(
    repo_path: &Path,
    file_path: &Path,
    symbol_name: &str,
) -> Result<Vec<HistoryEntry>> {
    if !git::is_git_repo(repo_path) {
        anyhow::bail!("Not a git repository: {}", repo_path.display());
    }

    let canonical_file = std::fs::canonicalize(file_path)
        .context("Failed to resolve file path")?;
    
    if !canonical_file.exists() {
        anyhow::bail!("File does not exist: {}", file_path.display());
    }

    let language = indexer::detect_language(&canonical_file);
    if language == Language::Unknown {
        anyhow::bail!("Unknown or unsupported file type: {}", file_path.display());
    }

    let commits = git::get_commits_for_file(repo_path, &canonical_file, None)?;
    
    if commits.is_empty() {
        anyhow::bail!("No git history found for file: {}", file_path.display());
    }

    let repo_root = git::get_repo_root(repo_path)?;
    let mut history: Vec<HistoryEntry> = Vec::new();
    let mut prev_signature: Option<String> = None;
    let mut prev_lines: Option<(usize, usize)> = None;

    for commit in commits.iter().rev() {
        let symbol_at_commit = get_symbol_at_commit(
            &repo_root,
            &canonical_file,
            &commit.hash,
            symbol_name,
            language
        )?;

        match symbol_at_commit {
            Some(sym) => {
                let current_sig = sym.signature.clone();
                let current_lines = Some((sym.line_start, sym.line_end));
                
                let sig_changed = prev_signature.as_ref() != current_sig.as_ref();
                let lines_changed = match (prev_lines, current_lines) {
                    (Some((ps, pe)), Some((cs, ce))) => {
                        (pe - ps) != (ce - cs)
                    }
                    (None, Some(_)) => true,
                    _ => false,
                };

                if history.is_empty() || sig_changed || lines_changed {
                    history.push(HistoryEntry {
                        commit: commit.clone(),
                        signature: current_sig.clone(),
                        lines: current_lines,
                        existed: true,
                    });
                }

                prev_signature = current_sig;
                prev_lines = current_lines;
            }
            None => {
                if prev_signature.is_some() {
                    history.push(HistoryEntry {
                        commit: commit.clone(),
                        signature: None,
                        lines: None,
                        existed: false,
                    });
                    prev_signature = None;
                    prev_lines = None;
                }
            }
        }
    }

    history.reverse();

    Ok(history)
}

fn get_symbol_at_commit(
    repo_root: &Path,
    file_path: &Path,
    commit: &str,
    symbol_name: &str,
    language: Language,
) -> Result<Option<Symbol>> {
    let content = match git::get_file_at_commit(repo_root, file_path, commit)? {
        Some(c) => c,
        None => return Ok(None),
    };

    let file_info = indexer::index_file(file_path, &content, language, None)
        .context("Failed to parse file at commit")?;

    let symbol = file_info.symbols.into_iter()
        .find(|s| s.name == symbol_name);

    Ok(symbol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blame_requires_git_repo() {
        let result = blame_symbol(
            Path::new("/nonexistent"),
            Path::new("/nonexistent/file.rs"),
            "test_symbol"
        );
        assert!(result.is_err());
    }
}

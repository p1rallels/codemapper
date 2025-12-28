use crate::models::{FileInfo, Symbol, SymbolType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
pub struct CodeIndex {
    files: HashMap<PathBuf, FileInfo>,
    symbols: Vec<Symbol>,
    symbol_index: HashMap<String, Vec<usize>>,
    file_symbols: HashMap<PathBuf, Vec<usize>>,
    dependencies: HashMap<PathBuf, Vec<String>>,
}

impl CodeIndex {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            symbols: Vec::new(),
            symbol_index: HashMap::new(),
            file_symbols: HashMap::new(),
            dependencies: HashMap::new(),
        }
    }

    pub fn add_file(&mut self, mut file_info: FileInfo) {
        let file_path = file_info.path.clone();
        let _symbol_start_idx = self.symbols.len();

        let mut symbol_indices = Vec::new();
        for symbol in file_info.symbols.drain(..) {
            let idx = self.symbols.len();
            symbol_indices.push(idx);

            self.symbol_index
                .entry(symbol.name.clone())
                .or_insert_with(Vec::new)
                .push(idx);

            self.symbols.push(symbol);
        }

        self.file_symbols.insert(file_path.clone(), symbol_indices);

        let deps: Vec<String> = file_info
            .dependencies
            .iter()
            .map(|d| d.import_name.clone())
            .collect();
        self.dependencies.insert(file_path.clone(), deps);

        self.files.insert(file_path, file_info);
    }

    /// Remove a file from the index (for incremental updates)
    pub fn remove_file(&mut self, path: &Path) {
        // Get symbol indices for this file
        let symbol_indices = match self.file_symbols.get(path) {
            Some(indices) => indices.clone(),
            None => return, // File not in index
        };

        // Remove from symbol_index: for each symbol from this file,
        // remove its index from the symbol_index map
        for &idx in &symbol_indices {
            if let Some(symbol) = self.symbols.get(idx) {
                let name = symbol.name.clone();
                if let Some(indices) = self.symbol_index.get_mut(&name) {
                    indices.retain(|&i| i != idx);
                    // Remove entry if empty
                    if indices.is_empty() {
                        self.symbol_index.remove(&name);
                    }
                }
            }
        }

        // Mark symbols as deleted (set empty placeholder)
        // We can't remove from Vec without invalidating indices
        // This is cleaned up when cache is saved/compacted
        for &idx in &symbol_indices {
            if idx < self.symbols.len() {
                // Clear the symbol but keep the slot
                // (compaction happens when saving cache)
                self.symbols[idx].name = String::new();
            }
        }

        // Remove from other maps
        self.file_symbols.remove(path);
        self.dependencies.remove(path);
        self.files.remove(path);
    }

    /// Compact the index by removing deleted symbols and rebuilding indices
    /// Call this after incremental updates to reclaim memory
    pub fn compact(&mut self) {
        // Collect all non-deleted symbols
        let mut new_symbols = Vec::new();
        let mut old_to_new_idx: HashMap<usize, usize> = HashMap::new();

        for (old_idx, symbol) in self.symbols.iter().enumerate() {
            if !symbol.name.is_empty() {
                let new_idx = new_symbols.len();
                old_to_new_idx.insert(old_idx, new_idx);
                new_symbols.push(symbol.clone());
            }
        }

        // Rebuild symbol_index with new indices
        let mut new_symbol_index: HashMap<String, Vec<usize>> = HashMap::new();
        for (old_idx, new_idx) in &old_to_new_idx {
            if let Some(symbol) = self.symbols.get(*old_idx) {
                new_symbol_index
                    .entry(symbol.name.clone())
                    .or_insert_with(Vec::new)
                    .push(*new_idx);
            }
        }

        // Rebuild file_symbols with new indices
        let mut new_file_symbols: HashMap<PathBuf, Vec<usize>> = HashMap::new();
        for (path, old_indices) in &self.file_symbols {
            let new_indices: Vec<usize> = old_indices
                .iter()
                .filter_map(|old_idx| old_to_new_idx.get(old_idx))
                .copied()
                .collect();
            if !new_indices.is_empty() {
                new_file_symbols.insert(path.clone(), new_indices);
            }
        }

        // Replace with compacted versions
        self.symbols = new_symbols;
        self.symbol_index = new_symbol_index;
        self.file_symbols = new_file_symbols;
    }

    pub fn query_symbol(&self, name: &str) -> Vec<&Symbol> {
        self.symbol_index
            .get(name)
            .map(|indices| indices.iter().map(|&idx| &self.symbols[idx]).collect())
            .unwrap_or_default()
    }

    pub fn fuzzy_search(&self, pattern: &str) -> Vec<&Symbol> {
        let pattern_lower = pattern.to_lowercase();
        let mut results: Vec<(&Symbol, i32)> = self
            .symbols
            .iter()
            .filter_map(|symbol| {
                let name_lower = symbol.name.to_lowercase();
                if name_lower.contains(&pattern_lower) {
                    let score = if name_lower == pattern_lower {
                        100
                    } else if name_lower.starts_with(&pattern_lower) {
                        50
                    } else {
                        levenshtein_distance(&name_lower, &pattern_lower)
                    };
                    Some((symbol, score))
                } else {
                    None
                }
            })
            .collect();

        results.sort_by(|a, b| b.1.cmp(&a.1));
        results.into_iter().map(|(s, _)| s).collect()
    }

    pub fn get_file_symbols(&self, path: &Path) -> Vec<&Symbol> {
        self.file_symbols
            .get(path)
            .map(|indices| indices.iter().map(|&idx| &self.symbols[idx]).collect())
            .unwrap_or_default()
    }

    pub fn get_dependencies(&self, path: &Path) -> Option<&Vec<String>> {
        self.dependencies.get(path)
    }

    pub fn files(&self) -> impl Iterator<Item = &FileInfo> {
        self.files.values()
    }

    pub fn total_files(&self) -> usize {
        self.files.len()
    }

    pub fn total_symbols(&self) -> usize {
        self.symbols.len()
    }

    pub fn symbols_by_type(&self, symbol_type: SymbolType) -> usize {
        self.symbols
            .iter()
            .filter(|s| s.symbol_type == symbol_type)
            .count()
    }

    /// Get all symbols (for use with type filtering)
    pub fn all_symbols(&self) -> Vec<&Symbol> {
        self.symbols.iter().collect()
    }
}

fn levenshtein_distance(s1: &str, s2: &str) -> i32 {
    let len1 = s1.chars().count();
    let len2 = s2.chars().count();

    if len1 == 0 {
        return -(len2 as i32);
    }
    if len2 == 0 {
        return -(len1 as i32);
    }

    let mut matrix = vec![vec![0; len2 + 1]; len1 + 1];

    for i in 0..=len1 {
        matrix[i][0] = i;
    }
    for j in 0..=len2 {
        matrix[0][j] = j;
    }

    let s1_chars: Vec<char> = s1.chars().collect();
    let s2_chars: Vec<char> = s2.chars().collect();

    for i in 1..=len1 {
        for j in 1..=len2 {
            let cost = if s1_chars[i - 1] == s2_chars[j - 1] {
                0
            } else {
                1
            };
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + cost);
        }
    }

    -(matrix[len1][len2] as i32)
}

impl Default for CodeIndex {
    fn default() -> Self {
        Self::new()
    }
}

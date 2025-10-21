use crate::models::{FileInfo, Symbol, SymbolType};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

    pub fn get_file(&self, path: &Path) -> Option<&FileInfo> {
        self.files.get(path)
    }

    pub fn files(&self) -> impl Iterator<Item = &FileInfo> {
        self.files.values()
    }

    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
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
            let cost = if s1_chars[i - 1] == s2_chars[j - 1] { 0 } else { 1 };
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

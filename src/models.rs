use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Python,
    JavaScript,
    TypeScript,
    Rust,
    Java,
    Go,
    C,
    Markdown,
    Unknown,
}

impl Language {
    pub fn from_extension(ext: &str) -> Self {
        match ext {
            "py" => Language::Python,
            "js" | "jsx" => Language::JavaScript,
            "ts" | "tsx" => Language::TypeScript,
            "rs" => Language::Rust,
            "java" => Language::Java,
            "go" => Language::Go,
            "c" | "h" => Language::C,
            "md" => Language::Markdown,
            _ => Language::Unknown,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::JavaScript => "javascript",
            Language::TypeScript => "typescript",
            Language::Rust => "rust",
            Language::Java => "java",
            Language::Go => "go",
            Language::C => "c",
            Language::Markdown => "markdown",
            Language::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolType {
    Function,
    Class,
    Method,
    Enum,
    StaticField,
    Heading,
    CodeBlock,
}

impl SymbolType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolType::Function => "function",
            SymbolType::Class => "class",
            SymbolType::Method => "method",
            SymbolType::Enum => "enum",
            SymbolType::StaticField => "static",
            SymbolType::Heading => "heading",
            SymbolType::CodeBlock => "code_block",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub symbol_type: SymbolType,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub line_start: usize,
    pub line_end: usize,
    pub parent_id: Option<usize>,
    pub file_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub import_name: String,
    pub from_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: PathBuf,
    pub language: Language,
    pub size: u64,
    pub hash: String,
    pub symbols: Vec<Symbol>,
    pub dependencies: Vec<Dependency>,
}

impl FileInfo {
    pub fn new(path: PathBuf, language: Language, size: u64, hash: String) -> Self {
        Self {
            path,
            language,
            size,
            hash,
            symbols: Vec::new(),
            dependencies: Vec::new(),
        }
    }
}

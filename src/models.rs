use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    Python,
    JavaScript,
    TypeScript,
    Rust,
    Java,
    Go,
    C,
    Swift,
    Markdown,
    Elixir,
    Unknown,
    Ruby,
}

impl Language {
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "py" => Language::Python,
            "js" | "jsx" => Language::JavaScript,
            "ts" | "tsx" => Language::TypeScript,
            "rs" => Language::Rust,
            "java" => Language::Java,
            "go" => Language::Go,
            "c" | "h" => Language::C,
            "swift" => Language::Swift,
            "rb" | "rbi" | "rake" | "gemspec" | "ru" => Language::Ruby,
            "ex" | "exs" => Language::Elixir,
            "md" => Language::Markdown,
            _ => Language::Unknown,
        }
    }

    pub fn from_path(path: &Path) -> Self {
        let by_extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(Language::from_extension)
            .unwrap_or(Language::Unknown);

        if by_extension != Language::Unknown {
            return by_extension;
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_ascii_lowercase());

        match file_name.as_deref() {
            Some("gemfile" | "rakefile" | "capfile" | "guardfile" | "thorfile") => Language::Ruby,
            Some("podfile" | "fastfile" | "appraisals" | "dangerfile" | ".irbrc" | ".pryrc") => {
                Language::Ruby
            }
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
            Language::Swift => "swift",
            Language::Ruby => "ruby",
            Language::Elixir => "elixir",
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
    Endpoint,
    Interface,
    TypeAlias,
    Module,
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
            SymbolType::Endpoint => "endpoint",
            SymbolType::Interface => "interface",
            SymbolType::TypeAlias => "type",
            SymbolType::Module => "module",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "function" | "func" | "fn" => Some(SymbolType::Function),
            "class" => Some(SymbolType::Class),
            "method" => Some(SymbolType::Method),
            "enum" => Some(SymbolType::Enum),
            "static" | "staticfield" => Some(SymbolType::StaticField),
            "heading" | "header" => Some(SymbolType::Heading),
            "code_block" | "codeblock" => Some(SymbolType::CodeBlock),
            "endpoint" | "endpoints" => Some(SymbolType::Endpoint),
            "interface" => Some(SymbolType::Interface),
            "type" | "typealias" | "type_alias" => Some(SymbolType::TypeAlias),
            "module" => Some(SymbolType::Module),
            _ => None,
        }
    }

    pub fn from_plural(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "functions" | "funcs" | "fns" => Some(SymbolType::Function),
            "classes" => Some(SymbolType::Class),
            "methods" => Some(SymbolType::Method),
            "enums" => Some(SymbolType::Enum),
            "statics" | "staticfields" => Some(SymbolType::StaticField),
            "headings" | "headers" => Some(SymbolType::Heading),
            "code_blocks" | "codeblocks" => Some(SymbolType::CodeBlock),
            "endpoints" => Some(SymbolType::Endpoint),
            "interfaces" => Some(SymbolType::Interface),
            "types" | "typealiases" | "type_aliases" => Some(SymbolType::TypeAlias),
            "modules" => Some(SymbolType::Module),
            _ => None,
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
    pub is_exported: bool,
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

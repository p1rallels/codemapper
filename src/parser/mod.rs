pub mod c;
pub mod go;
pub mod java;
pub mod javascript;
pub mod markdown;
pub mod python;
pub mod rust;

use crate::models::{Dependency, Symbol};
use anyhow::Result;
use std::path::Path;

pub trait Parser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult>;
}

pub trait LanguageParser {
    fn parse_file(&self, path: &Path, source: &str) -> Result<ParsedFile>;
}

#[derive(Debug, Default)]
pub struct ParseResult {
    pub symbols: Vec<Symbol>,
    pub dependencies: Vec<Dependency>,
}

impl ParseResult {
    pub fn new() -> Self {
        Self {
            symbols: Vec::new(),
            dependencies: Vec::new(),
        }
    }
}

pub struct ParsedFile {
    pub symbols: Vec<Symbol>,
    pub dependencies: Vec<Dependency>,
}

impl ParsedFile {
    pub fn new() -> Self {
        Self {
            symbols: Vec::new(),
            dependencies: Vec::new(),
        }
    }
}

impl Default for ParsedFile {
    fn default() -> Self {
        Self::new()
    }
}

// pub use javascript::JavaScriptParser;
// pub use python::PythonParser;

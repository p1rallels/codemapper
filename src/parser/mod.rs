pub mod c;
pub mod elixir;
pub mod go;
pub mod java;
pub mod javascript;
pub mod markdown;
pub mod python;
pub mod ruby;
pub mod rust;
pub mod swift;
pub mod typescript;

use crate::models::{Dependency, Symbol};
use anyhow::Result;
use std::path::Path;

pub trait Parser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult>;
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

// pub use javascript::JavaScriptParser;
// pub use python::PythonParser;

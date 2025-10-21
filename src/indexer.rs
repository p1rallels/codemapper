use crate::index::CodeIndex;
use crate::models::{FileInfo, Language};
use crate::parser::{c::CParser, go::GoParser, java::JavaParser, javascript::JavaScriptParser, markdown::MarkdownParser, python::PythonParser, rust::RustParser, Parser};
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
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

pub fn detect_language(path: &Path) -> Language {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(Language::from_extension)
        .unwrap_or(Language::Unknown)
}

fn read_file_content(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))
}

pub fn index_file(path: &Path, content: &str, language: Language) -> Result<FileInfo> {
    let size = content.len() as u64;
    let hash = format!("{:x}", md5::compute(content.as_bytes()));
    let mut file_info = FileInfo::new(path.to_path_buf(), language, size, hash);

    match language {
        Language::Python => {
            if let Ok(parser) = PythonParser::new() {
                if let Ok(parsed) = parser.parse(content, path) {
                    file_info.symbols = parsed.symbols;
                    file_info.dependencies = parsed.dependencies;
                }
            }
        }
        Language::JavaScript | Language::TypeScript => {
            if let Ok(parser) = JavaScriptParser::new() {
                if let Ok(parsed) = parser.parse(content, path) {
                    file_info.symbols = parsed.symbols;
                    file_info.dependencies = parsed.dependencies;
                }
            }
        }
        Language::Rust => {
            if let Ok(parser) = RustParser::new() {
                if let Ok(parsed) = parser.parse(content, path) {
                    file_info.symbols = parsed.symbols;
                    file_info.dependencies = parsed.dependencies;
                }
            }
        }
        Language::Java => {
            if let Ok(parser) = JavaParser::new() {
                if let Ok(parsed) = parser.parse(content, path) {
                    file_info.symbols = parsed.symbols;
                    file_info.dependencies = parsed.dependencies;
                }
            }
        }
        Language::Go => {
            if let Ok(parser) = GoParser::new() {
                if let Ok(parsed) = parser.parse(content, path) {
                    file_info.symbols = parsed.symbols;
                    file_info.dependencies = parsed.dependencies;
                }
            }
        }
        Language::C => {
            if let Ok(parser) = CParser::new() {
                if let Ok(parsed) = parser.parse(content, path) {
                    file_info.symbols = parsed.symbols;
                    file_info.dependencies = parsed.dependencies;
                }
            }
        }
        Language::Markdown => {
            if let Ok(parser) = MarkdownParser::new() {
                if let Ok(parsed) = parser.parse(content, path) {
                    file_info.symbols = parsed.symbols;
                    file_info.dependencies = parsed.dependencies;
                }
            }
        }
        Language::Unknown => {}
    }

    Ok(file_info)
}

pub fn index_directory(path: &Path, extensions: &[&str]) -> Result<CodeIndex> {
    if !path.exists() {
        anyhow::bail!("Directory does not exist: {}", path.display());
    }

    if !path.is_dir() {
        anyhow::bail!("Path is not a directory: {}", path.display());
    }

    let entries: Vec<PathBuf> = WalkDir::new(path)
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
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            if extensions.is_empty() {
                true
            } else {
                e.path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| extensions.contains(&ext))
                    .unwrap_or(false)
            }
        })
        .map(|e| e.into_path())
        .collect();

    let file_infos: Vec<FileInfo> = entries
        .par_iter()
        .filter_map(|file_path| {
            let language = detect_language(file_path);
            
            if language == Language::Unknown {
                return None;
            }

            let content = match read_file_content(file_path) {
                Ok(c) => c,
                Err(_) => return None,
            };

            match index_file(file_path, &content, language) {
                Ok(info) => Some(info),
                Err(_) => None,
            }
        })
        .collect();

    let mut index = CodeIndex::new();
    for file_info in file_infos {
        index.add_file(file_info);
    }

    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language(Path::new("test.py")), Language::Python);
        assert_eq!(detect_language(Path::new("test.js")), Language::JavaScript);
        assert_eq!(detect_language(Path::new("test.ts")), Language::TypeScript);
        assert_eq!(detect_language(Path::new("test.rs")), Language::Rust);
        assert_eq!(detect_language(Path::new("test.txt")), Language::Unknown);
    }

    #[test]
    fn test_ignored_dirs() {
        assert!(IGNORED_DIRS.contains(&".git"));
        assert!(IGNORED_DIRS.contains(&"node_modules"));
        assert!(IGNORED_DIRS.contains(&"__pycache__"));
    }
}

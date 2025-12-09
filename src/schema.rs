use crate::index::CodeIndex;
use crate::models::{Language, Symbol, SymbolType};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldInfo {
    pub name: String,
    pub type_name: String,
    pub default_value: Option<String>,
    pub is_optional: bool,
    pub docstring: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaInfo {
    pub symbol_name: String,
    pub symbol_type: SymbolType,
    pub file_path: PathBuf,
    pub line: usize,
    pub fields: Vec<FieldInfo>,
    pub language: Language,
}

pub fn analyze_schema(
    index: &CodeIndex,
    symbol_name: &str,
    fuzzy: bool,
) -> Result<Vec<SchemaInfo>> {
    let symbols: Vec<&Symbol> = if fuzzy {
        index.fuzzy_search(symbol_name)
    } else {
        index.query_symbol(symbol_name)
    };

    let class_symbols: Vec<&Symbol> = symbols
        .into_iter()
        .filter(|s| matches!(s.symbol_type, SymbolType::Class | SymbolType::Enum))
        .collect();

    let mut schemas = Vec::new();

    for symbol in class_symbols {
        let content = fs::read_to_string(&symbol.file_path)
            .with_context(|| format!("Failed to read file: {}", symbol.file_path.display()))?;

        let language = detect_language(&symbol.file_path);
        let fields = extract_fields(&content, symbol, language)?;

        schemas.push(SchemaInfo {
            symbol_name: symbol.name.clone(),
            symbol_type: symbol.symbol_type,
            file_path: symbol.file_path.clone(),
            line: symbol.line_start,
            fields,
            language,
        });
    }

    Ok(schemas)
}

fn detect_language(path: &PathBuf) -> Language {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(Language::from_extension)
        .unwrap_or(Language::Unknown)
}

fn extract_fields(content: &str, symbol: &Symbol, language: Language) -> Result<Vec<FieldInfo>> {
    match language {
        Language::Rust => extract_rust_fields(content, symbol),
        Language::Python => extract_python_fields(content, symbol),
        Language::TypeScript | Language::JavaScript => extract_typescript_fields(content, symbol),
        Language::Java => extract_java_fields(content, symbol),
        Language::Go => extract_go_fields(content, symbol),
        _ => Ok(Vec::new()),
    }
}

fn extract_rust_fields(content: &str, symbol: &Symbol) -> Result<Vec<FieldInfo>> {
    let mut parser = Parser::new();
    let language = tree_sitter_rust::LANGUAGE.into();
    parser
        .set_language(&language)
        .context("Failed to set Rust language")?;

    let tree = parser
        .parse(content, None)
        .context("Failed to parse Rust file")?;

    let root = tree.root_node();
    let mut fields = Vec::new();

    let query = Query::new(
        &language,
        r#"
        (struct_item
            name: (type_identifier) @struct.name
            body: (field_declaration_list
                (field_declaration
                    name: (field_identifier) @field.name
                    type: (_) @field.type))) @struct.def
        "#,
    )
    .context("Failed to create Rust struct field query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, root, content.as_bytes());

    while let Some(match_) = matches.next() {
        let captures = match_.captures;

        let struct_name_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("struct.name")
        });

        let struct_name = struct_name_cap
            .and_then(|c| c.node.utf8_text(content.as_bytes()).ok())
            .unwrap_or_default();

        if struct_name != symbol.name && !symbol.name.starts_with("impl ") {
            continue;
        }

        let field_name_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.name")
        });

        let field_type_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.type")
        });

        if let (Some(name_cap), Some(type_cap)) = (field_name_cap, field_type_cap) {
            let name = name_cap
                .node
                .utf8_text(content.as_bytes())
                .unwrap_or_default()
                .to_string();
            let type_name = type_cap
                .node
                .utf8_text(content.as_bytes())
                .unwrap_or_default()
                .to_string();

            let is_optional = type_name.starts_with("Option<");

            fields.push(FieldInfo {
                name,
                type_name,
                default_value: None,
                is_optional,
                docstring: None,
            });
        }
    }

    Ok(fields)
}

fn extract_python_fields(content: &str, symbol: &Symbol) -> Result<Vec<FieldInfo>> {
    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE.into();
    parser
        .set_language(&language)
        .context("Failed to set Python language")?;

    let tree = parser
        .parse(content, None)
        .context("Failed to parse Python file")?;

    let root = tree.root_node();
    let mut fields = Vec::new();

    let query = Query::new(
        &language,
        r#"
        (class_definition
            name: (identifier) @class.name
            body: (block
                (expression_statement
                    (assignment
                        left: (identifier) @field.name
                        type: (type) @field.type
                        right: (_)? @field.default)))) @class.def
        "#,
    )
    .context("Failed to create Python class field query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, root, content.as_bytes());

    while let Some(match_) = matches.next() {
        let captures = match_.captures;

        let class_name_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("class.name")
        });

        let class_name = class_name_cap
            .and_then(|c| c.node.utf8_text(content.as_bytes()).ok())
            .unwrap_or_default();

        if class_name != symbol.name {
            continue;
        }

        let field_name_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.name")
        });

        let field_type_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.type")
        });

        let field_default_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.default")
        });

        if let Some(name_cap) = field_name_cap {
            let name = name_cap
                .node
                .utf8_text(content.as_bytes())
                .unwrap_or_default()
                .to_string();

            let type_name = field_type_cap
                .and_then(|c| c.node.utf8_text(content.as_bytes()).ok())
                .unwrap_or_default()
                .to_string();

            let default_value = field_default_cap
                .and_then(|c| c.node.utf8_text(content.as_bytes()).ok())
                .map(|s| s.to_string());

            let is_optional = type_name.contains("Optional") || default_value.is_some();

            fields.push(FieldInfo {
                name,
                type_name,
                default_value,
                is_optional,
                docstring: None,
            });
        }
    }

    if fields.is_empty() {
        fields = extract_python_fields_fallback(content, symbol)?;
    }

    Ok(fields)
}

fn extract_python_fields_fallback(content: &str, symbol: &Symbol) -> Result<Vec<FieldInfo>> {
    let mut fields = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    if symbol.line_start == 0 || symbol.line_end > lines.len() {
        return Ok(fields);
    }

    let start_idx = symbol.line_start.saturating_sub(1);
    let end_idx = symbol.line_end.min(lines.len());

    for line in &lines[start_idx..end_idx] {
        let trimmed = line.trim();

        if trimmed.starts_with("def ") || trimmed.starts_with("class ") || trimmed.starts_with('#') {
            continue;
        }

        if let Some(colon_pos) = trimmed.find(':') {
            let potential_name = trimmed[..colon_pos].trim();

            if potential_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                && !potential_name.is_empty()
                && !potential_name.starts_with("return")
                && !potential_name.starts_with("self")
            {
                let rest = trimmed[colon_pos + 1..].trim();
                let (type_name, default_value) = if let Some(eq_pos) = rest.find('=') {
                    let type_part = rest[..eq_pos].trim().to_string();
                    let default_part = rest[eq_pos + 1..].trim().to_string();
                    (type_part, Some(default_part))
                } else {
                    (rest.to_string(), None)
                };

                if !type_name.is_empty() {
                    let is_optional = type_name.contains("Optional") || default_value.is_some();
                    fields.push(FieldInfo {
                        name: potential_name.to_string(),
                        type_name,
                        default_value,
                        is_optional,
                        docstring: None,
                    });
                }
            }
        }
    }

    Ok(fields)
}

fn extract_typescript_fields(content: &str, symbol: &Symbol) -> Result<Vec<FieldInfo>> {
    let mut parser = Parser::new();
    let language = tree_sitter_javascript::LANGUAGE.into();
    parser
        .set_language(&language)
        .context("Failed to set JavaScript/TypeScript language")?;

    let tree = parser
        .parse(content, None)
        .context("Failed to parse TypeScript file")?;

    let root = tree.root_node();
    let mut fields = Vec::new();

    let interface_query = Query::new(
        &language,
        r#"
        (interface_declaration
            name: (type_identifier) @interface.name
            body: (object_type
                (property_signature
                    name: (property_identifier) @prop.name
                    type: (type_annotation (_) @prop.type)))) @interface.def
        "#,
    )
    .context("Failed to create TypeScript interface query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&interface_query, root, content.as_bytes());

    while let Some(match_) = matches.next() {
        let captures = match_.captures;

        let iface_name_cap = captures.iter().find(|c| {
            interface_query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("interface.name")
        });

        let iface_name = iface_name_cap
            .and_then(|c| c.node.utf8_text(content.as_bytes()).ok())
            .unwrap_or_default();

        if iface_name != symbol.name {
            continue;
        }

        let prop_name_cap = captures.iter().find(|c| {
            interface_query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("prop.name")
        });

        let prop_type_cap = captures.iter().find(|c| {
            interface_query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("prop.type")
        });

        if let Some(name_cap) = prop_name_cap {
            let name = name_cap
                .node
                .utf8_text(content.as_bytes())
                .unwrap_or_default()
                .to_string();

            let type_name = prop_type_cap
                .and_then(|c| c.node.utf8_text(content.as_bytes()).ok())
                .unwrap_or_default()
                .to_string();

            let is_optional = name.ends_with('?') || type_name.contains("undefined");

            fields.push(FieldInfo {
                name: name.trim_end_matches('?').to_string(),
                type_name,
                default_value: None,
                is_optional,
                docstring: None,
            });
        }
    }

    let class_query = Query::new(
        &language,
        r#"
        (class_declaration
            name: (type_identifier) @class.name
            body: (class_body
                (public_field_definition
                    name: (property_identifier) @field.name
                    type: (type_annotation (_) @field.type)?
                    value: (_)? @field.value))) @class.def
        "#,
    )
    .context("Failed to create TypeScript class field query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&class_query, root, content.as_bytes());

    while let Some(match_) = matches.next() {
        let captures = match_.captures;

        let class_name_cap = captures.iter().find(|c| {
            class_query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("class.name")
        });

        let class_name = class_name_cap
            .and_then(|c| c.node.utf8_text(content.as_bytes()).ok())
            .unwrap_or_default();

        if class_name != symbol.name {
            continue;
        }

        let field_name_cap = captures.iter().find(|c| {
            class_query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.name")
        });

        let field_type_cap = captures.iter().find(|c| {
            class_query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.type")
        });

        let field_value_cap = captures.iter().find(|c| {
            class_query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.value")
        });

        if let Some(name_cap) = field_name_cap {
            let name = name_cap
                .node
                .utf8_text(content.as_bytes())
                .unwrap_or_default()
                .to_string();

            let type_name = field_type_cap
                .and_then(|c| c.node.utf8_text(content.as_bytes()).ok())
                .unwrap_or("any")
                .to_string();

            let default_value = field_value_cap
                .and_then(|c| c.node.utf8_text(content.as_bytes()).ok())
                .map(|s| s.to_string());

            let is_optional = type_name.contains("?") || type_name.contains("undefined");

            fields.push(FieldInfo {
                name,
                type_name,
                default_value,
                is_optional,
                docstring: None,
            });
        }
    }

    Ok(fields)
}

fn extract_java_fields(content: &str, symbol: &Symbol) -> Result<Vec<FieldInfo>> {
    let mut parser = Parser::new();
    let language = tree_sitter_java::LANGUAGE.into();
    parser
        .set_language(&language)
        .context("Failed to set Java language")?;

    let tree = parser
        .parse(content, None)
        .context("Failed to parse Java file")?;

    let root = tree.root_node();
    let mut fields = Vec::new();

    let query = Query::new(
        &language,
        r#"
        (class_declaration
            name: (identifier) @class.name
            body: (class_body
                (field_declaration
                    type: (_) @field.type
                    declarator: (variable_declarator
                        name: (identifier) @field.name
                        value: (_)? @field.value)))) @class.def
        "#,
    )
    .context("Failed to create Java class field query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, root, content.as_bytes());

    while let Some(match_) = matches.next() {
        let captures = match_.captures;

        let class_name_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("class.name")
        });

        let class_name = class_name_cap
            .and_then(|c| c.node.utf8_text(content.as_bytes()).ok())
            .unwrap_or_default();

        if class_name != symbol.name {
            continue;
        }

        let field_name_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.name")
        });

        let field_type_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.type")
        });

        let field_value_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.value")
        });

        if let (Some(name_cap), Some(type_cap)) = (field_name_cap, field_type_cap) {
            let name = name_cap
                .node
                .utf8_text(content.as_bytes())
                .unwrap_or_default()
                .to_string();

            let type_name = type_cap
                .node
                .utf8_text(content.as_bytes())
                .unwrap_or_default()
                .to_string();

            let default_value = field_value_cap
                .and_then(|c| c.node.utf8_text(content.as_bytes()).ok())
                .map(|s| s.to_string());

            let is_optional = type_name.contains("Optional");

            fields.push(FieldInfo {
                name,
                type_name,
                default_value,
                is_optional,
                docstring: None,
            });
        }
    }

    Ok(fields)
}

fn extract_go_fields(content: &str, symbol: &Symbol) -> Result<Vec<FieldInfo>> {
    let mut parser = Parser::new();
    let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
    parser
        .set_language(&language)
        .context("Failed to set Go language")?;

    let tree = parser
        .parse(content, None)
        .context("Failed to parse Go file")?;

    let root = tree.root_node();
    let mut fields = Vec::new();

    let query = Query::new(
        &language,
        r#"
        (type_declaration
            (type_spec
                name: (type_identifier) @struct.name
                type: (struct_type
                    (field_declaration_list
                        (field_declaration
                            name: (field_identifier) @field.name
                            type: (_) @field.type))))) @struct.def
        "#,
    )
    .context("Failed to create Go struct field query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, root, content.as_bytes());

    while let Some(match_) = matches.next() {
        let captures = match_.captures;

        let struct_name_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("struct.name")
        });

        let struct_name = struct_name_cap
            .and_then(|c| c.node.utf8_text(content.as_bytes()).ok())
            .unwrap_or_default();

        if struct_name != symbol.name {
            continue;
        }

        let field_name_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.name")
        });

        let field_type_cap = captures.iter().find(|c| {
            query
                .capture_names()
                .get(c.index as usize)
                .map(|s| s.as_ref())
                == Some("field.type")
        });

        if let (Some(name_cap), Some(type_cap)) = (field_name_cap, field_type_cap) {
            let name = name_cap
                .node
                .utf8_text(content.as_bytes())
                .unwrap_or_default()
                .to_string();

            let type_name = type_cap
                .node
                .utf8_text(content.as_bytes())
                .unwrap_or_default()
                .to_string();

            let is_optional = type_name.starts_with('*');

            fields.push(FieldInfo {
                name,
                type_name,
                default_value: None,
                is_optional,
                docstring: None,
            });
        }
    }

    Ok(fields)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_extract_rust_fields() -> Result<()> {
        let content = r#"
struct User {
    name: String,
    age: u32,
    email: Option<String>,
}
"#;
        let symbol = Symbol {
            name: "User".to_string(),
            symbol_type: SymbolType::Class,
            signature: None,
            docstring: None,
            line_start: 2,
            line_end: 6,
            parent_id: None,
            file_path: Path::new("test.rs").to_path_buf(),
            is_exported: false,
        };

        let fields = extract_rust_fields(content, &symbol)?;
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "name");
        assert_eq!(fields[0].type_name, "String");
        assert!(!fields[0].is_optional);
        assert!(fields[2].is_optional);
        Ok(())
    }

    #[test]
    fn test_extract_python_fields_fallback() -> Result<()> {
        let content = r#"
@dataclass
class User:
    name: str
    age: int
    email: Optional[str] = None
"#;
        let symbol = Symbol {
            name: "User".to_string(),
            symbol_type: SymbolType::Class,
            signature: None,
            docstring: None,
            line_start: 2,
            line_end: 6,
            parent_id: None,
            file_path: Path::new("test.py").to_path_buf(),
            is_exported: false,
        };

        let fields = extract_python_fields_fallback(content, &symbol)?;
        assert!(fields.len() >= 2);
        Ok(())
    }

    #[test]
    fn test_extract_go_fields() -> Result<()> {
        let content = r#"
package main

type User struct {
    Name  string
    Age   int
    Email *string
}
"#;
        let symbol = Symbol {
            name: "User".to_string(),
            symbol_type: SymbolType::Class,
            signature: Some("struct".to_string()),
            docstring: None,
            line_start: 4,
            line_end: 8,
            parent_id: None,
            file_path: Path::new("test.go").to_path_buf(),
            is_exported: false,
        };

        let fields = extract_go_fields(content, &symbol)?;
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "Name");
        assert_eq!(fields[0].type_name, "string");
        assert!(fields[2].is_optional);
        Ok(())
    }
}

use super::{ParseResult, Parser as ParserTrait};
use crate::models::{Dependency, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

pub struct RustParser;

fn has_pub_visibility(node: Node, source: &str) -> bool {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "visibility_modifier" {
                if let Ok(text) = child.utf8_text(source.as_bytes()) {
                    return text.starts_with("pub");
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

impl RustParser {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn extract_text(&self, node: Node, source: &str) -> Option<String> {
        let start = node.start_byte();
        let end = node.end_byte();
        if end <= source.len() && start <= end {
            source.get(start..end).map(|s| s.to_string())
        } else {
            None
        }
    }

    fn extract_docstring(&self, node: Node, source: &str) -> Option<String> {
        let mut cursor = node.walk();
        if !cursor.goto_first_child() {
            return None;
        }

        loop {
            let child = cursor.node();
            if child.kind() == "line_comment" {
                if let Some(text) = self.extract_text(child, source) {
                    if text.starts_with("///") || text.starts_with("//!") {
                        return Some(text);
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        None
    }

    fn extract_parameters(&self, params_node: Node, source: &str) -> Option<String> {
        self.extract_text(params_node, source)
    }

    fn find_parent_impl(&self, node: Node, symbols: &[Symbol]) -> Option<usize> {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "impl_item" {
                let parent_line = parent.start_position().row + 1;
                for (idx, symbol) in symbols.iter().enumerate() {
                    if symbol.symbol_type == SymbolType::Class && symbol.line_start == parent_line {
                        return Some(idx);
                    }
                }
            }
            current = parent;
        }
        None
    }

    fn is_inside_impl(&self, node: Node) -> bool {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "impl_item" {
                return true;
            }
            current = parent;
        }
        false
    }

    fn process_structs(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();

        let language = tree_sitter_rust::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (struct_item
                name: (type_identifier) @struct.name) @struct.def
            "#,
        )
        .context("Failed to create Rust struct query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut struct_name = None;
            let mut struct_node = None;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("struct.name") => {
                        struct_name = capture
                            .node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("struct.def") => {
                        struct_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (struct_name, struct_node) {
                let docstring = self.extract_docstring(node, source);
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;

                symbols.push(Symbol {
                    name,
                    symbol_type: SymbolType::Class,
                    signature: None,
                    docstring,
                    line_start,
                    line_end,
                    parent_id: None,
                    file_path: file_path.to_path_buf(),
                    is_exported: has_pub_visibility(node, source),
                });
            }
        }

        Ok(symbols)
    }

    fn process_enums(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();

        let language = tree_sitter_rust::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (enum_item
                name: (type_identifier) @enum.name) @enum.def
            "#,
        )
        .context("Failed to create Rust enum query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut enum_name = None;
            let mut enum_node = None;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("enum.name") => {
                        enum_name = capture
                            .node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("enum.def") => {
                        enum_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (enum_name, enum_node) {
                let docstring = self.extract_docstring(node, source);
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;

                symbols.push(Symbol {
                    name,
                    symbol_type: SymbolType::Enum,
                    signature: None,
                    docstring,
                    line_start,
                    line_end,
                    parent_id: None,
                    file_path: file_path.to_path_buf(),
                    is_exported: has_pub_visibility(node, source),
                });
            }
        }

        Ok(symbols)
    }

    fn process_static_fields(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();

        let language = tree_sitter_rust::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (const_item
                name: (identifier) @const.name
                type: (_) @const.type) @const.def
            (static_item
                name: (identifier) @static.name
                type: (_) @static.type) @static.def
            "#,
        )
        .context("Failed to create Rust static/const query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut field_name = None;
            let mut field_type = None;
            let mut field_node = None;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("const.name") | Some("static.name") => {
                        field_name = capture
                            .node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("const.type") | Some("static.type") => {
                        field_type = capture
                            .node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("const.def") | Some("static.def") => {
                        field_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (field_name, field_node) {
                let docstring = self.extract_docstring(node, source);
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;

                symbols.push(Symbol {
                    name,
                    symbol_type: SymbolType::StaticField,
                    signature: field_type,
                    docstring,
                    line_start,
                    line_end,
                    parent_id: None,
                    file_path: file_path.to_path_buf(),
                    is_exported: has_pub_visibility(node, source),
                });
            }
        }

        Ok(symbols)
    }

    fn process_impls(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();

        let language = tree_sitter_rust::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (impl_item
                type: (type_identifier) @impl.type) @impl.def
            "#,
        )
        .context("Failed to create Rust impl query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut impl_type = None;
            let mut impl_node = None;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("impl.type") => {
                        impl_type = capture
                            .node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("impl.def") => {
                        impl_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (impl_type, impl_node) {
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;

                symbols.push(Symbol {
                    name: format!("impl {}", name),
                    symbol_type: SymbolType::Class,
                    signature: None,
                    docstring: None,
                    line_start,
                    line_end,
                    parent_id: None,
                    file_path: file_path.to_path_buf(),
                    is_exported: false,
                });
            }
        }

        Ok(symbols)
    }

    fn process_functions(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
        symbols: &mut Vec<Symbol>,
    ) -> Result<()> {
        let language = tree_sitter_rust::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (function_item
                name: (identifier) @func.name
                parameters: (parameters) @func.params) @func.def
            "#,
        )
        .context("Failed to create Rust function query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut func_name = None;
            let mut func_params = None;
            let mut func_node = None;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("func.name") => {
                        func_name = capture
                            .node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("func.params") => {
                        func_params = Some(capture.node);
                    }
                    Some("func.def") => {
                        func_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (func_name, func_node) {
                let signature = func_params.and_then(|p| self.extract_parameters(p, source));
                let docstring = self.extract_docstring(node, source);
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;
                let is_method = self.is_inside_impl(node);
                let parent_id = if is_method {
                    self.find_parent_impl(node, symbols)
                } else {
                    None
                };

                symbols.push(Symbol {
                    name,
                    symbol_type: if is_method {
                        SymbolType::Method
                    } else {
                        SymbolType::Function
                    },
                    signature,
                    docstring,
                    line_start,
                    line_end,
                    parent_id,
                    file_path: file_path.to_path_buf(),
                    is_exported: has_pub_visibility(node, source),
                });
            }
        }

        Ok(())
    }

    fn process_imports(&self, tree_root: Node, source: &str) -> Result<Vec<Dependency>> {
        let mut imports = Vec::new();

        let language = tree_sitter_rust::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (use_declaration
                argument: (_) @use.path) @use.decl
            "#,
        )
        .context("Failed to create Rust use query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                if capture_name == Some("use.path") {
                    if let Some(import_text) = self.extract_text(capture.node, source) {
                        let cleaned = import_text
                            .replace("use ", "")
                            .replace(";", "")
                            .trim()
                            .to_string();

                        if !cleaned.is_empty() {
                            imports.push(Dependency {
                                import_name: cleaned,
                                from_file: None,
                            });
                        }
                    }
                }
            }
        }

        Ok(imports)
    }
}

impl ParserTrait for RustParser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult> {
        let mut parser = Parser::new();
        let language = tree_sitter_rust::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to set Rust language")?;

        let tree = parser
            .parse(content, None)
            .context("Failed to parse Rust file")?;

        let root = tree.root_node();
        let mut result = ParseResult::new();

        let structs = self.process_structs(root, content, file_path)?;
        result.symbols.extend(structs);

        let enums = self.process_enums(root, content, file_path)?;
        result.symbols.extend(enums);

        let static_fields = self.process_static_fields(root, content, file_path)?;
        result.symbols.extend(static_fields);

        let impls = self.process_impls(root, content, file_path)?;
        result.symbols.extend(impls);

        self.process_functions(root, content, file_path, &mut result.symbols)?;

        result.dependencies = self.process_imports(root, content)?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_function() -> Result<()> {
        let parser = RustParser::new()?;
        let source = r#"
fn hello_world() {
    println!("Hello, World!");
}
"#;
        let result = parser.parse(source, Path::new("test.rs"))?;
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "hello_world");
        assert_eq!(result.symbols[0].symbol_type, SymbolType::Function);
        Ok(())
    }

    #[test]
    fn test_parse_struct_with_impl() -> Result<()> {
        let parser = RustParser::new()?;
        let source = r#"
struct MyStruct {
    value: i32,
}

impl MyStruct {
    fn new() -> Self {
        Self { value: 0 }
    }

    fn get_value(&self) -> i32 {
        self.value
    }
}
"#;
        let result = parser.parse(source, Path::new("test.rs"))?;
        assert!(result.symbols.len() >= 3);
        Ok(())
    }

    #[test]
    fn test_parse_use_statements() -> Result<()> {
        let parser = RustParser::new()?;
        let source = r#"
use std::fs;
use std::path::Path;
use anyhow::Result;
"#;
        let result = parser.parse(source, Path::new("test.rs"))?;
        println!("Found {} dependencies:", result.dependencies.len());
        for dep in &result.dependencies {
            println!("  - {}", dep.import_name);
        }
        assert!(result.dependencies.len() >= 3);
        Ok(())
    }
}

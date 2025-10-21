use super::{ParseResult, Parser};
use crate::models::{Dependency, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser as TSParser, Query, QueryCursor};

pub struct JavaScriptParser;

impl JavaScriptParser {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn create_parser() -> Result<TSParser> {
        let mut parser = TSParser::new();
        let language = tree_sitter_javascript::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to set JavaScript language")?;
        Ok(parser)
    }

    fn extract_functions(
        &self,
        tree: &tree_sitter::Tree,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let language = tree_sitter_javascript::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            [
                (function_declaration
                    name: (identifier) @func.name) @func.def
                (arrow_function) @arrow.def
                (function_expression) @func_expr.def
                (variable_declarator
                    name: (identifier) @var.name
                    value: [(arrow_function) (function_expression)] @var.func)
            ]
            "#,
        )
        .context("Failed to create function query")?;

        let root_node = tree.root_node();
        let mut symbols = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root_node, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            if captures.is_empty() {
                continue;
            }

            if let Some(name) = self.extract_function_name(captures, source, &query)? {
                if let Some(node) = self.extract_function_node(captures, &query)? {
                    let signature = self.extract_signature(node, source)?;
                    let (line_start, line_end) = self.get_line_range(node);

                    symbols.push(Symbol {
                        name,
                        symbol_type: SymbolType::Function,
                        signature: Some(signature),
                        docstring: None,
                        line_start,
                        line_end,
                        parent_id: None,
                        file_path: file_path.to_path_buf(),
                    });
                }
            }
        }

        Ok(symbols)
    }

    fn extract_function_name(
        &self,
        captures: &[tree_sitter::QueryCapture],
        source: &str,
        query: &Query,
    ) -> Result<Option<String>> {
        for capture in captures {
            let capture_name = query
                .capture_names()
                .get(capture.index as usize)
                .map(|s| s.as_ref());

            match capture_name {
                Some("func.name") | Some("var.name") => {
                    return Ok(Some(
                        capture
                            .node
                            .utf8_text(source.as_bytes())
                            .context("Failed to extract function name")?
                            .to_string(),
                    ));
                }
                Some("arrow.def") | Some("func_expr.def") => {
                    return Ok(Some("anonymous".to_string()));
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn extract_function_node<'a>(
        &self,
        captures: &[tree_sitter::QueryCapture<'a>],
        query: &Query,
    ) -> Result<Option<Node<'a>>> {
        for capture in captures {
            let capture_name = query
                .capture_names()
                .get(capture.index as usize)
                .map(|s| s.as_ref());

            if matches!(
                capture_name,
                Some("func.def") | Some("arrow.def") | Some("func_expr.def") | Some("var.func")
            ) {
                return Ok(Some(capture.node));
            }
        }
        Ok(None)
    }

    fn extract_classes(
        &self,
        tree: &tree_sitter::Tree,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let language = tree_sitter_javascript::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (class_declaration
                name: (identifier) @class.name) @class.def
            "#,
        )
        .context("Failed to create class query")?;

        let root_node = tree.root_node();
        let mut symbols = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root_node, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            if captures.len() < 2 {
                continue;
            }

            let name_capture = captures.iter().find(|c| {
                query
                    .capture_names()
                    .get(c.index as usize)
                    .map(|s| s.as_ref())
                    == Some("class.name")
            });

            let def_capture = captures.iter().find(|c| {
                query
                    .capture_names()
                    .get(c.index as usize)
                    .map(|s| s.as_ref())
                    == Some("class.def")
            });

            if let (Some(name_cap), Some(def_cap)) = (name_capture, def_capture) {
                let name = name_cap
                    .node
                    .utf8_text(source.as_bytes())
                    .context("Failed to extract class name")?
                    .to_string();

                let (line_start, line_end) = self.get_line_range(def_cap.node);

                symbols.push(Symbol {
                    name,
                    symbol_type: SymbolType::Class,
                    signature: None,
                    docstring: None,
                    line_start,
                    line_end,
                    parent_id: None,
                    file_path: file_path.to_path_buf(),
                });
            }
        }

        Ok(symbols)
    }

    fn extract_methods(
        &self,
        tree: &tree_sitter::Tree,
        source: &str,
        file_path: &Path,
        class_symbols: &[Symbol],
    ) -> Result<Vec<Symbol>> {
        let language = tree_sitter_javascript::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (class_declaration
                body: (class_body
                    (method_definition
                        name: (_) @method.name) @method.def))
            "#,
        )
        .context("Failed to create method query")?;

        let root_node = tree.root_node();
        let mut symbols = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root_node, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            if captures.len() < 2 {
                continue;
            }

            let name_capture = captures.iter().find(|c| {
                query
                    .capture_names()
                    .get(c.index as usize)
                    .map(|s| s.as_ref())
                    == Some("method.name")
            });

            let def_capture = captures.iter().find(|c| {
                query
                    .capture_names()
                    .get(c.index as usize)
                    .map(|s| s.as_ref())
                    == Some("method.def")
            });

            if let (Some(name_cap), Some(def_cap)) = (name_capture, def_capture) {
                let name = name_cap
                    .node
                    .utf8_text(source.as_bytes())
                    .context("Failed to extract method name")?
                    .to_string();

                let signature = self.extract_signature(def_cap.node, source)?;
                let (line_start, line_end) = self.get_line_range(def_cap.node);

                let parent_id = self.find_parent_class(line_start, class_symbols);

                symbols.push(Symbol {
                    name,
                    symbol_type: SymbolType::Method,
                    signature: Some(signature),
                    docstring: None,
                    line_start,
                    line_end,
                    parent_id,
                    file_path: file_path.to_path_buf(),
                });
            }
        }

        Ok(symbols)
    }

    fn extract_dependencies(
        &self,
        tree: &tree_sitter::Tree,
        source: &str,
    ) -> Result<Vec<Dependency>> {
        let language = tree_sitter_javascript::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            [
                (import_statement
                    source: (string) @import.source)
                (call_expression
                    function: (identifier) @require.func
                    arguments: (arguments (string) @require.source))
            ]
            "#,
        )
        .context("Failed to create import query")?;

        let root_node = tree.root_node();
        let mut dependencies = Vec::new();
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root_node, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            if captures.is_empty() {
                continue;
            }

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                if matches!(capture_name, Some("import.source") | Some("require.source")) {
                    let import_text = capture
                        .node
                        .utf8_text(source.as_bytes())
                        .context("Failed to extract import source")?;

                    let import_name = self.clean_import_string(import_text);

                    dependencies.push(Dependency {
                        import_name,
                        from_file: None,
                    });
                }
            }
        }

        Ok(dependencies)
    }

    fn extract_signature(&self, node: Node, source: &str) -> Result<String> {
        let mut cursor = node.walk();

        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();

                if child.kind() == "formal_parameters" {
                    let params = child
                        .utf8_text(source.as_bytes())
                        .context("Failed to extract parameters")?;
                    return Ok(params.to_string());
                }

                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }

        Ok("()".to_string())
    }

    fn get_line_range(&self, node: Node) -> (usize, usize) {
        let start_pos = node.start_position();
        let end_pos = node.end_position();
        (start_pos.row + 1, end_pos.row + 1)
    }

    fn find_parent_class(&self, line_num: usize, class_symbols: &[Symbol]) -> Option<usize> {
        class_symbols
            .iter()
            .enumerate()
            .find(|(_, class)| line_num > class.line_start && line_num <= class.line_end)
            .map(|(idx, _)| idx)
    }

    fn clean_import_string(&self, s: &str) -> String {
        s.trim()
            .trim_matches('\'')
            .trim_matches('"')
            .to_string()
    }
}

impl Parser for JavaScriptParser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult> {
        let mut parser = Self::create_parser()?;
        let tree = parser
            .parse(content, None)
            .context("Failed to parse JavaScript content")?;

        let classes = self.extract_classes(&tree, content, file_path)?;
        let functions = self.extract_functions(&tree, content, file_path)?;
        let methods = self.extract_methods(&tree, content, file_path, &classes)?;
        let dependencies = self.extract_dependencies(&tree, content)?;

        let mut symbols = Vec::with_capacity(classes.len() + functions.len() + methods.len());
        symbols.extend(classes);
        symbols.extend(functions);
        symbols.extend(methods);

        Ok(ParseResult {
            symbols,
            dependencies,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_function_declaration() -> Result<()> {
        let parser = JavaScriptParser::new()?;
        let content = "function greet(name) { return 'Hello ' + name; }";
        let path = Path::new("test.js");

        let result = parser.parse(content, path)?;

        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "greet");
        assert_eq!(result.symbols[0].symbol_type, SymbolType::Function);
        Ok(())
    }

    #[test]
    fn test_parse_arrow_function() -> Result<()> {
        let parser = JavaScriptParser::new()?;
        let content = "const add = (a, b) => a + b;";
        let path = Path::new("test.js");

        let result = parser.parse(content, path)?;

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "add" && s.symbol_type == SymbolType::Function));
        Ok(())
    }

    #[test]
    fn test_parse_class_declaration() -> Result<()> {
        let parser = JavaScriptParser::new()?;
        let content = "class MyClass { constructor() {} }";
        let path = Path::new("test.js");

        let result = parser.parse(content, path)?;

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "MyClass" && s.symbol_type == SymbolType::Class));
        Ok(())
    }

    #[test]
    fn test_parse_method() -> Result<()> {
        let parser = JavaScriptParser::new()?;
        let content = r#"
class Calculator {
    add(a, b) {
        return a + b;
    }
}
        "#;
        let path = Path::new("test.js");

        let result = parser.parse(content, path)?;

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "add" && s.symbol_type == SymbolType::Method));
        Ok(())
    }

    #[test]
    fn test_parse_es6_import() -> Result<()> {
        let parser = JavaScriptParser::new()?;
        let content = "import React from 'react';";
        let path = Path::new("test.js");

        let result = parser.parse(content, path)?;

        assert_eq!(result.dependencies.len(), 1);
        assert_eq!(result.dependencies[0].import_name, "react");
        Ok(())
    }

    #[test]
    fn test_parse_require() -> Result<()> {
        let parser = JavaScriptParser::new()?;
        let content = "const fs = require('fs');";
        let path = Path::new("test.js");

        let result = parser.parse(content, path)?;

        assert_eq!(result.dependencies.len(), 1);
        assert_eq!(result.dependencies[0].import_name, "fs");
        Ok(())
    }
}

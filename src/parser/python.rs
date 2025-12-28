use super::{ParseResult, Parser as ParserTrait};
use crate::models::{Dependency, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

pub struct PythonParser;

impl PythonParser {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn extract_docstring(&self, body_node: Node, source: &str) -> Option<String> {
        let mut cursor = body_node.walk();
        if !cursor.goto_first_child() {
            return None;
        }

        loop {
            let node = cursor.node();
            if node.kind() == "expression_statement" {
                let mut expr_cursor = node.walk();
                if expr_cursor.goto_first_child() {
                    let child = expr_cursor.node();
                    if child.kind() == "string" {
                        return self.extract_text(child, source);
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        None
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

    fn extract_parameters(&self, params_node: Node, source: &str) -> Option<String> {
        self.extract_text(params_node, source)
    }

    fn find_parent_class(&self, node: Node, symbols: &[Symbol]) -> Option<usize> {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "class_definition" {
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

    fn is_inside_class(&self, node: Node) -> bool {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "class_definition" {
                return true;
            }
            current = parent;
        }
        false
    }

    fn process_classes(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();

        let language = tree_sitter_python::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (class_definition
                name: (identifier) @class.name
                body: (block) @class.body) @class.def
            "#,
        )
        .context("Failed to create Python class query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut class_name = None;
            let mut class_body = None;
            let mut class_node = None;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("class.name") => {
                        class_name = capture
                            .node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("class.body") => {
                        class_body = Some(capture.node);
                    }
                    Some("class.def") => {
                        class_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (class_name, class_node) {
                let docstring = class_body.and_then(|body| self.extract_docstring(body, source));
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;
                let parent_id = self.find_parent_class(node, &symbols);

                symbols.push(Symbol {
                    name,
                    symbol_type: SymbolType::Class,
                    signature: None,
                    docstring,
                    line_start,
                    line_end,
                    parent_id,
                    file_path: file_path.to_path_buf(),
                    is_exported: parent_id.is_none(),
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
        let language = tree_sitter_python::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (function_definition
                name: (identifier) @func.name
                parameters: (parameters) @func.params
                body: (block) @func.body) @func.def
            "#,
        )
        .context("Failed to create Python function query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut func_name = None;
            let mut func_params = None;
            let mut func_body = None;
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
                    Some("func.body") => {
                        func_body = Some(capture.node);
                    }
                    Some("func.def") => {
                        func_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (func_name, func_node) {
                let signature = func_params.and_then(|p| self.extract_parameters(p, source));
                let docstring = func_body.and_then(|body| self.extract_docstring(body, source));
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;
                let is_method = self.is_inside_class(node);
                let parent_id = if is_method {
                    self.find_parent_class(node, symbols)
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
                    is_exported: !is_method,
                });
            }
        }

        Ok(())
    }

    fn process_constants(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();
        let language = tree_sitter_python::LANGUAGE.into();

        let query = Query::new(
            &language,
            r#"
            (module
              (expression_statement
                (assignment
                  left: (identifier) @const.name
                  right: (_) @const.value))) @const.def
            "#,
        )
        .context("Failed to create Python constants query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut const_name = None;
            let mut const_node = None;
            let mut const_value = None;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("const.name") => {
                        const_name = capture
                            .node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("const.value") => {
                        const_value = self.extract_text(capture.node, source);
                    }
                    Some("const.def") => {
                        const_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (const_name, const_node) {
                if name
                    .chars()
                    .all(|c| c.is_uppercase() || c == '_' || c.is_numeric())
                    && name.len() > 1
                {
                    let line_start = node.start_position().row + 1;
                    let line_end = node.end_position().row + 1;

                    symbols.push(Symbol {
                        name,
                        symbol_type: SymbolType::StaticField,
                        signature: const_value,
                        docstring: None,
                        line_start,
                        line_end,
                        parent_id: None,
                        file_path: file_path.to_path_buf(),
                        is_exported: true,
                    });
                }
            }
        }

        Ok(symbols)
    }

    fn process_imports(&self, tree_root: Node, source: &str) -> Result<Vec<Dependency>> {
        let mut imports = Vec::new();
        let mut stack = vec![tree_root];

        while let Some(node) = stack.pop() {
            match node.kind() {
                "import_statement" => {
                    let mut child_cursor = node.walk();
                    if child_cursor.goto_first_child() {
                        loop {
                            let child = child_cursor.node();
                            match child.kind() {
                                "dotted_name" => {
                                    if let Some(import_name) = self.extract_text(child, source) {
                                        imports.push(Dependency {
                                            import_name,
                                            from_file: None,
                                        });
                                    }
                                }
                                "aliased_import" => {
                                    let mut alias_cursor = child.walk();
                                    if alias_cursor.goto_first_child() {
                                        let name_node = alias_cursor.node();
                                        if name_node.kind() == "dotted_name" {
                                            if let Some(import_name) =
                                                self.extract_text(name_node, source)
                                            {
                                                imports.push(Dependency {
                                                    import_name,
                                                    from_file: None,
                                                });
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                            if !child_cursor.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
                "import_from_statement" => {
                    let mut from_module = None;
                    let mut import_names = Vec::new();

                    let mut child_cursor = node.walk();
                    if child_cursor.goto_first_child() {
                        loop {
                            let child = child_cursor.node();
                            match child.kind() {
                                "dotted_name" if from_module.is_none() => {
                                    from_module = self.extract_text(child, source);
                                }
                                "dotted_name" => {
                                    if let Some(name) = self.extract_text(child, source) {
                                        import_names.push(name);
                                    }
                                }
                                "aliased_import" => {
                                    let mut alias_cursor = child.walk();
                                    if alias_cursor.goto_first_child() {
                                        let name_node = alias_cursor.node();
                                        if name_node.kind() == "dotted_name" {
                                            if let Some(name) = self.extract_text(name_node, source)
                                            {
                                                import_names.push(name);
                                            }
                                        }
                                    }
                                }
                                "wildcard_import" => {
                                    import_names.push("*".to_string());
                                }
                                _ => {}
                            }
                            if !child_cursor.goto_next_sibling() {
                                break;
                            }
                        }
                    }

                    for name in import_names {
                        imports.push(Dependency {
                            import_name: name,
                            from_file: from_module.clone(),
                        });
                    }
                }
                _ => {}
            }

            let mut child_cursor = node.walk();
            if child_cursor.goto_first_child() {
                loop {
                    stack.push(child_cursor.node());
                    if !child_cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }

        Ok(imports)
    }
}

impl ParserTrait for PythonParser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult> {
        // Create a fresh parser for each call
        let mut parser = Parser::new();
        let language = tree_sitter_python::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to set Python language")?;

        let tree = parser
            .parse(content, None)
            .context("Failed to parse Python file")?;

        let root = tree.root_node();
        let mut result = ParseResult::new();

        let classes = self.process_classes(root, content, file_path)?;
        result.symbols.extend(classes);

        let constants = self.process_constants(root, content, file_path)?;
        result.symbols.extend(constants);

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
        let parser = PythonParser::new()?;
        let source = r#"
def hello_world():
    """A simple function"""
    print("Hello, World!")
"#;
        let result = parser.parse(source, Path::new("test.py"))?;
        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "hello_world");
        assert_eq!(result.symbols[0].symbol_type, SymbolType::Function);
        Ok(())
    }

    #[test]
    fn test_parse_class_with_methods() -> Result<()> {
        let parser = PythonParser::new()?;
        let source = r#"
class MyClass:
    """A simple class"""
    def method_one(self):
        pass
    
    def method_two(self, x):
        pass
"#;
        let result = parser.parse(source, Path::new("test.py"))?;
        assert!(result.symbols.len() >= 3);

        let class_symbols: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.symbol_type == SymbolType::Class)
            .collect();
        assert_eq!(class_symbols.len(), 1);
        assert_eq!(class_symbols[0].name, "MyClass");

        let method_symbols: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.symbol_type == SymbolType::Method)
            .collect();
        assert_eq!(method_symbols.len(), 2);
        Ok(())
    }

    #[test]
    fn test_parse_imports() -> Result<()> {
        let parser = PythonParser::new()?;
        let source = r#"
import os
import sys
from pathlib import Path
from typing import List, Dict
"#;
        let result = parser.parse(source, Path::new("test.py"))?;
        assert!(result.dependencies.len() >= 4);
        Ok(())
    }
}

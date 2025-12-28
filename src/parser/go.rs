use super::{ParseResult, Parser as ParserTrait};
use crate::models::{Dependency, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

pub struct GoParser;

fn is_go_exported(name: &str) -> bool {
    name.chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
}

impl GoParser {
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

    fn extract_comment(&self, node: Node, source: &str) -> Option<String> {
        let mut current = node;
        while let Some(prev) = current.prev_sibling() {
            if prev.kind() == "comment" {
                return self.extract_text(prev, source);
            }
            if prev.kind() != "comment" {
                break;
            }
            current = prev;
        }
        None
    }

    fn process_functions(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();
        let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();

        // Query for function declarations
        let func_query = Query::new(&language, r#"(function_declaration) @func.def"#)
            .context("Failed to create Go function query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&func_query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            for capture in match_.captures {
                let node = capture.node;
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;

                // Extract function name and params
                let mut func_name = None;
                let mut func_params = None;

                let mut cursor = node.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        if child.kind() == "identifier" && func_name.is_none() {
                            func_name = self.extract_text(child, source);
                        } else if child.kind() == "parameter_list" {
                            func_params = self.extract_text(child, source);
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }

                if let Some(name) = func_name {
                    let docstring = self.extract_comment(node, source);
                    let exported = is_go_exported(&name);
                    symbols.push(Symbol {
                        name,
                        symbol_type: SymbolType::Function,
                        signature: func_params,
                        docstring,
                        line_start,
                        line_end,
                        parent_id: None,
                        file_path: file_path.to_path_buf(),
                        is_exported: exported,
                    });
                }
            }
        }

        // Query for method declarations
        let method_query = Query::new(&language, r#"(method_declaration) @method.def"#)
            .context("Failed to create Go method query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&method_query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            for capture in match_.captures {
                let node = capture.node;
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;

                // Extract method name and params
                let mut method_name = None;
                let mut method_params = None;

                let mut cursor = node.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        if child.kind() == "field_identifier" && method_name.is_none() {
                            method_name = self.extract_text(child, source);
                        } else if child.kind() == "parameter_list" {
                            method_params = self.extract_text(child, source);
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }

                if let Some(name) = method_name {
                    let docstring = self.extract_comment(node, source);
                    let exported = is_go_exported(&name);
                    symbols.push(Symbol {
                        name,
                        symbol_type: SymbolType::Method,
                        signature: method_params,
                        docstring,
                        line_start,
                        line_end,
                        parent_id: None,
                        file_path: file_path.to_path_buf(),
                        is_exported: exported,
                    });
                }
            }
        }

        Ok(symbols)
    }

    fn process_types(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();
        let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();

        // Query for type declarations (structs, interfaces, etc.)
        let type_query = Query::new(
            &language,
            r#"
            (type_declaration
                (type_spec
                    name: (type_identifier) @type.name)) @type.def
            "#,
        )
        .context("Failed to create Go type query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&type_query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let mut type_name = None;
            let mut type_node = None;

            for capture in match_.captures {
                let capture_name = type_query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("type.name") => {
                        type_name = capture
                            .node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("type.def") => {
                        type_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (type_name, type_node) {
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;
                let docstring = self.extract_comment(node, source);

                // Determine if it's a struct, interface, or other type
                let type_kind = if self
                    .extract_text(node, source)
                    .unwrap_or_default()
                    .contains("struct")
                {
                    "struct"
                } else if self
                    .extract_text(node, source)
                    .unwrap_or_default()
                    .contains("interface")
                {
                    "interface"
                } else {
                    "type"
                };

                let exported = is_go_exported(&name);
                symbols.push(Symbol {
                    name,
                    symbol_type: SymbolType::Class,
                    signature: Some(type_kind.to_string()),
                    docstring,
                    line_start,
                    line_end,
                    parent_id: None,
                    file_path: file_path.to_path_buf(),
                    is_exported: exported,
                });
            }
        }

        Ok(symbols)
    }

    fn process_consts(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();
        let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();

        let query = Query::new(
            &language,
            r#"
            (const_declaration
                (const_spec
                    name: (identifier) @const.name
                    type: (_)? @const.type
                    value: (_)? @const.value)) @const.def
            "#,
        )
        .context("Failed to create Go const query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut const_name = None;
            let mut const_type = None;
            let mut const_node = None;

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
                    Some("const.type") => {
                        const_type = self.extract_text(capture.node, source);
                    }
                    Some("const.def") => {
                        const_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (const_name, const_node) {
                let docstring = self.extract_comment(node, source);
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;
                let exported = is_go_exported(&name);

                symbols.push(Symbol {
                    name,
                    symbol_type: SymbolType::StaticField,
                    signature: const_type,
                    docstring,
                    line_start,
                    line_end,
                    parent_id: None,
                    file_path: file_path.to_path_buf(),
                    is_exported: exported,
                });
            }
        }

        Ok(symbols)
    }

    fn process_imports(&self, tree_root: Node, source: &str) -> Result<Vec<Dependency>> {
        let mut imports = Vec::new();
        let mut stack = vec![tree_root];

        while let Some(node) = stack.pop() {
            if node.kind() == "import_declaration" {
                let mut cursor = node.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        if child.kind() == "import_spec_list" {
                            // Handle import block: import ( ... )
                            let mut spec_list_cursor = child.walk();
                            if spec_list_cursor.goto_first_child() {
                                loop {
                                    let spec_node = spec_list_cursor.node();
                                    if spec_node.kind() == "import_spec" {
                                        let mut spec_cursor = spec_node.walk();
                                        if spec_cursor.goto_first_child() {
                                            loop {
                                                let spec_child = spec_cursor.node();
                                                if spec_child.kind() == "interpreted_string_literal"
                                                {
                                                    if let Some(import_path) =
                                                        self.extract_text(spec_child, source)
                                                    {
                                                        let clean_path = import_path
                                                            .trim_matches('"')
                                                            .to_string();
                                                        imports.push(Dependency {
                                                            import_name: clean_path,
                                                            from_file: None,
                                                        });
                                                    }
                                                }
                                                if !spec_cursor.goto_next_sibling() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    if !spec_list_cursor.goto_next_sibling() {
                                        break;
                                    }
                                }
                            }
                        } else if child.kind() == "import_spec" {
                            // Handle single import_spec without list
                            let mut spec_cursor = child.walk();
                            if spec_cursor.goto_first_child() {
                                loop {
                                    let spec_child = spec_cursor.node();
                                    if spec_child.kind() == "interpreted_string_literal" {
                                        if let Some(import_path) =
                                            self.extract_text(spec_child, source)
                                        {
                                            let clean_path =
                                                import_path.trim_matches('"').to_string();
                                            imports.push(Dependency {
                                                import_name: clean_path,
                                                from_file: None,
                                            });
                                        }
                                    }
                                    if !spec_cursor.goto_next_sibling() {
                                        break;
                                    }
                                }
                            }
                        } else if child.kind() == "interpreted_string_literal" {
                            // Handle simple import: import "fmt"
                            if let Some(import_path) = self.extract_text(child, source) {
                                let clean_path = import_path.trim_matches('"').to_string();
                                imports.push(Dependency {
                                    import_name: clean_path,
                                    from_file: None,
                                });
                            }
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
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

impl ParserTrait for GoParser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult> {
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to set Go language")?;

        let tree = parser
            .parse(content, None)
            .context("Failed to parse Go file")?;

        let root = tree.root_node();
        let mut result = ParseResult::new();

        let mut types = self.process_types(root, content, file_path)?;
        let functions = self.process_functions(root, content, file_path)?;
        let consts = self.process_consts(root, content, file_path)?;

        types.extend(functions);
        types.extend(consts);
        result.symbols = types;
        result.dependencies = self.process_imports(root, content)?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_function() -> Result<()> {
        let parser = GoParser::new()?;
        let source = r#"
package main

func HelloWorld() {
    println("Hello, World!")
}
"#;
        let result = parser.parse(source, Path::new("test.go"))?;
        assert!(result.symbols.len() >= 1);

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.symbol_type == SymbolType::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "HelloWorld");

        Ok(())
    }

    #[test]
    fn test_parse_struct() -> Result<()> {
        let parser = GoParser::new()?;
        let source = r#"
package main

type User struct {
    Name string
    Age  int
}
"#;
        let result = parser.parse(source, Path::new("test.go"))?;

        let structs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.symbol_type == SymbolType::Class)
            .collect();
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "User");

        Ok(())
    }

    #[test]
    fn test_parse_imports() -> Result<()> {
        let parser = GoParser::new()?;
        let source = r#"
package main

import (
    "fmt"
    "os"
)
"#;
        let result = parser.parse(source, Path::new("test.go"))?;
        assert!(result.dependencies.len() >= 2);

        Ok(())
    }
}

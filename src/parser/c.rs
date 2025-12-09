use super::{Parser as ParserTrait, ParseResult};
use crate::models::{Dependency, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

pub struct CParser;

impl CParser {
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
        let language: tree_sitter::Language = tree_sitter_c::LANGUAGE.into();

        // Query for function definitions
        let func_query = Query::new(
            &language,
            r#"(function_definition) @func.def"#,
        )
        .context("Failed to create C function query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&func_query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            for capture in match_.captures {
                let node = capture.node;
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;

                // Extract function name and params by traversing children
                let mut func_name = None;
                let mut func_params = None;

                let mut cursor = node.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        if child.kind() == "function_declarator" {
                            let mut decl_cursor = child.walk();
                            if decl_cursor.goto_first_child() {
                                loop {
                                    let decl_child = decl_cursor.node();
                                    if decl_child.kind() == "identifier" && func_name.is_none() {
                                        func_name = self.extract_text(decl_child, source);
                                    } else if decl_child.kind() == "parameter_list" {
                                        func_params = self.extract_text(decl_child, source);
                                    }
                                    if !decl_cursor.goto_next_sibling() {
                                        break;
                                    }
                                }
                            }
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }

                if let Some(name) = func_name {
                    let docstring = self.extract_comment(node, source);
                    symbols.push(Symbol {
                        name,
                        symbol_type: SymbolType::Function,
                        signature: func_params,
                        docstring,
                        line_start,
                        line_end,
                        parent_id: None,
                        file_path: file_path.to_path_buf(),
                        is_exported: false,
                    });
                }
            }
        }

        Ok(symbols)
    }

    fn process_structs(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();
        let language: tree_sitter::Language = tree_sitter_c::LANGUAGE.into();

        // Query for struct definitions
        let struct_query = Query::new(
            &language,
            r#"(struct_specifier) @struct.def"#,
        )
        .context("Failed to create C struct query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&struct_query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            for capture in match_.captures {
                let node = capture.node;
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;

                // Extract struct name
                let mut struct_name = None;
                let mut cursor = node.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        if child.kind() == "type_identifier" {
                            struct_name = self.extract_text(child, source);
                            break;
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }

                if let Some(name) = struct_name {
                    let docstring = self.extract_comment(node, source);
                    symbols.push(Symbol {
                        name,
                        symbol_type: SymbolType::Class,
                        signature: Some("struct".to_string()),
                        docstring,
                        line_start,
                        line_end,
                        parent_id: None,
                        file_path: file_path.to_path_buf(),
                        is_exported: false,
                    });
                }
            }
        }

        // Query for union definitions
        let union_query = Query::new(
            &language,
            r#"(union_specifier) @union.def"#,
        )
        .context("Failed to create C union query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&union_query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            for capture in match_.captures {
                let node = capture.node;
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;

                // Extract union name
                let mut union_name = None;
                let mut cursor = node.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        if child.kind() == "type_identifier" {
                            union_name = self.extract_text(child, source);
                            break;
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }

                if let Some(name) = union_name {
                    let docstring = self.extract_comment(node, source);
                    symbols.push(Symbol {
                        name,
                        symbol_type: SymbolType::Class,
                        signature: Some("union".to_string()),
                        docstring,
                        line_start,
                        line_end,
                        parent_id: None,
                        file_path: file_path.to_path_buf(),
                        is_exported: false,
                    });
                }
            }
        }

        Ok(symbols)
    }

    fn process_includes(&self, tree_root: Node, source: &str) -> Result<Vec<Dependency>> {
        let mut includes = Vec::new();
        let mut stack = vec![tree_root];

        while let Some(node) = stack.pop() {
            if node.kind() == "preproc_include" {
                let mut cursor = node.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        if child.kind() == "string_literal" || child.kind() == "system_lib_string" {
                            if let Some(include_path) = self.extract_text(child, source) {
                                let clean_path = include_path
                                    .trim_matches('"')
                                    .trim_matches('<')
                                    .trim_matches('>')
                                    .to_string();
                                includes.push(Dependency {
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

        Ok(includes)
    }
}

impl ParserTrait for CParser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult> {
        let mut parser = Parser::new();
        let language: tree_sitter::Language = tree_sitter_c::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to set C language")?;

        let tree = parser
            .parse(content, None)
            .context("Failed to parse C file")?;

        let root = tree.root_node();
        let mut result = ParseResult::new();

        let mut structs = self.process_structs(root, content, file_path)?;
        let functions = self.process_functions(root, content, file_path)?;

        structs.extend(functions);
        result.symbols = structs;
        result.dependencies = self.process_includes(root, content)?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_function() -> Result<()> {
        let parser = CParser::new()?;
        let source = r#"
int main(void) {
    printf("Hello, World!\n");
    return 0;
}
"#;
        let result = parser.parse(source, Path::new("test.c"))?;

        let funcs: Vec<_> = result.symbols.iter()
            .filter(|s| s.symbol_type == SymbolType::Function)
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "main");

        Ok(())
    }

    #[test]
    fn test_parse_struct() -> Result<()> {
        let parser = CParser::new()?;
        let source = r#"
struct User {
    char name[50];
    int age;
};
"#;
        let result = parser.parse(source, Path::new("test.c"))?;

        let structs: Vec<_> = result.symbols.iter()
            .filter(|s| s.symbol_type == SymbolType::Class)
            .collect();
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "User");

        Ok(())
    }

    #[test]
    fn test_parse_includes() -> Result<()> {
        let parser = CParser::new()?;
        let source = r#"
#include <stdio.h>
#include "myheader.h"
"#;
        let result = parser.parse(source, Path::new("test.c"))?;
        assert!(result.dependencies.len() >= 2);

        Ok(())
    }
}

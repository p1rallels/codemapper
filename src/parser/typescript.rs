use super::{ParseResult, Parser};
use crate::models::{Dependency, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser as TSParser, Query, QueryCursor};

pub struct TypeScriptParser;

fn is_exported(node: Node) -> bool {
    let mut current = node;
    const MAX_DEPTH: usize = 3;
    let mut depth = 0;

    while depth < MAX_DEPTH {
        if let Some(parent) = current.parent() {
            if parent.kind() == "export_statement" {
                return true;
            }
            current = parent;
            depth += 1;
        } else {
            break;
        }
    }
    false
}

impl TypeScriptParser {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn create_parser() -> Result<TSParser> {
        let mut parser = TSParser::new();
        let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        parser
            .set_language(&language)
            .context("Failed to set TypeScript language")?;
        Ok(parser)
    }

    fn extract_functions(
        &self,
        tree: &tree_sitter::Tree,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
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
                        is_exported: is_exported(node),
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
                    if let Some(test_name) = self.extract_test_context(capture.node, source) {
                        return Ok(Some(test_name));
                    }
                    return Ok(Some("anonymous".to_string()));
                }
                _ => {}
            }
        }
        Ok(None)
    }

    fn extract_test_context(&self, node: Node, source: &str) -> Option<String> {
        let mut current = node.parent()?;

        const MAX_DEPTH: usize = 10;
        let mut depth = 0;

        while depth < MAX_DEPTH {
            if current.kind() == "arguments" {
                if let Some(call_expr) = current.parent() {
                    if call_expr.kind() == "call_expression" {
                        return self.extract_test_name_from_call(call_expr, source);
                    }
                }
            }

            current = current.parent()?;
            depth += 1;
        }

        None
    }

    fn extract_test_name_from_call(&self, call_node: Node, source: &str) -> Option<String> {
        let mut cursor = call_node.walk();

        if !cursor.goto_first_child() {
            return None;
        }

        let mut func_name: Option<String> = None;
        let mut description: Option<String> = None;

        loop {
            let child = cursor.node();

            match child.kind() {
                "identifier" | "member_expression" => {
                    let name = child.utf8_text(source.as_bytes()).ok()?;
                    func_name = Some(name.to_string());
                }
                "arguments" => {
                    let mut args_cursor = child.walk();
                    if args_cursor.goto_first_child() {
                        loop {
                            let arg = args_cursor.node();
                            if arg.kind() == "string" || arg.kind() == "template_string" {
                                let text = arg.utf8_text(source.as_bytes()).ok()?;
                                let cleaned = text
                                    .trim_matches('\'')
                                    .trim_matches('"')
                                    .trim_matches('`')
                                    .to_string();
                                description = Some(cleaned);
                                break;
                            }
                            if !args_cursor.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }

        let func = func_name?;
        let func_lower = func.to_lowercase();

        match func_lower.as_str() {
            "describe" => {
                let desc = description.unwrap_or_else(|| "suite".to_string());
                Some(format!("describe:{}", desc))
            }
            "it" | "test" => {
                let desc = description.unwrap_or_else(|| "test".to_string());
                Some(format!("test:{}", desc))
            }
            "beforeeach" | "aftereach" | "beforeall" | "afterall" => Some(func_lower),
            _ => None,
        }
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
        let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let query = Query::new(
            &language,
            r#"
            (class_declaration
                name: (type_identifier) @class.name) @class.def
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
                    is_exported: is_exported(def_cap.node),
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
        let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
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
                    is_exported: false,
                });
            }
        }

        Ok(symbols)
    }

    fn extract_interfaces(
        &self,
        tree: &tree_sitter::Tree,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let query = Query::new(
            &language,
            r#"
            (interface_declaration
                name: (type_identifier) @interface.name) @interface.def
            "#,
        )
        .context("Failed to create interface query")?;

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
                    == Some("interface.name")
            });

            let def_capture = captures.iter().find(|c| {
                query
                    .capture_names()
                    .get(c.index as usize)
                    .map(|s| s.as_ref())
                    == Some("interface.def")
            });

            if let (Some(name_cap), Some(def_cap)) = (name_capture, def_capture) {
                let name = name_cap
                    .node
                    .utf8_text(source.as_bytes())
                    .context("Failed to extract interface name")?
                    .to_string();

                let (line_start, line_end) = self.get_line_range(def_cap.node);

                symbols.push(Symbol {
                    name,
                    symbol_type: SymbolType::Interface,
                    signature: None,
                    docstring: None,
                    line_start,
                    line_end,
                    parent_id: None,
                    file_path: file_path.to_path_buf(),
                    is_exported: is_exported(def_cap.node),
                });
            }
        }

        Ok(symbols)
    }

    fn extract_type_aliases(
        &self,
        tree: &tree_sitter::Tree,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let query = Query::new(
            &language,
            r#"
            (type_alias_declaration
                name: (type_identifier) @type.name) @type.def
            "#,
        )
        .context("Failed to create type alias query")?;

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
                    == Some("type.name")
            });

            let def_capture = captures.iter().find(|c| {
                query
                    .capture_names()
                    .get(c.index as usize)
                    .map(|s| s.as_ref())
                    == Some("type.def")
            });

            if let (Some(name_cap), Some(def_cap)) = (name_capture, def_capture) {
                let name = name_cap
                    .node
                    .utf8_text(source.as_bytes())
                    .context("Failed to extract type alias name")?
                    .to_string();

                let (line_start, line_end) = self.get_line_range(def_cap.node);

                symbols.push(Symbol {
                    name,
                    symbol_type: SymbolType::TypeAlias,
                    signature: None,
                    docstring: None,
                    line_start,
                    line_end,
                    parent_id: None,
                    file_path: file_path.to_path_buf(),
                    is_exported: is_exported(def_cap.node),
                });
            }
        }

        Ok(symbols)
    }

    fn extract_enums(
        &self,
        tree: &tree_sitter::Tree,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let query = Query::new(
            &language,
            r#"
            (enum_declaration
                name: (identifier) @enum.name) @enum.def
            "#,
        )
        .context("Failed to create enum query")?;

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
                    == Some("enum.name")
            });

            let def_capture = captures.iter().find(|c| {
                query
                    .capture_names()
                    .get(c.index as usize)
                    .map(|s| s.as_ref())
                    == Some("enum.def")
            });

            if let (Some(name_cap), Some(def_cap)) = (name_capture, def_capture) {
                let name = name_cap
                    .node
                    .utf8_text(source.as_bytes())
                    .context("Failed to extract enum name")?
                    .to_string();

                let (line_start, line_end) = self.get_line_range(def_cap.node);

                symbols.push(Symbol {
                    name,
                    symbol_type: SymbolType::Enum,
                    signature: None,
                    docstring: None,
                    line_start,
                    line_end,
                    parent_id: None,
                    file_path: file_path.to_path_buf(),
                    is_exported: is_exported(def_cap.node),
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
        let language = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
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
        s.trim().trim_matches('\'').trim_matches('"').to_string()
    }
}

impl Parser for TypeScriptParser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult> {
        let mut parser = Self::create_parser()?;
        let tree = parser
            .parse(content, None)
            .context("Failed to parse TypeScript content")?;

        let classes = self.extract_classes(&tree, content, file_path)?;
        let functions = self.extract_functions(&tree, content, file_path)?;
        let methods = self.extract_methods(&tree, content, file_path, &classes)?;
        let interfaces = self.extract_interfaces(&tree, content, file_path)?;
        let type_aliases = self.extract_type_aliases(&tree, content, file_path)?;
        let enums = self.extract_enums(&tree, content, file_path)?;
        let dependencies = self.extract_dependencies(&tree, content)?;

        let total_capacity = classes.len()
            + functions.len()
            + methods.len()
            + interfaces.len()
            + type_aliases.len()
            + enums.len();
        let mut symbols = Vec::with_capacity(total_capacity);
        symbols.extend(classes);
        symbols.extend(functions);
        symbols.extend(methods);
        symbols.extend(interfaces);
        symbols.extend(type_aliases);
        symbols.extend(enums);

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
        let parser = TypeScriptParser::new()?;
        let content = "function greet(name: string): string { return 'Hello ' + name; }";
        let path = Path::new("test.ts");

        let result = parser.parse(content, path)?;

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "greet" && s.symbol_type == SymbolType::Function));
        Ok(())
    }

    #[test]
    fn test_parse_arrow_function() -> Result<()> {
        let parser = TypeScriptParser::new()?;
        let content = "const add = (a: number, b: number): number => a + b;";
        let path = Path::new("test.ts");

        let result = parser.parse(content, path)?;

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "add" && s.symbol_type == SymbolType::Function));
        Ok(())
    }

    #[test]
    fn test_parse_class_declaration() -> Result<()> {
        let parser = TypeScriptParser::new()?;
        let content = "class MyClass { constructor() {} }";
        let path = Path::new("test.ts");

        let result = parser.parse(content, path)?;

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "MyClass" && s.symbol_type == SymbolType::Class));
        Ok(())
    }

    #[test]
    fn test_parse_interface() -> Result<()> {
        let parser = TypeScriptParser::new()?;
        let content = r#"
interface User {
    id: number;
    name: string;
    email?: string;
}
        "#;
        let path = Path::new("test.ts");

        let result = parser.parse(content, path)?;

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "User" && s.symbol_type == SymbolType::Interface));
        Ok(())
    }

    #[test]
    fn test_parse_type_alias() -> Result<()> {
        let parser = TypeScriptParser::new()?;
        let content = r#"
type UserId = string | number;
type UserRecord = { id: UserId; name: string };
        "#;
        let path = Path::new("test.ts");

        let result = parser.parse(content, path)?;

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "UserId" && s.symbol_type == SymbolType::TypeAlias));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "UserRecord" && s.symbol_type == SymbolType::TypeAlias));
        Ok(())
    }

    #[test]
    fn test_parse_enum() -> Result<()> {
        let parser = TypeScriptParser::new()?;
        let content = r#"
enum Status {
    Active = "active",
    Inactive = "inactive",
    Pending = "pending"
}
        "#;
        let path = Path::new("test.ts");

        let result = parser.parse(content, path)?;

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "Status" && s.symbol_type == SymbolType::Enum));
        Ok(())
    }

    #[test]
    fn test_parse_es6_import() -> Result<()> {
        let parser = TypeScriptParser::new()?;
        let content = "import { Component } from 'react';";
        let path = Path::new("test.ts");

        let result = parser.parse(content, path)?;

        assert_eq!(result.dependencies.len(), 1);
        assert_eq!(result.dependencies[0].import_name, "react");
        Ok(())
    }

    #[test]
    fn test_parse_method() -> Result<()> {
        let parser = TypeScriptParser::new()?;
        let content = r#"
class Calculator {
    add(a: number, b: number): number {
        return a + b;
    }
}
        "#;
        let path = Path::new("test.ts");

        let result = parser.parse(content, path)?;

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "add" && s.symbol_type == SymbolType::Method));
        Ok(())
    }

    #[test]
    fn test_parse_complete_typescript_file() -> Result<()> {
        let parser = TypeScriptParser::new()?;
        let content = r#"
import { BaseService } from './base';

interface Config {
    apiUrl: string;
    timeout: number;
}

type ConfigKey = keyof Config;

enum LogLevel {
    Debug,
    Info,
    Error
}

class ApiService {
    private config: Config;
    
    constructor(config: Config) {
        this.config = config;
    }
    
    async fetch(endpoint: string): Promise<Response> {
        return fetch(this.config.apiUrl + endpoint);
    }
}

function createService(config: Config): ApiService {
    return new ApiService(config);
}
        "#;
        let path = Path::new("test.ts");

        let result = parser.parse(content, path)?;

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "Config" && s.symbol_type == SymbolType::Interface));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "ConfigKey" && s.symbol_type == SymbolType::TypeAlias));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "LogLevel" && s.symbol_type == SymbolType::Enum));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "ApiService" && s.symbol_type == SymbolType::Class));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "createService" && s.symbol_type == SymbolType::Function));
        assert!(result
            .dependencies
            .iter()
            .any(|d| d.import_name == "./base"));
        Ok(())
    }

    #[test]
    fn test_parse_describe_block() -> Result<()> {
        let parser = TypeScriptParser::new()?;
        let content = r#"
describe('User Authentication', () => {
    console.log('test suite');
});
        "#;
        let path = Path::new("test.ts");

        let result = parser.parse(content, path)?;

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "describe:User Authentication"
                && s.symbol_type == SymbolType::Function));
        Ok(())
    }

    #[test]
    fn test_parse_it_block() -> Result<()> {
        let parser = TypeScriptParser::new()?;
        let content = r#"
it('should validate email', () => {
    expect(true).toBe(true);
});
        "#;
        let path = Path::new("test.ts");

        let result = parser.parse(content, path)?;

        assert!(result.symbols.iter().any(
            |s| s.name == "test:should validate email" && s.symbol_type == SymbolType::Function
        ));
        Ok(())
    }

    #[test]
    fn test_parse_test_block() -> Result<()> {
        let parser = TypeScriptParser::new()?;
        let content = r#"
test('handles empty input', () => {
    expect(parse('')).toBeNull();
});
        "#;
        let path = Path::new("test.ts");

        let result = parser.parse(content, path)?;

        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.name == "test:handles empty input"
                    && s.symbol_type == SymbolType::Function)
        );
        Ok(())
    }
}

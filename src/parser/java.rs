use super::{Parser as ParserTrait, ParseResult};
use crate::models::{Dependency, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

pub struct JavaParser;

impl JavaParser {
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

    fn extract_javadoc(&self, node: Node, source: &str) -> Option<String> {
        if let Some(parent) = node.parent() {
            let mut cursor = parent.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "block_comment" {
                        if let Some(text) = self.extract_text(child, source) {
                            if text.starts_with("/**") {
                                return Some(text);
                            }
                        }
                    }
                    if child.id() == node.id() {
                        break;
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        None
    }

    fn extract_parameters(&self, params_node: Node, source: &str) -> Option<String> {
        self.extract_text(params_node, source)
    }

    fn find_parent_class(&self, node: Node, symbols: &[Symbol]) -> Option<usize> {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "class_declaration" || parent.kind() == "interface_declaration" {
                let parent_line = parent.start_position().row + 1;
                for (idx, symbol) in symbols.iter().enumerate() {
                    if symbol.symbol_type == SymbolType::Class
                        && symbol.line_start == parent_line
                    {
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
            if parent.kind() == "class_declaration" || parent.kind() == "interface_declaration" {
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

        let language = tree_sitter_java::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (class_declaration
                name: (identifier) @class.name) @class.def
            "#,
        )
        .context("Failed to create Java class query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut class_name = None;
            let mut class_node = None;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("class.name") => {
                        class_name = capture.node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("class.def") => {
                        class_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (class_name, class_node) {
                let docstring = self.extract_javadoc(node, source);
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
                });
            }
        }

        Ok(symbols)
    }

    fn process_interfaces(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();

        let language = tree_sitter_java::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (interface_declaration
                name: (identifier) @interface.name) @interface.def
            "#,
        )
        .context("Failed to create Java interface query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut interface_name = None;
            let mut interface_node = None;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("interface.name") => {
                        interface_name = capture.node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("interface.def") => {
                        interface_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (interface_name, interface_node) {
                let docstring = self.extract_javadoc(node, source);
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
                });
            }
        }

        Ok(symbols)
    }

    fn process_methods(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
        symbols: &mut Vec<Symbol>,
    ) -> Result<()> {
        let language = tree_sitter_java::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (method_declaration
                name: (identifier) @method.name
                parameters: (formal_parameters) @method.params) @method.def
            "#,
        )
        .context("Failed to create Java method query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut method_name = None;
            let mut method_params = None;
            let mut method_node = None;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("method.name") => {
                        method_name = capture.node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("method.params") => {
                        method_params = Some(capture.node);
                    }
                    Some("method.def") => {
                        method_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (method_name, method_node) {
                let signature = method_params.and_then(|p| self.extract_parameters(p, source));
                let docstring = self.extract_javadoc(node, source);
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
                });
            }
        }

        Ok(())
    }

    fn process_enums(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();

        let language = tree_sitter_java::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (enum_declaration
                name: (identifier) @enum.name) @enum.def
            "#,
        )
        .context("Failed to create Java enum query")?;

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
                        enum_name = capture.node
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
                let docstring = self.extract_javadoc(node, source);
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

        let language = tree_sitter_java::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (field_declaration
                (modifiers) @field.mods
                type: (_) @field.type
                declarator: (variable_declarator
                    name: (identifier) @field.name)) @field.def
            "#,
        )
        .context("Failed to create Java static field query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut field_name = None;
            let mut field_type = None;
            let mut field_mods = None;
            let mut field_node = None;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("field.name") => {
                        field_name = capture.node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("field.type") => {
                        field_type = self.extract_text(capture.node, source);
                    }
                    Some("field.mods") => {
                        field_mods = self.extract_text(capture.node, source);
                    }
                    Some("field.def") => {
                        field_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (field_name, field_node) {
                if let Some(mods) = &field_mods {
                    if mods.contains("static") {
                        let docstring = self.extract_javadoc(node, source);
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
                        });
                    }
                }
            }
        }

        Ok(symbols)
    }

    fn process_constructors(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
        symbols: &mut Vec<Symbol>,
    ) -> Result<()> {
        let language = tree_sitter_java::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (constructor_declaration
                name: (identifier) @constructor.name
                parameters: (formal_parameters) @constructor.params) @constructor.def
            "#,
        )
        .context("Failed to create Java constructor query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            let mut constructor_name = None;
            let mut constructor_params = None;
            let mut constructor_node = None;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("constructor.name") => {
                        constructor_name = capture.node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("constructor.params") => {
                        constructor_params = Some(capture.node);
                    }
                    Some("constructor.def") => {
                        constructor_node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (constructor_name, constructor_node) {
                let signature = constructor_params.and_then(|p| self.extract_parameters(p, source));
                let docstring = self.extract_javadoc(node, source);
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;
                let parent_id = self.find_parent_class(node, symbols);

                symbols.push(Symbol {
                    name,
                    symbol_type: SymbolType::Method,
                    signature,
                    docstring,
                    line_start,
                    line_end,
                    parent_id,
                    file_path: file_path.to_path_buf(),
                });
            }
        }

        Ok(())
    }

    fn process_imports(&self, tree_root: Node, source: &str) -> Result<Vec<Dependency>> {
        let mut imports = Vec::new();

        let language = tree_sitter_java::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (import_declaration
                (scoped_identifier) @import.path) @import.decl
            "#,
        )
        .context("Failed to create Java import query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree_root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let captures = match_.captures;

            for capture in captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                if capture_name == Some("import.path") {
                    if let Some(import_text) = self.extract_text(capture.node, source) {
                        let cleaned = import_text.trim().to_string();

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

impl ParserTrait for JavaParser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult> {
        let mut parser = Parser::new();
        let language = tree_sitter_java::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to set Java language")?;

        let tree = parser
            .parse(content, None)
            .context("Failed to parse Java file")?;

        let root = tree.root_node();
        let mut result = ParseResult::new();

        let classes = self.process_classes(root, content, file_path)?;
        result.symbols.extend(classes);

        let interfaces = self.process_interfaces(root, content, file_path)?;
        result.symbols.extend(interfaces);

        let enums = self.process_enums(root, content, file_path)?;
        result.symbols.extend(enums);

        let static_fields = self.process_static_fields(root, content, file_path)?;
        result.symbols.extend(static_fields);

        self.process_methods(root, content, file_path, &mut result.symbols)?;
        self.process_constructors(root, content, file_path, &mut result.symbols)?;

        result.dependencies = self.process_imports(root, content)?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_class() -> Result<()> {
        let parser = JavaParser::new()?;
        let source = r#"
public class HelloWorld {
    public void sayHello() {
        System.out.println("Hello, World!");
    }
}
"#;
        let result = parser.parse(source, Path::new("test.java"))?;
        assert!(result.symbols.len() >= 2);
        Ok(())
    }

    #[test]
    fn test_parse_interface() -> Result<()> {
        let parser = JavaParser::new()?;
        let source = r#"
public interface MyInterface {
    void doSomething();
}
"#;
        let result = parser.parse(source, Path::new("test.java"))?;
        assert!(result.symbols.len() >= 1);
        Ok(())
    }

    #[test]
    fn test_parse_imports() -> Result<()> {
        let parser = JavaParser::new()?;
        let source = r#"
import java.util.List;
import java.util.ArrayList;

public class Test {
}
"#;
        let result = parser.parse(source, Path::new("test.java"))?;
        assert!(result.dependencies.len() >= 2);
        Ok(())
    }
}

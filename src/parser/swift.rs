use super::{ParseResult, Parser as ParserTrait};
use crate::models::{Dependency, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

pub struct SwiftParser;

impl SwiftParser {
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

    fn get_line_range(&self, node: Node) -> (usize, usize) {
        let start = node.start_position();
        let end = node.end_position();
        (start.row + 1, end.row + 1)
    }

    fn visibility_is_publicish(&self, node: Node, source: &str) -> bool {
        let mut cursor = node.walk();
        if !cursor.goto_first_child() {
            return false;
        }

        loop {
            let child = cursor.node();
            if child.kind() == "modifiers" {
                if let Some(text) = self.extract_text(child, source) {
                    let t = text.as_str();
                    return t.contains("public") || t.contains("open") || t.contains("package");
                }
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }

        false
    }

    fn type_symbols<'a>(&self, symbols: &'a [Symbol]) -> Vec<(usize, &'a Symbol)> {
        symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                matches!(
                    s.symbol_type,
                    SymbolType::Class | SymbolType::Interface | SymbolType::Enum
                )
            })
            .collect()
    }

    fn find_parent_type(&self, line: usize, symbols: &[Symbol]) -> Option<usize> {
        self.type_symbols(symbols)
            .into_iter()
            .find(|(_, s)| line > s.line_start && line <= s.line_end)
            .map(|(idx, _)| idx)
    }

    fn extract_types(&self, root: Node, source: &str, file_path: &Path) -> Result<Vec<Symbol>> {
        let language = tree_sitter_swift::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (class_declaration
              declaration_kind: ["actor" "class" "struct" "enum" "extension"] @type.kind
              name: (type_identifier) @type.name) @type.def

            (protocol_declaration
              declaration_kind: "protocol" @proto.kind
              name: (type_identifier) @proto.name) @proto.def
            "#,
        )
        .context("Failed to create Swift type query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let mut symbols = Vec::new();

        while let Some(match_) = matches.next() {
            let mut name: Option<String> = None;
            let mut node: Option<Node> = None;
            let mut kind: Option<SymbolType> = None;
            let mut decl_kind: Option<String> = None;

            for cap in match_.captures {
                let cap_name = query
                    .capture_names()
                    .get(cap.index as usize)
                    .map(|s| s.as_ref());

                match cap_name {
                    Some("type.name") | Some("proto.name") => {
                        name = cap
                            .node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("type.kind") | Some("proto.kind") => {
                        decl_kind = cap
                            .node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("type.def") => {
                        node = Some(cap.node);
                        kind = Some(SymbolType::Class);
                    }
                    Some("proto.def") => {
                        node = Some(cap.node);
                        kind = Some(SymbolType::Interface);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node), Some(symbol_type)) = (name, node, kind) {
                let (line_start, line_end) = self.get_line_range(node);
                let is_exported = self.visibility_is_publicish(node, source);

                let signature = decl_kind.as_ref().map(|k| format!("{}", k));

                symbols.push(Symbol {
                    name,
                    symbol_type,
                    signature,
                    docstring: None,
                    line_start,
                    line_end,
                    parent_id: None,
                    file_path: file_path.to_path_buf(),
                    is_exported,
                });
            }
        }

        Ok(symbols)
    }

    fn extract_functions_and_methods(
        &self,
        root: Node,
        source: &str,
        file_path: &Path,
        type_symbols: &[Symbol],
    ) -> Result<Vec<Symbol>> {
        let language = tree_sitter_swift::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            [
              (function_declaration name: (simple_identifier) @fn.name) @fn.def
              (init_declaration "init" @init.name) @init.def
              (deinit_declaration "deinit" @deinit.name) @deinit.def
            ]
            "#,
        )
        .context("Failed to create Swift function query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let mut symbols = Vec::new();

        while let Some(match_) = matches.next() {
            let mut name: Option<String> = None;
            let mut def: Option<Node> = None;
            let mut kind: Option<&str> = None;

            for cap in match_.captures {
                let cap_name = query
                    .capture_names()
                    .get(cap.index as usize)
                    .map(|s| s.as_ref());

                match cap_name {
                    Some("fn.name") => {
                        name = cap
                            .node
                            .utf8_text(source.as_bytes())
                            .ok()
                            .map(|s| s.to_string());
                    }
                    Some("fn.def") => {
                        def = Some(cap.node);
                        kind = Some("fn");
                    }
                    Some("init.def") => {
                        def = Some(cap.node);
                        kind = Some("init");
                    }
                    Some("deinit.def") => {
                        def = Some(cap.node);
                        kind = Some("deinit");
                    }
                    _ => {}
                }
            }

            let Some(def_node) = def else { continue };

            let (line_start, line_end) = self.get_line_range(def_node);
            let parent_id = self.find_parent_type(line_start, type_symbols);
            let is_method = parent_id.is_some();

            let symbol_type = if is_method {
                SymbolType::Method
            } else {
                SymbolType::Function
            };

            let final_name = match kind {
                Some("init") => "init".to_string(),
                Some("deinit") => "deinit".to_string(),
                _ => name.unwrap_or_else(|| "anonymous".to_string()),
            };

            let signature = if matches!(kind, Some("fn")) {
                self.extract_swift_signature(def_node, source)
            } else {
                None
            };

            symbols.push(Symbol {
                name: final_name,
                symbol_type,
                signature,
                docstring: None,
                line_start,
                line_end,
                parent_id,
                file_path: file_path.to_path_buf(),
                is_exported: self.visibility_is_publicish(def_node, source),
            });
        }

        Ok(symbols)
    }

    fn extract_swift_signature(&self, fn_node: Node, source: &str) -> Option<String> {
        // best-effort: Swift's grammar doesn't expose a stable "parameter_clause" node.
        // Grab the full function_declaration text, then slice from first "(" to matching ")",
        // and include a trailing return type if present.
        let text = fn_node.utf8_text(source.as_bytes()).ok()?;

        let open = text.find('(')?;
        let mut depth: i32 = 0;
        let mut close: Option<usize> = None;

        for (i, ch) in text.char_indices().skip(open) {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }

        let close = close?;
        let params = text.get(open..=close)?.trim().to_string();

        // return type: "-> ..." right after the params; stop at "{" or newline.
        let after = text.get(close + 1..).unwrap_or("");
        let after_trim = after.trim_start();
        if let Some(arrow_idx) = after_trim.find("->") {
            let ret_part = after_trim.get(arrow_idx..).unwrap_or("");
            let end = ret_part
                .find('{')
                .or_else(|| ret_part.find('\n'))
                .unwrap_or(ret_part.len());
            let ret = ret_part[..end].trim();
            if !ret.is_empty() {
                return Some(format!("{} {}", params, ret));
            }
        }

        Some(params)
    }

    fn extract_imports(&self, root: Node, source: &str) -> Result<Vec<Dependency>> {
        let language = tree_sitter_swift::LANGUAGE.into();
        let query = Query::new(
            &language,
            r#"
            (import_declaration (identifier (simple_identifier) @import.name))
            "#,
        )
        .context("Failed to create Swift import query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let mut deps = Vec::new();

        while let Some(match_) = matches.next() {
            for cap in match_.captures {
                let cap_name = query
                    .capture_names()
                    .get(cap.index as usize)
                    .map(|s| s.as_ref());

                if cap_name == Some("import.name") {
                    let text = cap
                        .node
                        .utf8_text(source.as_bytes())
                        .unwrap_or_default()
                        .trim()
                        .to_string();

                    if !text.is_empty() {
                        deps.push(Dependency {
                            import_name: text,
                            from_file: None,
                        });
                    }
                }
            }
        }

        Ok(deps)
    }
}

impl ParserTrait for SwiftParser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult> {
        let mut parser = Parser::new();
        let language = tree_sitter_swift::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to set Swift language")?;

        let tree = parser
            .parse(content, None)
            .context("Failed to parse Swift file")?;

        let root = tree.root_node();

        let mut result = ParseResult::new();

        let types = self.extract_types(root, content, file_path)?;
        result.symbols.extend(types);

        // snapshot current type symbols for parent resolution
        let type_symbols = result.symbols.clone();
        let fns = self.extract_functions_and_methods(root, content, file_path, &type_symbols)?;
        result.symbols.extend(fns);

        result.dependencies = self.extract_imports(root, content)?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_swift_types_and_functions() -> Result<()> {
        let parser = SwiftParser::new()?;
        let source = r#"
import Foundation

public struct User {
    public let name: String

    public init(name: String) {
        self.name = name
    }

    public func greet() -> String {
        return "hi \\(name)"
    }
}

func topLevel(x: Int) -> Int {
    return x + 1
}
"#;

        let result = parser.parse(source, Path::new("test.swift"))?;

        assert!(result
            .dependencies
            .iter()
            .any(|d| d.import_name == "Foundation"));

        assert!(result.symbols.iter().any(|s| s.name == "User"
            && s.symbol_type == SymbolType::Class
            && s.signature.as_deref() == Some("struct")));

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "greet" && s.symbol_type == SymbolType::Method));

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "topLevel" && s.symbol_type == SymbolType::Function));

        Ok(())
    }
}

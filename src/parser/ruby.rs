use super::{ParseResult, Parser as ParserTrait};
use crate::models::{Dependency, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

pub struct RubyParser;

impl RubyParser {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn language() -> tree_sitter::Language {
        tree_sitter_ruby::LANGUAGE.into()
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

    fn extract_signature_line(&self, node: Node, source: &str) -> Option<String> {
        source
            .lines()
            .nth(node.start_position().row)
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
    }

    fn clean_comment_line(line: &str) -> String {
        line.trim()
            .strip_prefix('#')
            .unwrap_or(line.trim())
            .trim()
            .to_string()
    }

    fn extract_comment_block(&self, node: Node, source: &str) -> Option<String> {
        let mut comments = Vec::new();
        let mut current = node;

        while let Some(prev) = current.prev_sibling() {
            if prev.kind() != "comment" {
                break;
            }

            if prev.end_position().row + 1 != current.start_position().row {
                break;
            }

            if let Some(text) = self.extract_text(prev, source) {
                comments.push(Self::clean_comment_line(&text));
            }
            current = prev;
        }

        if comments.is_empty() {
            None
        } else {
            comments.reverse();
            Some(comments.join("\n"))
        }
    }

    fn find_parent_scope(&self, node: Node, symbols: &[Symbol]) -> Option<usize> {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if matches!(parent.kind(), "class" | "module") {
                let parent_line = parent.start_position().row + 1;
                return symbols.iter().enumerate().find_map(|(idx, symbol)| {
                    (matches!(symbol.symbol_type, SymbolType::Class | SymbolType::Module)
                        && symbol.line_start == parent_line)
                        .then_some(idx)
                });
            }
            current = parent;
        }
        None
    }

    fn find_attribute_parent_scope(&self, node: Node, symbols: &[Symbol]) -> Option<usize> {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if matches!(parent.kind(), "method" | "singleton_method") {
                return None;
            }
            if matches!(parent.kind(), "class" | "module") {
                let parent_line = parent.start_position().row + 1;
                return symbols.iter().enumerate().find_map(|(idx, symbol)| {
                    (matches!(symbol.symbol_type, SymbolType::Class | SymbolType::Module)
                        && symbol.line_start == parent_line)
                        .then_some(idx)
                });
            }
            current = parent;
        }
        None
    }

    fn process_scopes(&self, root: Node, source: &str, file_path: &Path) -> Result<Vec<Symbol>> {
        let language = Self::language();
        let query = Query::new(
            &language,
            r#"
            (class name: (_) @scope.name) @class.def
            (module name: (_) @scope.name) @module.def
            "#,
        )
        .context("Failed to create Ruby scope query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let mut symbols = Vec::new();

        while let Some(match_) = matches.next() {
            let mut name = None;
            let mut node = None;
            let mut symbol_type = None;

            for capture in match_.captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("scope.name") => {
                        name = self.extract_text(capture.node, source);
                    }
                    Some("class.def") => {
                        node = Some(capture.node);
                        symbol_type = Some(SymbolType::Class);
                    }
                    Some("module.def") => {
                        node = Some(capture.node);
                        symbol_type = Some(SymbolType::Module);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node), Some(symbol_type)) = (name, node, symbol_type) {
                let (line_start, line_end) = self.get_line_range(node);
                let parent_id = self.find_parent_scope(node, &symbols);

                symbols.push(Symbol {
                    name,
                    symbol_type,
                    signature: self.extract_signature_line(node, source),
                    docstring: self.extract_comment_block(node, source),
                    line_start,
                    line_end,
                    parent_id,
                    file_path: file_path.to_path_buf(),
                    is_exported: true,
                });
            }
        }

        Ok(symbols)
    }

    fn process_methods(
        &self,
        root: Node,
        source: &str,
        file_path: &Path,
        symbols: &mut Vec<Symbol>,
    ) -> Result<()> {
        let language = Self::language();
        let query = Query::new(
            &language,
            r#"
            (method name: (_) @method.name) @method.def
            (singleton_method name: (_) @singleton.name) @singleton.def
            "#,
        )
        .context("Failed to create Ruby method query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let mut name = None;
            let mut node = None;
            let mut is_singleton = false;

            for capture in match_.captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("method.name") | Some("singleton.name") => {
                        name = self.extract_text(capture.node, source);
                    }
                    Some("method.def") => {
                        node = Some(capture.node);
                    }
                    Some("singleton.def") => {
                        node = Some(capture.node);
                        is_singleton = true;
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (name, node) {
                let (line_start, line_end) = self.get_line_range(node);
                let parent_id = self.find_parent_scope(node, symbols);
                let symbol_type = if parent_id.is_some() || is_singleton {
                    SymbolType::Method
                } else {
                    SymbolType::Function
                };

                symbols.push(Symbol {
                    name: name.clone(),
                    symbol_type,
                    signature: self.extract_signature_line(node, source),
                    docstring: self.extract_comment_block(node, source),
                    line_start,
                    line_end,
                    parent_id,
                    file_path: file_path.to_path_buf(),
                    is_exported: !name.starts_with('_'),
                });
            }
        }

        Ok(())
    }

    fn process_constants(
        &self,
        root: Node,
        source: &str,
        file_path: &Path,
        symbols: &[Symbol],
    ) -> Result<Vec<Symbol>> {
        let language = Self::language();
        let query = Query::new(
            &language,
            r#"
            [
              (assignment left: (constant) @constant.name)
              (assignment left: (scope_resolution name: (_) @constant.name))
            ] @constant.def
            "#,
        )
        .context("Failed to create Ruby constant query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let mut constants = Vec::new();

        while let Some(match_) = matches.next() {
            let mut name = None;
            let mut node = None;

            for capture in match_.captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("constant.name") => {
                        name = self.extract_text(capture.node, source);
                    }
                    Some("constant.def") => {
                        node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            if let (Some(name), Some(node)) = (name, node) {
                let (line_start, line_end) = self.get_line_range(node);
                let parent_id = self.find_parent_scope(node, symbols);

                constants.push(Symbol {
                    name,
                    symbol_type: SymbolType::StaticField,
                    signature: self.extract_signature_line(node, source),
                    docstring: self.extract_comment_block(node, source),
                    line_start,
                    line_end,
                    parent_id,
                    file_path: file_path.to_path_buf(),
                    is_exported: true,
                });
            }
        }

        Ok(constants)
    }

    fn strip_literal(text: &str) -> String {
        let trimmed = text.trim();
        if trimmed.len() >= 3 && trimmed.starts_with(":\"") && trimmed.ends_with('"') {
            return trimmed[2..trimmed.len() - 1].to_string();
        }
        if trimmed.len() >= 3 && trimmed.starts_with(":'") && trimmed.ends_with('\'') {
            return trimmed[2..trimmed.len() - 1].to_string();
        }
        if trimmed.len() >= 2 {
            let first = trimmed.as_bytes()[0] as char;
            let last = trimmed.as_bytes()[trimmed.len() - 1] as char;
            if matches!((first, last), ('\'', '\'') | ('"', '"')) {
                return trimmed[1..trimmed.len() - 1].to_string();
            }
        }
        trimmed.strip_prefix(':').unwrap_or(trimmed).to_string()
    }

    fn symbol_name_from_literal(&self, node: Node, source: &str) -> Option<String> {
        match node.kind() {
            "simple_symbol" | "bare_symbol" | "delimited_symbol" | "hash_key_symbol" | "string"
            | "bare_string" => self
                .extract_text(node, source)
                .map(|text| Self::strip_literal(&text))
                .map(|text| {
                    text.trim_end_matches(':')
                        .trim_start_matches('@')
                        .to_string()
                })
                .filter(|text| !text.is_empty()),
            _ => None,
        }
    }

    fn symbol_names_from_node(&self, node: Node, source: &str) -> Vec<String> {
        if matches!(node.kind(), "array" | "symbol_array" | "string_array") {
            let mut cursor = node.walk();
            return node
                .children(&mut cursor)
                .flat_map(|child| self.symbol_names_from_node(child, source))
                .collect();
        }

        if node.kind() == "hash" {
            let mut cursor = node.walk();
            return node
                .children(&mut cursor)
                .filter_map(|child| self.pair_parts(child, source).map(|(key, _)| key))
                .collect();
        }

        self.symbol_name_from_literal(node, source)
            .into_iter()
            .collect()
    }

    fn pair_parts<'a>(&self, node: Node<'a>, source: &str) -> Option<(String, Node<'a>)> {
        if node.kind() != "pair" {
            return None;
        }

        let key = node
            .child_by_field_name("key")
            .and_then(|key| self.symbol_name_from_literal(key, source))?;
        let value = node.child_by_field_name("value")?;
        Some((key, value))
    }

    fn argument_list_node<'a>(&self, node: Node<'a>) -> Option<Node<'a>> {
        if let Some(arguments) = node.child_by_field_name("arguments") {
            return Some(arguments);
        }

        let mut cursor = node.walk();
        let arguments = node
            .children(&mut cursor)
            .find(|child| child.kind() == "argument_list");
        arguments
    }

    fn generated_method_symbol(
        &self,
        name: String,
        node: Node,
        source: &str,
        file_path: &Path,
        parent_id: usize,
    ) -> Symbol {
        let (line_start, line_end) = self.get_line_range(node);

        Symbol {
            name,
            symbol_type: SymbolType::Method,
            signature: self.extract_signature_line(node, source),
            docstring: None,
            line_start,
            line_end,
            parent_id: Some(parent_id),
            file_path: file_path.to_path_buf(),
            is_exported: true,
        }
    }

    fn generated_attribute_symbols(
        &self,
        method: &str,
        attr_name: &str,
        node: Node,
        source: &str,
        file_path: &Path,
        parent_id: usize,
    ) -> Vec<Symbol> {
        match method {
            "attr_reader" => vec![self.generated_method_symbol(
                attr_name.to_string(),
                node,
                source,
                file_path,
                parent_id,
            )],
            "attr_writer" => vec![self.generated_method_symbol(
                format!("{}=", attr_name),
                node,
                source,
                file_path,
                parent_id,
            )],
            "attr_accessor" => vec![
                self.generated_method_symbol(
                    attr_name.to_string(),
                    node,
                    source,
                    file_path,
                    parent_id,
                ),
                self.generated_method_symbol(
                    format!("{}=", attr_name),
                    node,
                    source,
                    file_path,
                    parent_id,
                ),
            ],
            _ => Vec::new(),
        }
    }

    fn process_attribute_accessors(
        &self,
        root: Node,
        source: &str,
        file_path: &Path,
        symbols: &[Symbol],
    ) -> Result<Vec<Symbol>> {
        let language = Self::language();
        let query = Query::new(
            &language,
            r#"
            (call
              method: (identifier) @attr.method
              arguments: (argument_list (_) @attr.arg)) @attr.def
            "#,
        )
        .context("Failed to create Ruby attribute accessor query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let mut generated_symbols = Vec::new();

        while let Some(match_) = matches.next() {
            let mut method = None;
            let mut node = None;
            let mut attrs = Vec::new();

            for capture in match_.captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("attr.method") => {
                        method = self.extract_text(capture.node, source);
                    }
                    Some("attr.arg") => {
                        if let Some(name) = self.symbol_name_from_literal(capture.node, source) {
                            attrs.push(name);
                        }
                    }
                    Some("attr.def") => {
                        node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            let (Some(method), Some(node)) = (method, node) else {
                continue;
            };
            if !matches!(
                method.as_str(),
                "attr_reader" | "attr_writer" | "attr_accessor"
            ) {
                continue;
            }

            let Some(parent_id) = self.find_attribute_parent_scope(node, symbols) else {
                continue;
            };

            attrs
                .iter()
                .flat_map(|attr| {
                    self.generated_attribute_symbols(
                        &method, attr, node, source, file_path, parent_id,
                    )
                })
                .for_each(|symbol| generated_symbols.push(symbol));
        }

        Ok(generated_symbols)
    }

    fn delegate_prefix_name(
        &self,
        method_name: &str,
        target: &Option<String>,
        prefix: &Option<String>,
    ) -> String {
        let Some(prefix) = prefix else {
            return method_name.to_string();
        };

        let prefix = if prefix == "true" {
            target.as_deref().unwrap_or_default()
        } else {
            prefix.as_str()
        };

        if prefix.is_empty() || prefix == "false" || prefix == "nil" {
            method_name.to_string()
        } else {
            format!("{}_{}", prefix.trim_start_matches('@'), method_name)
        }
    }

    fn process_delegates(
        &self,
        root: Node,
        source: &str,
        file_path: &Path,
        symbols: &[Symbol],
    ) -> Result<Vec<Symbol>> {
        let language = Self::language();
        let query = Query::new(
            &language,
            r#"
            (call
              method: (identifier) @delegate.method) @delegate.def
            "#,
        )
        .context("Failed to create Ruby delegate query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let mut generated_symbols = Vec::new();

        while let Some(match_) = matches.next() {
            let mut method = None;
            let mut node = None;

            for capture in match_.captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("delegate.method") => {
                        method = self.extract_text(capture.node, source);
                    }
                    Some("delegate.def") => {
                        node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            let (Some(method), Some(node)) = (method, node) else {
                continue;
            };
            if method != "delegate" {
                continue;
            }

            let Some(parent_id) = self.find_attribute_parent_scope(node, symbols) else {
                continue;
            };
            let Some(arguments) = self.argument_list_node(node) else {
                continue;
            };

            let mut delegated_methods = Vec::new();
            let mut target = None;
            let mut prefix = None;
            let mut arg_cursor = arguments.walk();

            for arg in arguments.children(&mut arg_cursor) {
                if let Some((key, value)) = self.pair_parts(arg, source) {
                    match key.as_str() {
                        "to" => {
                            target = self.symbol_name_from_literal(value, source);
                        }
                        "prefix" => {
                            prefix = self
                                .symbol_name_from_literal(value, source)
                                .or_else(|| self.extract_text(value, source));
                        }
                        _ => {}
                    }
                } else {
                    delegated_methods.extend(self.symbol_names_from_node(arg, source));
                }
            }

            delegated_methods
                .iter()
                .map(|name| self.delegate_prefix_name(name, &target, &prefix))
                .map(|name| self.generated_method_symbol(name, node, source, file_path, parent_id))
                .for_each(|symbol| generated_symbols.push(symbol));
        }

        Ok(generated_symbols)
    }

    fn call_argument_parts<'a>(
        &self,
        arguments: Node<'a>,
        source: &str,
    ) -> (Vec<Node<'a>>, Vec<(String, Node<'a>)>) {
        let mut positional = Vec::new();
        let mut pairs = Vec::new();
        let mut cursor = arguments.walk();

        for arg in arguments.children(&mut cursor) {
            if let Some((key, value)) = self.pair_parts(arg, source) {
                pairs.push((key, value));
            } else if !matches!(arg.kind(), "," | "(" | ")") {
                positional.push(arg);
            }
        }

        (positional, pairs)
    }

    fn is_static_generated_name(name: &str) -> bool {
        !name.trim().is_empty() && !name.contains("#{")
    }

    fn singular_association_name(name: &str) -> String {
        match name {
            "people" => return "person".to_string(),
            "children" => return "child".to_string(),
            "men" => return "man".to_string(),
            "women" => return "woman".to_string(),
            "mice" => return "mouse".to_string(),
            _ => {}
        }

        if name.len() > 3 && name.ends_with("ies") {
            return format!("{}y", &name[..name.len() - 3]);
        }

        for suffix in ["ches", "shes", "sses", "xes", "zes"] {
            if name.len() > suffix.len() && name.ends_with(suffix) {
                return name[..name.len() - 2].to_string();
            }
        }

        if name.len() > 1 && name.ends_with('s') && !name.ends_with("ss") {
            return name[..name.len() - 1].to_string();
        }

        name.to_string()
    }

    fn generated_association_methods(&self, method: &str, association_name: &str) -> Vec<String> {
        if !Self::is_static_generated_name(association_name) {
            return Vec::new();
        }

        match method {
            "belongs_to" | "has_one" => vec![
                association_name.to_string(),
                format!("{}=", association_name),
                format!("build_{}", association_name),
                format!("create_{}", association_name),
                format!("create_{}!", association_name),
                format!("reload_{}", association_name),
                format!("reset_{}", association_name),
            ],
            "has_many" | "has_and_belongs_to_many" => {
                let singular = Self::singular_association_name(association_name);
                vec![
                    association_name.to_string(),
                    format!("{}=", association_name),
                    format!("{}_ids", singular),
                    format!("{}_ids=", singular),
                ]
            }
            _ => Vec::new(),
        }
    }

    fn generated_scope_methods(&self, scope_name: &str) -> Vec<String> {
        if Self::is_static_generated_name(scope_name) {
            vec![scope_name.to_string()]
        } else {
            Vec::new()
        }
    }

    fn generated_attribute_methods(&self, attr_name: &str) -> Vec<String> {
        if Self::is_static_generated_name(attr_name) {
            vec![attr_name.to_string(), format!("{}=", attr_name)]
        } else {
            Vec::new()
        }
    }

    fn generated_alias_attribute_methods(&self, alias_name: &str) -> Vec<String> {
        if Self::is_static_generated_name(alias_name) {
            vec![
                alias_name.to_string(),
                format!("{}=", alias_name),
                format!("{}?", alias_name),
            ]
        } else {
            Vec::new()
        }
    }

    fn generated_nested_attribute_methods(&self, association_name: &str) -> Vec<String> {
        if Self::is_static_generated_name(association_name) {
            vec![format!("{}_attributes=", association_name)]
        } else {
            Vec::new()
        }
    }

    fn generated_attached_methods(&self, method: &str, name: &str) -> Vec<String> {
        if !Self::is_static_generated_name(name) {
            return Vec::new();
        }

        match method {
            "has_one_attached" => vec![
                name.to_string(),
                format!("{}=", name),
                format!("{}_attachment", name),
                format!("{}_blob", name),
                format!("with_attached_{}", name),
            ],
            "has_many_attached" => vec![
                name.to_string(),
                format!("{}=", name),
                format!("{}_attachments", name),
                format!("{}_blobs", name),
                format!("with_attached_{}", name),
            ],
            _ => Vec::new(),
        }
    }

    fn generated_rich_text_methods(&self, name: &str) -> Vec<String> {
        if Self::is_static_generated_name(name) {
            vec![
                name.to_string(),
                format!("{}=", name),
                format!("rich_text_{}", name),
                format!("rich_text_{}=", name),
                format!("with_rich_text_{}", name),
                format!("with_rich_text_{}_and_embeds", name),
            ]
        } else {
            Vec::new()
        }
    }

    fn generated_secure_password_methods(&self, attr_name: Option<String>) -> Vec<String> {
        let name = attr_name.unwrap_or_else(|| "password".to_string());
        if !Self::is_static_generated_name(&name) {
            return Vec::new();
        }

        let mut methods = vec![
            name.clone(),
            format!("{}=", name),
            format!("{}_confirmation", name),
            format!("{}_confirmation=", name),
            format!("{}_challenge", name),
            format!("{}_challenge=", name),
            format!("authenticate_{}", name),
        ];

        if name == "password" {
            methods.push("authenticate".to_string());
        }

        methods
    }

    fn generated_composed_of_methods(&self, name: &str) -> Vec<String> {
        self.generated_attribute_methods(name)
    }

    fn store_affix_value(
        &self,
        pairs: &[(String, Node)],
        key: &str,
        store_name: &str,
        source: &str,
    ) -> Option<String> {
        pairs
            .iter()
            .find(|(candidate, _)| candidate == key)
            .and_then(|(_, node)| self.option_value_text(*node, source))
            .and_then(|value| match value.as_str() {
                "true" => Some(store_name.to_string()),
                "false" | "nil" => None,
                _ => Some(value),
            })
            .filter(|value| Self::is_static_generated_name(value))
    }

    fn store_accessor_base(name: &str, prefix: &Option<String>, suffix: &Option<String>) -> String {
        [prefix.as_deref(), Some(name), suffix.as_deref()]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("_")
    }

    fn generated_store_accessor_methods(
        &self,
        store_name: &str,
        accessor_names: Vec<String>,
        pairs: &[(String, Node)],
        source: &str,
    ) -> Vec<String> {
        if !Self::is_static_generated_name(store_name) {
            return Vec::new();
        }

        let prefix = self.store_affix_value(pairs, "prefix", store_name, source);
        let suffix = self.store_affix_value(pairs, "suffix", store_name, source);

        accessor_names
            .iter()
            .filter(|name| Self::is_static_generated_name(name))
            .flat_map(|name| {
                let base = Self::store_accessor_base(name, &prefix, &suffix);
                vec![base.clone(), format!("{}=", base)]
            })
            .collect()
    }

    fn generated_store_methods(
        &self,
        positional: &[Node],
        pairs: &[(String, Node)],
        source: &str,
    ) -> Vec<String> {
        let Some(store_name) = positional
            .first()
            .and_then(|node| self.symbol_name_from_literal(*node, source))
        else {
            return Vec::new();
        };

        let accessor_names = if positional.len() > 1 {
            positional
                .iter()
                .skip(1)
                .flat_map(|node| self.symbol_names_from_node(*node, source))
                .collect()
        } else {
            pairs
                .iter()
                .filter(|(key, _)| key == "accessors")
                .flat_map(|(_, value)| self.symbol_names_from_node(*value, source))
                .collect()
        };

        self.generated_store_accessor_methods(&store_name, accessor_names, pairs, source)
    }

    fn underscore_type_name(name: &str) -> String {
        let tail = name.rsplit("::").next().unwrap_or(name);
        tail.chars()
            .enumerate()
            .fold(String::new(), |mut acc, (idx, ch)| {
                if ch.is_uppercase() {
                    if idx > 0 {
                        acc.push('_');
                    }
                    ch.to_lowercase().for_each(|c| acc.push(c));
                } else {
                    acc.push(ch);
                }
                acc
            })
    }

    fn generated_delegated_type_methods(
        &self,
        role_name: &str,
        pairs: &[(String, Node)],
        source: &str,
    ) -> Vec<String> {
        if !Self::is_static_generated_name(role_name) {
            return Vec::new();
        }

        let mut methods = self.generated_association_methods("belongs_to", role_name);
        methods.push(format!("{}_id", role_name));
        methods.push(format!("{}_id=", role_name));
        methods.push(format!("{}_type", role_name));
        methods.push(format!("{}_type=", role_name));

        pairs
            .iter()
            .filter(|(key, _)| key == "types")
            .flat_map(|(_, value)| self.symbol_names_from_node(*value, source))
            .map(|name| Self::underscore_type_name(&name))
            .filter(|name| Self::is_static_generated_name(name))
            .for_each(|name| {
                methods.push(name.clone());
                methods.push(format!("{}?", name));
                methods.push(format!("{}_id", name));
            });

        methods
    }

    fn is_enum_option_key(key: &str) -> bool {
        matches!(
            key,
            "prefix"
                | "_prefix"
                | "suffix"
                | "_suffix"
                | "scopes"
                | "_scopes"
                | "instance_methods"
                | "default"
                | "_default"
                | "validate"
        )
    }

    fn option_value_text(&self, node: Node, source: &str) -> Option<String> {
        self.symbol_name_from_literal(node, source)
            .or_else(|| {
                self.extract_text(node, source)
                    .map(|text| Self::strip_literal(&text))
            })
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
    }

    fn option_is_false(&self, node: Node, source: &str) -> bool {
        self.option_value_text(node, source)
            .map(|text| matches!(text.as_str(), "false" | "nil"))
            .unwrap_or(false)
    }

    fn enum_affix_value(
        &self,
        pairs: &[(String, Node)],
        keys: &[&str],
        enum_name: &str,
        source: &str,
    ) -> Option<String> {
        pairs
            .iter()
            .find(|(key, _)| keys.iter().any(|candidate| *candidate == key))
            .and_then(|(_, node)| self.option_value_text(*node, source))
            .and_then(|value| match value.as_str() {
                "true" => Some(enum_name.to_string()),
                "false" | "nil" => None,
                _ => Some(value),
            })
            .filter(|value| Self::is_static_generated_name(value))
    }

    fn enum_option_disabled(&self, pairs: &[(String, Node)], keys: &[&str], source: &str) -> bool {
        pairs
            .iter()
            .find(|(key, _)| keys.iter().any(|candidate| *candidate == key))
            .map(|(_, node)| self.option_is_false(*node, source))
            .unwrap_or(false)
    }

    fn enum_method_base(value: &str, prefix: &Option<String>, suffix: &Option<String>) -> String {
        [prefix.as_deref(), Some(value), suffix.as_deref()]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("_")
    }

    fn generated_enum_methods_for(
        &self,
        enum_name: &str,
        values: Vec<String>,
        pairs: &[(String, Node)],
        source: &str,
    ) -> Vec<String> {
        if !Self::is_static_generated_name(enum_name) {
            return Vec::new();
        }

        let prefix = self.enum_affix_value(pairs, &["prefix", "_prefix"], enum_name, source);
        let suffix = self.enum_affix_value(pairs, &["suffix", "_suffix"], enum_name, source);
        let scopes_enabled = !self.enum_option_disabled(pairs, &["scopes", "_scopes"], source);
        let instance_methods_enabled =
            !self.enum_option_disabled(pairs, &["instance_methods"], source);

        values
            .iter()
            .filter(|value| Self::is_static_generated_name(value))
            .flat_map(|value| {
                let base = Self::enum_method_base(value, &prefix, &suffix);
                let mut methods = Vec::new();

                if scopes_enabled {
                    methods.push(base.clone());
                    methods.push(format!("not_{}", base));
                }

                if instance_methods_enabled {
                    methods.push(format!("{}?", base));
                    methods.push(format!("{}!", base));
                }

                methods
            })
            .collect()
    }

    fn generated_enum_methods(
        &self,
        positional: &[Node],
        pairs: &[(String, Node)],
        source: &str,
    ) -> Vec<String> {
        if let Some(enum_name) = positional
            .first()
            .and_then(|node| self.symbol_name_from_literal(*node, source))
        {
            let mut values = positional
                .get(1)
                .map(|node| self.symbol_names_from_node(*node, source))
                .unwrap_or_default();

            if values.is_empty() {
                values = pairs
                    .iter()
                    .filter(|(key, _)| !Self::is_enum_option_key(key))
                    .map(|(key, _)| key.clone())
                    .collect();
            }

            return self.generated_enum_methods_for(&enum_name, values, pairs, source);
        }

        pairs
            .iter()
            .filter(|(key, _)| !Self::is_enum_option_key(key))
            .flat_map(|(enum_name, value)| {
                let values = self.symbol_names_from_node(*value, source);
                self.generated_enum_methods_for(enum_name, values, pairs, source)
            })
            .collect()
    }

    fn process_rails_generated_methods(
        &self,
        root: Node,
        source: &str,
        file_path: &Path,
        symbols: &[Symbol],
    ) -> Result<Vec<Symbol>> {
        let language = Self::language();
        let query = Query::new(
            &language,
            r#"
            (call
              method: (identifier) @dsl.method) @dsl.def
            (identifier) @dsl.bare
            "#,
        )
        .context("Failed to create Ruby Rails DSL query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let mut generated_symbols = Vec::new();

        while let Some(match_) = matches.next() {
            let mut method = None;
            let mut node = None;

            for capture in match_.captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("dsl.method") => {
                        method = self.extract_text(capture.node, source);
                    }
                    Some("dsl.def") => {
                        node = Some(capture.node);
                    }
                    Some("dsl.bare") if capture.node.parent().map(|p| p.kind()) != Some("call") => {
                        method = self.extract_text(capture.node, source);
                        node = Some(capture.node);
                    }
                    _ => {}
                }
            }

            let (Some(method), Some(node)) = (method, node) else {
                continue;
            };
            let Some(parent_id) = self.find_attribute_parent_scope(node, symbols) else {
                continue;
            };

            let (positional, pairs) = if let Some(arguments) = self.argument_list_node(node) {
                self.call_argument_parts(arguments, source)
            } else if method == "has_secure_password" {
                (Vec::new(), Vec::new())
            } else {
                continue;
            };
            let method_names = match method.as_str() {
                "belongs_to" | "has_one" | "has_many" | "has_and_belongs_to_many" => positional
                    .first()
                    .and_then(|arg| self.symbol_name_from_literal(*arg, source))
                    .map(|name| self.generated_association_methods(&method, &name))
                    .unwrap_or_default(),
                "attribute" => positional
                    .first()
                    .and_then(|arg| self.symbol_name_from_literal(*arg, source))
                    .map(|name| self.generated_attribute_methods(&name))
                    .unwrap_or_default(),
                "alias_attribute" => positional
                    .first()
                    .and_then(|arg| self.symbol_name_from_literal(*arg, source))
                    .map(|name| self.generated_alias_attribute_methods(&name))
                    .unwrap_or_default(),
                "accepts_nested_attributes_for" => positional
                    .iter()
                    .flat_map(|arg| self.symbol_names_from_node(*arg, source))
                    .flat_map(|name| self.generated_nested_attribute_methods(&name))
                    .collect(),
                "has_one_attached" | "has_many_attached" => positional
                    .first()
                    .and_then(|arg| self.symbol_name_from_literal(*arg, source))
                    .map(|name| self.generated_attached_methods(&method, &name))
                    .unwrap_or_default(),
                "has_rich_text" => positional
                    .first()
                    .and_then(|arg| self.symbol_name_from_literal(*arg, source))
                    .map(|name| self.generated_rich_text_methods(&name))
                    .unwrap_or_default(),
                "has_secure_password" => self.generated_secure_password_methods(
                    positional
                        .first()
                        .and_then(|arg| self.symbol_name_from_literal(*arg, source)),
                ),
                "composed_of" => positional
                    .first()
                    .and_then(|arg| self.symbol_name_from_literal(*arg, source))
                    .map(|name| self.generated_composed_of_methods(&name))
                    .unwrap_or_default(),
                "store" | "store_accessor" => {
                    self.generated_store_methods(&positional, &pairs, source)
                }
                "delegated_type" => positional
                    .first()
                    .and_then(|arg| self.symbol_name_from_literal(*arg, source))
                    .map(|name| self.generated_delegated_type_methods(&name, &pairs, source))
                    .unwrap_or_default(),
                "scope" => positional
                    .first()
                    .and_then(|arg| self.symbol_name_from_literal(*arg, source))
                    .map(|name| self.generated_scope_methods(&name))
                    .unwrap_or_default(),
                "enum" => self.generated_enum_methods(&positional, &pairs, source),
                _ => Vec::new(),
            };

            method_names
                .into_iter()
                .map(|name| self.generated_method_symbol(name, node, source, file_path, parent_id))
                .for_each(|symbol| generated_symbols.push(symbol));
        }

        Ok(generated_symbols)
    }

    fn process_requires(&self, root: Node, source: &str) -> Result<Vec<Dependency>> {
        let language = Self::language();
        let query = Query::new(
            &language,
            r#"
            (call
              method: (identifier) @call.method
              arguments: (argument_list (_) @call.arg)) @call.def
            "#,
        )
        .context("Failed to create Ruby require query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let mut dependencies = Vec::new();

        while let Some(match_) = matches.next() {
            let mut method = None;
            let mut arg = None;

            for capture in match_.captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("call.method") => {
                        method = self.extract_text(capture.node, source);
                    }
                    Some("call.arg") if arg.is_none() => {
                        arg = self.extract_text(capture.node, source);
                    }
                    _ => {}
                }
            }

            let Some(method) = method else { continue };
            if !matches!(method.as_str(), "require" | "require_relative" | "load") {
                continue;
            }

            if let Some(import_name) = arg.map(|text| Self::strip_literal(&text)) {
                if !import_name.is_empty() {
                    dependencies.push(Dependency {
                        import_name,
                        from_file: None,
                    });
                }
            }
        }

        Ok(dependencies)
    }
}

impl ParserTrait for RubyParser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult> {
        let mut parser = Parser::new();
        let language = RubyParser::language();
        parser
            .set_language(&language)
            .context("Failed to set Ruby language")?;

        let tree = parser
            .parse(content, None)
            .context("Failed to parse Ruby file")?;

        let root = tree.root_node();
        let mut result = ParseResult::new();

        let scopes = self.process_scopes(root, content, file_path)?;
        result.symbols.extend(scopes);

        let constants = self.process_constants(root, content, file_path, &result.symbols)?;
        result.symbols.extend(constants);

        self.process_methods(root, content, file_path, &mut result.symbols)?;

        let generated_methods =
            self.process_attribute_accessors(root, content, file_path, &result.symbols)?;
        let generated_methods: Vec<_> = generated_methods
            .into_iter()
            .filter(|generated| {
                !result.symbols.iter().any(|existing| {
                    existing.name == generated.name
                        && existing.parent_id == generated.parent_id
                        && existing.symbol_type == generated.symbol_type
                })
            })
            .collect();
        result.symbols.extend(generated_methods);

        let delegate_methods = self.process_delegates(root, content, file_path, &result.symbols)?;
        let delegate_methods: Vec<_> = delegate_methods
            .into_iter()
            .filter(|generated| {
                !result.symbols.iter().any(|existing| {
                    existing.name == generated.name
                        && existing.parent_id == generated.parent_id
                        && existing.symbol_type == generated.symbol_type
                })
            })
            .collect();
        result.symbols.extend(delegate_methods);

        let rails_generated_methods =
            self.process_rails_generated_methods(root, content, file_path, &result.symbols)?;
        let rails_generated_methods: Vec<_> = rails_generated_methods
            .into_iter()
            .filter(|generated| {
                !result.symbols.iter().any(|existing| {
                    existing.name == generated.name
                        && existing.parent_id == generated.parent_id
                        && existing.symbol_type == generated.symbol_type
                })
            })
            .collect();
        result.symbols.extend(rails_generated_methods);

        result.dependencies = self.process_requires(root, content)?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ruby_scopes_methods_constants_and_requires() -> Result<()> {
        let parser = RubyParser::new()?;
        let source = r#"
require "set"
require_relative "support/current_attributes"

module ActiveSupport
  class CurrentAttributes < Base
    ATTRS = [:user]
    attr_reader :account
    attr_accessor :user, :request_id
    delegate :email, :admin?, :"!~", to: :user
    delegate :name, to: :account, prefix: true
    delegate :read, :write, to: :file, prefix: :io

    def self.attribute(*names, default: NOT_SET, &block)
      names.each { |name| define_method(name) { @attributes[name] } }
    end

    def initialize(attributes = nil)
      @attributes = attributes
    end
  end
end
"#;

        let result = parser.parse(source, Path::new("current_attributes.rb"))?;

        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "ActiveSupport" && s.symbol_type == SymbolType::Module }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "CurrentAttributes" && s.symbol_type == SymbolType::Class }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "ATTRS" && s.symbol_type == SymbolType::StaticField }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "attribute" && s.symbol_type == SymbolType::Method }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "initialize" && s.symbol_type == SymbolType::Method }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "account" && s.symbol_type == SymbolType::Method }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "user" && s.symbol_type == SymbolType::Method }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "user=" && s.symbol_type == SymbolType::Method }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "request_id" && s.symbol_type == SymbolType::Method }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "request_id=" && s.symbol_type == SymbolType::Method }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "email" && s.symbol_type == SymbolType::Method }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "admin?" && s.symbol_type == SymbolType::Method }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "!~" && s.symbol_type == SymbolType::Method }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "account_name" && s.symbol_type == SymbolType::Method }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "io_read" && s.symbol_type == SymbolType::Method }));
        assert!(result
            .symbols
            .iter()
            .any(|s| { s.name == "io_write" && s.symbol_type == SymbolType::Method }));
        assert_eq!(result.dependencies.len(), 2);
        assert!(result
            .dependencies
            .iter()
            .any(|dep| dep.import_name == "set"));
        Ok(())
    }

    #[test]
    fn test_parse_top_level_ruby_method_as_function() -> Result<()> {
        let parser = RubyParser::new()?;
        let source = r#"
def boot_app(path)
  require path
end
"#;

        let result = parser.parse(source, Path::new("Rakefile"))?;
        let symbol = result
            .symbols
            .iter()
            .find(|s| s.name == "boot_app")
            .expect("boot_app should be indexed");

        assert_eq!(symbol.symbol_type, SymbolType::Function);
        assert_eq!(symbol.signature.as_deref(), Some("def boot_app(path)"));
        Ok(())
    }

    #[test]
    fn test_attr_reader_inside_method_is_not_synthesized() -> Result<()> {
        let parser = RubyParser::new()?;
        let source = r#"
class DynamicThing
  def configure(target)
    target.attr_reader :dynamic_value
  end
end
"#;

        let result = parser.parse(source, Path::new("dynamic_thing.rb"))?;
        assert!(!result.symbols.iter().any(|s| s.name == "dynamic_value"));
        Ok(())
    }

    #[test]
    fn test_parse_rails_dsl_generated_methods() -> Result<()> {
        let parser = RubyParser::new()?;
        let source = r#"
class Post < ApplicationRecord
  belongs_to :author
  has_one :cover_photo
  has_many :comments
  has_and_belongs_to_many :tags
  scope :published, -> { where(published: true) }
  enum :status, { draft: 0, published: 1 }, prefix: true
  enum role: [:admin, :member], _suffix: true
end
"#;

        let result = parser.parse(source, Path::new("post.rb"))?;
        let has_method = |name: &str| {
            result
                .symbols
                .iter()
                .any(|s| s.name == name && s.symbol_type == SymbolType::Method)
        };

        for name in [
            "author",
            "author=",
            "build_author",
            "create_author!",
            "reset_author",
            "cover_photo",
            "build_cover_photo",
            "comments",
            "comments=",
            "comment_ids",
            "comment_ids=",
            "tags",
            "tag_ids",
            "published",
            "status_draft",
            "not_status_draft",
            "status_draft?",
            "status_published!",
            "admin_role",
            "not_admin_role",
            "admin_role?",
            "member_role!",
        ] {
            assert!(has_method(name), "expected generated method {name}");
        }

        Ok(())
    }

    #[test]
    fn test_parse_rails_dogfood_gap_generated_methods() -> Result<()> {
        let parser = RubyParser::new()?;
        let source = r#"
class User < ApplicationRecord
  attribute :display_name
  alias_attribute :nickname, :display_name
  has_one_attached :avatar
  has_many_attached :photos
  has_rich_text :bio
  accepts_nested_attributes_for :photos, :account
  store :settings, accessors: %i[color homepage], prefix: true
  store_accessor :configs, :login_retry, suffix: :config
  has_secure_password
  has_secure_password :recovery_password, validations: false
  composed_of :address
  delegated_type :entryable, types: %w[ Message Comment ]
end
"#;

        let result = parser.parse(source, Path::new("user.rb"))?;
        let has_method = |name: &str| {
            result
                .symbols
                .iter()
                .any(|s| s.name == name && s.symbol_type == SymbolType::Method)
        };

        for name in [
            "display_name",
            "display_name=",
            "nickname",
            "nickname?",
            "avatar",
            "avatar=",
            "avatar_attachment",
            "avatar_blob",
            "with_attached_avatar",
            "photos",
            "photos=",
            "photos_attachments",
            "photos_blobs",
            "with_attached_photos",
            "bio",
            "bio=",
            "rich_text_bio",
            "with_rich_text_bio_and_embeds",
            "photos_attributes=",
            "account_attributes=",
            "settings_color",
            "settings_color=",
            "settings_homepage",
            "login_retry_config",
            "login_retry_config=",
            "password",
            "password=",
            "password_confirmation",
            "password_challenge=",
            "authenticate",
            "authenticate_password",
            "recovery_password",
            "authenticate_recovery_password",
            "address",
            "address=",
            "entryable",
            "entryable_type=",
            "message",
            "message?",
            "message_id",
            "comment",
            "comment?",
            "comment_id",
        ] {
            assert!(has_method(name), "expected generated method {name}");
        }

        Ok(())
    }

    #[test]
    fn test_dynamic_delegate_name_is_not_synthesized() -> Result<()> {
        let parser = RubyParser::new()?;
        let source = r#"
class DynamicThing
  delegate method_name, to: :target
end
"#;

        let result = parser.parse(source, Path::new("dynamic_thing.rb"))?;
        assert!(!result.symbols.iter().any(|s| s.name == "method_name"));
        Ok(())
    }
}

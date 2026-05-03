use anyhow::Result;
use regex::Regex;
use std::fs;
use std::path::PathBuf;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

use crate::index::CodeIndex;
use crate::models::Language;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImplementsKind {
    Implements,
    Extends,
    Impl,
    Inherits,
}

impl ImplementsKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImplementsKind::Implements => "implements",
            ImplementsKind::Extends => "extends",
            ImplementsKind::Impl => "impl",
            ImplementsKind::Inherits => "inherits",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Implementation {
    pub implementor_name: String,
    pub interface_name: String,
    pub file_path: PathBuf,
    pub line: usize,
    pub kind: ImplementsKind,
    pub language: Language,
}

pub fn find_implementations(
    index: &CodeIndex,
    interface: &str,
    fuzzy: bool,
    trait_only: bool,
) -> Result<Vec<Implementation>> {
    let mut results = Vec::new();
    let interface_lower = interface.to_lowercase();

    for file in index.files() {
        let content = fs::read_to_string(&file.path).unwrap_or_default();
        if content.is_empty() {
            continue;
        }

        let file_impls = match file.language {
            Language::Rust => {
                find_rust_implementations(&content, interface, fuzzy, &interface_lower)
            }
            Language::Python => {
                find_python_implementations(&content, interface, fuzzy, &interface_lower)
            }
            Language::TypeScript | Language::JavaScript => {
                find_ts_implementations(&content, interface, fuzzy, &interface_lower)
            }
            Language::Java => {
                find_java_implementations(&content, interface, fuzzy, &interface_lower)
            }
            Language::Go => find_go_implementations(&content, interface, fuzzy, &interface_lower),
            Language::Ruby => {
                if ruby_content_may_reference_interface(
                    &content,
                    interface,
                    fuzzy,
                    &interface_lower,
                ) {
                    find_ruby_implementations(&content, interface, fuzzy, &interface_lower)
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        };

        for (implementor, iface, line, kind) in file_impls {
            // For inherent impls (impl Type), implementor_name == interface_name
            // Filter these out if trait_only is true
            let is_inherent = implementor == iface;
            if trait_only && is_inherent {
                continue;
            }

            results.push(Implementation {
                implementor_name: implementor,
                interface_name: iface,
                file_path: file.path.clone(),
                line,
                kind,
                language: file.language,
            });
        }
    }

    results.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then_with(|| a.line.cmp(&b.line))
    });

    Ok(results)
}

fn matches_interface(name: &str, interface: &str, fuzzy: bool, interface_lower: &str) -> bool {
    if fuzzy {
        name.to_lowercase().contains(interface_lower)
    } else {
        name == interface
    }
}

fn find_rust_implementations(
    content: &str,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let mut results = Vec::new();

    // Pattern: impl Trait for Type
    let impl_for_re = Regex::new(r"impl\s+(?:<[^>]*>\s*)?(\w+)(?:<[^>]*>)?\s+for\s+(\w+)")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    // Pattern: impl Type (inherent impl - type implements its own methods)
    let impl_self_re = Regex::new(r"impl\s+(?:<[^>]*>\s*)?(\w+)(?:<[^>]*>)?\s*\{")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Check impl Trait for Type
        if let Some(caps) = impl_for_re.captures(trimmed) {
            let trait_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let type_name = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            // allow searching either by trait name ("Display") OR by implementor type name ("Parser")
            // since users may ask "what does Parser implement?".
            if matches_interface(trait_name, interface, fuzzy, interface_lower)
                || matches_interface(type_name, interface, fuzzy, interface_lower)
            {
                results.push((
                    type_name.to_string(),
                    trait_name.to_string(),
                    line_num + 1,
                    ImplementsKind::Impl,
                ));
            }
        }

        // Check impl Type (searching for types that match the interface name)
        if let Some(caps) = impl_self_re.captures(trimmed) {
            if !trimmed.contains(" for ") {
                let type_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
                if matches_interface(type_name, interface, fuzzy, interface_lower) {
                    results.push((
                        type_name.to_string(),
                        type_name.to_string(),
                        line_num + 1,
                        ImplementsKind::Impl,
                    ));
                }
            }
        }
    }

    results
}

fn find_python_implementations(
    content: &str,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let mut results = Vec::new();

    // Pattern: class ClassName(ParentClass, AnotherParent):
    let class_re = Regex::new(r"class\s+(\w+)\s*\(([^)]+)\)\s*:")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        if let Some(caps) = class_re.captures(trimmed) {
            let class_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let parents_str = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            // Split parents by comma, handling potential generics
            for parent in parents_str.split(',') {
                let parent = parent.trim();
                // Remove generic parameters like [T] or typing stuff
                let parent_name = parent
                    .split('[')
                    .next()
                    .unwrap_or(parent)
                    .split('.')
                    .last()
                    .unwrap_or(parent)
                    .trim();

                if !parent_name.is_empty()
                    && matches_interface(parent_name, interface, fuzzy, interface_lower)
                {
                    results.push((
                        class_name.to_string(),
                        parent_name.to_string(),
                        line_num + 1,
                        ImplementsKind::Inherits,
                    ));
                }
            }
        }
    }

    results
}

fn find_ts_implementations(
    content: &str,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let mut results = Vec::new();

    // Pattern: class ClassName implements Interface1, Interface2
    let implements_re = Regex::new(
        r"class\s+(\w+)(?:<[^>]*>)?(?:\s+extends\s+\w+(?:<[^>]*>)?)?\s+implements\s+([^{]+)",
    )
    .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    // Pattern: class ClassName extends ParentClass
    let extends_re = Regex::new(r"class\s+(\w+)(?:<[^>]*>)?\s+extends\s+(\w+)")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Check implements
        if let Some(caps) = implements_re.captures(trimmed) {
            let class_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let interfaces_str = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            for iface in interfaces_str.split(',') {
                let iface = iface.trim().split('<').next().unwrap_or("").trim();
                if !iface.is_empty() && matches_interface(iface, interface, fuzzy, interface_lower)
                {
                    results.push((
                        class_name.to_string(),
                        iface.to_string(),
                        line_num + 1,
                        ImplementsKind::Implements,
                    ));
                }
            }
        }

        // Check extends
        if let Some(caps) = extends_re.captures(trimmed) {
            let class_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let parent_name = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            if matches_interface(parent_name, interface, fuzzy, interface_lower) {
                results.push((
                    class_name.to_string(),
                    parent_name.to_string(),
                    line_num + 1,
                    ImplementsKind::Extends,
                ));
            }
        }
    }

    results
}

fn find_java_implementations(
    content: &str,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let mut results = Vec::new();

    // Pattern: class ClassName implements Interface1, Interface2
    let implements_re = Regex::new(
        r"class\s+(\w+)(?:<[^>]*>)?(?:\s+extends\s+\w+(?:<[^>]*>)?)?\s+implements\s+([^{]+)",
    )
    .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    // Pattern: class ClassName extends ParentClass
    let extends_re = Regex::new(r"class\s+(\w+)(?:<[^>]*>)?\s+extends\s+(\w+)")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    // Pattern: interface InterfaceName extends ParentInterface
    let interface_extends_re = Regex::new(r"interface\s+(\w+)(?:<[^>]*>)?\s+extends\s+([^{]+)")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Check class implements
        if let Some(caps) = implements_re.captures(trimmed) {
            let class_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let interfaces_str = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            for iface in interfaces_str.split(',') {
                let iface = iface.trim().split('<').next().unwrap_or("").trim();
                if !iface.is_empty() && matches_interface(iface, interface, fuzzy, interface_lower)
                {
                    results.push((
                        class_name.to_string(),
                        iface.to_string(),
                        line_num + 1,
                        ImplementsKind::Implements,
                    ));
                }
            }
        }

        // Check class extends
        if let Some(caps) = extends_re.captures(trimmed) {
            let class_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let parent_name = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            if matches_interface(parent_name, interface, fuzzy, interface_lower) {
                results.push((
                    class_name.to_string(),
                    parent_name.to_string(),
                    line_num + 1,
                    ImplementsKind::Extends,
                ));
            }
        }

        // Check interface extends
        if let Some(caps) = interface_extends_re.captures(trimmed) {
            let iface_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let parents_str = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            for parent in parents_str.split(',') {
                let parent = parent.trim().split('<').next().unwrap_or("").trim();
                if !parent.is_empty()
                    && matches_interface(parent, interface, fuzzy, interface_lower)
                {
                    results.push((
                        iface_name.to_string(),
                        parent.to_string(),
                        line_num + 1,
                        ImplementsKind::Extends,
                    ));
                }
            }
        }
    }

    results
}

fn find_go_implementations(
    content: &str,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let mut results = Vec::new();

    // Go uses implicit interface implementation
    // We look for:
    // 1. type TypeName struct that embeds the interface
    // 2. func (receiver TypeName) MethodName patterns that match interface methods

    // Pattern: type Name struct { embedded Interface }
    let struct_embed_re = Regex::new(r"type\s+(\w+)\s+struct\s*\{")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    // We'll track struct definitions and look for embedded interfaces
    let mut current_struct: Option<(String, usize)> = None;
    let mut brace_depth = 0;

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Track struct definitions
        if let Some(caps) = struct_embed_re.captures(trimmed) {
            let struct_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            current_struct = Some((struct_name.to_string(), line_num + 1));
            brace_depth = 1;
            continue;
        }

        // Track brace depth for struct scope
        if current_struct.is_some() {
            brace_depth += trimmed.matches('{').count() as i32;
            brace_depth -= trimmed.matches('}').count() as i32;

            if brace_depth <= 0 {
                current_struct = None;
                brace_depth = 0;
                continue;
            }

            // Look for embedded interface (just the interface name on its own line)
            let field = trimmed.split_whitespace().next().unwrap_or("");
            if matches_interface(field, interface, fuzzy, interface_lower) {
                if let Some((ref struct_name, struct_line)) = current_struct {
                    results.push((
                        struct_name.clone(),
                        field.to_string(),
                        struct_line,
                        ImplementsKind::Implements,
                    ));
                }
            }
        }
    }

    results
}

fn ruby_language() -> tree_sitter::Language {
    tree_sitter_ruby::LANGUAGE.into()
}

fn ruby_text(node: Node, source: &str) -> Option<String> {
    node.utf8_text(source.as_bytes())
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn ruby_name_tail(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

fn matches_ruby_name(name: &str, interface: &str, fuzzy: bool, interface_lower: &str) -> bool {
    let interface_tail = ruby_name_tail(interface);
    let interface_tail_lower = interface_tail.to_lowercase();

    matches_interface(name, interface, fuzzy, interface_lower)
        || matches_interface(ruby_name_tail(name), interface, fuzzy, interface_lower)
        || matches_interface(name, interface_tail, fuzzy, &interface_tail_lower)
        || matches_interface(
            ruby_name_tail(name),
            interface_tail,
            fuzzy,
            &interface_tail_lower,
        )
}

fn ruby_content_may_reference_interface(
    content: &str,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> bool {
    let interface_tail = ruby_name_tail(interface);

    if fuzzy {
        let content_lower = content.to_lowercase();
        let interface_tail_lower = interface_tail.to_lowercase();
        content_lower.contains(interface_lower) || content_lower.contains(&interface_tail_lower)
    } else {
        content.contains(interface) || content.contains(interface_tail)
    }
}

fn ruby_names_from_node(node: Node, source: &str) -> Vec<String> {
    if node.kind() == "array" {
        let mut cursor = node.walk();
        return node
            .children(&mut cursor)
            .flat_map(|child| ruby_names_from_node(child, source))
            .collect();
    }

    match node.kind() {
        "constant" | "scope_resolution" | "identifier" => ruby_text(node, source)
            .into_iter()
            .map(|name| name.trim_start_matches('@').to_string())
            .collect(),
        _ => Vec::new(),
    }
}

fn ruby_scope_name(node: Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|name| ruby_text(name, source))
}

fn ruby_qualified_scope_name(node: Node, source: &str) -> Option<String> {
    let mut names = Vec::new();
    let mut current = Some(node);

    while let Some(scope) = current {
        if matches!(scope.kind(), "class" | "module") {
            if let Some(name) = ruby_scope_name(scope, source) {
                names.push(name);
            }
        }
        current = scope.parent();
    }

    if names.is_empty() {
        None
    } else {
        names.reverse();
        Some(names.join("::"))
    }
}

fn ruby_parent_static_scope_name(node: Node, source: &str) -> Option<String> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if matches!(parent.kind(), "method" | "singleton_method") {
            return None;
        }
        if matches!(parent.kind(), "class" | "module") {
            return ruby_qualified_scope_name(parent, source);
        }
        current = parent;
    }
    None
}

fn ruby_mixin_kind(method: &str) -> Option<ImplementsKind> {
    match method {
        "include" | "prepend" => Some(ImplementsKind::Implements),
        "extend" => Some(ImplementsKind::Extends),
        _ => None,
    }
}

fn find_ruby_implementations(
    content: &str,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let mut parser = Parser::new();
    let language = ruby_language();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }

    let Some(tree) = parser.parse(content, None) else {
        return Vec::new();
    };

    let mut results = Vec::new();
    results.extend(find_ruby_superclasses(
        tree.root_node(),
        content,
        &language,
        interface,
        fuzzy,
        interface_lower,
    ));
    results.extend(find_ruby_mixins(
        tree.root_node(),
        content,
        &language,
        interface,
        fuzzy,
        interface_lower,
    ));
    results
}

fn find_ruby_superclasses(
    root: Node,
    source: &str,
    language: &tree_sitter::Language,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let query = match Query::new(
        language,
        r#"
        (class
          name: (_) @class.name
          superclass: (superclass (_) @class.super)) @class.def
        "#,
    ) {
        Ok(query) => query,
        Err(_) => return Vec::new(),
    };

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, root, source.as_bytes());
    let mut results = Vec::new();

    while let Some(match_) = matches.next() {
        let mut class_node = None;
        let mut super_name = None;

        for capture in match_.captures {
            let capture_name = query
                .capture_names()
                .get(capture.index as usize)
                .map(|s| s.as_ref());

            match capture_name {
                Some("class.def") => class_node = Some(capture.node),
                Some("class.super") => super_name = ruby_text(capture.node, source),
                _ => {}
            }
        }

        let (Some(class_node), Some(super_name)) = (class_node, super_name) else {
            continue;
        };
        if !matches_ruby_name(&super_name, interface, fuzzy, interface_lower) {
            continue;
        }

        if let Some(class_name) = ruby_qualified_scope_name(class_node, source) {
            results.push((
                class_name,
                super_name,
                class_node.start_position().row + 1,
                ImplementsKind::Inherits,
            ));
        }
    }

    results
}

fn find_ruby_mixins(
    root: Node,
    source: &str,
    language: &tree_sitter::Language,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let query = match Query::new(
        language,
        r#"
        (call
          method: (identifier) @mixin.method
          arguments: (argument_list (_) @mixin.arg)) @mixin.def
        "#,
    ) {
        Ok(query) => query,
        Err(_) => return Vec::new(),
    };

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, root, source.as_bytes());
    let mut results = Vec::new();

    while let Some(match_) = matches.next() {
        let mut method = None;
        let mut call_node = None;
        let mut modules = Vec::new();

        for capture in match_.captures {
            let capture_name = query
                .capture_names()
                .get(capture.index as usize)
                .map(|s| s.as_ref());

            match capture_name {
                Some("mixin.method") => method = ruby_text(capture.node, source),
                Some("mixin.arg") => modules.extend(ruby_names_from_node(capture.node, source)),
                Some("mixin.def") => call_node = Some(capture.node),
                _ => {}
            }
        }

        let (Some(_), Some(call_node), Some(kind)) = (
            method.as_deref(),
            call_node,
            method.as_deref().and_then(ruby_mixin_kind),
        ) else {
            continue;
        };
        let Some(scope_name) = ruby_parent_static_scope_name(call_node, source) else {
            continue;
        };

        modules
            .iter()
            .filter(|module| matches_ruby_name(module, interface, fuzzy, interface_lower))
            .for_each(|module| {
                results.push((
                    scope_name.clone(),
                    module.clone(),
                    call_node.start_position().row + 1,
                    kind.clone(),
                ));
            });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_impl_for() {
        let content = r#"
impl Display for MyType {
    fn fmt(&self, f: &mut Formatter) -> Result {
    }
}
"#;
        let results = find_rust_implementations(content, "Display", false, "display");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "MyType");
        assert_eq!(results[0].1, "Display");
    }

    #[test]
    fn test_python_inheritance() {
        let content = r#"
class MyService(BaseService):
    pass
"#;
        let results = find_python_implementations(content, "BaseService", false, "baseservice");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "MyService");
    }

    #[test]
    fn test_ts_implements() {
        let content = r#"
class UserRepository implements Repository {
    async find(id: string) {}
}
"#;
        let results = find_ts_implementations(content, "Repository", false, "repository");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "UserRepository");
    }

    #[test]
    fn test_java_implements() {
        let content = r#"
public class ArrayList implements List, Serializable {
}
"#;
        let results = find_java_implementations(content, "List", false, "list");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "ArrayList");
    }

    #[test]
    fn test_fuzzy_search() {
        let content = r#"
impl Iterator for MyIterator {}
impl IntoIterator for MyCollection {}
"#;
        let results = find_rust_implementations(content, "iter", true, "iter");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_inherent_impl_filtering() {
        let content = r#"
impl Parser {
    fn new() -> Self {}
}
impl Display for Parser {
    fn fmt(&self, f: &mut Formatter) -> Result {}
}
"#;
        let all_results = find_rust_implementations(content, "Parser", false, "parser");
        // Should find both: inherent impl and trait impl
        assert_eq!(all_results.len(), 2);

        // The inherent impl has implementor == interface_name
        let inherent = all_results.iter().find(|r| r.0 == r.1).unwrap();
        assert_eq!(inherent.0, "Parser");
        assert_eq!(inherent.1, "Parser");
    }

    #[test]
    fn test_ruby_mixins() {
        let content = r#"
module ActiveModel
  module Attributes
    include ActiveModel::AttributeRegistration
    extend ActiveSupport::Concern
  end
end
"#;

        let includes = find_ruby_implementations(
            content,
            "AttributeRegistration",
            false,
            "attributeregistration",
        );
        assert_eq!(includes.len(), 1);
        assert_eq!(includes[0].0, "ActiveModel::Attributes");
        assert_eq!(includes[0].1, "ActiveModel::AttributeRegistration");
        assert_eq!(includes[0].3, ImplementsKind::Implements);

        let extends = find_ruby_implementations(
            content,
            "ActiveSupport::Concern",
            false,
            "activesupport::concern",
        );
        assert_eq!(extends.len(), 1);
        assert_eq!(extends[0].0, "ActiveModel::Attributes");
        assert_eq!(extends[0].3, ImplementsKind::Extends);
    }

    #[test]
    fn test_ruby_inheritance() {
        let content = r#"
module ActiveModel
  class EachValidator < Validator
  end
end
"#;

        let results = find_ruby_implementations(content, "Validator", false, "validator");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "ActiveModel::EachValidator");
        assert_eq!(results[0].1, "Validator");
        assert_eq!(results[0].3, ImplementsKind::Inherits);

        let qualified_results = find_ruby_implementations(
            content,
            "ActiveModel::Validator",
            false,
            "activemodel::validator",
        );
        assert_eq!(qualified_results.len(), 1);
        assert_eq!(qualified_results[0].0, "ActiveModel::EachValidator");
    }

    #[test]
    fn test_ruby_include_inside_method_is_not_a_mixin() {
        let content = r#"
class Checker
  def valid?(list)
    list.include Thing
  end
end
"#;

        let results = find_ruby_implementations(content, "Thing", false, "thing");
        assert!(results.is_empty());
    }
}

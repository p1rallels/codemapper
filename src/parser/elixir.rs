use super::{ParseResult, Parser as ParserTrait};
use crate::models::{Dependency, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

pub struct ElixirParser;

fn is_module_definition(kind: &str) -> bool {
    matches!(kind, "defmodule" | "defprotocol" | "defimpl")
}

fn module_symbol_type(kind: &str) -> SymbolType {
    match kind {
        "defprotocol" => SymbolType::Interface,
        _ => SymbolType::Module,
    }
}

fn is_function_definition(kind: &str) -> bool {
    matches!(
        kind,
        "def"
            | "defp"
            | "defdelegate"
            | "defguard"
            | "defguardp"
            | "defmacro"
            | "defmacrop"
            | "defn"
            | "defnp"
    )
}

fn is_public_definition(kind: &str) -> bool {
    !matches!(kind, "defp" | "defguardp" | "defmacrop" | "defnp")
}

fn is_dependency_macro(kind: &str) -> bool {
    matches!(kind, "alias" | "import" | "require" | "use")
}

impl ElixirParser {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    fn language() -> tree_sitter::Language {
        tree_sitter_elixir::LANGUAGE.into()
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

    fn line_range(node: Node) -> (usize, usize) {
        (node.start_position().row + 1, node.end_position().row + 1)
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

    fn attribute_belongs_to_module(
        &self,
        attr_node: Node,
        module_node: Node,
        source: &str,
    ) -> bool {
        let mut current = attr_node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "call" {
                let target = self.call_target_text(parent, source).unwrap_or_default();
                if is_module_definition(&target) {
                    return parent.start_byte() == module_node.start_byte();
                }
            }
            current = parent;
        }
        false
    }

    fn extract_module_docstring(&self, node: Node, source: &str) -> Option<String> {
        let language = Self::language();
        let query = Query::new(
            &language,
            r#"
            (unary_operator
              operand: (call
                target: (identifier) @attr.name)) @attr.def
            "#,
        )
        .ok()?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, node, source.as_bytes());

        while let Some(match_) = matches.next() {
            let mut attr_name = None;
            let mut attr_node = None;

            for capture in match_.captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("attr.name") => attr_name = self.extract_text(capture.node, source),
                    Some("attr.def") => attr_node = Some(capture.node),
                    _ => {}
                }
            }

            if attr_name.as_deref() == Some("moduledoc") {
                if let Some(doc_node) = attr_node {
                    if self.attribute_belongs_to_module(doc_node, node, source) {
                        return self.extract_text(doc_node, source);
                    }
                }
            }
        }

        None
    }

    fn extract_docstring(&self, node: Node, source: &str) -> Option<String> {
        let mut docs = Vec::new();
        let mut current = node;

        while let Some(prev) = current.prev_named_sibling() {
            if prev.end_position().row + 1 != current.start_position().row {
                break;
            }

            match prev.kind() {
                "comment" => {
                    if let Some(text) = self.extract_text(prev, source) {
                        docs.push(Self::clean_comment_line(&text));
                    }
                    current = prev;
                }
                "unary_operator" => {
                    let text = self.extract_text(prev, source).unwrap_or_default();
                    let trimmed = text.trim();
                    if trimmed.starts_with("@doc") || trimmed.starts_with("@moduledoc") {
                        docs.push(trimmed.to_string());
                        current = prev;
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }

        if docs.is_empty() {
            None
        } else {
            docs.reverse();
            Some(docs.join("\n"))
        }
    }

    fn call_target_text(&self, node: Node, source: &str) -> Option<String> {
        node.child_by_field_name("target")
            .and_then(|target| self.extract_text(target, source))
    }

    fn find_parent_module(&self, node: Node, source: &str, symbols: &[Symbol]) -> Option<usize> {
        let mut current = node;
        while let Some(parent) = current.parent() {
            if parent.kind() == "call" {
                let target = self.call_target_text(parent, source).unwrap_or_default();
                if is_module_definition(&target) {
                    let parent_line = parent.start_position().row + 1;
                    return symbols.iter().enumerate().find_map(|(idx, symbol)| {
                        (matches!(
                            symbol.symbol_type,
                            SymbolType::Module | SymbolType::Interface
                        ) && symbol.line_start == parent_line)
                            .then_some(idx)
                    });
                }
            }
            current = parent;
        }
        None
    }

    fn process_modules(&self, root: Node, source: &str, file_path: &Path) -> Result<Vec<Symbol>> {
        let language = Self::language();
        let query = Query::new(
            &language,
            r#"
            (call
              target: (identifier) @module.kind
              (arguments (alias) @module.name)) @module.def
            "#,
        )
        .context("Failed to create Elixir module query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let mut symbols = Vec::new();

        while let Some(match_) = matches.next() {
            let mut kind = None;
            let mut name = None;
            let mut node = None;

            for capture in match_.captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("module.kind") => kind = self.extract_text(capture.node, source),
                    Some("module.name") => name = self.extract_text(capture.node, source),
                    Some("module.def") => node = Some(capture.node),
                    _ => {}
                }
            }

            let (Some(kind), Some(name), Some(node)) = (kind, name, node) else {
                continue;
            };
            if !is_module_definition(&kind) {
                continue;
            }

            let (line_start, line_end) = Self::line_range(node);
            symbols.push(Symbol {
                name,
                symbol_type: module_symbol_type(&kind),
                signature: self.extract_signature_line(node, source),
                docstring: self
                    .extract_module_docstring(node, source)
                    .or_else(|| self.extract_docstring(node, source)),
                line_start,
                line_end,
                parent_id: self.find_parent_module(node, source, &symbols),
                file_path: file_path.to_path_buf(),
                is_exported: true,
            });
        }

        Ok(symbols)
    }

    fn process_functions(
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
            (call
              target: (identifier) @function.kind
              (arguments
                [
                  (identifier) @function.name
                  (call target: (identifier) @function.name)
                  (binary_operator left: (identifier) @function.name)
                  (binary_operator left: (call target: (identifier) @function.name))
                ])) @function.def
            "#,
        )
        .context("Failed to create Elixir function query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let mut kind = None;
            let mut name = None;
            let mut node = None;

            for capture in match_.captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("function.kind") => kind = self.extract_text(capture.node, source),
                    Some("function.name") => name = self.extract_text(capture.node, source),
                    Some("function.def") => node = Some(capture.node),
                    _ => {}
                }
            }

            let (Some(kind), Some(name), Some(node)) = (kind, name, node) else {
                continue;
            };
            if !is_function_definition(&kind) {
                continue;
            }

            let (line_start, line_end) = Self::line_range(node);
            symbols.push(Symbol {
                name,
                symbol_type: SymbolType::Function,
                signature: self.extract_signature_line(node, source),
                docstring: self.extract_docstring(node, source),
                line_start,
                line_end,
                parent_id: self.find_parent_module(node, source, symbols),
                file_path: file_path.to_path_buf(),
                is_exported: is_public_definition(&kind),
            });
        }

        Ok(())
    }

    fn process_tests(
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
            (call
              target: (identifier) @test.kind
              (arguments (string) @test.name)) @test.def
            "#,
        )
        .context("Failed to create Elixir test query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());

        while let Some(match_) = matches.next() {
            let mut kind = None;
            let mut name = None;
            let mut node = None;

            for capture in match_.captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("test.kind") => kind = self.extract_text(capture.node, source),
                    Some("test.name") => name = self.extract_text(capture.node, source),
                    Some("test.def") => node = Some(capture.node),
                    _ => {}
                }
            }

            let (Some(kind), Some(name), Some(node)) = (kind, name, node) else {
                continue;
            };
            if kind != "test" {
                continue;
            }

            let (line_start, line_end) = Self::line_range(node);
            symbols.push(Symbol {
                name: format!("test {}", name.trim_matches('"')),
                symbol_type: SymbolType::Function,
                signature: self.extract_signature_line(node, source),
                docstring: self.extract_docstring(node, source),
                line_start,
                line_end,
                parent_id: self.find_parent_module(node, source, symbols),
                file_path: file_path.to_path_buf(),
                is_exported: false,
            });
        }

        Ok(())
    }

    fn extract_dependencies(&self, root: Node, source: &str) -> Result<Vec<Dependency>> {
        let language = Self::language();
        let query = Query::new(
            &language,
            r#"
            (call
              target: (identifier) @dep.kind
              (arguments
                [
                  (alias) @dep.name
                  (dot
                    left: (alias) @dep.prefix
                    right: (tuple (alias) @dep.group))
                ])) @dep.def
            "#,
        )
        .context("Failed to create Elixir dependency query")?;

        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, root, source.as_bytes());
        let mut dependencies = Vec::new();

        while let Some(match_) = matches.next() {
            let mut kind = None;
            let mut name = None;
            let mut prefix = None;
            let mut groups = Vec::new();

            for capture in match_.captures {
                let capture_name = query
                    .capture_names()
                    .get(capture.index as usize)
                    .map(|s| s.as_ref());

                match capture_name {
                    Some("dep.kind") => kind = self.extract_text(capture.node, source),
                    Some("dep.name") => name = self.extract_text(capture.node, source),
                    Some("dep.prefix") => prefix = self.extract_text(capture.node, source),
                    Some("dep.group") => {
                        if let Some(group) = self.extract_text(capture.node, source) {
                            groups.push(group);
                        }
                    }
                    _ => {}
                }
            }

            let Some(kind) = kind else {
                continue;
            };
            if !is_dependency_macro(&kind) {
                continue;
            }

            if let Some(name) = name {
                dependencies.push(Dependency {
                    import_name: name,
                    from_file: Some(kind.clone()),
                });
            }

            if let Some(prefix) = prefix {
                for group in groups {
                    dependencies.push(Dependency {
                        import_name: format!("{}.{}", prefix, group),
                        from_file: Some(kind.clone()),
                    });
                }
            }
        }

        Ok(dependencies)
    }
}

impl ParserTrait for ElixirParser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult> {
        let mut parser = Parser::new();
        let language = Self::language();
        parser
            .set_language(&language)
            .context("Failed to set Elixir language")?;

        let tree = parser
            .parse(content, None)
            .context("Failed to parse Elixir file")?;
        let root = tree.root_node();
        let mut result = ParseResult::new();

        let mut symbols = self.process_modules(root, content, file_path)?;
        self.process_functions(root, content, file_path, &mut symbols)?;
        self.process_tests(root, content, file_path, &mut symbols)?;
        result.symbols = symbols;
        result.dependencies = self.extract_dependencies(root, content)?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_modules_functions_and_dependencies() -> Result<()> {
        let parser = ElixirParser::new()?;
        let source = r#"
defmodule MyApp.User do
  @moduledoc "users"
  alias MyApp.Repo
  alias MyApp.{Account, Team}

  @doc "public"
  def public_name(user) do
    String.trim(user.name)
  end

  defp private_name(%{name: name}) when is_binary(name), do: name

  defmacro my_macro(arg) do
    quote do: unquote(arg)
  end
end
"#;

        let result = parser.parse(source, Path::new("user.ex"))?;
        let module = result
            .symbols
            .iter()
            .find(|symbol| symbol.name == "MyApp.User")
            .expect("module should be indexed");
        let public_function = result
            .symbols
            .iter()
            .find(|symbol| symbol.name == "public_name")
            .expect("public function should be indexed");
        let private_function = result
            .symbols
            .iter()
            .find(|symbol| symbol.name == "private_name")
            .expect("private function should be indexed");
        let macro_function = result
            .symbols
            .iter()
            .find(|symbol| symbol.name == "my_macro")
            .expect("macro should be indexed");

        assert_eq!(module.symbol_type, SymbolType::Module);
        assert_eq!(module.docstring.as_deref(), Some("@moduledoc \"users\""));
        assert_eq!(public_function.parent_id, Some(0));
        assert!(public_function.is_exported);
        assert_eq!(
            public_function.docstring.as_deref(),
            Some("@doc \"public\"")
        );
        assert!(!private_function.is_exported);
        assert!(macro_function.is_exported);
        let dependency_names: Vec<&str> = result
            .dependencies
            .iter()
            .map(|dep| dep.import_name.as_str())
            .collect();
        assert!(dependency_names.contains(&"MyApp.Repo"));
        assert!(dependency_names.contains(&"MyApp.Account"));
        assert!(dependency_names.contains(&"MyApp.Team"));
        Ok(())
    }

    #[test]
    fn test_parse_exunit_tests() -> Result<()> {
        let parser = ElixirParser::new()?;
        let source = r#"
defmodule MyApp.UserTest do
  use ExUnit.Case

  test "trims names" do
    assert trim_name(" a ") == "a"
  end
end
"#;
        let result = parser.parse(source, Path::new("user_test.exs"))?;

        assert!(result
            .symbols
            .iter()
            .any(|symbol| symbol.name == "test trims names"));
        assert!(result
            .dependencies
            .iter()
            .any(|dep| dep.import_name == "ExUnit.Case"));
        Ok(())
    }
}

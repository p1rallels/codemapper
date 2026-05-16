use super::{ParseResult, Parser as ParserTrait};
use crate::models::{Symbol, SymbolType};
use anyhow::{Context, Result};
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;
use tree_sitter::{Node, Parser};

pub struct MarkdownParser;

impl MarkdownParser {
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

    fn extract_header_text(&self, node: Node, source: &str) -> Option<String> {
        let mut cursor = node.walk();
        if !cursor.goto_first_child() {
            return None;
        }

        let mut parts = Vec::new();
        loop {
            let child = cursor.node();
            if child.kind() != "atx_h1_marker"
                && child.kind() != "atx_h2_marker"
                && child.kind() != "atx_h3_marker"
                && child.kind() != "atx_h4_marker"
                && child.kind() != "atx_h5_marker"
                && child.kind() != "atx_h6_marker"
            {
                if let Some(text) = self.extract_text(child, source) {
                    parts.push(text);
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }

        let result = parts.join("").trim().to_string();
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    fn get_header_level(&self, kind: &str) -> Option<usize> {
        match kind {
            "atx_h1_marker" => Some(1),
            "atx_h2_marker" => Some(2),
            "atx_h3_marker" => Some(3),
            "atx_h4_marker" => Some(4),
            "atx_h5_marker" => Some(5),
            "atx_h6_marker" => Some(6),
            _ => None,
        }
    }

    fn find_parent_header(&self, current_level: usize, symbols: &[Symbol]) -> Option<usize> {
        for (idx, symbol) in symbols.iter().enumerate().rev() {
            if symbol.symbol_type != SymbolType::Heading {
                continue;
            }

            let symbol_level = symbol
                .signature
                .as_deref()
                .and_then(|sig| sig.strip_prefix('h'))
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(1);

            if symbol_level < current_level {
                return Some(idx);
            }
        }
        None
    }

    fn build_line_start_bytes(&self, source: &str) -> Vec<usize> {
        let mut starts = vec![0usize];
        for (i, b) in source.as_bytes().iter().enumerate() {
            if *b == b'\n' {
                starts.push(i + 1);
            }
        }
        starts
    }

    fn line_for_byte(&self, line_starts: &[usize], byte_offset: usize) -> usize {
        match line_starts.binary_search(&byte_offset) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }

    fn nearest_heading_before_line(
        &self,
        headings: &[(usize, usize)],
        line: usize,
    ) -> Option<usize> {
        headings
            .iter()
            .take_while(|(l, _)| *l <= line)
            .map(|(_, idx)| *idx)
            .last()
    }

    fn extract_endpoints(
        &self,
        source: &str,
        file_path: &Path,
        headings: &[(usize, usize)],
        heading_symbols: &[Symbol],
    ) -> Result<Vec<Symbol>> {
        let line_starts = self.build_line_start_bytes(source);

        let mut endpoints = Vec::new();
        let mut seen = HashSet::<(usize, String)>::new();

        let patterns = [
            Regex::new(r"(?m)^[>\s]*?(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\s+(/[^\s`]+)")?,
            Regex::new(r"(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\s+(/[^`\s]+)")?,
            Regex::new(
                r"(?s)`(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\n[^`\n]*\n[^`\n]*\n(/[^`\n]+)`",
            )?,
        ];

        for re in patterns {
            for caps in re.captures_iter(source) {
                let method = caps.get(1).map(|m| m.as_str()).unwrap_or("");
                let mut path = caps.get(2).map(|m| m.as_str()).unwrap_or("").trim();
                if method.is_empty() || path.is_empty() {
                    continue;
                }

                while path.ends_with('*')
                    || path.ends_with(')')
                    || path.ends_with(',')
                    || path.ends_with('.')
                    || path.ends_with(';')
                {
                    path = &path[..path.len().saturating_sub(1)];
                    path = path.trim_end();
                }

                let full = format!("{} {}", method, path);
                let byte_offset = caps.get(0).map(|m| m.start()).unwrap_or(0);
                let line = self.line_for_byte(&line_starts, byte_offset);

                if !seen.insert((line, full.clone())) {
                    continue;
                }

                let parent_id = self.nearest_heading_before_line(headings, line);
                let name = match parent_id {
                    Some(pid) => {
                        let parent_name = heading_symbols
                            .get(pid)
                            .map(|s| s.name.as_str())
                            .unwrap_or("?");
                        format!("{} > {}", parent_name, full)
                    }
                    None => full,
                };

                endpoints.push(Symbol {
                    name,
                    symbol_type: SymbolType::Endpoint,
                    signature: Some(method.to_string()),
                    docstring: None,
                    line_start: line,
                    line_end: line,
                    parent_id,
                    file_path: file_path.to_path_buf(),
                    is_exported: false,
                });
            }
        }

        Ok(endpoints)
    }

    fn process_headers(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut symbols = Vec::new();
        let mut stack = vec![tree_root];

        while let Some(node) = stack.pop() {
            if node.kind() == "atx_heading" {
                let mut level = 1;
                let mut cursor = node.walk();
                if cursor.goto_first_child() {
                    let marker = cursor.node();
                    level = self.get_header_level(marker.kind()).unwrap_or(1);
                }

                if let Some(text) = self.extract_header_text(node, source) {
                    let line_start = node.start_position().row + 1;
                    let line_end = node.end_position().row + 1;
                    let parent_id = self.find_parent_header(level, &symbols);

                    let name = match parent_id {
                        Some(pid) => format!("{} > {}", symbols[pid].name, text),
                        None => text,
                    };

                    let level_prefix = "#".repeat(level);

                    symbols.push(Symbol {
                        name,
                        symbol_type: SymbolType::Heading,
                        signature: Some(format!("h{} ({})", level, level_prefix)),
                        docstring: None,
                        line_start,
                        line_end,
                        parent_id,
                        file_path: file_path.to_path_buf(),
                        is_exported: false,
                    });
                }
            }

            let mut child_cursor = node.walk();
            if child_cursor.goto_first_child() {
                let mut children = Vec::new();
                loop {
                    children.push(child_cursor.node());
                    if !child_cursor.goto_next_sibling() {
                        break;
                    }
                }
                // Push in reverse order so they're popped in document order
                for child in children.into_iter().rev() {
                    stack.push(child);
                }
            }
        }

        Ok(symbols)
    }

    fn process_code_blocks(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
        headings: &[(usize, usize)],
        heading_symbols: &[Symbol],
    ) -> Result<Vec<Symbol>> {
        let mut code_blocks = Vec::new();
        let mut stack = vec![tree_root];

        while let Some(node) = stack.pop() {
            if node.kind() == "fenced_code_block" {
                let line_start = node.start_position().row + 1;
                let line_end = node.end_position().row + 1;

                let mut language = "unknown".to_string();
                let mut code_content = String::new();

                let mut cursor = node.walk();
                if cursor.goto_first_child() {
                    loop {
                        let child = cursor.node();
                        match child.kind() {
                            "info_string" => {
                                if let Some(lang) = self.extract_text(child, source) {
                                    language = lang.trim().to_string();
                                }
                            }
                            "code_fence_content" => {
                                if let Some(content) = self.extract_text(child, source) {
                                    let lines: Vec<&str> = content.lines().take(3).collect();
                                    code_content = lines.join("\n");
                                    if content.lines().count() > 3 {
                                        code_content.push_str("\n...");
                                    }
                                }
                            }
                            _ => {}
                        }
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }

                let parent_id = self.nearest_heading_before_line(headings, line_start);
                let name = match parent_id {
                    Some(pid) => {
                        let parent_name = heading_symbols
                            .get(pid)
                            .map(|s| s.name.as_str())
                            .unwrap_or("?");
                        format!("{} > [code: {}]", parent_name, language)
                    }
                    None => format!("[code: {}]", language),
                };

                code_blocks.push(Symbol {
                    name,
                    symbol_type: SymbolType::CodeBlock,
                    signature: Some(language.clone()),
                    docstring: if code_content.is_empty() {
                        None
                    } else {
                        Some(code_content)
                    },
                    line_start,
                    line_end,
                    parent_id,
                    file_path: file_path.to_path_buf(),
                    is_exported: false,
                });
            }

            let mut child_cursor = node.walk();
            if child_cursor.goto_first_child() {
                let mut children = Vec::new();
                loop {
                    children.push(child_cursor.node());
                    if !child_cursor.goto_next_sibling() {
                        break;
                    }
                }
                // Push in reverse order so they're popped in document order
                for child in children.into_iter().rev() {
                    stack.push(child);
                }
            }
        }

        Ok(code_blocks)
    }
}

impl ParserTrait for MarkdownParser {
    fn parse(&self, content: &str, file_path: &Path) -> Result<ParseResult> {
        let mut parser = Parser::new();
        let language = tree_sitter_md::LANGUAGE.into();
        parser
            .set_language(&language)
            .context("Failed to set Markdown language")?;

        let tree = parser
            .parse(content, None)
            .context("Failed to parse Markdown file")?;

        let root = tree.root_node();
        let mut result = ParseResult::new();

        let mut symbols = self.process_headers(root, content, file_path)?;

        let mut heading_positions: Vec<(usize, usize)> = symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.symbol_type == SymbolType::Heading)
            .map(|(idx, s)| (s.line_start, idx))
            .collect();
        heading_positions.sort_by_key(|(line, _)| *line);

        let code_blocks =
            self.process_code_blocks(root, content, file_path, &heading_positions, &symbols)?;
        let endpoints = self.extract_endpoints(content, file_path, &heading_positions, &symbols)?;

        symbols.extend(code_blocks);
        symbols.extend(endpoints);
        result.symbols = symbols;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_headers() -> Result<()> {
        let parser = MarkdownParser::new()?;
        let source = r#"# Main Header
Some content here.

## Subsection
More content.

### Smaller section
Even more content.
"#;
        let result = parser.parse(source, Path::new("test.md"))?;
        assert!(result.symbols.len() >= 3);

        let heading_symbols: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.symbol_type == SymbolType::Heading)
            .collect();
        assert_eq!(heading_symbols.len(), 3);
        assert_eq!(heading_symbols[0].name, "Main Header");
        assert_eq!(heading_symbols[1].name, "Main Header > Subsection");
        assert_eq!(
            heading_symbols[2].name,
            "Main Header > Subsection > Smaller section"
        );

        Ok(())
    }

    #[test]
    fn test_parse_code_blocks() -> Result<()> {
        let parser = MarkdownParser::new()?;
        let source = r#"# Example

```json
{
  "key": "value"
}
```

```python
def hello():
    print("world")
```
"#;
        let result = parser.parse(source, Path::new("test.md"))?;

        let code_blocks: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.symbol_type == SymbolType::CodeBlock)
            .collect();

        assert_eq!(code_blocks.len(), 2);

        assert!(code_blocks
            .iter()
            .any(|s| s.name == "Example > [code: json]"));
        assert!(code_blocks
            .iter()
            .any(|s| s.name == "Example > [code: python]"));

        Ok(())
    }

    #[test]
    fn test_extract_endpoints() -> Result<()> {
        let parser = MarkdownParser::new()?;
        let source = r#"# Rest

## Orders

> GET /v1/orders?limit=1

`POST /v1/orders`

**GET /v1/orders**

`GET
2020-01-01T00:00:00Z
example.com
/v1/orders`
"#;

        let result = parser.parse(source, Path::new("test.md"))?;

        let endpoints: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.symbol_type == SymbolType::Endpoint)
            .collect();

        assert!(endpoints
            .iter()
            .any(|s| s.name == "Rest > Orders > GET /v1/orders?limit=1"));
        assert!(endpoints
            .iter()
            .any(|s| s.name == "Rest > Orders > POST /v1/orders"));
        assert!(endpoints
            .iter()
            .any(|s| s.name == "Rest > Orders > GET /v1/orders"));
        assert_eq!(
            endpoints
                .iter()
                .filter(|s| s.name == "Rest > Orders > GET /v1/orders")
                .count(),
            2
        );

        Ok(())
    }
}

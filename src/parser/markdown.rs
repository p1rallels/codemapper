use super::{Parser as ParserTrait, ParseResult};
use crate::models::{Dependency, Symbol, SymbolType};
use anyhow::{Context, Result};
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
                && child.kind() != "atx_h6_marker" {
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

            let symbol_level = if let Some(sig) = &symbol.signature {
                sig.parse::<usize>().unwrap_or(1)
            } else {
                1
            };

            if symbol_level < current_level {
                return Some(idx);
            }
        }
        None
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
            match node.kind() {
                "atx_heading" => {
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

                        let level_prefix = "#".repeat(level);

                        symbols.push(Symbol {
                            name: text,
                            symbol_type: SymbolType::Heading,
                            signature: Some(format!("h{} ({})", level, level_prefix)),
                            docstring: None,
                            line_start,
                            line_end,
                            parent_id,
                            file_path: file_path.to_path_buf(),
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

        Ok(symbols)
    }

    fn process_code_blocks(
        &self,
        tree_root: Node,
        source: &str,
        file_path: &Path,
    ) -> Result<Vec<Symbol>> {
        let mut code_blocks = Vec::new();
        let mut stack = vec![tree_root];

        while let Some(node) = stack.pop() {
            match node.kind() {
                "fenced_code_block" => {
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

                    code_blocks.push(Symbol {
                        name: format!("[code: {}]", language),
                        symbol_type: SymbolType::CodeBlock,
                        signature: Some(language.clone()),
                        docstring: if code_content.is_empty() { None } else { Some(code_content) },
                        line_start,
                        line_end,
                        parent_id: None,
                        file_path: file_path.to_path_buf(),
                    });
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

        let mut headers = self.process_headers(root, content, file_path)?;
        let code_blocks = self.process_code_blocks(root, content, file_path)?;

        headers.extend(code_blocks);
        result.symbols = headers;

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

        let heading_symbols: Vec<_> = result.symbols.iter()
            .filter(|s| s.symbol_type == SymbolType::Heading)
            .collect();
        assert_eq!(heading_symbols.len(), 3);
        assert_eq!(heading_symbols[0].name, "Main Header");

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

        let code_blocks: Vec<_> = result.symbols.iter()
            .filter(|s| s.symbol_type == SymbolType::CodeBlock)
            .collect();

        assert!(code_blocks.len() >= 2);

        let has_json = code_blocks.iter()
            .any(|s| s.signature.as_ref().map(|sig| sig.contains("json")).unwrap_or(false));
        let has_python = code_blocks.iter()
            .any(|s| s.signature.as_ref().map(|sig| sig.contains("python")).unwrap_or(false));

        assert!(has_json);
        assert!(has_python);

        Ok(())
    }
}

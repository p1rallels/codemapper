use crate::index::CodeIndex;
use crate::models::{Language, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

#[derive(Debug, Clone)]
pub struct TestInfo {
    pub test_name: String,
    pub test_type: SymbolType,
    pub file_path: String,
    pub line: usize,
    pub call_line: usize,
    pub context: String,
}

#[derive(Debug, Clone)]
pub struct CallInfo {
    pub caller_name: String,
    pub caller_type: SymbolType,
    pub file_path: String,
    pub line: usize,
    pub context: String,
}

pub fn find_callers(
    index: &CodeIndex,
    symbol_name: &str,
    fuzzy: bool,
) -> Result<Vec<CallInfo>> {
    let mut callers = Vec::new();
    let mut seen = HashSet::new();

    for file_info in index.files() {
        let content = match fs::read_to_string(&file_info.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let calls = extract_calls_from_file(&content, &file_info.path, file_info.language)?;

        for (call_name, line, context) in calls {
            let matches = if fuzzy {
                call_name.to_lowercase().contains(&symbol_name.to_lowercase())
            } else {
                call_name == symbol_name
            };

            if matches {
                let key = format!("{}:{}", file_info.path.display(), line);
                if seen.contains(&key) {
                    continue;
                }
                seen.insert(key);

                let caller_symbol = find_enclosing_symbol(index, &file_info.path, line);

                callers.push(CallInfo {
                    caller_name: caller_symbol
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| "<top-level>".to_string()),
                    caller_type: caller_symbol
                        .map(|s| s.symbol_type)
                        .unwrap_or(SymbolType::Function),
                    file_path: file_info.path.display().to_string(),
                    line,
                    context: context.trim().to_string(),
                });
            }
        }
    }

    Ok(callers)
}

pub fn find_callees(
    index: &CodeIndex,
    symbol_name: &str,
    fuzzy: bool,
) -> Result<Vec<CallInfo>> {
    let symbols = if fuzzy {
        index.fuzzy_search(symbol_name)
    } else {
        index.query_symbol(symbol_name)
    };

    if symbols.is_empty() {
        return Ok(Vec::new());
    }

    let symbol = symbols.first().context("No symbol found")?;

    let content = fs::read_to_string(&symbol.file_path)
        .context("Failed to read symbol file")?;

    let lines: Vec<&str> = content.lines().collect();
    let start_idx = symbol.line_start.saturating_sub(1);
    let end_idx = symbol.line_end.min(lines.len());

    if start_idx >= lines.len() {
        return Ok(Vec::new());
    }

    let symbol_body: String = lines[start_idx..end_idx].join("\n");

    let language = Language::from_extension(
        symbol.file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
    );

    let calls = extract_calls_from_source(&symbol_body, language)?;

    let mut callees = Vec::new();
    let mut seen = HashSet::new();

    for (call_name, relative_line, context) in calls {
        if seen.contains(&call_name) {
            continue;
        }
        seen.insert(call_name.clone());

        let target_symbols = index.query_symbol(&call_name);

        if let Some(target) = target_symbols.first() {
            callees.push(CallInfo {
                caller_name: call_name,
                caller_type: target.symbol_type,
                file_path: target.file_path.display().to_string(),
                line: target.line_start,
                context: target.signature.clone().unwrap_or_default(),
            });
        } else {
            callees.push(CallInfo {
                caller_name: call_name,
                caller_type: SymbolType::Function,
                file_path: "<external>".to_string(),
                line: symbol.line_start + relative_line,
                context: context.trim().to_string(),
            });
        }
    }

    Ok(callees)
}

fn find_enclosing_symbol<'a>(
    index: &'a CodeIndex,
    path: &Path,
    line: usize,
) -> Option<&'a Symbol> {
    let symbols = index.get_file_symbols(path);

    symbols
        .into_iter()
        .filter(|s| s.line_start <= line && s.line_end >= line)
        .min_by_key(|s| s.line_end - s.line_start)
}

fn extract_calls_from_file(
    content: &str,
    path: &Path,
    language: Language,
) -> Result<Vec<(String, usize, String)>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let lang = Language::from_extension(ext);

    if lang == Language::Unknown {
        return Ok(Vec::new());
    }

    extract_calls_from_source(content, language)
}

fn extract_calls_from_source(
    content: &str,
    language: Language,
) -> Result<Vec<(String, usize, String)>> {
    match language {
        Language::Rust => extract_rust_calls(content),
        Language::Python => extract_python_calls(content),
        Language::JavaScript | Language::TypeScript => extract_js_calls(content),
        Language::Go => extract_go_calls(content),
        Language::Java => extract_java_calls(content),
        Language::C => extract_c_calls(content),
        _ => Ok(Vec::new()),
    }
}

fn extract_rust_calls(content: &str) -> Result<Vec<(String, usize, String)>> {
    let mut parser = Parser::new();
    let language = tree_sitter_rust::LANGUAGE.into();
    parser.set_language(&language).context("Failed to set Rust language")?;

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Ok(Vec::new()),
    };

    let query = Query::new(
        &language,
        r#"
        (call_expression
            function: (identifier) @call.name) @call.expr
        (call_expression
            function: (field_expression
                field: (field_identifier) @call.method)) @call.method_expr
        (call_expression
            function: (scoped_identifier
                name: (identifier) @call.scoped)) @call.scoped_expr
        (macro_invocation
            macro: (identifier) @macro.name) @macro.expr
        "#,
    ).context("Failed to create call query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    let mut calls = Vec::new();
    let mut seen_lines = HashSet::new();

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = query.capture_names().get(capture.index as usize).map(|s| s.as_ref());

            let is_name = matches!(capture_name, Some("call.name") | Some("call.method") | Some("call.scoped") | Some("macro.name"));

            if is_name {
                let name = capture.node.utf8_text(content.as_bytes()).unwrap_or_default().to_string();
                let line = capture.node.start_position().row + 1;
                
                if seen_lines.contains(&(name.clone(), line)) {
                    continue;
                }
                seen_lines.insert((name.clone(), line));

                let context = content.lines().nth(line - 1).unwrap_or("").to_string();
                calls.push((name, line, context));
            }
        }
    }

    Ok(calls)
}

fn extract_python_calls(content: &str) -> Result<Vec<(String, usize, String)>> {
    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE.into();
    parser.set_language(&language).context("Failed to set Python language")?;

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Ok(Vec::new()),
    };

    let query = Query::new(
        &language,
        r#"
        (call
            function: (identifier) @call.name) @call.expr
        (call
            function: (attribute
                attribute: (identifier) @call.method)) @call.method_expr
        "#,
    ).context("Failed to create call query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    let mut calls = Vec::new();
    let mut seen_lines = HashSet::new();

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = query.capture_names().get(capture.index as usize).map(|s| s.as_ref());

            let is_name = matches!(capture_name, Some("call.name") | Some("call.method"));

            if is_name {
                let name = capture.node.utf8_text(content.as_bytes()).unwrap_or_default().to_string();
                let line = capture.node.start_position().row + 1;

                if seen_lines.contains(&(name.clone(), line)) {
                    continue;
                }
                seen_lines.insert((name.clone(), line));

                let context = content.lines().nth(line - 1).unwrap_or("").to_string();
                calls.push((name, line, context));
            }
        }
    }

    Ok(calls)
}

fn extract_js_calls(content: &str) -> Result<Vec<(String, usize, String)>> {
    let mut parser = Parser::new();
    let language = tree_sitter_javascript::LANGUAGE.into();
    parser.set_language(&language).context("Failed to set JavaScript language")?;

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Ok(Vec::new()),
    };

    let query = Query::new(
        &language,
        r#"
        (call_expression
            function: (identifier) @call.name) @call.expr
        (call_expression
            function: (member_expression
                property: (property_identifier) @call.method)) @call.method_expr
        "#,
    ).context("Failed to create call query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    let mut calls = Vec::new();
    let mut seen_lines = HashSet::new();

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = query.capture_names().get(capture.index as usize).map(|s| s.as_ref());

            let is_name = matches!(capture_name, Some("call.name") | Some("call.method"));

            if is_name {
                let name = capture.node.utf8_text(content.as_bytes()).unwrap_or_default().to_string();
                let line = capture.node.start_position().row + 1;

                if seen_lines.contains(&(name.clone(), line)) {
                    continue;
                }
                seen_lines.insert((name.clone(), line));

                let context = content.lines().nth(line - 1).unwrap_or("").to_string();
                calls.push((name, line, context));
            }
        }
    }

    Ok(calls)
}

fn extract_go_calls(content: &str) -> Result<Vec<(String, usize, String)>> {
    let mut parser = Parser::new();
    let language = tree_sitter_go::LANGUAGE.into();
    parser.set_language(&language).context("Failed to set Go language")?;

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Ok(Vec::new()),
    };

    let query = Query::new(
        &language,
        r#"
        (call_expression
            function: (identifier) @call.name) @call.expr
        (call_expression
            function: (selector_expression
                field: (field_identifier) @call.method)) @call.method_expr
        "#,
    ).context("Failed to create call query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    let mut calls = Vec::new();
    let mut seen_lines = HashSet::new();

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = query.capture_names().get(capture.index as usize).map(|s| s.as_ref());

            let is_name = matches!(capture_name, Some("call.name") | Some("call.method"));

            if is_name {
                let name = capture.node.utf8_text(content.as_bytes()).unwrap_or_default().to_string();
                let line = capture.node.start_position().row + 1;

                if seen_lines.contains(&(name.clone(), line)) {
                    continue;
                }
                seen_lines.insert((name.clone(), line));

                let context = content.lines().nth(line - 1).unwrap_or("").to_string();
                calls.push((name, line, context));
            }
        }
    }

    Ok(calls)
}

fn extract_java_calls(content: &str) -> Result<Vec<(String, usize, String)>> {
    let mut parser = Parser::new();
    let language = tree_sitter_java::LANGUAGE.into();
    parser.set_language(&language).context("Failed to set Java language")?;

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Ok(Vec::new()),
    };

    let query = Query::new(
        &language,
        r#"
        (method_invocation
            name: (identifier) @call.name) @call.expr
        "#,
    ).context("Failed to create call query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    let mut calls = Vec::new();
    let mut seen_lines = HashSet::new();

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = query.capture_names().get(capture.index as usize).map(|s| s.as_ref());

            if capture_name == Some("call.name") {
                let name = capture.node.utf8_text(content.as_bytes()).unwrap_or_default().to_string();
                let line = capture.node.start_position().row + 1;

                if seen_lines.contains(&(name.clone(), line)) {
                    continue;
                }
                seen_lines.insert((name.clone(), line));

                let context = content.lines().nth(line - 1).unwrap_or("").to_string();
                calls.push((name, line, context));
            }
        }
    }

    Ok(calls)
}

fn extract_c_calls(content: &str) -> Result<Vec<(String, usize, String)>> {
    let mut parser = Parser::new();
    let language = tree_sitter_c::LANGUAGE.into();
    parser.set_language(&language).context("Failed to set C language")?;

    let tree = match parser.parse(content, None) {
        Some(t) => t,
        None => return Ok(Vec::new()),
    };

    let query = Query::new(
        &language,
        r#"
        (call_expression
            function: (identifier) @call.name) @call.expr
        "#,
    ).context("Failed to create call query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    let mut calls = Vec::new();
    let mut seen_lines = HashSet::new();

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = query.capture_names().get(capture.index as usize).map(|s| s.as_ref());

            if capture_name == Some("call.name") {
                let name = capture.node.utf8_text(content.as_bytes()).unwrap_or_default().to_string();
                let line = capture.node.start_position().row + 1;

                if seen_lines.contains(&(name.clone(), line)) {
                    continue;
                }
                seen_lines.insert((name.clone(), line));

                let context = content.lines().nth(line - 1).unwrap_or("").to_string();
                calls.push((name, line, context));
            }
        }
    }

    Ok(calls)
}

pub fn is_test_file(path: &Path, language: Language) -> bool {
    let file_name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let path_str = path.to_string_lossy();
    
    match language {
        Language::Rust => {
            file_name.ends_with("_test.rs") 
                || file_name.starts_with("test_")
                || path_str.contains("/tests/")
                || path_str.contains("\\tests\\")
        }
        Language::Python => {
            file_name.starts_with("test_") 
                || file_name.ends_with("_test.py")
                || path_str.contains("/tests/")
                || path_str.contains("\\tests\\")
        }
        Language::JavaScript | Language::TypeScript => {
            file_name.ends_with(".test.js")
                || file_name.ends_with(".spec.js")
                || file_name.ends_with(".test.ts")
                || file_name.ends_with(".spec.ts")
                || file_name.ends_with(".test.jsx")
                || file_name.ends_with(".spec.jsx")
                || file_name.ends_with(".test.tsx")
                || file_name.ends_with(".spec.tsx")
                || path_str.contains("__tests__")
                || path_str.contains("/tests/")
                || path_str.contains("\\tests\\")
        }
        Language::Go => {
            file_name.ends_with("_test.go")
        }
        Language::Java => {
            file_name.ends_with("Test.java")
                || file_name.starts_with("Test")
                || path_str.contains("/test/")
                || path_str.contains("\\test\\")
        }
        _ => false,
    }
}

pub fn is_test_symbol(symbol: &Symbol, content: &str, language: Language) -> bool {
    let name = &symbol.name;
    
    match language {
        Language::Rust => {
            if let Some(line_idx) = symbol.line_start.checked_sub(1) {
                let lines: Vec<&str> = content.lines().collect();
                for i in (0..=line_idx.min(lines.len().saturating_sub(1))).rev() {
                    let line = lines.get(i).unwrap_or(&"");
                    if line.contains("#[test]") || line.contains("#[tokio::test]") {
                        return true;
                    }
                    if !line.trim().is_empty() 
                        && !line.trim().starts_with("#[") 
                        && !line.trim().starts_with("//") 
                    {
                        break;
                    }
                }
            }
            name.starts_with("test_")
        }
        Language::Python => {
            name.starts_with("test_") || name.starts_with("Test")
        }
        Language::JavaScript | Language::TypeScript => {
            name.starts_with("test") 
                || name == "it" 
                || name == "describe"
                || name.starts_with("Test")
        }
        Language::Go => {
            name.starts_with("Test") || name.starts_with("Benchmark")
        }
        Language::Java => {
            if let Some(line_idx) = symbol.line_start.checked_sub(1) {
                let lines: Vec<&str> = content.lines().collect();
                for i in (0..=line_idx.min(lines.len().saturating_sub(1))).rev() {
                    let line = lines.get(i).unwrap_or(&"");
                    if line.contains("@Test") {
                        return true;
                    }
                    if !line.trim().is_empty() 
                        && !line.trim().starts_with("@") 
                        && !line.trim().starts_with("//")
                    {
                        break;
                    }
                }
            }
            name.starts_with("test")
        }
        _ => false,
    }
}

pub fn find_tests(
    index: &CodeIndex,
    symbol_name: &str,
    fuzzy: bool,
) -> Result<Vec<TestInfo>> {
    let mut tests = Vec::new();
    let mut seen = HashSet::new();

    for file_info in index.files() {
        let is_test_file = is_test_file(&file_info.path, file_info.language);
        
        let content = match fs::read_to_string(&file_info.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let calls = extract_calls_from_file(&content, &file_info.path, file_info.language)?;

        for (call_name, line, context) in calls {
            let matches = if fuzzy {
                call_name.to_lowercase().contains(&symbol_name.to_lowercase())
            } else {
                call_name == symbol_name
            };

            if !matches {
                continue;
            }

            let caller_symbol = find_enclosing_symbol(index, &file_info.path, line);

            let is_test = match caller_symbol {
                Some(sym) => {
                    is_test_file || is_test_symbol(sym, &content, file_info.language)
                }
                None => is_test_file,
            };

            if !is_test {
                continue;
            }

            let key = format!("{}:{}", file_info.path.display(), line);
            if seen.contains(&key) {
                continue;
            }
            seen.insert(key);

            let (test_name, test_line) = match caller_symbol {
                Some(sym) => (sym.name.clone(), sym.line_start),
                None => ("<test-file-level>".to_string(), line),
            };

            tests.push(TestInfo {
                test_name,
                test_type: caller_symbol
                    .map(|s| s.symbol_type)
                    .unwrap_or(SymbolType::Function),
                file_path: file_info.path.display().to_string(),
                line: test_line,
                call_line: line,
                context: context.trim().to_string(),
            });
        }
    }

    Ok(tests)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_rust_calls() -> Result<()> {
        let source = r#"
fn main() {
    let x = foo();
    bar(x);
    obj.method();
    println!("test");
}
"#;
        let calls = extract_rust_calls(source)?;
        let names: Vec<&str> = calls.iter().map(|(n, _, _)| n.as_str()).collect();
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"bar"));
        assert!(names.contains(&"method"));
        assert!(names.contains(&"println"));
        Ok(())
    }

    #[test]
    fn test_extract_python_calls() -> Result<()> {
        let source = r#"
def main():
    x = foo()
    bar(x)
    obj.method()
"#;
        let calls = extract_python_calls(source)?;
        let names: Vec<&str> = calls.iter().map(|(n, _, _)| n.as_str()).collect();
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"bar"));
        assert!(names.contains(&"method"));
        Ok(())
    }
}

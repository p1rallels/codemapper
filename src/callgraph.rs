use crate::index::CodeIndex;
use crate::models::{Language, Symbol, SymbolType};
use anyhow::{Context, Result};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::Path;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

#[derive(Debug, Clone)]
pub struct TraceStep {
    pub symbol_name: String,
    pub symbol_type: SymbolType,
    pub file_path: String,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct TracePath {
    pub steps: Vec<TraceStep>,
    pub found: bool,
}

impl TracePath {
    pub fn not_found() -> Self {
        Self {
            steps: Vec::new(),
            found: false,
        }
    }
}

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
pub struct TestDep {
    pub name: String,
    pub symbol_type: SymbolType,
    pub file_path: String,
    pub line: usize,
    pub called_from_line: usize,
}

#[derive(Debug, Clone)]
pub struct UntestedInfo {
    pub name: String,
    pub symbol_type: SymbolType,
    pub file_path: String,
    pub line: usize,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrypointCategory {
    MainEntry,
    ApiFunction,
    PossiblyUnused,
}

impl EntrypointCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            EntrypointCategory::MainEntry => "Main Entrypoint",
            EntrypointCategory::ApiFunction => "API Function",
            EntrypointCategory::PossiblyUnused => "Possibly Unused",
        }
    }
}

#[derive(Debug, Clone)]
pub struct EntrypointInfo {
    pub name: String,
    pub symbol_type: SymbolType,
    pub file_path: String,
    pub line: usize,
    pub signature: Option<String>,
    pub is_exported: bool,
    pub category: EntrypointCategory,
}

#[derive(Debug, Clone)]
pub struct CallInfo {
    pub caller_name: String,
    pub caller_type: SymbolType,
    pub file_path: String,
    pub line: usize,
    pub context: String,
}

pub fn find_callers(index: &CodeIndex, symbol_name: &str, fuzzy: bool) -> Result<Vec<CallInfo>> {
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
                call_name
                    .to_lowercase()
                    .contains(&symbol_name.to_lowercase())
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

pub fn find_callees(index: &CodeIndex, symbol_name: &str, fuzzy: bool) -> Result<Vec<CallInfo>> {
    let symbols = if fuzzy {
        index.fuzzy_search(symbol_name)
    } else {
        index.query_symbol(symbol_name)
    };

    if symbols.is_empty() {
        return Ok(Vec::new());
    }

    let mut all_callees = Vec::new();
    let mut global_seen = HashSet::new();

    // Process ALL symbols with this name, not just the first one
    for symbol in &symbols {
        let content = match fs::read_to_string(&symbol.file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let start_idx = symbol.line_start.saturating_sub(1);
        let end_idx = symbol.line_end.min(lines.len());

        if start_idx >= lines.len() {
            continue;
        }

        let symbol_body: String = lines[start_idx..end_idx].join("\n");

        let language = Language::from_extension(
            symbol
                .file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or(""),
        );

        let calls = extract_calls_from_source(&symbol_body, language)?;

        for (call_name, relative_line, context) in calls {
            let dedup_key = format!(
                "{}:{}:{}",
                symbol.file_path.display(),
                call_name,
                relative_line
            );
            if global_seen.contains(&dedup_key) {
                continue;
            }
            global_seen.insert(dedup_key);

            let target_symbols = index.query_symbol(&call_name);

            if let Some(target) = target_symbols.first() {
                all_callees.push(CallInfo {
                    caller_name: call_name,
                    caller_type: target.symbol_type,
                    file_path: target.file_path.display().to_string(),
                    line: target.line_start,
                    context: target.signature.clone().unwrap_or_default(),
                });
            } else {
                all_callees.push(CallInfo {
                    caller_name: call_name,
                    caller_type: SymbolType::Function,
                    file_path: "<external>".to_string(),
                    line: symbol.line_start + relative_line,
                    context: context.trim().to_string(),
                });
            }
        }
    }

    Ok(all_callees)
}

fn find_enclosing_symbol<'a>(index: &'a CodeIndex, path: &Path, line: usize) -> Option<&'a Symbol> {
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
    parser
        .set_language(&language)
        .context("Failed to set Rust language")?;

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
    )
    .context("Failed to create call query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    let mut calls = Vec::new();
    let mut seen_lines = HashSet::new();

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = query
                .capture_names()
                .get(capture.index as usize)
                .map(|s| s.as_ref());

            let capture_name = match capture_name {
                Some(n) => n,
                None => continue,
            };

            let mut name_opt: Option<String> = None;

            match capture_name {
                "call.name" | "call.method" | "call.scoped" => {
                    name_opt = Some(
                        capture
                            .node
                            .utf8_text(content.as_bytes())
                            .unwrap_or_default()
                            .to_string(),
                    );
                }
                "macro.name" => {
                    // treat `println!(...)` as a call to `println`, etc.
                    name_opt = Some(
                        capture
                            .node
                            .utf8_text(content.as_bytes())
                            .unwrap_or_default()
                            .to_string(),
                    );

                    // also scan the token_tree for call-ish patterns like `x.foo(...)` inside macros
                    if let Some(macro_node) = match_.captures.iter().find_map(|c| {
                        let n: &str = &query.capture_names()[c.index as usize];
                        if n == "macro.expr" {
                            Some(c.node)
                        } else {
                            None
                        }
                    }) {
                        let mut cursor = macro_node.walk();
                        for child in macro_node.children(&mut cursor) {
                            if child.kind() == "token_tree" {
                                collect_identifiers_from_token_tree(
                                    child,
                                    content,
                                    &mut calls,
                                    &mut seen_lines,
                                );
                            }
                        }
                    }
                }
                _ => {}
            }

            if let Some(name) = name_opt {
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

fn collect_identifiers_from_token_tree(
    token_tree: tree_sitter::Node,
    source: &str,
    calls: &mut Vec<(String, usize, String)>,
    seen_lines: &mut HashSet<(String, usize)>,
) {
    let mut cursor = token_tree.walk();
    let children: Vec<tree_sitter::Node> = token_tree.children(&mut cursor).collect();

    // heuristic: capture `foo` in patterns like `x.foo(...)` or `Type::foo(...)` inside macro token trees
    for window in children.windows(4) {
        if window[0].kind() == "identifier"
            && window[1].kind() == "."
            && window[2].kind() == "identifier"
            && window[3].kind() == "token_tree"
        {
            let name = window[2]
                .utf8_text(source.as_bytes())
                .unwrap_or_default()
                .to_string();
            let line = window[2].start_position().row + 1;
            if seen_lines.insert((name.clone(), line)) {
                let context = source.lines().nth(line - 1).unwrap_or("").to_string();
                calls.push((name, line, context));
            }
        }

        if window[0].kind() == "identifier"
            && window[1].kind() == "::"
            && window[2].kind() == "identifier"
            && window[3].kind() == "token_tree"
        {
            let name = window[2]
                .utf8_text(source.as_bytes())
                .unwrap_or_default()
                .to_string();
            let line = window[2].start_position().row + 1;
            if seen_lines.insert((name.clone(), line)) {
                let context = source.lines().nth(line - 1).unwrap_or("").to_string();
                calls.push((name, line, context));
            }
        }
    }

    // recurse
    let mut cursor = token_tree.walk();
    for child in token_tree.children(&mut cursor) {
        if child.kind() == "token_tree" {
            collect_identifiers_from_token_tree(child, source, calls, seen_lines);
        }
    }
}

fn extract_python_calls(content: &str) -> Result<Vec<(String, usize, String)>> {
    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE.into();
    parser
        .set_language(&language)
        .context("Failed to set Python language")?;

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
    )
    .context("Failed to create call query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    let mut calls = Vec::new();
    let mut seen_lines = HashSet::new();

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = query
                .capture_names()
                .get(capture.index as usize)
                .map(|s| s.as_ref());

            let is_name = matches!(capture_name, Some("call.name") | Some("call.method"));

            if is_name {
                let name = capture
                    .node
                    .utf8_text(content.as_bytes())
                    .unwrap_or_default()
                    .to_string();
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
    parser
        .set_language(&language)
        .context("Failed to set JavaScript language")?;

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
    )
    .context("Failed to create call query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    let mut calls = Vec::new();
    let mut seen_lines = HashSet::new();

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = query
                .capture_names()
                .get(capture.index as usize)
                .map(|s| s.as_ref());

            let is_name = matches!(capture_name, Some("call.name") | Some("call.method"));

            if is_name {
                let name = capture
                    .node
                    .utf8_text(content.as_bytes())
                    .unwrap_or_default()
                    .to_string();
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
    parser
        .set_language(&language)
        .context("Failed to set Go language")?;

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
    )
    .context("Failed to create call query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    let mut calls = Vec::new();
    let mut seen_lines = HashSet::new();

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = query
                .capture_names()
                .get(capture.index as usize)
                .map(|s| s.as_ref());

            let is_name = matches!(capture_name, Some("call.name") | Some("call.method"));

            if is_name {
                let name = capture
                    .node
                    .utf8_text(content.as_bytes())
                    .unwrap_or_default()
                    .to_string();
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
    parser
        .set_language(&language)
        .context("Failed to set Java language")?;

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
    )
    .context("Failed to create call query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    let mut calls = Vec::new();
    let mut seen_lines = HashSet::new();

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = query
                .capture_names()
                .get(capture.index as usize)
                .map(|s| s.as_ref());

            if capture_name == Some("call.name") {
                let name = capture
                    .node
                    .utf8_text(content.as_bytes())
                    .unwrap_or_default()
                    .to_string();
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
    parser
        .set_language(&language)
        .context("Failed to set C language")?;

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
    )
    .context("Failed to create call query")?;

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), content.as_bytes());
    let mut calls = Vec::new();
    let mut seen_lines = HashSet::new();

    while let Some(match_) = matches.next() {
        for capture in match_.captures {
            let capture_name = query
                .capture_names()
                .get(capture.index as usize)
                .map(|s| s.as_ref());

            if capture_name == Some("call.name") {
                let name = capture
                    .node
                    .utf8_text(content.as_bytes())
                    .unwrap_or_default()
                    .to_string();
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
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
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
        Language::Go => file_name.ends_with("_test.go"),
        Language::Java => {
            file_name.ends_with("Test.java")
                || file_name.starts_with("Test")
                || path_str.contains("/test/")
                || path_str.contains("\\test\\")
        }
        _ => false,
    }
}

pub fn find_test_deps(index: &CodeIndex, test_file: &Path) -> Result<Vec<TestDep>> {
    let ext = test_file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let language = Language::from_extension(ext);

    if !is_test_file(test_file, language) {
        anyhow::bail!(
            "File does not appear to be a test file: {}",
            test_file.display()
        );
    }

    let content = fs::read_to_string(test_file).context("Failed to read test file")?;

    let calls = extract_calls_from_source(&content, language)?;

    let mut deps = Vec::new();
    let mut seen = HashSet::new();

    for (call_name, call_line, _context) in calls {
        if seen.contains(&call_name) {
            continue;
        }

        let target_symbols = index.query_symbol(&call_name);

        if let Some(target) = target_symbols.first() {
            if target.file_path == test_file {
                continue;
            }

            let target_content = match fs::read_to_string(&target.file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let target_lang = Language::from_extension(
                target
                    .file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or(""),
            );

            if is_test_file(&target.file_path, target_lang) {
                continue;
            }

            if is_test_symbol(target, &target_content, target_lang) {
                continue;
            }

            seen.insert(call_name.clone());

            deps.push(TestDep {
                name: target.name.clone(),
                symbol_type: target.symbol_type,
                file_path: target.file_path.display().to_string(),
                line: target.line_start,
                called_from_line: call_line,
            });
        }
    }

    deps.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));

    Ok(deps)
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
        Language::Python => name.starts_with("test_") || name.starts_with("Test"),
        Language::JavaScript | Language::TypeScript => {
            name.starts_with("test")
                || name == "it"
                || name == "describe"
                || name.starts_with("Test")
        }
        Language::Go => name.starts_with("Test") || name.starts_with("Benchmark"),
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

pub fn find_tests(index: &CodeIndex, symbol_name: &str, fuzzy: bool) -> Result<Vec<TestInfo>> {
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
                call_name
                    .to_lowercase()
                    .contains(&symbol_name.to_lowercase())
            } else {
                call_name == symbol_name
            };

            if !matches {
                continue;
            }

            let caller_symbol = find_enclosing_symbol(index, &file_info.path, line);

            let is_test = match caller_symbol {
                Some(sym) => is_test_file || is_test_symbol(sym, &content, file_info.language),
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

pub fn find_untested(index: &CodeIndex) -> Result<Vec<UntestedInfo>> {
    let mut tested_symbols: HashSet<String> = HashSet::new();

    for file_info in index.files() {
        let is_test_file_flag = is_test_file(&file_info.path, file_info.language);

        let content = match fs::read_to_string(&file_info.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if is_test_file_flag {
            let calls = extract_calls_from_file(&content, &file_info.path, file_info.language)?;
            for (call_name, _, _) in calls {
                tested_symbols.insert(call_name);
            }
        } else {
            let file_symbols = index.get_file_symbols(&file_info.path);
            for symbol in file_symbols {
                if is_test_symbol(symbol, &content, file_info.language) {
                    let start_idx = symbol.line_start.saturating_sub(1);
                    let end_idx = symbol.line_end.min(content.lines().count());
                    let lines: Vec<&str> = content.lines().collect();

                    if start_idx < lines.len() {
                        let symbol_body: String =
                            lines[start_idx..end_idx.min(lines.len())].join("\n");
                        let calls = extract_calls_from_source(&symbol_body, file_info.language)?;
                        for (call_name, _, _) in calls {
                            tested_symbols.insert(call_name);
                        }
                    }
                }
            }
        }
    }

    let mut untested = Vec::new();

    for file_info in index.files() {
        if is_test_file(&file_info.path, file_info.language) {
            continue;
        }

        let content = match fs::read_to_string(&file_info.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let file_symbols = index.get_file_symbols(&file_info.path);

        for symbol in file_symbols {
            if is_test_symbol(symbol, &content, file_info.language) {
                continue;
            }

            if symbol.name.starts_with('_') && file_info.language == Language::Python {
                continue;
            }

            if symbol.name.is_empty() {
                continue;
            }

            if matches!(
                symbol.symbol_type,
                SymbolType::Heading | SymbolType::CodeBlock
            ) {
                continue;
            }

            if !tested_symbols.contains(&symbol.name) {
                untested.push(UntestedInfo {
                    name: symbol.name.clone(),
                    symbol_type: symbol.symbol_type,
                    file_path: file_info.path.display().to_string(),
                    line: symbol.line_start,
                    signature: symbol.signature.clone(),
                });
            }
        }
    }

    untested.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)));

    Ok(untested)
}

pub fn find_entrypoints(index: &CodeIndex) -> Result<Vec<EntrypointInfo>> {
    let mut all_called_symbols: HashSet<String> = HashSet::new();

    for file_info in index.files() {
        let content = match fs::read_to_string(&file_info.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let calls = match extract_calls_from_file(&content, &file_info.path, file_info.language) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for (call_name, _, _) in calls {
            all_called_symbols.insert(call_name);
        }
    }

    let mut entrypoints = Vec::new();

    for file_info in index.files() {
        if is_test_file(&file_info.path, file_info.language) {
            continue;
        }

        let content = match fs::read_to_string(&file_info.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let file_symbols = index.get_file_symbols(&file_info.path);

        for symbol in file_symbols {
            if symbol.name.is_empty() {
                continue;
            }

            if matches!(
                symbol.symbol_type,
                SymbolType::Heading | SymbolType::CodeBlock
            ) {
                continue;
            }

            if is_test_symbol(symbol, &content, file_info.language) {
                continue;
            }

            if all_called_symbols.contains(&symbol.name) {
                continue;
            }

            let is_exported = is_symbol_exported(symbol, &content, file_info.language);

            if !is_exported {
                continue;
            }

            let category = categorize_entrypoint(&symbol.name, symbol.symbol_type);

            entrypoints.push(EntrypointInfo {
                name: symbol.name.clone(),
                symbol_type: symbol.symbol_type,
                file_path: file_info.path.display().to_string(),
                line: symbol.line_start,
                signature: symbol.signature.clone(),
                is_exported,
                category,
            });
        }
    }

    entrypoints.sort_by(|a, b| match (&a.category, &b.category) {
        (EntrypointCategory::MainEntry, EntrypointCategory::MainEntry) => {
            a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line))
        }
        (EntrypointCategory::MainEntry, _) => std::cmp::Ordering::Less,
        (_, EntrypointCategory::MainEntry) => std::cmp::Ordering::Greater,
        (EntrypointCategory::ApiFunction, EntrypointCategory::ApiFunction) => {
            a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line))
        }
        (EntrypointCategory::ApiFunction, _) => std::cmp::Ordering::Less,
        (_, EntrypointCategory::ApiFunction) => std::cmp::Ordering::Greater,
        _ => a.file_path.cmp(&b.file_path).then(a.line.cmp(&b.line)),
    });

    Ok(entrypoints)
}

fn is_symbol_exported(symbol: &Symbol, content: &str, language: Language) -> bool {
    let name = &symbol.name;
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = symbol.line_start.saturating_sub(1);
    let definition_line = lines.get(line_idx).unwrap_or(&"");

    match language {
        Language::Rust => {
            if definition_line.trim().starts_with("pub fn")
                || definition_line.trim().starts_with("pub struct")
                || definition_line.trim().starts_with("pub enum")
                || definition_line.trim().starts_with("pub trait")
                || definition_line.trim().starts_with("pub async fn")
                || definition_line.trim().starts_with("pub const")
                || definition_line.trim().starts_with("pub static")
                || definition_line.trim().starts_with("pub type")
            {
                let is_restricted = definition_line.contains("pub(crate)")
                    || definition_line.contains("pub(super)")
                    || definition_line.contains("pub(self)");
                return !is_restricted;
            }
            false
        }
        Language::Python => !name.starts_with('_'),
        Language::JavaScript | Language::TypeScript => {
            definition_line.contains("export ")
                || definition_line.contains("module.exports")
                || definition_line.contains("exports.")
        }
        Language::Go => name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false),
        Language::Java => definition_line.contains("public "),
        Language::C => !name.starts_with('_'),
        _ => true,
    }
}

fn categorize_entrypoint(name: &str, symbol_type: SymbolType) -> EntrypointCategory {
    let name_lower = name.to_lowercase();

    let main_patterns = ["main", "run", "start", "init", "execute", "cli", "app"];
    for pattern in main_patterns {
        if name_lower == pattern || name_lower.starts_with(&format!("{}_", pattern)) {
            return EntrypointCategory::MainEntry;
        }
    }

    let api_patterns = [
        "get",
        "post",
        "put",
        "delete",
        "patch",
        "handle",
        "serve",
        "route",
        "api",
        "endpoint",
        "create",
        "read",
        "update",
        "list",
        "fetch",
        "process",
        "export",
        "import",
        "parse",
        "validate",
        "transform",
    ];

    for pattern in api_patterns {
        if name_lower.starts_with(pattern) || name_lower.ends_with(pattern) {
            return EntrypointCategory::ApiFunction;
        }
    }

    if matches!(symbol_type, SymbolType::Class | SymbolType::Enum) {
        return EntrypointCategory::ApiFunction;
    }

    if name
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
        && matches!(symbol_type, SymbolType::Function)
    {
        return EntrypointCategory::ApiFunction;
    }

    EntrypointCategory::PossiblyUnused
}

const MAX_TRACE_DEPTH: usize = 10;

pub fn trace_path(index: &CodeIndex, from: &str, to: &str, fuzzy: bool) -> Result<TracePath> {
    let source_symbols = if fuzzy {
        index.fuzzy_search(from)
    } else {
        index.query_symbol(from)
    };

    if source_symbols.is_empty() {
        return Ok(TracePath::not_found());
    }

    let target_symbols = if fuzzy {
        index.fuzzy_search(to)
    } else {
        index.query_symbol(to)
    };

    if target_symbols.is_empty() {
        return Ok(TracePath::not_found());
    }

    let target_names: HashSet<String> = target_symbols
        .iter()
        .map(|s| s.name.to_lowercase())
        .collect();

    let mut queue: VecDeque<(TraceStep, Vec<TraceStep>)> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();

    // Start BFS from ALL matching source symbols, not just the first one
    for source in &source_symbols {
        let start_step = TraceStep {
            symbol_name: source.name.clone(),
            symbol_type: source.symbol_type,
            file_path: source.file_path.display().to_string(),
            line: source.line_start,
        };

        let visit_key = format!(
            "{}:{}",
            source.file_path.display(),
            source.name.to_lowercase()
        );
        if !visited.contains(&visit_key) {
            visited.insert(visit_key);
            queue.push_back((start_step.clone(), vec![start_step]));
        }
    }

    while let Some((current, path)) = queue.pop_front() {
        if path.len() > MAX_TRACE_DEPTH {
            continue;
        }

        // Find the specific symbol instance from the current step
        let current_symbols = index.query_symbol(&current.symbol_name);
        let current_symbol = current_symbols.iter().find(|s| {
            s.file_path.display().to_string() == current.file_path && s.line_start == current.line
        });

        if current_symbol.is_none() {
            continue;
        }

        let callees = find_callees_for_symbol(index, current_symbol.unwrap())?;

        for callee in callees {
            let callee_lower = callee.caller_name.to_lowercase();

            if target_names.contains(&callee_lower) {
                let mut final_path = path.clone();
                final_path.push(TraceStep {
                    symbol_name: callee.caller_name.clone(),
                    symbol_type: callee.caller_type,
                    file_path: callee.file_path.clone(),
                    line: callee.line,
                });
                return Ok(TracePath {
                    steps: final_path,
                    found: true,
                });
            }

            let visit_key = format!("{}:{}", callee.file_path, callee_lower);
            if !visited.contains(&visit_key) {
                visited.insert(visit_key);

                let next_step = TraceStep {
                    symbol_name: callee.caller_name.clone(),
                    symbol_type: callee.caller_type,
                    file_path: callee.file_path.clone(),
                    line: callee.line,
                };

                let mut new_path = path.clone();
                new_path.push(next_step.clone());
                queue.push_back((next_step, new_path));
            }
        }
    }

    Ok(TracePath::not_found())
}

fn find_callees_for_symbol(index: &CodeIndex, symbol: &Symbol) -> Result<Vec<CallInfo>> {
    let content = match fs::read_to_string(&symbol.file_path) {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()),
    };

    let lines: Vec<&str> = content.lines().collect();
    let start_idx = symbol.line_start.saturating_sub(1);
    let end_idx = symbol.line_end.min(lines.len());

    if start_idx >= lines.len() {
        return Ok(Vec::new());
    }

    let symbol_body: String = lines[start_idx..end_idx].join("\n");

    let language = Language::from_extension(
        symbol
            .file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or(""),
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

fn find_callees_by_name(index: &CodeIndex, symbol_name: &str) -> Result<Vec<CallInfo>> {
    let symbols = index.query_symbol(symbol_name);

    if symbols.is_empty() {
        return Ok(Vec::new());
    }

    let mut all_callees = Vec::new();
    let mut global_seen = HashSet::new();

    // Process ALL symbols with this name, not just the first one
    for symbol in &symbols {
        let content = match fs::read_to_string(&symbol.file_path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lines: Vec<&str> = content.lines().collect();
        let start_idx = symbol.line_start.saturating_sub(1);
        let end_idx = symbol.line_end.min(lines.len());

        if start_idx >= lines.len() {
            continue;
        }

        let symbol_body: String = lines[start_idx..end_idx].join("\n");

        let language = Language::from_extension(
            symbol
                .file_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or(""),
        );

        let calls = extract_calls_from_source(&symbol_body, language)?;

        for (call_name, relative_line, context) in calls {
            let dedup_key = format!(
                "{}:{}:{}",
                symbol.file_path.display(),
                call_name,
                relative_line
            );
            if global_seen.contains(&dedup_key) {
                continue;
            }
            global_seen.insert(dedup_key);

            let target_symbols = index.query_symbol(&call_name);

            if let Some(target) = target_symbols.first() {
                all_callees.push(CallInfo {
                    caller_name: call_name,
                    caller_type: target.symbol_type,
                    file_path: target.file_path.display().to_string(),
                    line: target.line_start,
                    context: target.signature.clone().unwrap_or_default(),
                });
            } else {
                all_callees.push(CallInfo {
                    caller_name: call_name,
                    caller_type: SymbolType::Function,
                    file_path: "<external>".to_string(),
                    line: symbol.line_start + relative_line,
                    context: context.trim().to_string(),
                });
            }
        }
    }

    Ok(all_callees)
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

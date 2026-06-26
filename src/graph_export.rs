use crate::callgraph;
use crate::implements::{self, Implementation};
use crate::index::CodeIndex;
use crate::models::{FileInfo, Language, Symbol, SymbolType};
use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExportOptions<'a> {
    pub root_path: &'a Path,
    pub include_generated_at: bool,
    pub cm_version: &'a str,
}

#[derive(Debug, Serialize)]
pub(crate) struct GraphExport {
    pub schema_version: u32,
    pub meta: ExportMeta,
    pub nodes: ExportNodes,
    pub edges: ExportEdges,
    pub diagnostics: Vec<ExportDiagnostic>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ExportMeta {
    pub root: String,
    pub root_name: Option<String>,
    pub generated_at: Option<String>,
    pub cm_version: String,
    pub languages: Vec<String>,
    pub stats: ExportStats,
}

#[derive(Debug, Serialize)]
pub(crate) struct ExportStats {
    pub files: usize,
    pub symbols: usize,
    pub edges: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct ExportNodes {
    pub files: Vec<ExportFileNode>,
    pub symbols: Vec<ExportSymbolNode>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ExportEdges {
    pub contains: Vec<ContainsEdge>,
    pub imports: Vec<ImportEdge>,
    pub calls: Vec<CallEdge>,
    pub implements: Vec<ImplementsEdge>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ExportDiagnostic {
    pub severity: String,
    pub message: String,
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ExportFileNode {
    pub id: String,
    pub path: String,
    pub language: String,
    pub bytes: u64,
    pub hash: String,
    pub symbol_count: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct ExportSymbolNode {
    pub id: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file_id: String,
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub column_start: Option<usize>,
    pub column_end: Option<usize>,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    pub is_exported: bool,
    pub parent_symbol_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ExportRange {
    pub line_start: usize,
    pub line_end: usize,
    pub column_start: Option<usize>,
    pub column_end: Option<usize>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ContainsEdge {
    pub id: String,
    pub from_id: String,
    pub to_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ImportEdge {
    pub id: String,
    pub file_id: String,
    pub path: String,
    pub import_name: String,
    pub from_file: Option<String>,
    pub target_file_id: Option<String>,
    pub target_symbol_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CallEdge {
    pub id: String,
    pub from_symbol_id: Option<String>,
    pub to_symbol_id: Option<String>,
    pub callee_name: String,
    pub resolution: String,
    pub path: String,
    pub callsite: ExportRange,
}

#[derive(Debug, Serialize)]
pub(crate) struct ImplementsEdge {
    pub id: String,
    pub from_symbol_id: Option<String>,
    pub to_symbol_id: Option<String>,
    pub interface_name: String,
    pub implementor_name: String,
    pub kind: String,
    pub path: String,
    pub range: ExportRange,
}

#[derive(Debug, Clone)]
struct SymbolRecord {
    index: usize,
    path: String,
    name: String,
    qualified_name: String,
    kind: String,
    parent_index: Option<usize>,
    line_start: usize,
    line_end: usize,
    signature: Option<String>,
    docstring: Option<String>,
    is_exported: bool,
    file_id: String,
    id: String,
}

struct ExportBuildContext<'a> {
    index: &'a CodeIndex,
    root_dir: PathBuf,
    file_id_by_path: HashMap<PathBuf, String>,
    file_path_by_path: HashMap<PathBuf, String>,
    symbol_id_by_index: HashMap<usize, String>,
    symbol_records: Vec<SymbolRecord>,
    diagnostics: Vec<ExportDiagnostic>,
}

pub(crate) fn to_json(index: &CodeIndex, options: ExportOptions<'_>) -> Result<String> {
    let export = build(index, options)?;
    serde_json::to_string_pretty(&export).map_err(Into::into)
}

pub(crate) fn build(index: &CodeIndex, options: ExportOptions<'_>) -> Result<GraphExport> {
    let root_dir = export_root_dir(options.root_path);
    let mut context = ExportBuildContext {
        index,
        root_dir: root_dir.clone(),
        file_id_by_path: HashMap::new(),
        file_path_by_path: HashMap::new(),
        symbol_id_by_index: HashMap::new(),
        symbol_records: Vec::new(),
        diagnostics: Vec::new(),
    };

    let files = build_file_nodes(&mut context);
    context.symbol_records = build_symbol_records(&context);
    assign_symbol_ids(&mut context.symbol_records);
    context.symbol_id_by_index = context
        .symbol_records
        .iter()
        .map(|record| (record.index, record.id.clone()))
        .collect();

    let symbols = build_symbol_nodes(&context);
    let contains = build_contains_edges(&context);
    let imports = build_import_edges(&context);
    let calls = build_call_edges(&mut context);
    let implements = build_implements_edges(&context)?;
    let edge_count = contains.len() + imports.len() + calls.len() + implements.len();

    let mut diagnostics = context.diagnostics;
    diagnostics.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.message.cmp(&b.message))
    });

    Ok(GraphExport {
        schema_version: 1,
        meta: ExportMeta {
            root: String::new(),
            root_name: root_name(&root_dir),
            generated_at: generated_at(options.include_generated_at),
            cm_version: options.cm_version.to_string(),
            languages: languages(&files),
            stats: ExportStats {
                files: files.len(),
                symbols: symbols.len(),
                edges: edge_count,
            },
        },
        nodes: ExportNodes { files, symbols },
        edges: ExportEdges {
            contains,
            imports,
            calls,
            implements,
        },
        diagnostics,
    })
}

fn build_file_nodes(context: &mut ExportBuildContext<'_>) -> Vec<ExportFileNode> {
    let mut files: Vec<ExportFileNode> = context
        .index
        .files()
        .map(|file| {
            let path = normalized_relative_path(&context.root_dir, &file.path);
            let id = prefixed_hash("file", &[&path]);
            context
                .file_id_by_path
                .insert(file.path.clone(), id.clone());
            context
                .file_path_by_path
                .insert(file.path.clone(), path.clone());
            ExportFileNode {
                id,
                path,
                language: file.language.as_str().to_string(),
                bytes: file.size,
                hash: file.hash.clone(),
                symbol_count: context.index.get_file_symbols(&file.path).len(),
            }
        })
        .collect();

    files.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.id.cmp(&b.id)));
    files
}

fn build_symbol_records(context: &ExportBuildContext<'_>) -> Vec<SymbolRecord> {
    context
        .index
        .all_symbols_with_indices()
        .filter(|(_, symbol)| !symbol.name.is_empty())
        .filter_map(|(index, symbol)| {
            let file_id = context.file_id_by_path.get(&symbol.file_path)?.clone();
            let path = context.file_path_by_path.get(&symbol.file_path)?.clone();
            let qualified_name = qualified_name(context.index, index);
            Some(SymbolRecord {
                index,
                path,
                name: symbol.name.clone(),
                qualified_name,
                kind: export_symbol_kind(symbol),
                parent_index: symbol.parent_id,
                line_start: symbol.line_start,
                line_end: symbol.line_end,
                signature: symbol.signature.clone(),
                docstring: symbol.docstring.clone(),
                is_exported: symbol.is_exported,
                file_id,
                id: String::new(),
            })
        })
        .collect()
}

fn assign_symbol_ids(records: &mut [SymbolRecord]) {
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();

    records.iter().enumerate().for_each(|(idx, record)| {
        let base = prefixed_hash("sym", &[&record.path, &record.qualified_name, &record.kind]);
        groups.entry(base).or_default().push(idx);
    });

    groups.into_iter().for_each(|(base, mut indices)| {
        indices.sort_by(|left, right| symbol_record_cmp(&records[*left], &records[*right]));
        indices.iter().enumerate().for_each(|(ordinal, idx)| {
            records[*idx].id = if ordinal == 0 {
                base.clone()
            } else {
                format!("{}_{}", base, ordinal + 1)
            };
        });
    });
}

fn symbol_record_cmp(left: &SymbolRecord, right: &SymbolRecord) -> std::cmp::Ordering {
    left.path
        .cmp(&right.path)
        .then_with(|| left.qualified_name.cmp(&right.qualified_name))
        .then_with(|| left.kind.cmp(&right.kind))
        .then_with(|| left.line_start.cmp(&right.line_start))
        .then_with(|| left.line_end.cmp(&right.line_end))
        .then_with(|| left.signature.cmp(&right.signature))
        .then_with(|| left.name.cmp(&right.name))
}

fn build_symbol_nodes(context: &ExportBuildContext<'_>) -> Vec<ExportSymbolNode> {
    let mut symbols: Vec<ExportSymbolNode> = context
        .symbol_records
        .iter()
        .map(|record| ExportSymbolNode {
            id: record.id.clone(),
            name: record.name.clone(),
            qualified_name: record.qualified_name.clone(),
            kind: record.kind.clone(),
            file_id: record.file_id.clone(),
            path: record.path.clone(),
            line_start: record.line_start,
            line_end: record.line_end,
            column_start: None,
            column_end: None,
            signature: record.signature.clone(),
            docstring: record.docstring.clone(),
            is_exported: record.is_exported,
            parent_symbol_id: record
                .parent_index
                .and_then(|idx| context.symbol_id_by_index.get(&idx).cloned()),
        })
        .collect();

    symbols.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.line_start.cmp(&b.line_start))
            .then_with(|| a.line_end.cmp(&b.line_end))
            .then_with(|| a.qualified_name.cmp(&b.qualified_name))
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.id.cmp(&b.id))
    });
    symbols
}

fn build_contains_edges(context: &ExportBuildContext<'_>) -> Vec<ContainsEdge> {
    let mut edges: Vec<ContainsEdge> = context
        .symbol_records
        .iter()
        .map(|record| {
            let from_id = record
                .parent_index
                .and_then(|idx| context.symbol_id_by_index.get(&idx).cloned())
                .unwrap_or_else(|| record.file_id.clone());
            let to_id = record.id.clone();
            ContainsEdge {
                id: prefixed_hash("edge", &["contains", &from_id, &to_id]),
                from_id,
                to_id,
            }
        })
        .collect();

    edges.sort_by(|a, b| a.id.cmp(&b.id));
    edges.dedup_by(|a, b| a.id == b.id);
    edges
}

fn build_import_edges(context: &ExportBuildContext<'_>) -> Vec<ImportEdge> {
    let mut seen = BTreeSet::new();
    let mut edges = Vec::new();
    let mut files: Vec<&FileInfo> = context.index.files().collect();
    files.sort_by_key(|file| normalized_relative_path(&context.root_dir, &file.path));

    files.iter().for_each(|file| {
        let Some(file_id) = context.file_id_by_path.get(&file.path) else {
            return;
        };
        let Some(path) = context.file_path_by_path.get(&file.path) else {
            return;
        };

        file.dependencies.iter().for_each(|dependency| {
            let from_file = dependency.from_file.clone();
            let seed = [
                "imports".to_string(),
                file_id.clone(),
                dependency.import_name.clone(),
                from_file.clone().unwrap_or_default(),
            ];
            if !seen.insert(seed.clone()) {
                return;
            }

            edges.push(ImportEdge {
                id: prefixed_hash(
                    "edge",
                    &[
                        "imports",
                        file_id,
                        &dependency.import_name,
                        from_file.as_deref().unwrap_or_default(),
                    ],
                ),
                file_id: file_id.clone(),
                path: path.clone(),
                import_name: dependency.import_name.clone(),
                from_file,
                target_file_id: None,
                target_symbol_id: None,
            });
        });
    });

    edges.sort_by(|a, b| a.id.cmp(&b.id));
    edges
}

fn build_call_edges(context: &mut ExportBuildContext<'_>) -> Vec<CallEdge> {
    let file_symbols = symbols_by_file(context);
    let mut seen = BTreeSet::new();
    let mut edges = Vec::new();
    let mut files: Vec<&FileInfo> = context.index.files().collect();
    files.sort_by_key(|file| normalized_relative_path(&context.root_dir, &file.path));

    files.into_iter().for_each(|file| {
        let path = context
            .file_path_by_path
            .get(&file.path)
            .cloned()
            .unwrap_or_else(|| normalized_relative_path(&context.root_dir, &file.path));
        let content = match fs::read_to_string(&file.path) {
            Ok(content) => content,
            Err(error) => {
                context.diagnostics.push(ExportDiagnostic {
                    severity: "warning".to_string(),
                    message: format!("failed to read file for call extraction: {error}"),
                    path: Some(path),
                });
                return;
            }
        };

        let calls = match callgraph::extract_calls_from_file(&content, &file.path, file.language) {
            Ok(calls) => calls,
            Err(error) => {
                context.diagnostics.push(ExportDiagnostic {
                    severity: "warning".to_string(),
                    message: format!("failed to extract calls: {error}"),
                    path: Some(path),
                });
                return;
            }
        };

        calls.into_iter().for_each(|(callee_name, line, _context)| {
            let from_index = enclosing_symbol_index(&file_symbols, &file.path, line);
            let from_symbol_id =
                from_index.and_then(|idx| context.symbol_id_by_index.get(&idx).cloned());
            let target_indices = context.index.query_symbol_indices(&callee_name);
            let (to_symbol_id, resolution) = match target_indices.as_slice() {
                [] => (None, "unresolved"),
                [idx] => (context.symbol_id_by_index.get(idx).cloned(), "internal"),
                _ => (None, "ambiguous"),
            };
            let from_seed = from_symbol_id.clone().unwrap_or_default();
            let to_seed = to_symbol_id.clone().unwrap_or_default();
            let key = (
                "calls".to_string(),
                from_seed.clone(),
                to_seed.clone(),
                callee_name.clone(),
                path.clone(),
                line,
                resolution.to_string(),
            );
            if !seen.insert(key) {
                return;
            }

            edges.push(CallEdge {
                id: prefixed_hash(
                    "edge",
                    &[
                        "calls",
                        &from_seed,
                        &to_seed,
                        &callee_name,
                        &path,
                        &line.to_string(),
                        resolution,
                    ],
                ),
                from_symbol_id,
                to_symbol_id,
                callee_name,
                resolution: resolution.to_string(),
                path: path.clone(),
                callsite: line_range(line),
            });
        });
    });

    edges.sort_by(|a, b| a.id.cmp(&b.id));
    edges
}

fn build_implements_edges(context: &ExportBuildContext<'_>) -> Result<Vec<ImplementsEdge>> {
    let implementations = implements::find_all_implementations(context.index, true)?;
    let mut seen = BTreeSet::new();
    let mut edges: Vec<ImplementsEdge> = implementations
        .into_iter()
        .filter_map(|implementation| {
            let key = implementation_key(&implementation);
            if !seen.insert(key) {
                return None;
            }
            let path = normalized_relative_path(&context.root_dir, &implementation.file_path);
            let from_symbol_id = resolve_named_symbol_id(context, &implementation.interface_name);
            let to_symbol_id = resolve_named_symbol_id(context, &implementation.implementor_name);
            Some(ImplementsEdge {
                id: prefixed_hash(
                    "edge",
                    &[
                        "implements",
                        implementation.kind.as_str(),
                        &implementation.interface_name,
                        &implementation.implementor_name,
                        &path,
                        &implementation.line.to_string(),
                    ],
                ),
                from_symbol_id,
                to_symbol_id,
                interface_name: implementation.interface_name,
                implementor_name: implementation.implementor_name,
                kind: implementation.kind.as_str().to_string(),
                path,
                range: line_range(implementation.line),
            })
        })
        .collect();

    edges.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(edges)
}

fn symbols_by_file(
    context: &ExportBuildContext<'_>,
) -> HashMap<PathBuf, Vec<(usize, usize, usize)>> {
    let mut by_file: HashMap<PathBuf, Vec<(usize, usize, usize)>> = HashMap::new();
    context
        .index
        .all_symbols_with_indices()
        .for_each(|(idx, symbol)| {
            by_file.entry(symbol.file_path.clone()).or_default().push((
                idx,
                symbol.line_start,
                symbol.line_end,
            ));
        });

    by_file.values_mut().for_each(|symbols| {
        symbols.sort_by(|a, b| {
            let a_len = a.2.saturating_sub(a.1);
            let b_len = b.2.saturating_sub(b.1);
            a_len
                .cmp(&b_len)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| a.0.cmp(&b.0))
        });
    });

    by_file
}

fn enclosing_symbol_index(
    symbols_by_file: &HashMap<PathBuf, Vec<(usize, usize, usize)>>,
    path: &Path,
    line: usize,
) -> Option<usize> {
    symbols_by_file.get(path).and_then(|symbols| {
        symbols
            .iter()
            .find(|(_, start, end)| *start <= line && *end >= line)
            .map(|(idx, _, _)| *idx)
    })
}

fn implementation_name_candidates(name: &str) -> Vec<String> {
    let trimmed = name.trim().strip_prefix("impl ").unwrap_or(name.trim());
    let mut candidates = vec![trimmed.to_string()];
    if let Some((_, tail)) = trimmed.rsplit_once("::") {
        candidates.push(tail.to_string());
    }
    if let Some((_, tail)) = trimmed.rsplit_once('.') {
        candidates.push(tail.to_string());
    }
    candidates
        .into_iter()
        .filter(|candidate| !candidate.is_empty())
        .collect()
}

fn implementation_key(implementation: &Implementation) -> (String, String, PathBuf, usize, String) {
    (
        implementation.implementor_name.clone(),
        implementation.interface_name.clone(),
        implementation.file_path.clone(),
        implementation.line,
        implementation.kind.as_str().to_string(),
    )
}

fn resolve_named_symbol_id(context: &ExportBuildContext<'_>, name: &str) -> Option<String> {
    let candidates = implementation_name_candidates(name);
    let ids: BTreeSet<String> = candidates
        .iter()
        .flat_map(|candidate| matching_symbol_ids(context, candidate))
        .collect();

    if ids.len() == 1 {
        ids.into_iter().next()
    } else {
        None
    }
}

fn matching_symbol_ids(context: &ExportBuildContext<'_>, name: &str) -> Vec<String> {
    let by_symbol_name = context
        .symbol_records
        .iter()
        .filter(|record| record.name == name)
        .map(|record| record.id.clone());
    let by_qualified_name = context
        .symbol_records
        .iter()
        .filter(|record| record.qualified_name == name)
        .map(|record| record.id.clone());

    by_symbol_name.chain(by_qualified_name).collect()
}

fn qualified_name(index: &CodeIndex, symbol_index: usize) -> String {
    let Some(symbol) = index.symbol_at(symbol_index) else {
        return String::new();
    };

    let mut parent_names = Vec::new();
    let mut current = symbol.parent_id;
    let mut seen = BTreeSet::new();

    while let Some(parent_index) = current {
        if !seen.insert(parent_index) {
            break;
        }
        let Some(parent) = index.symbol_at(parent_index) else {
            break;
        };
        if parent.file_path != symbol.file_path {
            break;
        }
        parent_names.push(parent.name.clone());
        current = parent.parent_id;
    }

    parent_names.reverse();
    if parent_names
        .last()
        .is_some_and(|parent| symbol.name.starts_with(parent))
    {
        return symbol.name.clone();
    }

    parent_names.push(symbol.name.clone());
    parent_names.join(".")
}

fn export_symbol_kind(symbol: &Symbol) -> String {
    let kind = match symbol.symbol_type {
        SymbolType::Function => "function",
        SymbolType::Class if symbol.name.starts_with("impl ") => "impl",
        SymbolType::Class => "class",
        SymbolType::Method => "method",
        SymbolType::Enum => "enum",
        SymbolType::StaticField => "field",
        SymbolType::Interface => interface_kind(symbol),
        SymbolType::TypeAlias => "unknown",
        SymbolType::Module => "module",
        SymbolType::Heading | SymbolType::CodeBlock | SymbolType::Endpoint => "unknown",
    };
    kind.to_string()
}

fn interface_kind(symbol: &Symbol) -> &'static str {
    match Language::from_path(&symbol.file_path) {
        Language::Rust | Language::Elixir => "trait",
        _ => "interface",
    }
}

fn line_range(line: usize) -> ExportRange {
    ExportRange {
        line_start: line,
        line_end: line,
        column_start: None,
        column_end: None,
    }
}

fn languages(files: &[ExportFileNode]) -> Vec<String> {
    files
        .iter()
        .map(|file| file.language.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn generated_at(include_generated_at: bool) -> Option<String> {
    include_generated_at.then(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| format!("unix:{}", duration.as_secs()))
            .unwrap_or_else(|_| "unix:0".to_string())
    })
}

fn export_root_dir(path: &Path) -> PathBuf {
    if path.is_file() {
        path.parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    } else {
        path.to_path_buf()
    }
}

fn root_name(root_dir: &Path) -> Option<String> {
    fs::canonicalize(root_dir)
        .unwrap_or_else(|_| root_dir.to_path_buf())
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
}

fn normalized_relative_path(root_dir: &Path, path: &Path) -> String {
    let canonical_root = fs::canonicalize(root_dir).ok();
    let canonical_path = fs::canonicalize(path).ok();

    if let (Some(root), Some(file)) = (canonical_root.as_deref(), canonical_path.as_deref()) {
        if let Ok(relative) = file.strip_prefix(root) {
            return path_to_posix(relative);
        }
    }

    if path.is_relative() {
        return path_to_posix(path);
    }

    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn path_to_posix(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_string()),
            Component::ParentDir => Some("..".to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn prefixed_hash(prefix: &str, parts: &[&str]) -> String {
    let mut hasher = blake3::Hasher::new();
    parts.iter().for_each(|part| {
        hasher.update(part.len().to_string().as_bytes());
        hasher.update(b":");
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    });
    let hash = hasher.finalize().to_hex().to_string();
    format!("{}_{}", prefix, &hash[..20])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::CodeIndex;
    use crate::indexer;
    use crate::models::Language;
    use tempfile::tempdir;

    #[test]
    fn exports_nested_symbols_imports_and_calls() -> Result<()> {
        let dir = tempdir()?;
        fs::write(
            dir.path().join("service.py"),
            "from helper import helper\n\nclass Service:\n    def run(self):\n        return helper()\n",
        )?;
        fs::write(
            dir.path().join("helper.py"),
            "def helper():\n    return 1\n",
        )?;

        let index = indexer::index_directory(dir.path(), &["py"])?;
        let graph = build(
            &index,
            ExportOptions {
                root_path: dir.path(),
                include_generated_at: false,
                cm_version: "test",
            },
        )?;

        assert_eq!(graph.schema_version, 1);
        assert_eq!(graph.meta.root, "");
        assert_eq!(graph.meta.generated_at, None);
        assert!(graph.nodes.files.iter().all(|file| {
            !file.path.starts_with('/')
                && !file.path.starts_with("./")
                && !file.id.starts_with("blake3")
        }));
        assert!(graph.edges.imports.iter().any(
            |edge| edge.import_name == "helper" && edge.from_file.as_deref() == Some("helper")
        ));

        let service = graph
            .nodes
            .symbols
            .iter()
            .find(|symbol| symbol.name == "Service")
            .expect("service symbol");
        let run = graph
            .nodes
            .symbols
            .iter()
            .find(|symbol| symbol.name == "run")
            .expect("run symbol");
        assert_eq!(run.parent_symbol_id.as_deref(), Some(service.id.as_str()));
        assert!(graph
            .edges
            .contains
            .iter()
            .any(|edge| edge.from_id == service.id && edge.to_id == run.id));
        assert!(graph.edges.calls.iter().any(|edge| {
            edge.from_symbol_id.as_deref() == Some(run.id.as_str())
                && edge.callee_name == "helper"
                && edge.resolution == "internal"
        }));

        Ok(())
    }

    #[test]
    fn go_struct_fields_do_not_export_implements_edges() -> Result<()> {
        let dir = tempdir()?;
        fs::write(
            dir.path().join("main.go"),
            "package main\n\nimport \"sync\"\n\ntype runner struct {\n    logsMu sync.Mutex\n}\n\ntype ord struct {\n    result string\n}\n",
        )?;

        let index = indexer::index_directory(dir.path(), &["go"])?;
        let graph = build(
            &index,
            ExportOptions {
                root_path: dir.path(),
                include_generated_at: false,
                cm_version: "test",
            },
        )?;

        assert!(graph.edges.implements.is_empty());
        Ok(())
    }

    #[test]
    fn stable_symbol_ids_do_not_depend_on_file_add_order() -> Result<()> {
        let dir = tempdir()?;
        let a_path = dir.path().join("a.py");
        let b_path = dir.path().join("b.py");
        fs::write(&a_path, "class A:\n    def run(self):\n        pass\n")?;
        fs::write(&b_path, "def helper():\n    pass\n")?;

        let a_content = fs::read_to_string(&a_path)?;
        let b_content = fs::read_to_string(&b_path)?;

        let mut forward = CodeIndex::new();
        forward.add_file(indexer::index_file(
            &a_path,
            &a_content,
            Language::Python,
            None,
        )?);
        forward.add_file(indexer::index_file(
            &b_path,
            &b_content,
            Language::Python,
            None,
        )?);

        let mut reverse = CodeIndex::new();
        reverse.add_file(indexer::index_file(
            &b_path,
            &b_content,
            Language::Python,
            None,
        )?);
        reverse.add_file(indexer::index_file(
            &a_path,
            &a_content,
            Language::Python,
            None,
        )?);

        let forward_graph = build(
            &forward,
            ExportOptions {
                root_path: dir.path(),
                include_generated_at: false,
                cm_version: "test",
            },
        )?;
        let reverse_graph = build(
            &reverse,
            ExportOptions {
                root_path: dir.path(),
                include_generated_at: false,
                cm_version: "test",
            },
        )?;

        assert_eq!(
            symbol_ids_by_key(&forward_graph),
            symbol_ids_by_key(&reverse_graph)
        );
        Ok(())
    }

    fn symbol_ids_by_key(graph: &GraphExport) -> BTreeMap<String, String> {
        graph
            .nodes
            .symbols
            .iter()
            .map(|symbol| {
                (
                    format!(
                        "{}:{}:{}:{}",
                        symbol.path, symbol.qualified_name, symbol.kind, symbol.line_start
                    ),
                    symbol.id.clone(),
                )
            })
            .collect()
    }
}

use ::ignore::WalkBuilder;
use anyhow::{Context, Result};
use grep::regex::RegexMatcher;
use grep::searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::ignore;
use crate::indexer::{detect_language, index_file, matches_extension_filter};
use crate::models::Symbol;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMatch {
    pub path: PathBuf,
    pub line_number: u64,
    pub line: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefilterStopReason {
    Exhausted,
    CandidateLimit,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefilterResult {
    pub candidates: Vec<PathBuf>,
    pub elapsed: Duration,
    pub stop_reason: PrefilterStopReason,
}

/// Fast text search using ripgrep-style grep for prefiltering candidate files
pub struct GrepFilter {
    pattern: String,
    case_sensitive: bool,
    extensions: Vec<String>,
}

/// Collects file paths that match the grep pattern
struct CandidateCollector {
    files: Vec<PathBuf>,
    current_path: Option<PathBuf>,
}

struct TextMatchCollector {
    matches: Vec<TextMatch>,
    current_path: Option<PathBuf>,
    limit: Option<usize>,
}

impl CandidateCollector {
    fn new() -> Self {
        Self {
            files: Vec::new(),
            current_path: None,
        }
    }

    fn set_path(&mut self, path: PathBuf) {
        self.current_path = Some(path);
    }
}

impl Sink for CandidateCollector {
    type Error = std::io::Error;

    fn matched(&mut self, _searcher: &Searcher, _mat: &SinkMatch) -> Result<bool, Self::Error> {
        // Add the file to our collection on first match
        if let Some(path) = self.current_path.take() {
            self.files.push(path);
        }
        // Return Ok(false) to stop searching this file after first match
        Ok(false)
    }
}

impl TextMatchCollector {
    fn new(limit: Option<usize>) -> Self {
        Self {
            matches: Vec::new(),
            current_path: None,
            limit,
        }
    }

    fn set_path(&mut self, path: PathBuf) {
        self.current_path = Some(path);
    }

    fn is_done(&self) -> bool {
        self.limit
            .map(|limit| self.matches.len() >= limit)
            .unwrap_or(false)
    }
}

impl Sink for TextMatchCollector {
    type Error = std::io::Error;

    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch) -> Result<bool, Self::Error> {
        if self.is_done() {
            return Ok(false);
        }

        if let Some(path) = self.current_path.clone() {
            self.matches.push(TextMatch {
                path,
                line_number: mat.line_number().unwrap_or(0),
                line: String::from_utf8_lossy(mat.bytes())
                    .trim_end_matches(&['\r', '\n'][..])
                    .to_string(),
            });
        }

        Ok(!self.is_done())
    }
}

fn prefilter_chunk_size() -> usize {
    (rayon::current_num_threads() * 8).clamp(32, 256)
}

fn deadline_expired(deadline: Option<Instant>) -> bool {
    deadline
        .map(|deadline| Instant::now() >= deadline)
        .unwrap_or(false)
}

impl GrepFilter {
    /// Create a new GrepFilter
    pub fn new(pattern: &str, case_sensitive: bool, extensions: Vec<String>) -> Self {
        Self {
            pattern: pattern.to_string(),
            case_sensitive,
            extensions,
        }
    }

    /// Stage 1: Fast text search to find candidate files
    /// Returns list of files that contain the pattern (supports | OR syntax)
    pub fn prefilter_paths_with_budget(
        &self,
        paths: &[PathBuf],
        limit: Option<usize>,
        time_budget: Option<Duration>,
    ) -> Result<PrefilterResult> {
        let started = Instant::now();
        if limit == Some(0) {
            return Ok(PrefilterResult {
                candidates: Vec::new(),
                elapsed: started.elapsed(),
                stop_reason: PrefilterStopReason::CandidateLimit,
            });
        }

        let pattern = self.prefilter_pattern();
        let matcher = RegexMatcher::new(&pattern).context("Failed to create regex matcher")?;
        let max_matches = limit.unwrap_or(usize::MAX);
        let deadline = time_budget.map(|budget| started + budget);
        let mut candidates = Vec::new();
        let mut stop_reason = PrefilterStopReason::Exhausted;

        for chunk in paths.chunks(prefilter_chunk_size()) {
            if candidates.len() >= max_matches {
                stop_reason = PrefilterStopReason::CandidateLimit;
                break;
            }

            if deadline_expired(deadline) {
                stop_reason = PrefilterStopReason::TimedOut;
                break;
            }

            let remaining = max_matches.saturating_sub(candidates.len());
            let found_count = AtomicUsize::new(0);
            let timed_out = AtomicBool::new(false);
            let mut chunk_candidates: Vec<PathBuf> = chunk
                .par_iter()
                .filter_map(|path| {
                    if found_count.load(Ordering::Relaxed) >= remaining {
                        return None;
                    }

                    if deadline_expired(deadline) {
                        timed_out.store(true, Ordering::Relaxed);
                        return None;
                    }

                    let mut collector = CandidateCollector::new();
                    let mut searcher = SearcherBuilder::new()
                        .binary_detection(BinaryDetection::quit(b'\x00'))
                        .line_number(false)
                        .build();

                    collector.set_path(path.clone());
                    let _ = searcher.search_path(&matcher, path, &mut collector);
                    if deadline_expired(deadline) {
                        timed_out.store(true, Ordering::Relaxed);
                    }
                    if collector.files.is_empty() {
                        return None;
                    }

                    let slot = found_count.fetch_add(1, Ordering::Relaxed);
                    if slot < remaining {
                        Some(path.clone())
                    } else {
                        None
                    }
                })
                .collect();

            candidates.append(&mut chunk_candidates);
            if candidates.len() >= max_matches {
                stop_reason = PrefilterStopReason::CandidateLimit;
                break;
            }

            if timed_out.load(Ordering::Relaxed) || deadline_expired(deadline) {
                stop_reason = PrefilterStopReason::TimedOut;
                break;
            }
        }

        candidates.sort();

        Ok(PrefilterResult {
            candidates,
            elapsed: started.elapsed(),
            stop_reason,
        })
    }

    fn prefilter_pattern(&self) -> String {
        let terms: Vec<&str> = self
            .pattern
            .split('|')
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();
        if terms.len() > 1 {
            let escaped: Vec<String> = terms.iter().map(|t| regex::escape(t)).collect();
            let alternation = escaped.join("|");
            if self.case_sensitive {
                alternation
            } else {
                format!("(?i){}", alternation)
            }
        } else if self.case_sensitive {
            self.pattern.clone()
        } else {
            format!("(?i){}", regex::escape(&self.pattern))
        }
    }

    /// Check if file extension matches our filter
    fn matches_extension(&self, path: &Path) -> bool {
        matches_extension_filter(path, &self.extensions)
    }

    pub fn search_text(&self, root: &Path, limit: Option<usize>) -> Result<Vec<TextMatch>> {
        let pattern = if self.case_sensitive {
            self.pattern.clone()
        } else {
            format!("(?i){}", self.pattern)
        };
        let matcher = RegexMatcher::new(&pattern).context("Failed to create regex matcher")?;
        let mut collector = TextMatchCollector::new(limit);
        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(b'\x00'))
            .line_number(true)
            .build();

        if root.is_file() {
            if self.matches_extension(root) {
                collector.set_path(root.to_path_buf());
                let _ = searcher.search_path(&matcher, root, &mut collector);
            }
            return Ok(collector.matches);
        }

        let walker = WalkBuilder::new(root)
            .hidden(false)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(false)
            .filter_entry(|e| {
                if e.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    let name = e.file_name().to_string_lossy();
                    !ignore::is_ignored_dir(&name)
                } else {
                    true
                }
            })
            .build();

        for entry in walker {
            if collector.is_done() {
                break;
            }

            let entry = entry.context("Failed to read directory entry")?;
            if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                continue;
            }

            let path = entry.path();
            if !self.matches_extension(path) {
                continue;
            }

            collector.set_path(path.to_path_buf());
            let _ = searcher.search_path(&matcher, path, &mut collector);
        }

        Ok(collector.matches)
    }

    /// Stage 2: AST validation of candidate files
    /// Parse only candidate files and extract matching symbols
    pub fn validate(
        &self,
        candidates: Vec<PathBuf>,
        query: &str,
        fuzzy: bool,
    ) -> Result<Vec<Symbol>> {
        let symbol_batches: Vec<Vec<Symbol>> = candidates
            .par_iter()
            .map(|path| {
                let content = match fs::read_to_string(path) {
                    Ok(content) => content,
                    Err(_) => return Vec::new(),
                };

                let language = detect_language(path);
                let file_info = match index_file(path, &content, language, None) {
                    Ok(file_info) => file_info,
                    Err(_) => return Vec::new(),
                };

                file_info
                    .symbols
                    .into_iter()
                    .filter(|symbol| self.symbol_matches(&symbol.name, query, fuzzy))
                    .collect()
            })
            .collect();

        Ok(symbol_batches.into_iter().flatten().collect())
    }

    /// Check if a symbol name matches the query (supports | OR syntax)
    fn symbol_matches(&self, name: &str, query: &str, fuzzy: bool) -> bool {
        let terms: Vec<&str> = query
            .split('|')
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();
        let terms = if terms.is_empty() { vec![query] } else { terms };
        terms.iter().any(|term| {
            if fuzzy {
                name.to_lowercase().contains(&term.to_lowercase())
            } else {
                name == *term
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grep_filter_creation() {
        let filter = GrepFilter::new("test", true, vec!["rs".to_string()]);
        assert_eq!(filter.pattern, "test");
        assert!(filter.case_sensitive);
        assert_eq!(filter.extensions.len(), 1);
    }

    #[test]
    fn test_matches_extension() {
        let filter = GrepFilter::new("test", true, vec!["rs".to_string(), "py".to_string()]);

        let path_rs = Path::new("test.rs");
        let path_py = Path::new("test.py");
        let path_js = Path::new("test.js");
        let path_gemfile = Path::new("Gemfile");

        assert!(filter.matches_extension(path_rs));
        assert!(filter.matches_extension(path_py));
        assert!(!filter.matches_extension(path_js));

        let ruby_filter = GrepFilter::new("test", true, vec!["gemfile".to_string()]);
        assert!(ruby_filter.matches_extension(path_gemfile));
    }

    #[test]
    fn test_prefilter_respects_limit() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(temp.path().join("first.rs"), "fn Needle() {}\n").unwrap();
        fs::write(temp.path().join("second.rs"), "fn Needle() {}\n").unwrap();

        let filter = GrepFilter::new("Needle", true, vec!["rs".to_string()]);
        let paths = vec![temp.path().join("first.rs"), temp.path().join("second.rs")];
        let result = filter
            .prefilter_paths_with_budget(&paths, Some(1), None)
            .unwrap();

        assert_eq!(result.candidates.len(), 1);
    }

    #[test]
    fn test_prefilter_reports_candidate_limit() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(temp.path().join("first.rs"), "fn Needle() {}\n").unwrap();
        fs::write(temp.path().join("second.rs"), "fn Needle() {}\n").unwrap();

        let filter = GrepFilter::new("Needle", true, vec!["rs".to_string()]);
        let paths = vec![temp.path().join("first.rs"), temp.path().join("second.rs")];
        let result = filter
            .prefilter_paths_with_budget(&paths, Some(1), None)
            .unwrap();

        assert_eq!(result.candidates.len(), 1);
        assert_eq!(result.stop_reason, PrefilterStopReason::CandidateLimit);
    }

    #[test]
    fn test_prefilter_reports_timeout() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(temp.path().join("first.rs"), "fn Needle() {}\n").unwrap();

        let filter = GrepFilter::new("Needle", true, vec!["rs".to_string()]);
        let paths = vec![temp.path().join("first.rs")];
        let result = filter
            .prefilter_paths_with_budget(&paths, Some(1), Some(Duration::from_millis(0)))
            .unwrap();

        assert!(result.candidates.is_empty());
        assert_eq!(result.stop_reason, PrefilterStopReason::TimedOut);
    }

    #[test]
    fn test_search_text_returns_rg_style_lines() {
        let temp = tempfile::TempDir::new().unwrap();
        let first = temp.path().join("first.rs");
        let second = temp.path().join("second.rs");
        fs::write(&first, "fn main() {}\n// todo: fix\n").unwrap();
        fs::write(&second, "// future work\nfn done() {}\n").unwrap();

        let filter = GrepFilter::new("todo|future", true, vec!["rs".to_string()]);
        let matches = filter.search_text(temp.path(), None).unwrap();

        assert_eq!(matches.len(), 2);
        assert!(matches
            .iter()
            .any(|m| m.path == first && m.line_number == 2 && m.line == "// todo: fix"));
        assert!(matches
            .iter()
            .any(|m| m.path == second && m.line_number == 1 && m.line == "// future work"));
    }

    #[test]
    fn test_search_text_respects_limit() {
        let temp = tempfile::TempDir::new().unwrap();
        let path = temp.path().join("test.rs");
        fs::write(&path, "// todo one\n// todo two\n").unwrap();

        let filter = GrepFilter::new("todo", true, vec!["rs".to_string()]);
        let matches = filter.search_text(temp.path(), Some(1)).unwrap();

        assert_eq!(matches.len(), 1);
    }
}

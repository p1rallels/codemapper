use anyhow::{Context, Result};
use grep::regex::RegexMatcher;
use grep::searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};
use rayon::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::indexer::{detect_language, index_file};
use crate::models::Symbol;

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
}

/// Collects file paths that match the grep pattern
struct CandidateCollector {
    files: Vec<PathBuf>,
    current_path: Option<PathBuf>,
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
    pub fn new(pattern: &str, case_sensitive: bool) -> Self {
        Self {
            pattern: pattern.to_string(),
            case_sensitive,
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
            let timed_out = AtomicBool::new(false);
            let mut chunk_candidates: Vec<PathBuf> = chunk
                .par_iter()
                .filter_map(|path| {
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
                        None
                    } else {
                        Some(path.clone())
                    }
                })
                .collect();

            chunk_candidates.sort();
            chunk_candidates.truncate(remaining);
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
                name.contains(*term)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grep_filter_creation() {
        let filter = GrepFilter::new("test", true);
        assert_eq!(filter.pattern, "test");
        assert!(filter.case_sensitive);
    }

    #[test]
    fn test_prefilter_respects_limit() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(temp.path().join("first.rs"), "fn Needle() {}\n").unwrap();
        fs::write(temp.path().join("second.rs"), "fn Needle() {}\n").unwrap();

        let filter = GrepFilter::new("Needle", true);
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

        let filter = GrepFilter::new("Needle", true);
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

        let filter = GrepFilter::new("Needle", true);
        let paths = vec![temp.path().join("first.rs")];
        let result = filter
            .prefilter_paths_with_budget(&paths, Some(1), Some(Duration::from_millis(0)))
            .unwrap();

        assert!(result.candidates.is_empty());
        assert_eq!(result.stop_reason, PrefilterStopReason::TimedOut);
    }

    #[test]
    fn test_symbol_matches_case_sensitive_substrings() {
        let filter = GrepFilter::new("Parser", true);

        assert!(filter.symbol_matches("RustParser", "Parser", false));
        assert!(!filter.symbol_matches("rustparser", "Parser", false));
    }
}

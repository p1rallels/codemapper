mod cache;
mod fast_search;
mod index;
mod indexer;
mod models;
mod output;
mod parser;

use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::*;
use cache::FileChangeKind;
use indicatif::{ProgressBar, ProgressStyle};
use models::Symbol;
use output::{OutputFormat, OutputFormatter};
use rayon::prelude::*;
use std::path::PathBuf;
use std::time::Instant;

#[derive(clap::Parser)]
#[command(name = "cm")]
#[command(
    about = "CodeMapper (cm) - Fast Code Analysis Tool

WHAT IT DOES:
  Instantly analyze codebases by parsing source files into symbols (functions, classes, methods)
  using tree-sitter AST parsing. Works entirely in-memory with no database overhead.

=== QUICKSTART FOR LLMs (Step-by-Step to Orient Yourself) ===

STEP 1: Get the lay of the land (10 seconds)
  $ cm stats . --format ai
  → See: How many files, what languages, symbol counts
  → Tells you: Project size and composition

STEP 2: Map the project structure (15 seconds)
  $ cm map . --level 2 --format ai
  → See: All files with symbol counts per file
  → Tells you: Where the code lives, which files are important

STEP 3: Find what you're looking for (5 seconds)
  $ cm query <symbol_name> . --fuzzy --format ai
  → See: All functions/classes matching your search
  → Tells you: Exact locations (file:line)

STEP 4 (Optional): Deep dive into specific files
  $ cm inspect ./path/to/file.py --format ai
  → See: All symbols in that file with signatures

STEP 5 (Optional): Understand dependencies
  $ cm deps <symbol_or_file> . --direction used-by --format ai
  → See: Where a symbol/file is used across the codebase

PRO TIPS FOR LLMs:
  • Always use --format ai (most token-efficient, easy to parse)
  • Use --fuzzy for flexible searches (auth finds authenticate, Authorization, etc.)
  • Small repos (< 300ms): No cache overhead, always fast
  • Large repos: First run builds cache (~10s), cache hits ~0.5s, incremental rebuilds ~1s
  • Cache validates automatically - no need to manually invalidate
  • Incremental updates are 45-55x faster than full reindex
  • No .codemapper/ clutter on small projects - caching is smart!
  • Start with stats → map → query workflow for any new codebase

COMMAND REFERENCE:
  stats   - File counts, symbol breakdown (START HERE)
  map     - Project structure at different detail levels (1-3)
  query   - Find symbols by name (use --fuzzy for flexibility)
  inspect - See all symbols in a specific file
  deps    - Track imports and find usage locations
  index   - Validate indexing (rarely needed)

OUTPUT FORMATS:
  --format default  → Markdown (readable, structured)
  --format human    → Tables (best for terminal viewing)
  --format ai       → Compact (token-efficient for LLMs)

SUPPORTED LANGUAGES:
  Python, JavaScript, TypeScript, Rust, Java, Go, C, Markdown

PERFORMANCE:
  • Small projects (< 100 files): < 20ms cold start
  • Large projects (1000+ files): Auto-enables fast mode (10-100x speedup)
  • Example: 18,457 files in 1.2s vs 76s (63x faster)

SMART CACHING (Automatic + Threshold-based):
  • Cache is ONLY created when indexing takes ≥ 300ms (automatic, no config needed)
  • Small repos (< 300ms): No cache created - runs are already fast enough!
  • Large repos (≥ 300ms): Auto-caches on first run, then loads from cache
  • Cache location: <project_root>/.codemapper/cache/ (auto-added to .gitignore)

  CACHE BEHAVIOR:
  ✓ Small repos (< 100 files, < 300ms): No cache, re-index every time (imperceptible)
  ✓ First run on large repo: Parses all files, creates cache if ≥ 300ms
  ✓ Cache hit (no changes): Instant load ~0.5s (21k files validated in < 1s)
  ✓ Incremental update (1 file changed): ~1s (validates all, re-parses changed, 45-55x faster)
  ✓ Incremental update (few files changed): Scales linearly with changed file count
  ✓ Major changes (>10% files): Auto-rebuilds entire cache for consistency

  VALIDATION STRATEGY:
  • Skips ignored directories (.git, node_modules, __pycache__, target, dist, build)
  • Uses mtime + size pre-filter (like git) - only hashes files that look changed
  • Detects new files, deleted files, and modified files
  • Smart enough to skip false alarms (mtime changed but content didn't)
  • Blake3 hashing for fast, secure file change detection

  CACHE FLAGS:
  --no-cache        Skip cache entirely, always reindex (useful for benchmarking)
  --rebuild-cache   Force rebuild cache from scratch (use after git operations)

  WHY THE 300ms THRESHOLD?:
  Below 300ms, re-indexing is faster than cache validation overhead. Above 300ms,
  caching saves significant time. This is automatic - repos that grow past 300ms
  will start caching automatically. Small repos stay clean with no .codemapper/ clutter!

EXAMPLES:
  cm stats /my/project              # Quick codebase overview
  cm map . --level 2 --format human # File listing with symbol counts
  cm query authenticate --fuzzy     # Find authentication functions
  cm inspect ./auth.py --show-body  # See all symbols in auth.py
  cm deps User --direction used-by  # Find where User class is used
"
)]
struct Cli {
    /// Output format: 'default' (markdown), 'human' (tables), 'ai' (token-efficient)
    #[arg(short, long, global = true, default_value = "default")]
    format: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// [DISCOVERY] Quick codebase overview - see file counts and symbol totals
    #[command(
        about = "Display statistics: file counts, symbol breakdown, and parse performance",
        long_about = "USE CASE: Start here when exploring a new codebase
  • See how many files and what languages are present
  • Understand symbol distribution (functions vs classes vs methods)
  • Verify that files are being indexed correctly
  • Results are cached ONLY if indexing takes ≥ 300ms (automatic)

SMART CACHE BEHAVIOR:
  • Small repos (< 300ms): No cache created - always fast, no .codemapper/ clutter
  • Large repos (≥ 300ms): Cache created on first run, then loads instantly
  • Subsequent runs: Validates cache in ~1s, loads instantly if no changes
  • File changes: Auto-detects and re-parses only modified files (~2s)
  • Cache location: .codemapper/cache/ in project root (only created when needed)

TIP: Fastest way to understand codebase size and composition"
    )]
    #[command(after_help = "EXAMPLES:
  cm stats                           # Analyze current directory (smart caching)
  cm stats /path/to/project          # Analyze specific project (auto-caches if slow)
  cm stats . --format human          # Pretty tables for terminal
  cm stats . --extensions py,rs      # Only Python and Rust files
  cm stats . --rebuild-cache         # Force fresh rebuild (may skip cache if fast)
  cm stats . --no-cache              # Skip cache, always reindex (benchmarking)

TYPICAL WORKFLOW:
  1. Run 'cm stats .' first to understand the codebase
  2. Small repo? No cache created, runs stay fast (< 300ms)
  3. Large repo? Cache created, subsequent runs < 1s
  4. Changed files? Auto-detected and re-parsed (~2s validation overhead)
  5. Then use 'cm map' for structure or 'cm query' to find symbols")]
    Stats {
        /// Directory path to analyze
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,md')
        #[arg(long, default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,md")]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache (invalidate and reindex)
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [DISCOVERY] Hierarchical project structure - from overview to detailed symbol listings
    #[command(
        about = "Generate a map showing project organization at different detail levels (1-3)",
        long_about = "USE CASE: Visualize how your codebase is organized
  • Level 1: High-level overview (languages, totals) - START HERE
  • Level 2: File listing with symbol counts per file
  • Level 3: Complete catalog with all symbol signatures
  • Results are cached for instant loading on subsequent runs

WHEN TO USE WHICH LEVEL:
  Level 1 → Getting oriented in a new project
  Level 2 → Finding which files contain what you need
  Level 3 → Comprehensive reference (warning: verbose for large projects)

TIP: Use --format human for best terminal readability"
    )]
    #[command(after_help = "EXAMPLES:
  cm map . --level 1                    # Quick overview (languages, counts)
  cm map . --level 2                    # File listing with symbol counts
  cm map . --level 3                    # Full symbol signatures (verbose)
  cm map ./src --level 2 --format human # Pretty tables for src/ directory
  cm map . --level 2 --format ai        # Token-efficient for LLM context

TYPICAL WORKFLOW:
  1. Start with level 1 to see the big picture
  2. Use level 2 to find relevant files
  3. Then 'cm inspect <file>' or 'cm query <symbol>' for details")]
    Map {
        /// Directory path to map
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Detail level: 1=overview, 2=files, 3=symbols with details
        #[arg(long, default_value = "1", value_parser = clap::value_parser!(u8).range(1..=3))]
        level: u8,

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,md')
        #[arg(long, default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,md")]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache (invalidate and reindex)
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [SEARCH] Find symbols by name - the main workhorse for code exploration
    #[command(
        about = "Search for functions, classes, and methods across your codebase",
        long_about = "USE CASE: The primary command for finding code
  • Exact search: Find 'authenticate' (case-sensitive)
  • Fuzzy search: Find 'auth' matches 'authenticate', 'Authorization', etc.
  • Fast mode: Auto-enables for 1000+ files (10-100x speedup)
  • Results are cached for instant loading (< 10ms for most projects)

SEARCH MODES:
  Exact   → cm query MyClass              (case-sensitive, precise)
  Fuzzy   → cm query myclass --fuzzy      (case-insensitive, flexible)

CONTEXT OPTIONS:
  --context minimal → Signatures only (default, fast)
  --context full    → Includes docstrings and metadata
  --show-body       → Show actual code implementation

PERFORMANCE (Fast Mode):
  • 18,457 files: 76s → 1.2s (63x faster)
  • 17,005 files: 122s → 9.6s (12x faster)
  • Auto-enabled for 1000+ files
  • Two-stage: ripgrep text search → AST validation

TIP: Start with fuzzy search, it's more forgiving"
    )]
    #[command(after_help = "EXAMPLES:
  # Basic searches
  cm query authenticate                      # Exact match (case-sensitive)
  cm query auth --fuzzy                      # Fuzzy search (flexible)

  # With context
  cm query process_payment --context full    # Include docstrings
  cm query validate --show-body              # Show implementation

  # Fast mode (for large codebases)
  cm query MyClass /large/repo --fast        # Explicit fast mode
  cm query auth /monorepo --fuzzy            # Auto-enabled for 1000+ files

  # Output formats
  cm query Parser --format human             # Pretty tables
  cm query CodeIndex --format ai             # Token-efficient for LLMs

TYPICAL WORKFLOW:
  1. Quick fuzzy search: cm query auth --fuzzy
  2. Get more context: cm query authenticate --context full
  3. See implementation: cm query authenticate --show-body
  4. Find usage: cm deps authenticate --direction used-by

WHEN TO USE:
  ✓ \"Where is the authenticate function?\"
  ✓ \"Find all parser-related code\"
  ✓ \"What methods does the User class have?\"
  ✓ \"Show me the validate_input implementation\"")]
    Query {
        /// Symbol name to search for (function, class, or method name)
        symbol: String,

        /// Directory path to search in
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Enable fuzzy matching for flexible search (recommended)
        #[arg(long, default_value = "false")]
        fuzzy: bool,

        /// Context level: 'minimal' (signatures only) or 'full' (includes docstrings)
        #[arg(long, default_value = "minimal")]
        context: String,

        /// Show the actual code implementation in results
        #[arg(long, default_value = "false")]
        show_body: bool,

        /// Enable fast mode explicitly (auto-enabled for 1000+ files)
        #[arg(long, default_value = "false")]
        fast: bool,

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,md')
        #[arg(long, default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,md")]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache (invalidate and reindex)
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [SEARCH] Explore a single file in detail - see all symbols with their signatures
    #[command(
        about = "Analyze one file and list all functions, classes, and methods it contains",
        long_about = "USE CASE: Deep dive into a specific file
  • See all symbols defined in a single file
  • Understand file organization and structure
  • Review function signatures and documentation

WHEN TO USE:
  → You know the file but want to see what's inside
  → Reviewing a file before making changes
  → Understanding a specific module's API

VS. QUERY: Use inspect for \"show me this file\", use query for \"find this symbol\"

TIP: Combine with --show-body to see implementations"
    )]
    #[command(after_help = "EXAMPLES:
  cm inspect ./src/main.rs                 # List all symbols in main.rs
  cm inspect ./auth.py --show-body         # Show implementations
  cm inspect ./parser.rs --format human    # Pretty table format
  cm inspect ./utils.js --format ai        # Token-efficient output

TYPICAL WORKFLOW:
  1. Use 'cm map --level 2' to find interesting files
  2. Inspect specific files: cm inspect ./path/to/file.py
  3. Then query symbols or check dependencies

WHEN TO USE:
  ✓ \"What functions are in auth.py?\"
  ✓ \"Show me everything in this module\"
  ✓ \"What's the structure of parser.rs?\"")]
    Inspect {
        /// Path to the file to analyze
        file_path: PathBuf,

        /// Show the actual code implementation for each symbol
        #[arg(long, default_value = "false")]
        show_body: bool,
    },

    /// [ANALYSIS] Track dependencies - see what imports what, or find all usages
    #[command(
        about = "Analyze import relationships and symbol usage across the codebase",
        long_about = "USE CASE: Understand code relationships and dependencies
  • For files: See imports and reverse dependencies
  • For symbols: Find all places where they're used

TWO MODES:
  imports  → What does this file/symbol import?
  used-by  → What files use this symbol? (most useful!)

FILE ANALYSIS:
  cm deps ./auth.py                       → Shows auth.py's imports
  cm deps ./auth.py --direction used-by   → Shows files importing auth.py

SYMBOL ANALYSIS:
  cm deps authenticate --direction used-by → Find all authenticate() calls
  cm deps User --direction used-by        → Find where User class is used

LIMITATIONS:
  • Text-based search for usages (not full AST call graph yet)
  • May find false positives in comments/strings

TIP: Great for impact analysis before refactoring"
    )]
    #[command(after_help = "EXAMPLES:
  # File dependencies
  cm deps ./src/auth.py                           # What does auth.py import?
  cm deps ./utils.js --direction used-by          # What imports utils.js?

  # Symbol usage (requires --direction used-by)
  cm deps authenticate --direction used-by        # Find all authenticate() calls
  cm deps User --direction used-by                # Where is User class used?
  cm deps process_payment --direction used-by     # Track payment processing usage

  # Output formats
  cm deps CodeIndex --direction used-by --format human  # Pretty tables
  cm deps ./main.rs --format ai                         # Token-efficient

TYPICAL WORKFLOW:
  1. Find symbol: cm query MyClass --fuzzy
  2. Check usage: cm deps MyClass --direction used-by
  3. Review files that use it
  4. Make informed changes

WHEN TO USE:
  ✓ \"What files import this module?\"
  ✓ \"Where is this function called?\"
  ✓ \"Safe to refactor this class?\" (check used-by first)
  ✓ \"What does this file depend on?\"")]
    Deps {
        /// File path (./src/auth.py) or symbol name (authenticate)
        target: String,

        /// Directory path to search in
        #[arg(default_value = ".")]
        path: PathBuf,

        /// 'imports' (dependencies) or 'used-by' (reverse dependencies/usages)
        #[arg(long, default_value = "imports")]
        direction: String,

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,md')
        #[arg(long, default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,md")]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache (invalidate and reindex)
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [UTILITY] Validate indexing - mostly for testing and debugging
    #[command(
        about = "Test that files can be indexed correctly (reports file count and timing)",
        long_about = "USE CASE: Verify CodeMapper can parse your files
  • Validate indexing works on your codebase
  • Check parsing performance
  • Debug file detection issues

NOTE: This command doesn't cache results yet (future feature)

RARELY NEEDED: Most users should use 'stats', 'map', or 'query' instead

TIP: Use this to verify file extensions are being detected"
    )]
    #[command(after_help = "EXAMPLES:
  cm index                          # Index current directory
  cm index /path/to/project         # Index specific project
  cm index . --extensions py,rs     # Only Python and Rust files

WHEN TO USE:
  ✓ Testing CodeMapper on a new language/extension
  ✓ Debugging why files aren't being found
  ✓ Benchmarking parse performance

NOTE: For normal usage, use 'cm stats' or 'cm map' instead")]
    Index {
        /// Directory path to index
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,md')
        #[arg(long, default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,md")]
        extensions: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let format = OutputFormat::from_str(&cli.format).unwrap_or_else(|| {
        eprintln!("{} Invalid format '{}', using default", "Warning:".yellow(), cli.format);
        OutputFormat::Default
    });

    match cli.command {
        Commands::Stats { path, extensions, no_cache, rebuild_cache } => {
            cmd_stats(path, extensions, no_cache, rebuild_cache, format)?;
        }
        Commands::Map { path, level, extensions, no_cache, rebuild_cache } => {
            cmd_map(path, level, extensions, no_cache, rebuild_cache, format)?;
        }
        Commands::Query { symbol, path, fuzzy, context, show_body, fast, extensions, no_cache, rebuild_cache } => {
            cmd_query(symbol, path, context, fuzzy, fast, show_body, extensions, no_cache, rebuild_cache, format)?;
        }
        Commands::Inspect { file_path, show_body } => {
            cmd_inspect(file_path, show_body, format)?;
        }
        Commands::Deps { target, path, direction, extensions, no_cache, rebuild_cache } => {
            cmd_deps(target, path, direction, extensions, no_cache, rebuild_cache, format)?;
        }
        Commands::Index { path, extensions } => {
            cmd_index(path, extensions)?;
        }
    }

    Ok(())
}

/// Auto-rebuild wrapper: Try cache first, rebuild if needed
fn try_load_or_rebuild(
    path: &PathBuf,
    extensions: &[&str],
    no_cache: bool,
    rebuild_cache: bool,
) -> Result<index::CodeIndex> {
    use cache::CacheManager;

    // Skip cache if flags set
    if no_cache || rebuild_cache {
        if rebuild_cache {
            eprintln!("{} Rebuilding cache (--rebuild-cache)", "→".cyan());
            CacheManager::invalidate(path, extensions).ok(); // Ignore errors
        }
        let start = Instant::now();
        let index = indexer::index_directory(path, extensions)?;
        let elapsed_ms = start.elapsed().as_millis();

        // Save to cache only if indexing took >= 300ms (unless --no-cache)
        if !no_cache && elapsed_ms >= 300 {
            match CacheManager::save(&index, path, extensions) {
                Ok(_) => eprintln!("{} Cached index for future use ({}ms)",
                    "✓".green(), elapsed_ms),
                Err(e) => eprintln!("{} Warning: Failed to save cache: {}",
                    "⚠".yellow(), e),
            }
        } else if !no_cache && elapsed_ms < 300 {
            eprintln!("{} Indexed in {}ms (no cache needed for small repos)",
                "✓".green(), elapsed_ms);
        }

        return Ok(index);
    }

    // Try to load from cache
    match CacheManager::load(path, extensions) {
        Ok(Some((index, metadata, changed_files))) if changed_files.is_empty() => {
            // Cache hit - no changes
            eprintln!("{} Loaded from cache - {} files, {} symbols",
                "✓".green(),
                metadata.file_count.to_string().bold(),
                metadata.symbol_count.to_string().bold()
            );
            Ok(index)
        }
        Ok(Some((mut index, metadata, changed_files))) => {
            // Incremental update needed
            eprintln!("{} Indexing, changes detected",
                "→".cyan()
            );

            // Create progress bar
            let pb = ProgressBar::new(changed_files.len() as u64);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.cyan} [{bar:40.cyan/blue}] {percent}% ({pos}/{len} files)")
                    .unwrap()
                    .progress_chars("=>-")
            );

            let start = Instant::now();

            for change in &changed_files {
                index.remove_file(&change.path);
            }

            use std::sync::{Arc, Mutex};
            let pb_wrapper = Arc::new(Mutex::new(pb));

            let new_file_infos: Vec<_> = changed_files
                .par_iter()
                .filter(|change| change.kind != FileChangeKind::Deleted)
                .filter_map(|change| {
                    let path = change.path.clone();
                    let result = match std::fs::read_to_string(&path) {
                        Ok(content) => {
                            let language = models::Language::from_extension(
                                path.extension()
                                    .and_then(|e| e.to_str())
                                    .unwrap_or("")
                            );

                            match indexer::index_file(&path, &content, language, change.hash.as_deref()) {
                                Ok(file_info) => Some((change.clone(), file_info)),
                                Err(e) => {
                                    eprintln!("{} Warning: Failed to parse {}: {}",
                                        "⚠".yellow(), path.display(), e);
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("{} Warning: Failed to read {}: {}",
                                "⚠".yellow(), path.display(), e);
                            None
                        }
                    };

                    if let Ok(pb) = pb_wrapper.lock() {
                        pb.inc(1);
                    }

                    result
                })
                .collect();

            if let Ok(pb) = pb_wrapper.lock() {
                pb.finish_with_message("Done");
                eprintln!(); // Add newline after progress bar
            }

            for (change, file_info) in new_file_infos {
                index.remove_file(&change.path);
                index.add_file(file_info);
            }

            // Compact the index to remove deleted symbols
            index.compact();

            let elapsed_ms = start.elapsed().as_millis();

            // Always save updated cache for incremental updates (cache already exists)
            match CacheManager::save_with_changes(&index, path, extensions, &metadata, &changed_files) {
                Ok(_) => eprintln!("{} Cache updated ({}ms)",
                    "✓".green(), elapsed_ms),
                Err(e) => eprintln!("{} Warning: Failed to save cache: {}",
                    "⚠".yellow(), e),
            }

            Ok(index)
        }
        Ok(None) => {
            // Cache miss or invalid - index from scratch
            eprintln!("{} Indexing...", "→".cyan());

            // Create progress bar
            let pb = ProgressBar::new(0);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.cyan} [{bar:40.cyan/blue}] {percent}% ({pos}/{len} files)")
                    .unwrap()
                    .progress_chars("=>-")
            );

            let start = Instant::now();
            let index = indexer::index_directory_with_progress(path, extensions, Some(pb))?;
            let elapsed_ms = start.elapsed().as_millis();

            // Save to cache only if indexing took >= 300ms
            if elapsed_ms >= 300 {
                match CacheManager::save(&index, path, extensions) {
                    Ok(_) => eprintln!("{} Cache not found (.codemapper), created new cache ({} files, {}ms)",
                        "✓".green(),
                        index.total_files().to_string().bold(),
                        elapsed_ms),
                    Err(e) => eprintln!("{} Warning: Failed to save cache: {}",
                        "⚠".yellow(), e),
                }
            } else {
                // Small repo - simple completion message
                eprintln!("{} Indexed ({} files, {}ms)",
                    "✓".green(),
                    index.total_files().to_string().bold(),
                    elapsed_ms);
            }

            Ok(index)
        }
        Err(e) => {
            // Cache error - fallback to rebuild
            eprintln!("{} Cache error: {}. Rebuilding...", "⚠".yellow(), e);
            let start = Instant::now();
            let index = indexer::index_directory(path, extensions)?;
            let elapsed_ms = start.elapsed().as_millis();

            eprintln!("{} Indexed ({} files, {}ms)",
                "✓".green(),
                index.total_files().to_string().bold(),
                elapsed_ms
            );

            Ok(index)
        }
    }
}

fn cmd_index(path: PathBuf, extensions: String) -> Result<()> {
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    
    println!("{} Indexing directory: {}", "→".cyan(), path.display());
    
    let start = Instant::now();
    let index = indexer::index_directory(&path, &ext_list)?;
    let elapsed_ms = start.elapsed().as_millis();
    
    println!("{} Indexed {} files in {}ms", 
        "✓".green(), 
        index.total_files().to_string().bold(),
        elapsed_ms.to_string().bold()
    );
    println!("{} Total symbols: {}", 
        "→".cyan(), 
        index.total_symbols().to_string().bold()
    );
    
    Ok(())
}

fn cmd_map(path: PathBuf, level: u8, extensions: String, no_cache: bool, rebuild_cache: bool, format: OutputFormat) -> Result<()> {
    if level < 1 || level > 3 {
        eprintln!("{} Level must be between 1 and 3", "Error:".red());
        std::process::exit(1);
    }

    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();

    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache)?;

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_map(&index, level);

    println!("{}", output);

    Ok(())
}

fn cmd_query(
    symbol: String,
    path: PathBuf,
    context: String,
    fuzzy: bool,
    fast: bool,
    show_body: bool,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat
) -> Result<()> {
    use fast_search::GrepFilter;

    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();

    // Count files for auto-detection
    let file_count = count_indexable_files(&path, &ext_list)?;

    // Auto-enable fast mode for large codebases (1000+ files)
    let use_fast_mode = fast || file_count >= 1000;

    if use_fast_mode {
        if fast {
            eprintln!("{} Fast mode enabled by --fast flag ({} files)", "→".cyan(), file_count);
        } else {
            eprintln!("{} Fast mode auto-enabled ({} files detected)", "→".cyan(), file_count);
        }

        // Stage 1: Ripgrep prefilter
        let extensions_vec: Vec<String> = ext_list.iter().map(|s| s.to_string()).collect();
        let filter = GrepFilter::new(&symbol, !fuzzy, extensions_vec);

        let candidates = filter.prefilter(&path)?;

        if candidates.is_empty() {
            eprintln!("{} No text matches found, falling back to full AST scan", "→".yellow());
            // Fallback: Use normal mode with cache
            let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache)?;
            let symbols = if fuzzy {
                index.fuzzy_search(&symbol)
            } else {
                index.query_symbol(&symbol)
            };

            if symbols.is_empty() {
                println!("{} No symbols found matching '{}'", "✗".red(), symbol.bold());
                return Ok(());
            }

            let show_context = context.to_lowercase() == "full";
            let formatter = OutputFormatter::new(format);
            let output = formatter.format_query(symbols, show_context, show_body);
            println!("{}", output);
        } else {
            eprintln!("{} Found {} candidate files, validating with AST...", "→".cyan(), candidates.len());

            // Stage 2: AST validation
            let owned_symbols = filter.validate(candidates, &symbol, fuzzy)?;

            if owned_symbols.is_empty() {
                println!("{} No symbols found matching '{}'", "✗".red(), symbol.bold());
                return Ok(());
            }

            let show_context = context.to_lowercase() == "full";
            let formatter = OutputFormatter::new(format);
            // Convert owned symbols to references for formatter
            let symbol_refs: Vec<&Symbol> = owned_symbols.iter().collect();
            let output = formatter.format_query(symbol_refs, show_context, show_body);
            println!("{}", output);
        }
    } else {
        // Normal mode for small codebases with cache
        let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache)?;
        let symbols = if fuzzy {
            index.fuzzy_search(&symbol)
        } else {
            index.query_symbol(&symbol)
        };

        if symbols.is_empty() {
            println!("{} No symbols found matching '{}'", "✗".red(), symbol.bold());
            return Ok(());
        }

        let show_context = context.to_lowercase() == "full";
        let formatter = OutputFormatter::new(format);
        let output = formatter.format_query(symbols, show_context, show_body);
        println!("{}", output);
    }

    Ok(())
}

/// Count indexable files in directory for auto-detection logic
fn count_indexable_files(path: &PathBuf, extensions: &[&str]) -> Result<usize> {
    use ignore::WalkBuilder;

    let mut count = 0;
    let walker = WalkBuilder::new(path)
        .hidden(false)
        .git_ignore(true)
        .build();

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        if extensions.is_empty() {
            count += 1;
        } else {
            if entry.path()
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| extensions.contains(&ext))
                .unwrap_or(false)
            {
                count += 1;
            }
        }
    }

    Ok(count)
}

fn cmd_deps(
    target: String,
    path: PathBuf,
    direction: String,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat
) -> Result<()> {
    
    use std::path::Path;

    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache)?;

    // Detect if target is a file path or symbol name
    let target_path = Path::new(&target);
    let is_file = target_path.exists() || target.contains('/') || target.contains('\\');

    if is_file {
        // Original file-based dependency tracking
        cmd_deps_file(target, index, direction, format)
    } else {
        // New symbol-based usage tracking
        cmd_deps_symbol(target, index, direction, format)
    }
}

fn cmd_deps_file(
    target: String,
    index: index::CodeIndex,
    direction: String,
    format: OutputFormat
) -> Result<()> {
    use std::path::PathBuf;

    let target_path = PathBuf::from(&target);
    let target_canonical = std::fs::canonicalize(&target_path).unwrap_or(target_path.clone());

    let deps = if direction.to_lowercase() == "imports" {
        // Try both relative and canonical paths
        index.get_dependencies(&target_path)
            .or_else(|| index.get_dependencies(&target_canonical))
            .map(|d| d.clone())
            .unwrap_or_default()
    } else if direction.to_lowercase() == "used-by" {
        let mut used_by = Vec::new();
        for file in index.files() {
            if let Some(file_deps) = index.get_dependencies(&file.path) {
                let target_name = target_canonical.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");

                for dep in file_deps {
                    if dep.contains(target_name) {
                        used_by.push(file.path.display().to_string());
                        break;
                    }
                }
            }
        }
        used_by
    } else {
        eprintln!("{} Invalid direction '{}', use 'imports' or 'used-by'",
            "Error:".red(), direction);
        std::process::exit(1);
    };

    if deps.is_empty() {
        println!("{} No dependencies found for {}",
            "✗".yellow(),
            target
        );
        return Ok(());
    }

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_deps(&target, deps, &direction);

    println!("{}", output);

    Ok(())
}

fn cmd_deps_symbol(
    symbol_name: String,
    index: index::CodeIndex,
    direction: String,
    format: OutputFormat
) -> Result<()> {
    use std::fs;

    if direction.to_lowercase() != "used-by" {
        eprintln!("{} For symbols, only '--direction used-by' is supported",
            "Error:".red());
        eprintln!("{} Use a file path to see imports", "Hint:".cyan());
        std::process::exit(1);
    }

    // Find the symbol definition
    let symbols = index.query_symbol(&symbol_name);

    if symbols.is_empty() {
        println!("{} Symbol '{}' not found in codebase",
            "✗".yellow(),
            symbol_name.bold()
        );
        return Ok(());
    }

    // For now, use simple string search to find usages
    // Future: can be upgraded to AST-based call detection
    let mut usages: Vec<String> = Vec::new();

    for file in index.files() {
        if let Ok(content) = fs::read_to_string(&file.path) {
            for (line_num, line) in content.lines().enumerate() {
                if line.contains(&symbol_name) {
                    // Skip the definition itself
                    let is_definition = symbols.iter().any(|s| {
                        s.file_path == file.path &&
                        (line_num + 1) >= s.line_start &&
                        (line_num + 1) <= s.line_end
                    });

                    if !is_definition {
                        usages.push(format!("{}:{}", file.path.display(), line_num + 1));
                    }
                }
            }
        }
    }

    if usages.is_empty() {
        println!("{} No usages found for symbol '{}'",
            "✗".yellow(),
            symbol_name.bold()
        );
        return Ok(());
    }

    // Show summary first
    println!("{} Found {} usage(s) of '{}'\n",
        "✓".green(),
        usages.len().to_string().bold(),
        symbol_name.bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_deps(&symbol_name, usages, "used-by");

    println!("{}", output);

    Ok(())
}

fn cmd_stats(path: PathBuf, extensions: String, no_cache: bool, rebuild_cache: bool, format: OutputFormat) -> Result<()> {
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();

    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache)?;

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_stats(&index);

    println!("{}", output);

    Ok(())
}

fn cmd_inspect(file_path: PathBuf, show_body: bool, format: OutputFormat) -> Result<()> {
    use std::fs;

    if !file_path.exists() {
        anyhow::bail!("File does not exist: {}", file_path.display());
    }

    if !file_path.is_file() {
        anyhow::bail!("Path is not a file: {}", file_path.display());
    }

    let language = indexer::detect_language(&file_path);
    if language == models::Language::Unknown {
        anyhow::bail!("Unknown or unsupported file type: {}", file_path.display());
    }

    let content = fs::read_to_string(&file_path)?;
    let start = Instant::now();
    let file_info = indexer::index_file(&file_path, &content, language, None)?;
    let elapsed_ms = start.elapsed().as_millis();

    if file_info.symbols.is_empty() {
        println!("{} No symbols found in {}", "✗".yellow(), file_path.display());
        return Ok(());
    }

    let formatter = OutputFormatter::new(format);

    match format {
        OutputFormat::AI => {
            println!("[FILE:{}]", file_path.display());
            println!("LANG:{} SIZE:{} SYMS:{}",
                language.as_str(),
                file_info.size,
                file_info.symbols.len()
            );
            for symbol in &file_info.symbols {
                print!("{}|{}|{}-{}",
                    symbol.name,
                    symbol.symbol_type.as_str().chars().next().unwrap(),
                    symbol.line_start,
                    symbol.line_end
                );
                if let Some(ref sig) = symbol.signature {
                    print!("|sig:{}", sig);
                }
                println!();
            }
        }
        _ => {
            println!("{} Inspecting: {}\n", "→".cyan(), file_path.display().to_string().bold());
            println!("Language: {}", language.as_str());
            println!("Size: {} bytes", file_info.size);
            println!("Symbols: {}\n", file_info.symbols.len());

            let symbol_refs: Vec<&models::Symbol> = file_info.symbols.iter().collect();
            let output = formatter.format_query(symbol_refs, false, show_body);
            println!("{}", output);
        }
    }

    println!("\n{} Parse time: {}ms", "→".cyan(), elapsed_ms.to_string().bold());

    Ok(())
}

mod blame;
mod cache;
mod callgraph;
mod diff;
mod fast_search;
mod git;
mod ignore;
mod impact;
mod implements;
mod index;
mod indexer;
mod inspect_md;
mod models;
mod output;
mod parser;
mod pipe;
mod schema;
mod snapshot;
mod types;

use anyhow::Result;
use cache::FileChangeKind;
use clap::{Parser, Subcommand};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle};
use models::Symbol;
use output::{OutputFormat, OutputFormatter};
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(clap::Parser)]
#[command(name = "cm")]
#[command(
    about = "codemapper (cm) - fast code analysis at llm speed",
    long_about = "CodeMapper (cm) - Code Analysis at LLM Speed

Analyze codebases instantly by mapping symbols (functions, classes, methods)
using tree-sitter AST parsing. Everything runs in-memory, no databases.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

TYPICAL WORKFLOWS

1. EXPLORING UNKNOWN CODE
   Step 1: cm stats .
   → Size and composition (how big? what languages?)
   
   Step 2: cm map . --level 2 --format ai
   → File structure (where's the logic?)
   
   Step 3: cm query <symbol>
   → Find code (where exactly?)
   
   Step 4: cm inspect ./path/to/file
   → Deep dive (what's in this module?)

2. FINDING A BUG (you know the symptom, need the source)
   Step 1: cm query <suspected_function> --show-body
   → See the implementation
   
   Step 2: cm callers <function>
   → Who calls this? (Is it called from where the bug manifests?)
   
   Step 3: cm trace <entry_point> <suspected_function>
   → Trace the call path (how does the bug get triggered?)
   
   Step 4: cm tests <function>
   → Find tests (are there existing tests for this?)

3. BEFORE REFACTORING
   Step 1: cm callers <function>
   → Understand impact (who depends on this?)
   
   Step 2: cm callees <function>
   → What does it depend on? (what breaks if we change this?)
   
   Step 3: cm tests <function>
   → Run tests (verify nothing breaks)
   
   Step 4: cm since main --breaking
   → (After refactor) Did we break anything vs main?

4. UNDERSTANDING AN API
   Step 1: cm entrypoints .
   → What's exported? (what's the public surface?)
   
   Step 2: cm implements <interface>
   → Find all implementations (how many ways is this used?)
   
   Step 3: cm schema <DataClass>
   → See field structure (what does the data look like?)

5. VALIDATING CODE HEALTH
   Step 1: cm untested .
   → Find uncovered symbols (what's not tested?)
   
   Step 2: cm since <last_release> --breaking
   → Did we break anything? (breaking changes since release?)
   
   Step 3: cm since <last_release>
   → Full changelog (what changed?)

6. NAVIGATING LARGE MARKDOWN API DOCS
   Step 1: cm inspect ./docs/api.md --tree --sizes --level 2
   → Triage sections by size (what's worth reading?)

   Step 2: cm inspect ./docs/api.md --section 'Orders'
   → Scope symbols to a section (reduce noise)

   Step 3: cm query '/v1/orders' ./docs --type endpoint
   → Find endpoints fast (queryable route symbols)

   Step 4: cm query TWAP ./docs --show-body
   → Jump to the relevant section content

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

COMMANDS (organized by task)

[DISCOVERY - Start here]
  stats        → Project size and composition (functions, classes, imports)
  map          → File listing with symbol counts (3 detail levels)
  query        → Find symbols by name (main search tool)
  inspect      → List all symbols in one file (markdown: tree navigation)
  deps         → Track imports and usage

[CALL GRAPH - Understand code flow]
  callers      → WHO calls this function? (reverse dependencies)
  callees      → What DOES this function call? (forward dependencies)
  trace        → CALL PATH from A → B (shortest route)
  entrypoints  → Public APIs with no internal callers (dead code?)
  tests        → Which tests call this symbol?
  test-deps    → What production code does a test touch?

[GIT HISTORY - Blame and timeline]
  diff         → Symbol-level changes vs a commit (what changed?)
  since        → Breaking changes since commit (what broke?)
  blame        → Who last touched this symbol? (when, commit, author)
  history      → Full evolution of a symbol (all commits touching it)

[TYPE ANALYSIS - Understand data flow]
  types        → Parameter types and return type (where are they defined?)
  implements   → Find all implementations of an interface
  schema       → Field structure (structs, classes, dataclasses)

[SNAPSHOTS - Compare over time]
  snapshot     → Save current state (named checkpoint)
  compare      → Diff current vs saved snapshot (what changed?)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

KEY FACTS

OUTPUT FORMATS:
  --format ai       → Compact (LLM context, token-efficient) ← DEFAULT
  --format human    → Tables (terminal viewing, pretty)
  --format default  → Markdown (documentation, readable)

PERFORMANCE:
  Small repos (< 100 files)    → < 20ms instant
  Medium repos (100-1000)      → Cached, ~0.5s load
  Large repos (1000+)          → Fast mode auto-enabled (10-100x speedup)
  Incremental rebuilds         → 45-55x faster than full reindex

CACHING:
  Auto-enabled on projects ≥ 300ms to parse
  Small projects never create cache (no .codemapper/ clutter)
  Subsequent runs load from cache (~0.5s)
  File changes auto-detected (you don't manage cache)
  Location: .codemapper/ in project root (default)
  Custom location: --cache-dir <path> or CODEMAPPER_CACHE_DIR env var
  Flags: --no-cache (skip), --rebuild-cache (force rebuild)

SEARCH MODES:
  Fuzzy   → cm query myclass          (DEFAULT: case-insensitive, flexible)
  Grep    → cm query MyClass --grep   (case-sensitive + text prefilter)
  Grep    → cm grep MyClass           (exact symbol search, rg-style muscle memory)
  Multi   → cm query 'foo|bar|baz'   (OR search, matches any term)

LANGUAGES SUPPORTED:
  ✓ Python       → Functions, classes, methods, imports
  ✓ JavaScript   → Functions, classes, methods, imports
  ✓ TypeScript   → Functions, classes, methods, interfaces, types, enums
  ✓ Rust         → Functions, structs, impl blocks, traits, enums
  ✓ Java         → Classes, interfaces, methods, enums, javadoc
  ✓ Go           → Functions, structs, methods, interfaces
  ✓ C            → Functions, structs, includes
  ✓ Swift        → Functions, types (struct/class/enum/extension), methods, imports
  ✓ Ruby         → Classes, modules, methods, generated attr/delegate/Rails DSL methods, mixins, callback refs, constants, requires
  ✓ Markdown     → Hierarchical headings, code blocks, endpoint-like routes (e.g. GET /v1/orders)

GIT REQUIREMENTS:
  diff      → Must be in a git repo
  since     → Must be in a git repo
  blame     → Must be in a git repo
  history   → Must be in a git repo
  (Other commands work anywhere)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

COMMON FLAGS

--grep               → Strict grep-backed matching (default query mode is fuzzy)
--format <format>    → Output style: ai (default), human (tables), default (markdown)
--show-body          → Include actual code (not just signatures)
--exports-only       → Public symbols only (functions with export, pub, etc.)
--full               → Include anonymous/lambda functions (normally hidden)
--context minimal    → Signatures only (default, fast)
--context full       → Include docstrings and metadata
--no-cache           → Skip cache, always reindex (troubleshooting)
--rebuild-cache      → Force cache rebuild
--extensions py,rs   → Comma-separated file types to include (swift and ruby included by default)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

TROUBLESHOOTING

NO SYMBOLS FOUND?
  ✓ Fuzzy matching by default (matches more)
  ✓ Check --extensions py,js,ts (default: py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md) — swift and ruby are INCLUDED by default
  ✓ Verify file encoding is UTF-8
  ✓ Run: cm stats . (to see what's indexed)

SLOW QUERIES?
  ✓ Large repo (1000+ files)? Fast mode auto-enables (use --fast explicitly)
  ✓ First run builds cache (~10s), cache hits ~0.5s after
  ✓ Try --no-cache if cache is stale (rare)

GIT COMMANDS FAIL?
  ✓ Must be in a git repository (diff, since, blame, history need git)
  ✓ Commit must exist (HEAD~1, abc123, main, v1.0 all work)
  ✓ File must have git history (blame, history)

OUTPUT TOO VERBOSE?
  ✓ AI format is the default for compact LLM-ready output
  ✓ Use --format human for terminal tables
  ✓ Use --context minimal (signatures only)

NO TEST COVERAGE?
  ✓ Run: cm untested .
  ✓ Test detection by file pattern (_test.rs, test_*.py, *.test.js, etc.)
  ✓ Test detection by naming convention (test*, Test*, #[test], @Test, etc.)

EXAMPLES:
  # Get the lay of the land
  cm stats .                           # Project overview
  cm map . --level 2                   # File structure (AI format by default)
  
  # Find and explore
  cm query authenticate                # Search (fuzzy by default)
  cm inspect ./src/auth.py             # Deep dive
  cm query Parser --show-body          # See implementation
  
  # Understand flow
  cm callers process_payment           # Who calls it?
  cm callees process_payment           # What does it call?
  cm trace main process_payment        # Call path
  
  # Before refactoring
  cm callers my_function               # Impact radius
  cm tests my_function                 # Verify coverage exists
  
  # Git analysis
  cm diff main                         # Changes vs main
  cm since v1.0 --breaking             # Breaking changes since v1.0
  cm blame authenticate ./auth.py      # Who last touched it?
  
  # Type analysis
  cm types process_payment             # What types flow through?
  cm schema Order                      # Field structure
  cm implements Iterator               # Find all implementations
  
  # Health check
  cm untested .                        # What's not tested?
  cm entrypoints .                     # Public API surface

For detailed help on any command: cm <command> --help
"
)]
struct Cli {
    /// Output format: 'ai' (default), 'human' (tables), or 'default' (markdown)
    #[arg(short, long, global = true, default_value = "ai")]
    format: String,

    /// Override cache directory location (default: .codemapper in project root)
    /// Can also be set via CODEMAPPER_CACHE_DIR environment variable
    #[arg(long, global = true, env = "CODEMAPPER_CACHE_DIR")]
    cache_dir: Option<PathBuf>,

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
  • Cache location: .codemapper/cache/ in project root (override with --cache-dir or CODEMAPPER_CACHE_DIR)

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

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,rb,md')
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
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

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,rb,md')
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
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
  • Fuzzy search: Find 'auth' matches 'authenticate', 'Authorization', etc.
  • Grep search: Find exact symbols with text-prefilter speed
  • Fast mode: Auto-enables for 1000+ files (10-100x speedup)
  • Results are cached for instant loading (< 10ms for most projects)

SEARCH MODES:
  Fuzzy   → cm query myclass              (DEFAULT: case-insensitive, flexible)
  Grep    → cm query MyClass --grep       (case-sensitive + text prefilter)
  Grep    → cm grep MyClass               (exact symbol search, rg-style muscle memory)
  Multi   → cm query 'foo|bar|baz'        (OR search, matches any term)

CONTEXT OPTIONS:
  --context minimal → Signatures only (default, fast)
  --context full    → Includes docstrings and metadata
  --show-body       → Show actual code implementation

PERFORMANCE (Fast Mode):
  • 18,457 files: 76s → 1.2s (63x faster)
  • 17,005 files: 122s → 9.6s (12x faster)
  • Auto-enabled for 1000+ files
  • Two-stage: ripgrep text search → AST validation

PIPELINES:
  cm query Parser | cm inspect -          → inspect files from prior results
  cm query 'parse|index' | cm callers -   → chase callers for prior symbols
  cm map . --level 2 | cm inspect -       → inspect files emitted by cm

TIP: Start with fuzzy search, use grep mode when you want strict symbols"
    )]
    #[command(after_help = "EXAMPLES:
  # Basic searches
  cm query authenticate                      # Exact match (case-sensitive)
  cm query auth                              # Fuzzy search (default)

  # With context
  cm query process_payment --context full    # Include docstrings
  cm query validate --show-body              # Show implementation
  cm query REST ./docs --section 'Orders'    # Scope to a markdown section

  # Fast mode (for large codebases)
  cm query MyClass /large/repo --fast        # Explicit fast mode
  cm query auth /monorepo                    # Auto-enabled fast mode for 1000+ files

  # OR search (pipe-separated)
  cm query 'parse|index|cache'               # Match any of the terms
  cm query 'Foo|Bar' --grep                  # Grep/exact match on multiple names

  # Output formats
  cm query Parser --format human             # Pretty tables
  cm query CodeIndex                         # AI format is default

TYPICAL WORKFLOW:
  1. Quick fuzzy search: cm query auth
  2. Get more context: cm query authenticate --context full
  3. See implementation: cm query authenticate --show-body
  4. Find usage: cm deps authenticate --direction used-by

WHEN TO USE:
  ✓ \"Where is the authenticate function?\"
  ✓ \"Find all parser-related code\"
  ✓ \"What methods does the User class have?\"
  ✓ \"Show me the validate_input implementation\"")]
    Query {
        /// Symbol name to search for (supports 'foo|bar|baz' OR syntax, '-' reads stdin)
        symbol: String,

        /// Directory path to search in
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Use grep-style exact matching with text prefiltering
        #[arg(long = "grep", default_value_t = false)]
        grep: bool,

        /// Filter by symbol type: 'function', 'class', 'module', 'method', 'enum', 'static', 'heading', 'code_block', 'endpoint'
        #[arg(long)]
        r#type: Option<String>,

        /// Context level: 'minimal' (signatures only) or 'full' (includes docstrings)
        #[arg(long, default_value = "minimal")]
        context: String,

        /// Show the actual code implementation in results
        #[arg(long, default_value = "false")]
        show_body: bool,

        /// Enable fast mode explicitly (auto-enabled for 1000+ files)
        #[arg(long, default_value = "false")]
        fast: bool,

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,rb,md')
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache (invalidate and reindex)
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,

        /// Show anonymous/lambda functions (default: filtered out)
        #[arg(long, default_value_t = false)]
        full: bool,

        /// Show only exported/public symbols (functions/classes with export keyword, pub visibility, etc.)
        #[arg(long, default_value_t = false)]
        exports_only: bool,

        /// Markdown only: scope results to a heading path (e.g. "Orders" or "Orders > Order Strategy")
        #[arg(long)]
        section: Option<String>,

        /// Maximum number of results to return (prevents overwhelming output)
        #[arg(long)]
        limit: Option<usize>,
    },

    /// [SEARCH] Exact symbol search with grep-style text prefiltering
    #[command(
        alias = "g",
        about = "Grep-style exact symbol search backed by AST validation",
        long_about = "USE CASE: Replace rg/grep muscle memory with cm-native symbol search
  • Uses text prefiltering first, then validates matches with the parser
  • Case-sensitive exact symbol matching by default
  • Outputs AI format by default, so results chain cleanly into other cm commands

PIPELINES:
  cm grep Parser | cm inspect -
  cm grep 'parse|index' | cm callers -
  cm grep CodeIndex --show-body"
    )]
    #[command(after_help = "EXAMPLES:
  cm grep Parser                         # Exact symbol search
  cm grep 'parse|index' ./src            # OR search in src
  cm grep CodeIndex --show-body          # Show implementation when small enough
  cm grep Parser | cm inspect -          # Inspect files from results
  cm grep parse | cm callers -           # Find callers of matched symbols")]
    Grep {
        /// Symbol pattern to search for (supports 'foo|bar|baz' OR syntax, '-' reads stdin)
        pattern: String,

        /// Directory path to search in
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Filter by symbol type: 'function', 'class', 'module', 'method', 'enum', 'static', 'heading', 'code_block', 'endpoint'
        #[arg(long)]
        r#type: Option<String>,

        /// Context level: 'minimal' (signatures only) or 'full' (includes docstrings)
        #[arg(long, default_value = "minimal")]
        context: String,

        /// Show the actual code implementation in results
        #[arg(long, default_value = "false")]
        show_body: bool,

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,rb,md')
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache (invalidate and reindex)
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,

        /// Show anonymous/lambda functions (default: filtered out)
        #[arg(long, default_value_t = false)]
        full: bool,

        /// Show only exported/public symbols (functions/classes with export keyword, pub visibility, etc.)
        #[arg(long, default_value_t = false)]
        exports_only: bool,

        /// Markdown only: scope results to a heading path (e.g. "Orders" or "Orders > Order Strategy")
        #[arg(long)]
        section: Option<String>,

        /// Maximum number of results to return (prevents overwhelming output)
        #[arg(long)]
        limit: Option<usize>,
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

  # markdown docs
  cm inspect ./docs/api.md --level 1       # just h1 headings
  cm inspect ./docs/api.md --tree --sizes  # heading tree + section sizes
  cm inspect ./docs/api.md --section 'Orders' --code-blocks

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

        /// Markdown only: show headings as an indented tree
        #[arg(long, default_value_t = false)]
        tree: bool,

        /// Markdown only: show headings up to level N (e.g. 1 = h1, 2 = h1+h2)
        #[arg(long)]
        level: Option<usize>,

        /// Markdown only: scope output to a heading path (e.g. "Orders" or "Orders > Order Strategy")
        #[arg(long)]
        section: Option<String>,

        /// Markdown only (with --tree): show section sizes in lines
        #[arg(long, default_value_t = false)]
        sizes: bool,

        /// Markdown only: show fenced code blocks (optionally scoped by --section)
        #[arg(long, default_value_t = false)]
        code_blocks: bool,

        /// Show anonymous/lambda functions (default: filtered out)
        #[arg(long, default_value_t = false)]
        full: bool,

        /// Show only exported/public symbols (functions/classes with export keyword, pub visibility, etc.)
        #[arg(long, default_value_t = false)]
        exports_only: bool,
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

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,rb,md')
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
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

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,rb,md')
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,
    },

    /// [ANALYSIS] Symbol-level diff between current code and a git commit
    #[command(
        about = "Show symbol-level changes between current code and a git commit",
        long_about = "USE CASE: Understand what symbols changed between commits
  • See which functions/classes were added, deleted, or modified
  • Detect signature changes (parameter/return type modifications)
  • Review changes at symbol granularity instead of line-by-line

CHANGE TYPES:
  ADDED            → New symbols that didn't exist in the commit
  DELETED          → Symbols that were removed since the commit
  MODIFIED         → Symbols with body/line changes (same signature)
  SIGNATURE_CHANGED → Symbols with parameter or return type changes

REQUIREMENTS:
  • Must be run inside a git repository
  • Commit reference can be: HEAD~1, abc123, branch-name, tag-name

TIP: Great for code review and understanding PR impact"
    )]
    #[command(after_help = "EXAMPLES:
  cm diff HEAD~1                        # Compare to previous commit
  cm diff HEAD~3 ./src                  # Compare src/ to 3 commits ago
  cm diff main                          # Compare to main branch
  cm diff abc1234 --format human        # Pretty table output
  cm diff HEAD~5 --extensions py,rs     # Only Python and Rust files

TYPICAL WORKFLOW:
  1. Before PR review: cm diff main --format human
  2. Check recent changes: cm diff HEAD~1
  3. Impact analysis: cm diff release-v1.0 ./src

WHEN TO USE:
  ✓ \"What functions changed in this PR?\"
  ✓ \"Did any signatures change since last release?\"
  ✓ \"What was added/removed in the last 5 commits?\"")]
    Diff {
        /// Git commit reference (e.g., HEAD~1, abc123, main, v1.0)
        commit: String,

        /// Directory or file path to analyze (optional, defaults to entire repo)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,rb,md')
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Show anonymous/lambda functions (default: filtered out)
        #[arg(long, default_value_t = false)]
        full: bool,
    },

    /// [ANALYSIS] Find all call sites of a function (reverse call graph)
    #[command(
        about = "Find all places where a function/method is called",
        long_about = "USE CASE: Understand who calls a function
  • Find all call sites of a specific function
  • Useful for impact analysis before refactoring
  • Shows the enclosing function/method making each call
  • Uses AST-based call detection (not text search)
  • Tip: use qualified names (e.g. `Foo::new`) to reduce noise for common method names

SUPPORTED LANGUAGES:
  Python, JavaScript, TypeScript, Rust, Go, Java, C

TIP: Great for understanding function usage patterns"
    )]
    #[command(after_help = "EXAMPLES:
  cm callers parse_file                    # Find all callers of parse_file
  cm callers parse ./src --fuzzy           # Fuzzy match 'parse' in src/
  cm callers Foo::new                      # Disambiguate common method names
  cm callers validate --format human       # Pretty table output
  cm callers process_data . --format ai    # Token-efficient output

TYPICAL WORKFLOW:
  1. Find the function: cm query my_func --fuzzy
  2. See who calls it: cm callers my_func
  3. Understand the call chain before refactoring")]
    Callers {
        /// Symbol name to find callers for
        symbol: String,

        /// Directory path to search in
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Enable fuzzy matching for symbol lookup
        #[arg(long, default_value = "false")]
        fuzzy: bool,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,

        /// Maximum number of results to return (prevents overwhelming output)
        #[arg(long)]
        limit: Option<usize>,
    },

    /// [ANALYSIS] Find all functions called by a symbol (forward call graph)
    #[command(
        about = "Find all functions/methods that a symbol calls",
        long_about = "USE CASE: Understand what a function depends on
  • See all functions called within a symbol's body
  • Useful for understanding code dependencies
  • Links to definitions when found in codebase
  • Marks external/built-in functions separately

SUPPORTED LANGUAGES:
  Python, JavaScript, TypeScript, Rust, Go, Java, C

TIP: Great for understanding function complexity and dependencies"
    )]
    #[command(after_help = "EXAMPLES:
  cm callees main                          # What does main() call?
  cm callees process_data --fuzzy          # Fuzzy match
  cm callees cmd_query --format human      # Pretty table output
  cm callees validate . --format ai        # Token-efficient output

TYPICAL WORKFLOW:
  1. Find the function: cm query my_func --fuzzy
  2. See what it calls: cm callees my_func
  3. Understand the dependency graph")]
    Callees {
        /// Symbol name to find callees for
        symbol: String,

        /// Directory path to search in
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Enable fuzzy matching for symbol lookup
        #[arg(long, default_value = "false")]
        fuzzy: bool,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,

        /// Maximum number of results to return (prevents overwhelming output)
        #[arg(long)]
        limit: Option<usize>,
    },

    /// [ANALYSIS] Find tests that call a symbol
    #[command(
        about = "Find test functions that call a given symbol",
        long_about = "USE CASE: Find tests covering a specific function or method
  • Identifies test files by naming convention (_test.rs, test_*.py, *.test.js)
  • Detects test functions by attributes (#[test], @Test) or naming (test_*, Test*)
  • Shows where in the test the symbol is called

TEST DETECTION:
  Rust     → #[test] attribute, _test.rs files, tests/ directory
  Python   → test_*.py files, functions starting with test_
  Go       → *_test.go files, functions starting with Test
  JS/TS    → *.test.js, *.spec.ts, __tests__/ directory
  Java     → @Test annotation, *Test.java files

TIP: Use before refactoring to understand test coverage"
    )]
    #[command(after_help = "EXAMPLES:
  cm tests parse_file                     # Find tests calling parse_file
  cm tests authenticate --fuzzy           # Fuzzy match test discovery
  cm tests validate ./src --format human  # Pretty table output
  cm tests process_payment --format ai    # Token-efficient for LLMs

TYPICAL WORKFLOW:
  1. Identify function to refactor: cm query my_function
  2. Find tests: cm tests my_function
  3. Run tests, make changes, verify")]
    Tests {
        /// Symbol name to find tests for
        symbol: String,

        /// Directory path to search in
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Enable fuzzy matching for flexible search
        #[arg(long, default_value = "false")]
        fuzzy: bool,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [ANALYSIS] Find symbols with no test coverage
    #[command(
        about = "Find functions and methods that are not called by any test",
        long_about = "USE CASE: Identify code without test coverage
  • Finds symbols not called from any test file or test function
  • Excludes test functions themselves from the output
  • Excludes private/internal helpers (leading underscore in Python)
  • Shows coverage percentage and untested symbol count

TEST DETECTION:
  Rust     → #[test] attribute, _test.rs files, tests/ directory
  Python   → test_*.py files, functions starting with test_
  Go       → *_test.go files, functions starting with Test
  JS/TS    → *.test.js, *.spec.ts, __tests__/ directory
  Java     → @Test annotation, *Test.java files

TIP: Use to identify areas needing test coverage"
    )]
    #[command(after_help = "EXAMPLES:
  cm untested                           # Find untested symbols in current dir
  cm untested ./src                     # Check specific directory
  cm untested . --format human          # Pretty table output
  cm untested . --format ai             # Token-efficient for LLMs

TYPICAL WORKFLOW:
  1. Check coverage: cm untested .
  2. Pick symbol to test: identify critical untested code
  3. Write tests for high-priority symbols")]
    Untested {
        /// Directory path to analyze
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [ANALYSIS] List breaking changes since a known-good commit
    #[command(
        about = "Show breaking API changes since a git commit (removed symbols, signature changes)",
        long_about = "USE CASE: Identify breaking changes that could affect callers
  • Removed symbols (DELETED) - definitely breaks callers
  • Signature changes (params/return type modified) - breaks callers
  • Filters out non-breaking changes (internal body modifications)

BREAKING CHANGES:
  DELETED          → Symbol was removed (callers will fail)
  SIGNATURE_CHANGED → Function signature modified (callers may need updates)

NON-BREAKING (filtered out with --breaking):
  ADDED            → New symbols (safe)
  MODIFIED         → Body changes only (internal, safe for callers)

REQUIREMENTS:
  • Must be run inside a git repository
  • Commit reference can be: HEAD~1, abc123, branch-name, tag-name

TIP: Use before releases to identify API-breaking changes"
    )]
    #[command(after_help = "EXAMPLES:
  cm since HEAD~10 --breaking              # Breaking changes in last 10 commits
  cm since main --breaking                 # Breaking changes since main branch
  cm since v1.0 --breaking --format human  # Breaking changes since v1.0 tag
  cm since HEAD~5 src/ --breaking          # Breaking changes in src/ directory
  cm since abc1234                         # All changes (not just breaking)

TYPICAL WORKFLOW:
  1. Before release: cm since last-release --breaking
  2. Check PR impact: cm since main --breaking --format human
  3. Full changelog: cm since v1.0 (without --breaking)

WHEN TO USE:
  ✓ \"What breaks if I upgrade from v1.0?\"
  ✓ \"Did this PR introduce breaking changes?\"
  ✓ \"Is it safe to merge this into main?\"")]
    Since {
        /// Git commit reference (e.g., HEAD~1, abc123, main, v1.0)
        commit: String,

        /// Directory or file path to analyze (optional, defaults to entire repo)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Comma-separated file extensions to include (e.g., 'py,js,rs,go,c,h,rb,md')
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Show only breaking changes (deleted symbols, signature changes)
        #[arg(long, default_value_t = false)]
        breaking: bool,
    },

    /// [ANALYSIS] Find exported/public symbols with no internal callers (API surface)
    #[command(
        about = "Find entrypoints: exported symbols that are not called internally",
        long_about = "USE CASE: Identify API surface and potentially unused code
  • Finds public/exported functions that have no callers within the codebase
  • Useful for identifying: main entry points, exported APIs, dead code
  • Groups by: Main Entrypoints, API Functions, Possibly Unused

EXPORT DETECTION BY LANGUAGE:
  Rust     → pub fn, pub struct (excluding pub(crate), pub(super))
  Python   → No leading underscore, or in __all__
  JS/TS    → export function, export default, module.exports
  Go       → Capitalized function names
  Java     → public modifier
  C        → No leading underscore

CATEGORIES:
  Main Entrypoint → main, run, start, init, execute, cli, app
  API Function    → get*, post*, handle*, create*, process*, classes, enums
  Possibly Unused → Other exported symbols with no callers

TIP: Use to identify dead code or document your public API"
    )]
    #[command(after_help = "EXAMPLES:
  cm entrypoints                         # Find entrypoints in current dir
  cm entrypoints ./src                   # Check specific directory
  cm entrypoints . --format human        # Pretty table output
  cm entrypoints . --format ai           # Token-efficient for LLMs

TYPICAL WORKFLOW:
  1. Check entrypoints: cm entrypoints .
  2. Review 'Possibly Unused' - candidates for removal
  3. Document 'API Functions' as your public interface

WHEN TO USE:
  ✓ \"What's our public API surface?\"
  ✓ \"Is this function used anywhere?\"
  ✓ \"Find dead code candidates\"
  ✓ \"What are the entry points to this codebase?\"")]
    Entrypoints {
        /// Directory path to analyze
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [ANALYSIS] Trace the call path between two symbols
    #[command(
        about = "Show the shortest call path from symbol A to symbol B",
        long_about = "USE CASE: Understand how code flows between two symbols
  • Find the call chain: A calls X, X calls Y, Y calls B
  • Uses BFS to find the shortest path
  • Supports fuzzy matching for flexible symbol lookup

ALGORITHM:
  • Breadth-first search from source symbol
  • Follows call edges (function/method calls)
  • Maximum depth: 10 levels to avoid infinite loops
  • Returns shortest path found

OUTPUT:
  • Shows each step in the call chain
  • Includes symbol type and file location for each step

TIP: Use --fuzzy if you're not sure of exact symbol names"
    )]
    #[command(after_help = "EXAMPLES:
  cm trace main parse_file                   # Trace from main to parse_file
  cm trace authenticate validate --fuzzy    # Fuzzy match both symbols
  cm trace handler response ./src           # Trace within specific path
  cm trace cmd_query format_output --format human  # Pretty table

TYPICAL WORKFLOW:
  1. Find symbols: cm query func_a --fuzzy
  2. Trace path: cm trace func_a func_b
  3. Review the call chain to understand code flow

WHEN TO USE:
  ✓ \"How does data flow from A to B?\"
  ✓ \"What's the call path from main to this function?\"
  ✓ \"Understanding control flow in unfamiliar code\"")]
    Trace {
        /// Source symbol name (start of the path)
        from: String,

        /// Target symbol name (end of the path)
        to: String,

        /// Directory path to search in
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Enable fuzzy matching for symbol names
        #[arg(long, default_value = "false")]
        fuzzy: bool,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [ANALYSIS] Quick breakage report for a symbol (definition + callers + tests)
    #[command(
        about = "Quick breakage report for a symbol (definition + callers + tests)",
        long_about = "USE CASE: Tight edit loop safety check
  • After you edit a function, run this to see what you likely broke
  • Shows definition + signature, all callers, and tests that touch it
  • Intended to be fast enough to run repeatedly during refactors

TIP: Run this after changing a function signature"
    )]
    #[command(after_help = "EXAMPLES:
  cm impact symbols_by_type               # quick: counts + top callsites/tests
  cm impact parse_file ./src --grep       # restrict scope + exact match
  cm impact auth . --format ai            # token-efficient output
  cm impact output --include-docs         # allow matching headings/code blocks
  cm impact big_function --all            # print full lists (no truncation)")]
    Impact {
        /// Symbol name to analyze
        symbol: String,

        /// Directory path to search in
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Use grep/exact matching (default is fuzzy)
        #[arg(long = "grep", default_value_t = false)]
        grep: bool,

        /// Include markdown headings/code blocks as candidates (default: code symbols only)
        #[arg(long, default_value_t = false)]
        include_docs: bool,

        /// Maximum number of callers/tests to show (default: 10 each)
        #[arg(long)]
        limit: Option<usize>,

        /// Show full callers/tests lists (ignores --limit)
        #[arg(long, default_value_t = false)]
        all: bool,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [ANALYSIS] Show what production symbols a test file calls
    #[command(
        about = "List production (non-test) symbols called by a test file",
        long_about = "USE CASE: Understand test scope and coverage
  • Parse a test file to find all function/method calls
  • Filter to only show production code (not test helpers)
  • Useful for understanding what a test actually tests

FILTERS OUT:
  • Calls to other test functions/files
  • Calls to symbols in the same test file
  • External/built-in functions not in codebase

TIP: Great for reviewing test scope before refactoring"
    )]
    #[command(after_help = "EXAMPLES:
  cm test-deps ./tests/test_auth.py              # Show production deps
  cm test-deps ./src/parser_test.rs --format ai  # Token-efficient output
  cm test-deps ./auth.test.ts --format human     # Pretty table

TYPICAL WORKFLOW:
  1. Find test file: cm map . --level 2 (look for test files)
  2. Check test scope: cm test-deps ./path/to/test_file.py
  3. Ensure test covers intended functionality")]
    TestDeps {
        /// Path to the test file to analyze
        test_file: PathBuf,

        /// Directory path to search for production symbols
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [GIT] Find when a symbol was last modified
    #[command(
        about = "Show who last modified a symbol and when",
        long_about = "USE CASE: Git blame for symbols, not lines
  • Find the commit that last modified a specific function/class/method
  • See author, date, and commit message
  • Compare old vs new signature if it changed

REQUIREMENTS:
  • Must be run inside a git repository
  • File must have git history

TIP: Use with 'cm history' to see full evolution of a symbol"
    )]
    #[command(after_help = "EXAMPLES:
  cm blame parse_file ./src/parser.rs          # Blame for parse_file function
  cm blame MyClass ./src/models.py --format ai # Token-efficient output
  cm blame validate ./utils.go --format human  # Pretty table

TYPICAL WORKFLOW:
  1. Find symbol: cm query my_func --fuzzy
  2. See last change: cm blame my_func ./path/to/file.rs
  3. See full history: cm history my_func ./path/to/file.rs")]
    Blame {
        /// Symbol name to blame
        symbol: String,

        /// Path to the file containing the symbol
        file: PathBuf,
    },

    /// [GIT] Show all commits that touched a symbol
    #[command(
        about = "Show the evolution of a symbol across git history",
        long_about = "USE CASE: Track how a symbol evolved over time
  • See all commits where a symbol was added, modified, or deleted
  • Track signature changes across versions
  • Understand when and why a function changed

REQUIREMENTS:
  • Must be run inside a git repository
  • File must have git history

CHANGE TRACKING:
  • Detects signature changes (parameter/return type modifications)
  • Detects body size changes (function grew or shrunk)
  • Shows when symbol was created or deleted

TIP: Combine with 'cm blame' for quick last-change info"
    )]
    #[command(after_help = "EXAMPLES:
  cm history parse_file ./src/parser.rs           # Full history
  cm history authenticate ./auth.py --format ai   # Token-efficient
  cm history MyClass ./models.go --format human   # Pretty table

TYPICAL WORKFLOW:
  1. Find symbol: cm query my_func --fuzzy
  2. See history: cm history my_func ./path/to/file.rs
  3. Compare specific versions using git diff")]
    History {
        /// Symbol name to track
        symbol: String,

        /// Path to the file containing the symbol
        file: PathBuf,
    },

    /// [ANALYSIS] Find all implementations of an interface/trait/protocol
    #[command(
        about = "Find all classes/structs that implement a given interface or trait",
        long_about = "USE CASE: Discover all implementors of an interface
  • Rust: Find `impl Trait for Type` patterns
  • Python: Find class inheritance `class Foo(Interface):`
  • TypeScript/JavaScript: Find `class Foo implements Interface`
  • Java: Find `implements` and `extends` clauses
  • Go: Find structs that embed interfaces

LANGUAGE PATTERNS:
  Rust     → impl Trait for Type, impl Type
  Python   → class Name(Interface):
  TS/JS    → class Name implements Interface, extends Parent
  Java     → class Name implements Interface, extends Parent
  Go       → type Name struct { Interface }

TIP: Use --fuzzy to find partial matches"
    )]
    #[command(after_help = "EXAMPLES:
  cm implements Iterator                    # Find all Iterator implementations
  cm implements Repository --fuzzy          # Fuzzy match implementations
  cm implements Handler ./src --format ai   # Search in src/, AI format
  cm implements Serializable --format human # Pretty table output

TYPICAL WORKFLOW:
  1. cm implements MyInterface --fuzzy
  2. Review which types implement the interface
  3. cm inspect <file> to see the implementation details

WHEN TO USE:
  ✓ \"What types implement this trait?\"
  ✓ \"Find all subclasses of this base class\"
  ✓ \"Which structs satisfy this interface?\"")]
    Implements {
        /// Interface, trait, or protocol name to search for
        interface: String,

        /// Directory path to search in
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Enable fuzzy matching for interface name
        #[arg(long, default_value = "false")]
        fuzzy: bool,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,

        /// Only show trait implementations (filter out inherent impls)
        #[arg(long, default_value_t = false)]
        trait_only: bool,
    },

    /// [ANALYSIS] Show type information for a symbol's parameters and return type
    #[command(
        about = "Analyze types used in a symbol's signature and locate their definitions",
        long_about = "USE CASE: Understand types flowing through a function/method
  • Extract parameter types and return type from signature
  • Locate where each custom type is defined in the codebase
  • Support for Rust, Python, TypeScript, Go, Java, and C

LANGUAGE SUPPORT:
  Rust     → fn name(x: Type) -> RetType
  Python   → def name(x: Type) -> RetType (type hints)
  TS/JS    → function name(x: Type): RetType
  Go       → func name(x Type) RetType
  Java     → RetType name(Type x)
  C        → RetType name(Type x)

OUTPUT:
  • Symbol signature
  • Table of parameter names, types, and where each type is defined
  • Return type with its definition location

TIP: Useful for understanding API boundaries and type dependencies"
    )]
    #[command(after_help = "EXAMPLES:
  cm types process_payment                    # Analyze types in process_payment
  cm types authenticate --fuzzy               # Fuzzy search for symbol
  cm types parse ./src --format human         # Pretty table output
  cm types validate_input --format ai         # Token-efficient for LLMs

TYPICAL WORKFLOW:
  1. Find function: cm query my_func --fuzzy
  2. Analyze types: cm types my_func
  3. Inspect type definition: cm query TypeName to see implementation

WHEN TO USE:
  ✓ \"What types does this function accept?\"
  ✓ \"Where is this parameter type defined?\"
  ✓ \"What's the return type and its definition?\"")]
    Types {
        /// Symbol name to analyze
        symbol: String,

        /// Directory path to search in
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Enable fuzzy matching for symbol name
        #[arg(long, default_value = "false")]
        fuzzy: bool,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [ANALYSIS] Show field structure for structs, classes, dataclasses, etc.
    #[command(
        about = "Display field schema for data structures (structs, classes, dataclasses, etc.)",
        long_about = "USE CASE: Understand the field structure of data types
  • Parse struct/class bodies to extract fields with types
  • Works with Rust structs, Python dataclasses/Pydantic, TypeScript interfaces, Java classes, Go structs
  • Shows field names, types, optionality, and default values

LANGUAGE SUPPORT:
  Rust       → struct fields (name: Type)
  Python     → dataclass, TypedDict, Pydantic BaseModel fields
  TypeScript → interface/class properties
  Java       → class fields
  Go         → struct fields

TIP: Use --fuzzy for flexible symbol matching"
    )]
    #[command(after_help = "EXAMPLES:
  cm schema User                           # Show fields of User struct/class
  cm schema Config --fuzzy                 # Fuzzy search for Config
  cm schema UserRequest ./src --format ai  # Token-efficient output
  cm schema MyModel --format human         # Pretty table format

TYPICAL WORKFLOW:
  1. Find class: cm query MyClass --fuzzy
  2. Show schema: cm schema MyClass
  3. Understand types: cm types related_function

WHEN TO USE:
  ✓ \"What fields does this struct have?\"
  ✓ \"What's the shape of this dataclass?\"
  ✓ \"Which fields are optional in this model?\"")]
    Schema {
        /// Symbol name (struct/class/interface name)
        symbol: String,

        /// Directory path to search in
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Enable fuzzy matching for symbol name
        #[arg(long, default_value = "false")]
        fuzzy: bool,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [UTILITY] Save current symbol state as a named snapshot
    #[command(
        about = "Save a snapshot of current codebase symbols for later comparison",
        long_about = "USE CASE: Create reference points for tracking code evolution
  • Save current symbol state as a named reference
  • Includes all symbols with signatures and locations
  • Records git commit hash if in a git repository
  • Stored in .codemapper/snapshots/<name>.json (override with --cache-dir or CODEMAPPER_CACHE_DIR)

COMMON USES:
  • Save baseline before major refactoring
  • Track API surface changes over time
  • Compare feature branches to main

TIP: Use 'cm compare <name>' to see changes since the snapshot"
    )]
    #[command(after_help = "EXAMPLES:
  cm snapshot baseline                      # Save snapshot named 'baseline'
  cm snapshot v1.0                          # Save snapshot named 'v1.0'
  cm snapshot pre-refactor --format human   # Save with confirmation table
  cm snapshot --list                        # List all saved snapshots
  cm snapshot --delete old-snap             # Delete a snapshot

TYPICAL WORKFLOW:
  1. Save baseline: cm snapshot before-changes
  2. Make code changes
  3. Compare: cm compare before-changes
  4. Review added/deleted/modified symbols")]
    Snapshot {
        /// Snapshot name (required unless --list or --delete)
        #[arg(required_unless_present_any = ["list", "delete"])]
        name: Option<String>,

        /// Directory path to analyze
        #[arg(default_value = ".")]
        path: PathBuf,

        /// List all saved snapshots
        #[arg(long, default_value_t = false)]
        list: bool,

        /// Delete a snapshot by name
        #[arg(long)]
        delete: Option<String>,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },

    /// [ANALYSIS] Compare current codebase to a saved snapshot
    #[command(
        about = "Show symbol-level changes between current code and a saved snapshot",
        long_about = "USE CASE: Track code evolution since a snapshot was taken
  • Compares current symbols against a previously saved snapshot
  • Shows ADDED, DELETED, MODIFIED, and SIGNATURE_CHANGED symbols
  • Like 'cm diff' but against a saved state instead of a git commit

CHANGE TYPES:
  ADDED            → New symbols that didn't exist in the snapshot
  DELETED          → Symbols that were removed since the snapshot
  MODIFIED         → Symbols with body/line changes (same signature)
  SIGNATURE_CHANGED → Symbols with parameter or return type changes

TIP: Save snapshots at key milestones for easy comparison"
    )]
    #[command(after_help = "EXAMPLES:
  cm compare baseline                     # Compare to 'baseline' snapshot
  cm compare v1.0 --format human          # Pretty table output
  cm compare pre-refactor --format ai     # Token-efficient for LLMs
  cm compare release --extensions py,rs   # Only Python and Rust files

TYPICAL WORKFLOW:
  1. Saved snapshot earlier: cm snapshot milestone
  2. Made code changes
  3. Review changes: cm compare milestone
  4. See what was added/deleted/modified")]
    Compare {
        /// Snapshot name to compare against
        snapshot: String,

        /// Directory path to analyze
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Comma-separated file extensions to include
        #[arg(
            long,
            default_value = "py,js,ts,jsx,tsx,rs,java,go,c,h,swift,rb,rbi,rake,gemspec,ru,gemfile,rakefile,capfile,guardfile,thorfile,podfile,fastfile,appraisals,dangerfile,md"
        )]
        extensions: String,

        /// Disable cache (always reindex)
        #[arg(long, default_value_t = false)]
        no_cache: bool,

        /// Force rebuild cache
        #[arg(long, default_value_t = false)]
        rebuild_cache: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let format = OutputFormat::from_str(&cli.format).unwrap_or_else(|err| {
        eprintln!("{}", err);
        std::process::exit(1);
    });

    let cache_dir = cli.cache_dir.as_deref();

    match cli.command {
        Commands::Stats {
            path,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            cmd_stats(path, extensions, no_cache, rebuild_cache, format, cache_dir)?;
        }
        Commands::Map {
            path,
            level,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            cmd_map(
                path,
                level,
                extensions,
                no_cache,
                rebuild_cache,
                format,
                cache_dir,
            )?;
        }
        Commands::Query {
            symbol,
            path,
            grep,
            r#type,
            context,
            show_body,
            fast,
            extensions,
            no_cache,
            rebuild_cache,
            full,
            exports_only,
            section,
            limit,
        } => {
            cmd_query(
                symbol,
                path,
                context,
                !grep,
                fast || grep,
                grep,
                show_body,
                r#type,
                section,
                extensions,
                no_cache,
                rebuild_cache,
                !full,
                exports_only,
                format,
                limit,
                cache_dir,
            )?;
        }
        Commands::Grep {
            pattern,
            path,
            r#type,
            context,
            show_body,
            extensions,
            no_cache,
            rebuild_cache,
            full,
            exports_only,
            section,
            limit,
        } => {
            cmd_query(
                pattern,
                path,
                context,
                false,
                true,
                true,
                show_body,
                r#type,
                section,
                extensions,
                no_cache,
                rebuild_cache,
                !full,
                exports_only,
                format,
                limit,
                cache_dir,
            )?;
        }
        Commands::Inspect {
            file_path,
            show_body,
            tree,
            level,
            section,
            sizes,
            code_blocks,
            full,
            exports_only,
        } => {
            cmd_inspect(
                file_path,
                show_body,
                tree,
                level,
                section,
                sizes,
                code_blocks,
                !full,
                exports_only,
                format,
            )?;
        }
        Commands::Deps {
            target,
            path,
            direction,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            cmd_deps(
                target,
                path,
                direction,
                extensions,
                no_cache,
                rebuild_cache,
                format,
                cache_dir,
            )?;
        }
        Commands::Index { path, extensions } => {
            cmd_index(path, extensions)?;
        }
        Commands::Diff {
            commit,
            path,
            extensions,
            full,
        } => {
            cmd_diff(commit, path, extensions, !full, format)?;
        }
        Commands::Callers {
            symbol,
            path,
            fuzzy,
            extensions,
            no_cache,
            rebuild_cache,
            limit,
        } => {
            cmd_callers(
                symbol,
                path,
                fuzzy,
                extensions,
                no_cache,
                rebuild_cache,
                limit,
                format,
                cache_dir,
            )?;
        }
        Commands::Callees {
            symbol,
            path,
            fuzzy,
            extensions,
            no_cache,
            rebuild_cache,
            limit,
        } => {
            cmd_callees(
                symbol,
                path,
                fuzzy,
                extensions,
                no_cache,
                rebuild_cache,
                limit,
                format,
                cache_dir,
            )?;
        }
        Commands::Tests {
            symbol,
            path,
            fuzzy,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            cmd_tests(
                symbol,
                path,
                fuzzy,
                extensions,
                no_cache,
                rebuild_cache,
                format,
                cache_dir,
            )?;
        }
        Commands::Untested {
            path,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            cmd_untested(path, extensions, no_cache, rebuild_cache, format, cache_dir)?;
        }
        Commands::Since {
            commit,
            path,
            extensions,
            breaking,
        } => {
            cmd_since(commit, path, extensions, breaking, format)?;
        }
        Commands::Entrypoints {
            path,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            cmd_entrypoints(path, extensions, no_cache, rebuild_cache, format, cache_dir)?;
        }
        Commands::Trace {
            from,
            to,
            path,
            fuzzy,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            cmd_trace(
                from,
                to,
                path,
                fuzzy,
                extensions,
                no_cache,
                rebuild_cache,
                format,
                cache_dir,
            )?;
        }
        Commands::Impact {
            symbol,
            path,
            grep,
            include_docs,
            limit,
            all,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            impact::cmd_impact(
                symbol,
                path,
                grep,
                include_docs,
                limit,
                all,
                extensions,
                no_cache,
                rebuild_cache,
                format,
                cache_dir,
            )?;
        }
        Commands::TestDeps {
            test_file,
            path,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            cmd_test_deps(
                test_file,
                path,
                extensions,
                no_cache,
                rebuild_cache,
                format,
                cache_dir,
            )?;
        }
        Commands::Blame { symbol, file } => {
            cmd_blame(symbol, file, format)?;
        }
        Commands::History { symbol, file } => {
            cmd_history(symbol, file, format)?;
        }
        Commands::Implements {
            interface,
            path,
            fuzzy,
            extensions,
            no_cache,
            rebuild_cache,
            trait_only,
        } => {
            cmd_implements(
                interface,
                path,
                fuzzy,
                extensions,
                no_cache,
                rebuild_cache,
                trait_only,
                format,
                cache_dir,
            )?;
        }
        Commands::Types {
            symbol,
            path,
            fuzzy,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            cmd_types(
                symbol,
                path,
                fuzzy,
                extensions,
                no_cache,
                rebuild_cache,
                format,
                cache_dir,
            )?;
        }
        Commands::Schema {
            symbol,
            path,
            fuzzy,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            cmd_schema(
                symbol,
                path,
                fuzzy,
                extensions,
                no_cache,
                rebuild_cache,
                format,
                cache_dir,
            )?;
        }
        Commands::Snapshot {
            name,
            path,
            list,
            delete,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            cmd_snapshot(
                name,
                path,
                list,
                delete,
                extensions,
                no_cache,
                rebuild_cache,
                format,
                cache_dir,
            )?;
        }
        Commands::Compare {
            snapshot,
            path,
            extensions,
            no_cache,
            rebuild_cache,
        } => {
            cmd_compare(
                snapshot,
                path,
                extensions,
                no_cache,
                rebuild_cache,
                format,
                cache_dir,
            )?;
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
    cache_dir: Option<&Path>,
) -> Result<index::CodeIndex> {
    use cache::CacheManager;

    // Skip cache if flags set
    if no_cache || rebuild_cache {
        if rebuild_cache {
            eprintln!("{} Rebuilding cache (--rebuild-cache)", "→".cyan());
            CacheManager::invalidate(path, extensions, cache_dir).ok(); // Ignore errors
        }
        let start = Instant::now();
        let index = indexer::index_directory(path, extensions)?;
        let elapsed_ms = start.elapsed().as_millis();

        // Save to cache only if indexing took >= 300ms (unless --no-cache)
        if !no_cache && elapsed_ms >= 300 {
            match CacheManager::save(&index, path, extensions, cache_dir) {
                Ok(_) => eprintln!(
                    "{} Cached index for future use ({}ms)",
                    "✓".green(),
                    elapsed_ms
                ),
                Err(e) => eprintln!("{} Warning: Failed to save cache: {}", "⚠".yellow(), e),
            }
        } else if !no_cache && elapsed_ms < 300 {
            eprintln!(
                "{} Indexed in {}ms (no cache needed for small repos)",
                "✓".green(),
                elapsed_ms
            );
        }

        return Ok(index);
    }

    // Try to load from cache
    match CacheManager::load(path, extensions, cache_dir) {
        Ok(Some((index, metadata, changed_files))) if changed_files.is_empty() => {
            // Cache hit - no changes
            eprintln!(
                "{} Loaded from cache - {} files, {} symbols",
                "✓".green(),
                metadata.file_count.to_string().bold(),
                metadata.symbol_count.to_string().bold()
            );
            Ok(index)
        }
        Ok(Some((mut index, metadata, changed_files))) => {
            // Incremental update needed
            eprintln!("{} Indexing, changes detected", "→".cyan());

            // Create progress bar
            let pb = ProgressBar::new(changed_files.len() as u64);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.cyan} [{bar:40.cyan/blue}] {percent}% ({pos}/{len} files)")
                    .unwrap()
                    .progress_chars("=>-"),
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
                            let language = models::Language::from_path(&path);

                            match indexer::index_file(
                                &path,
                                &content,
                                language,
                                change.hash.as_deref(),
                            ) {
                                Ok(file_info) => Some((change.clone(), file_info)),
                                Err(e) => {
                                    eprintln!(
                                        "{} Warning: Failed to parse {}: {}",
                                        "⚠".yellow(),
                                        path.display(),
                                        e
                                    );
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "{} Warning: Failed to read {}: {}",
                                "⚠".yellow(),
                                path.display(),
                                e
                            );
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
            match CacheManager::save_with_changes(
                &index,
                path,
                extensions,
                &metadata,
                &changed_files,
                cache_dir,
            ) {
                Ok(_) => eprintln!("{} Cache updated ({}ms)", "✓".green(), elapsed_ms),
                Err(e) => eprintln!("{} Warning: Failed to save cache: {}", "⚠".yellow(), e),
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
                    .progress_chars("=>-"),
            );

            let start = Instant::now();
            let index = indexer::index_directory_with_progress(path, extensions, Some(pb))?;
            let elapsed_ms = start.elapsed().as_millis();

            // Save to cache only if indexing took >= 300ms
            if elapsed_ms >= 300 {
                match CacheManager::save(&index, path, extensions, cache_dir) {
                    Ok(_) => eprintln!(
                        "{} Cache not found, created new cache ({} files, {}ms)",
                        "✓".green(),
                        index.total_files().to_string().bold(),
                        elapsed_ms
                    ),
                    Err(e) => eprintln!("{} Warning: Failed to save cache: {}", "⚠".yellow(), e),
                }
            } else {
                // Small repo - simple completion message
                eprintln!(
                    "{} Indexed ({} files, {}ms)",
                    "✓".green(),
                    index.total_files().to_string().bold(),
                    elapsed_ms
                );
            }

            Ok(index)
        }
        Err(e) => {
            // Cache error - fallback to rebuild
            eprintln!("{} Cache error: {}. Rebuilding...", "⚠".yellow(), e);
            let start = Instant::now();
            let index = indexer::index_directory(path, extensions)?;
            let elapsed_ms = start.elapsed().as_millis();

            eprintln!(
                "{} Indexed ({} files, {}ms)",
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

    println!(
        "{} Indexed {} files in {}ms",
        "✓".green(),
        index.total_files().to_string().bold(),
        elapsed_ms.to_string().bold()
    );
    println!(
        "{} Total symbols: {}",
        "→".cyan(),
        index.total_symbols().to_string().bold()
    );

    Ok(())
}

fn cmd_map(
    path: PathBuf,
    level: u8,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    if level < 1 || level > 3 {
        eprintln!("{} Level must be between 1 and 3", "Error:".red());
        std::process::exit(1);
    }

    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();

    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

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
    grep_mode: bool,
    show_body: bool,
    symbol_type_filter: Option<String>,
    section: Option<String>,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    skip_anonymous: bool,
    exports_only: bool,
    format: OutputFormat,
    limit: Option<usize>,
    cache_dir: Option<&Path>,
) -> Result<()> {
    use fast_search::GrepFilter;
    use models::SymbolType;

    let symbol = pipe::read_symbol_arg(symbol)?;
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();

    // Check if symbol terms are plural forms of symbol types (e.g., "functions", "classes")
    // Pipe-aware: split on |, check each term independently, partition into type filters vs search terms
    let (symbol, symbol_type_filter, extra_type_filters) = if symbol_type_filter.is_none() {
        let terms: Vec<&str> = symbol
            .split('|')
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();
        let mut plural_types: Vec<SymbolType> = Vec::new();
        let mut search_terms: Vec<&str> = Vec::new();
        for term in &terms {
            if let Some(plural_type) = SymbolType::from_plural(term) {
                plural_types.push(plural_type);
            } else {
                search_terms.push(term);
            }
        }
        if plural_types.is_empty() {
            (symbol, symbol_type_filter, Vec::new())
        } else if search_terms.is_empty() {
            // All terms are plurals — use first as primary type filter, rest as extras
            let primary = plural_types.remove(0);
            (
                String::new(),
                Some(primary.as_str().to_string()),
                plural_types,
            )
        } else {
            // Mix of plurals and search terms
            (search_terms.join("|"), symbol_type_filter, plural_types)
        }
    } else {
        (symbol, symbol_type_filter, Vec::new())
    };

    // Parse symbol type filter if provided
    let type_filter = if let Some(ref type_str) = symbol_type_filter {
        match SymbolType::from_str(type_str) {
            Some(t) => Some(t),
            None => {
                eprintln!("{} Invalid symbol type '{}', valid types: function, class, module, method, enum, static, heading, code_block, endpoint", "Error:".red(), type_str);
                return Ok(());
            }
        }
    } else {
        None
    };

    // Validate context level
    let context_lower = context.to_lowercase();
    if context_lower != "minimal" && context_lower != "full" {
        eprintln!(
            "Invalid context '{}'. Valid options: minimal, full",
            context
        );
        std::process::exit(1);
    }

    // Check if user wants all symbols of a specific type (empty symbol name with type filter)
    let search_all =
        symbol.trim().is_empty() && (type_filter.is_some() || !extra_type_filters.is_empty());

    // Count files for auto-detection
    let file_count = count_indexable_files(&path, &ext_list)?;

    // Auto-enable fast mode for large codebases (1000+ files), but not when searching for all symbols
    let use_fast_mode = !search_all && (fast || file_count >= 1000);

    if use_fast_mode {
        if grep_mode {
            eprintln!("{} Grep mode enabled ({} files)", "→".cyan(), file_count);
        } else if fast {
            eprintln!(
                "{} Fast mode enabled by --fast flag ({} files)",
                "→".cyan(),
                file_count
            );
        } else {
            eprintln!(
                "{} Fast mode auto-enabled ({} files detected)",
                "→".cyan(),
                file_count
            );
        }

        // Stage 1: Ripgrep prefilter
        let extensions_vec: Vec<String> = ext_list.iter().map(|s| s.to_string()).collect();
        let filter = GrepFilter::new(&symbol, !fuzzy, extensions_vec);

        let candidates = filter.prefilter(&path)?;

        if candidates.is_empty() {
            eprintln!(
                "{} No text matches found, falling back to full AST scan",
                "→".yellow()
            );
            // Fallback: Use normal mode with cache
            let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;
            let mut symbols = if search_all {
                index.all_symbols()
            } else if fuzzy {
                index.fuzzy_search(&symbol)
            } else {
                index.query_symbol(&symbol)
            };

            // Apply type filter if specified
            if let Some(ref filter_type) = type_filter {
                symbols.retain(|s| s.symbol_type == *filter_type);
            }

            // Merge in symbols matching extra type filters (from pipe plural syntax)
            if !extra_type_filters.is_empty() {
                let extra: Vec<&Symbol> = index
                    .all_symbols()
                    .into_iter()
                    .filter(|s| extra_type_filters.contains(&s.symbol_type))
                    .collect();
                symbols.extend(extra);
                dedup_symbols_by_ref(&mut symbols);
            }

            // Filter anonymous if requested
            if skip_anonymous {
                symbols.retain(|s| s.name != "anonymous");
            }

            // Filter to exports only if requested
            if exports_only {
                symbols.retain(|s| s.is_exported);
            }

            // Markdown section scoping (qualified name prefix)
            if let Some(ref sec) = section {
                let sec = sec.trim();
                if !sec.is_empty() {
                    let sec_prefix = format!("{} > ", sec);
                    symbols.retain(|s| {
                        s.file_path
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("md"))
                            .unwrap_or(false)
                            && (s.name == sec || s.name.starts_with(&sec_prefix))
                    });
                }
            }

            // Apply limit if specified
            if let Some(n) = limit {
                symbols.truncate(n);
            }

            if symbols.is_empty() {
                println!(
                    "{} No symbols found matching '{}'",
                    "✗".red(),
                    symbol.bold()
                );
                return Ok(());
            }

            let show_context = context.to_lowercase() == "full";
            let formatter = OutputFormatter::new(format);
            let output = formatter.format_query(symbols, show_context, show_body);
            println!("{}", output);
        } else {
            eprintln!(
                "{} Found {} candidate files, validating with AST...",
                "→".cyan(),
                candidates.len()
            );

            // Stage 2: AST validation
            let mut owned_symbols = filter.validate(candidates, &symbol, fuzzy)?;

            // Apply type filter if specified
            if let Some(ref filter_type) = type_filter {
                owned_symbols.retain(|s| s.symbol_type == *filter_type);
            }

            // Merge in symbols matching extra type filters (from pipe plural syntax)
            // Fast mode can't do this without a full index, so fall back to index for extras
            if !extra_type_filters.is_empty() {
                let index =
                    try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;
                let extra: Vec<Symbol> = index
                    .all_symbols()
                    .into_iter()
                    .filter(|s| extra_type_filters.contains(&s.symbol_type))
                    .cloned()
                    .collect();
                owned_symbols.extend(extra);
                dedup_symbols_owned(&mut owned_symbols);
            }

            // Filter anonymous if requested
            if skip_anonymous {
                owned_symbols.retain(|s| s.name != "anonymous");
            }

            // Filter to exports only if requested
            if exports_only {
                owned_symbols.retain(|s| s.is_exported);
            }

            // Markdown section scoping (qualified name prefix)
            if let Some(ref sec) = section {
                let sec = sec.trim();
                if !sec.is_empty() {
                    let sec_prefix = format!("{} > ", sec);
                    owned_symbols.retain(|s| {
                        s.file_path
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("md"))
                            .unwrap_or(false)
                            && (s.name == sec || s.name.starts_with(&sec_prefix))
                    });
                }
            }

            // Apply limit if specified
            if let Some(n) = limit {
                owned_symbols.truncate(n);
            }

            if owned_symbols.is_empty() {
                eprintln!(
                    "{} No AST matches in text candidates, falling back to full AST scan",
                    "→".yellow()
                );
                let index =
                    try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;
                let mut symbols = if fuzzy {
                    index.fuzzy_search(&symbol)
                } else {
                    index.query_symbol(&symbol)
                };

                if let Some(ref filter_type) = type_filter {
                    symbols.retain(|s| s.symbol_type == *filter_type);
                }

                if !extra_type_filters.is_empty() {
                    let extra: Vec<&Symbol> = index
                        .all_symbols()
                        .into_iter()
                        .filter(|s| extra_type_filters.contains(&s.symbol_type))
                        .collect();
                    symbols.extend(extra);
                    dedup_symbols_by_ref(&mut symbols);
                }

                if skip_anonymous {
                    symbols.retain(|s| s.name != "anonymous");
                }

                if exports_only {
                    symbols.retain(|s| s.is_exported);
                }

                if let Some(ref sec) = section {
                    let sec = sec.trim();
                    if !sec.is_empty() {
                        let sec_prefix = format!("{} > ", sec);
                        symbols.retain(|s| {
                            s.file_path
                                .extension()
                                .and_then(|e| e.to_str())
                                .map(|e| e.eq_ignore_ascii_case("md"))
                                .unwrap_or(false)
                                && (s.name == sec || s.name.starts_with(&sec_prefix))
                        });
                    }
                }

                if let Some(n) = limit {
                    symbols.truncate(n);
                }

                if symbols.is_empty() {
                    println!(
                        "{} No symbols found matching '{}'",
                        "✗".red(),
                        symbol.bold()
                    );
                    return Ok(());
                }

                let show_context = context.to_lowercase() == "full";
                let formatter = OutputFormatter::new(format);
                let output = formatter.format_query(symbols, show_context, show_body);
                println!("{}", output);
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
        let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;
        let mut symbols = if search_all {
            // Get all symbols when searching for all of a specific type
            index.all_symbols()
        } else if fuzzy {
            index.fuzzy_search(&symbol)
        } else {
            index.query_symbol(&symbol)
        };

        // Apply type filter if specified
        if let Some(ref filter_type) = type_filter {
            symbols.retain(|s| s.symbol_type == *filter_type);
        }

        // Merge in symbols matching extra type filters (from pipe plural syntax)
        if !extra_type_filters.is_empty() {
            let extra: Vec<&Symbol> = index
                .all_symbols()
                .into_iter()
                .filter(|s| extra_type_filters.contains(&s.symbol_type))
                .collect();
            symbols.extend(extra);
            dedup_symbols_by_ref(&mut symbols);
        }

        // Filter anonymous if requested
        if skip_anonymous {
            symbols.retain(|s| s.name != "anonymous");
        }

        // Filter to exports only if requested
        if exports_only {
            symbols.retain(|s| s.is_exported);
        }

        // Markdown section scoping (qualified name prefix)
        if let Some(ref sec) = section {
            let sec = sec.trim();
            if !sec.is_empty() {
                let sec_prefix = format!("{} > ", sec);
                symbols.retain(|s| {
                    s.file_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.eq_ignore_ascii_case("md"))
                        .unwrap_or(false)
                        && (s.name == sec || s.name.starts_with(&sec_prefix))
                });
            }
        }

        // Apply limit if specified
        if let Some(n) = limit {
            symbols.truncate(n);
        }

        if symbols.is_empty() {
            println!(
                "{} No symbols found matching '{}'",
                "✗".red(),
                symbol.bold()
            );
            return Ok(());
        }

        let show_context = context.to_lowercase() == "full";
        let formatter = OutputFormatter::new(format);
        let output = formatter.format_query(symbols, show_context, show_body);
        println!("{}", output);
    }

    Ok(())
}

fn dedup_symbols_by_ref<'a>(symbols: &mut Vec<&'a Symbol>) {
    let mut seen = std::collections::HashSet::new();
    symbols.retain(|s| {
        let key = (&s.name, s.line_start, &s.file_path);
        seen.insert((key.0.clone(), key.1, key.2.clone()))
    });
}

fn dedup_symbols_owned(symbols: &mut Vec<Symbol>) {
    let mut seen = std::collections::HashSet::new();
    symbols.retain(|s| {
        let key = (s.name.clone(), s.line_start, s.file_path.clone());
        seen.insert(key)
    });
}

/// Count indexable files in directory for auto-detection logic
fn count_indexable_files(path: &PathBuf, extensions: &[&str]) -> Result<usize> {
    use ::ignore::WalkBuilder;

    use crate::ignore;

    let mut count = 0;
    let walker = WalkBuilder::new(path)
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

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        if indexer::matches_extension_filter(entry.path(), extensions) {
            count += 1;
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
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

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
    format: OutputFormat,
) -> Result<()> {
    use std::path::PathBuf;

    let target_path = PathBuf::from(&target);
    let target_canonical = std::fs::canonicalize(&target_path).unwrap_or(target_path.clone());

    let deps = if direction.to_lowercase() == "imports" {
        // Try both relative and canonical paths
        index
            .get_dependencies(&target_path)
            .or_else(|| index.get_dependencies(&target_canonical))
            .map(|d| d.clone())
            .unwrap_or_default()
    } else if direction.to_lowercase() == "used-by" {
        let mut used_by = Vec::new();
        for file in index.files() {
            if let Some(file_deps) = index.get_dependencies(&file.path) {
                let target_name = target_canonical
                    .file_name()
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
        eprintln!(
            "{} Invalid direction '{}', use 'imports' or 'used-by'",
            "Error:".red(),
            direction
        );
        std::process::exit(1);
    };

    if deps.is_empty() {
        println!("{} No dependencies found for {}", "✗".yellow(), target);
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
    format: OutputFormat,
) -> Result<()> {
    use std::fs;

    if direction.to_lowercase() != "used-by" {
        eprintln!(
            "{} For symbols, only '--direction used-by' is supported",
            "Error:".red()
        );
        eprintln!("{} Use a file path to see imports", "Hint:".cyan());
        std::process::exit(1);
    }

    // Find the symbol definition
    let symbols = index.query_symbol(&symbol_name);

    if symbols.is_empty() {
        println!(
            "{} Symbol '{}' not found in codebase",
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
                        s.file_path == file.path
                            && (line_num + 1) >= s.line_start
                            && (line_num + 1) <= s.line_end
                    });

                    if !is_definition {
                        usages.push(format!("{}:{}", file.path.display(), line_num + 1));
                    }
                }
            }
        }
    }

    if usages.is_empty() {
        println!(
            "{} No usages found for symbol '{}'",
            "✗".yellow(),
            symbol_name.bold()
        );
        return Ok(());
    }

    // Show summary first
    println!(
        "{} Found {} usage(s) of '{}'\n",
        "✓".green(),
        usages.len().to_string().bold(),
        symbol_name.bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_deps(&symbol_name, usages, "used-by");

    println!("{}", output);

    Ok(())
}

fn cmd_stats(
    path: PathBuf,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();

    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_stats(&index);

    println!("{}", output);

    Ok(())
}

fn cmd_inspect(
    file_path: PathBuf,
    show_body: bool,
    tree: bool,
    level: Option<usize>,
    section: Option<String>,
    sizes: bool,
    code_blocks: bool,
    skip_anonymous: bool,
    exports_only: bool,
    format: OutputFormat,
) -> Result<()> {
    use std::fs;

    if pipe::is_stdin_path(&file_path) {
        for path in pipe::read_path_arg(file_path)? {
            cmd_inspect(
                path,
                show_body,
                tree,
                level,
                section.clone(),
                sizes,
                code_blocks,
                skip_anonymous,
                exports_only,
                format,
            )?;
        }
        return Ok(());
    }

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
    let mut file_info = indexer::index_file(&file_path, &content, language, None)?;
    let elapsed_ms = start.elapsed().as_millis();

    // Filter anonymous if requested
    if skip_anonymous {
        file_info.symbols.retain(|s| s.name != "anonymous");
    }

    // Filter to exports only if requested
    if exports_only {
        file_info.symbols.retain(|s| s.is_exported);
    }

    if file_info.symbols.is_empty() {
        println!(
            "{} No symbols found in {}",
            "✗".yellow(),
            file_path.display()
        );
        return Ok(());
    }

    let is_markdown = language == models::Language::Markdown;
    let wants_markdown_view =
        is_markdown && (tree || level.is_some() || section.is_some() || sizes || code_blocks);

    if wants_markdown_view {
        use crate::inspect_md::{
            compute_heading_end_lines, heading_leaf, heading_level, is_descendant,
            nearest_printable_heading_ancestor, select_section_heading,
        };
        use std::collections::{HashMap, HashSet};

        let total_lines = content.lines().count().max(1);

        let headings: Vec<(usize, usize, usize)> = file_info
            .symbols
            .iter()
            .enumerate()
            .filter(|(_, s)| s.symbol_type == models::SymbolType::Heading)
            .filter_map(|(i, s)| heading_level(s).map(|lvl| (i, lvl, s.line_start)))
            .collect();

        let end_lines = compute_heading_end_lines(&headings, total_lines);

        let section_root_idx = match section.as_deref() {
            Some(s) => Some(select_section_heading(&file_info.symbols, s)?),
            None => None,
        };

        let section_root_level =
            section_root_idx.and_then(|idx| heading_level(&file_info.symbols[idx]));

        let max_level_abs = level.map(|n| {
            if let Some(root_level) = section_root_level {
                root_level.saturating_add(n.saturating_sub(1))
            } else {
                n
            }
        });

        let in_scope = |idx: usize| -> bool {
            section_root_idx
                .map(|root| is_descendant(&file_info.symbols, idx, root))
                .unwrap_or(true)
        };

        let heading_allowed = |idx: usize, lvl: usize| -> bool {
            if !in_scope(idx) {
                return false;
            }
            max_level_abs.map(|max| lvl <= max).unwrap_or(true)
        };

        let base_level = section_root_level.unwrap_or(1);

        if tree {
            #[derive(Clone)]
            enum EntryKind {
                Heading {
                    idx: usize,
                    level: usize,
                },
                CodeBlock {
                    idx: usize,
                    parent: usize,
                    parent_level: usize,
                },
            }

            #[derive(Clone)]
            struct Entry {
                line: usize,
                order: u8,
                kind: EntryKind,
            }

            let mut printable_headings = HashSet::<usize>::new();
            for (idx, lvl, _line) in &headings {
                if heading_allowed(*idx, *lvl) {
                    printable_headings.insert(*idx);
                }
            }

            let mut entries = Vec::<Entry>::new();
            for (idx, lvl, line) in &headings {
                if !printable_headings.contains(idx) {
                    continue;
                }
                entries.push(Entry {
                    line: *line,
                    order: 0,
                    kind: EntryKind::Heading {
                        idx: *idx,
                        level: *lvl,
                    },
                });
            }

            if code_blocks {
                for (idx, sym) in file_info.symbols.iter().enumerate() {
                    if sym.symbol_type != models::SymbolType::CodeBlock {
                        continue;
                    }
                    if !in_scope(idx) {
                        continue;
                    }

                    let parent = sym
                        .parent_id
                        .filter(|pid| printable_headings.contains(pid))
                        .or_else(|| {
                            nearest_printable_heading_ancestor(
                                &file_info.symbols,
                                idx,
                                &printable_headings,
                            )
                        });

                    let Some(parent_idx) = parent else {
                        continue;
                    };

                    let Some(parent_level_abs) = heading_level(&file_info.symbols[parent_idx])
                    else {
                        continue;
                    };

                    entries.push(Entry {
                        line: sym.line_start,
                        order: 1,
                        kind: EntryKind::CodeBlock {
                            idx,
                            parent: parent_idx,
                            parent_level: parent_level_abs,
                        },
                    });
                }
            }

            entries.sort_by_key(|e| (e.line, e.order));

            let mut parent_indent_cache = HashMap::<usize, usize>::new();
            let indent_for_heading =
                |level_abs: usize| -> usize { level_abs.saturating_sub(base_level) + 1 };

            let mut out_lines = Vec::<String>::new();
            for entry in entries {
                match entry.kind {
                    EntryKind::Heading { idx, level } => {
                        let sym = &file_info.symbols[idx];
                        let lvl = indent_for_heading(level);
                        let indent = "  ".repeat(lvl.saturating_sub(1));
                        let hashes = "#".repeat(lvl.min(6));
                        let leaf = heading_leaf(&sym.name);
                        let size = end_lines
                            .get(&idx)
                            .copied()
                            .unwrap_or(total_lines)
                            .saturating_sub(sym.line_start)
                            + 1;

                        parent_indent_cache.insert(idx, lvl);

                        if sizes {
                            out_lines.push(format!(
                                "{}{} {} (L{}, {} lines)",
                                indent, hashes, leaf, sym.line_start, size
                            ));
                        } else {
                            out_lines.push(format!(
                                "{}{} {} (L{})",
                                indent, hashes, leaf, sym.line_start
                            ));
                        }
                    }
                    EntryKind::CodeBlock {
                        idx,
                        parent,
                        parent_level,
                    } => {
                        let sym = &file_info.symbols[idx];
                        let parent_lvl = *parent_indent_cache
                            .entry(parent)
                            .or_insert_with(|| indent_for_heading(parent_level));
                        let indent = "  ".repeat(parent_lvl);
                        let leaf = heading_leaf(&sym.name);
                        out_lines.push(format!(
                            "{}- {} (L{}-L{})",
                            indent, leaf, sym.line_start, sym.line_end
                        ));
                    }
                }
            }

            match format {
                OutputFormat::AI => {
                    println!("[FILE:{}]", file_path.display());
                    println!(
                        "LANG:{} SIZE:{} SYMS:{}",
                        language.as_str(),
                        file_info.size,
                        out_lines.len()
                    );
                    println!("[TREE]");
                    for l in out_lines {
                        println!("{}", l);
                    }
                }
                _ => {
                    println!(
                        "{} Inspecting: {}\n",
                        "→".cyan(),
                        file_path.display().to_string().bold()
                    );
                    println!("Language: {}", language.as_str());
                    println!("Size: {} bytes\n", file_info.size);
                    for l in out_lines {
                        println!("{}", l);
                    }
                }
            }
        } else {
            let mut filtered_idxs: Vec<usize> = (0..file_info.symbols.len()).collect();

            if let Some(root) = section_root_idx {
                filtered_idxs.retain(|idx| is_descendant(&file_info.symbols, *idx, root));
            }

            if code_blocks {
                filtered_idxs.retain(|idx| {
                    file_info.symbols[*idx].symbol_type == models::SymbolType::CodeBlock
                });
            } else if section_root_idx.is_none() {
                if let Some(max) = max_level_abs {
                    filtered_idxs.retain(|idx| {
                        let s = &file_info.symbols[*idx];
                        s.symbol_type == models::SymbolType::Heading
                            && heading_level(s).map(|lvl| lvl <= max).unwrap_or(false)
                    });
                }
            }

            let sym_count = filtered_idxs.len();

            match format {
                OutputFormat::AI => {
                    println!("[FILE:{}]", file_path.display());
                    println!(
                        "LANG:{} SIZE:{} SYMS:{}",
                        language.as_str(),
                        file_info.size,
                        sym_count
                    );

                    for idx in &filtered_idxs {
                        let symbol = &file_info.symbols[*idx];
                        print!(
                            "{}|{}|{}-{}",
                            symbol.name,
                            match symbol.symbol_type {
                                models::SymbolType::Function => "f",
                                models::SymbolType::Class => "c",
                                models::SymbolType::Method => "m",
                                models::SymbolType::Enum => "e",
                                models::SymbolType::StaticField => "s",
                                models::SymbolType::Heading => "h",
                                models::SymbolType::CodeBlock => "cb",
                                models::SymbolType::Endpoint => "ep",
                                models::SymbolType::Interface => "if",
                                models::SymbolType::TypeAlias => "ty",
                                models::SymbolType::Module => "mod",
                            },
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
                    let formatter = OutputFormatter::new(format);
                    println!(
                        "{} Inspecting: {}\n",
                        "→".cyan(),
                        file_path.display().to_string().bold()
                    );
                    println!("Language: {}", language.as_str());
                    println!("Size: {} bytes", file_info.size);
                    println!("Symbols: {}\n", sym_count);
                    let symbol_refs: Vec<&models::Symbol> = filtered_idxs
                        .iter()
                        .map(|i| &file_info.symbols[*i])
                        .collect();
                    let output = formatter.format_query(symbol_refs, false, show_body);
                    println!("{}", output);
                }
            }
        }

        if let Some(root) = section_root_idx {
            if show_body {
                let start_line = file_info.symbols[root].line_start;
                let end_line = end_lines.get(&root).copied().unwrap_or(total_lines);
                let slice = content
                    .lines()
                    .skip(start_line.saturating_sub(1))
                    .take(end_line.saturating_sub(start_line) + 1)
                    .collect::<Vec<_>>()
                    .join("\n");

                match format {
                    OutputFormat::AI => {
                        println!("[SECTION_BODY:{}-{}]", start_line, end_line);
                        println!("{}", slice);
                    }
                    _ => {
                        println!("\nSection body (L{}-L{}):\n", start_line, end_line);
                        println!("{}", slice);
                    }
                }
            }
        }
    } else {
        let formatter = OutputFormatter::new(format);

        match format {
            OutputFormat::AI => {
                println!("[FILE:{}]", file_path.display());
                println!(
                    "LANG:{} SIZE:{} SYMS:{}",
                    language.as_str(),
                    file_info.size,
                    file_info.symbols.len()
                );
                for symbol in &file_info.symbols {
                    print!(
                        "{}|{}|{}-{}",
                        symbol.name,
                        match symbol.symbol_type {
                            models::SymbolType::Function => "f",
                            models::SymbolType::Class => "c",
                            models::SymbolType::Method => "m",
                            models::SymbolType::Enum => "e",
                            models::SymbolType::StaticField => "s",
                            models::SymbolType::Heading => "h",
                            models::SymbolType::CodeBlock => "cb",
                            models::SymbolType::Endpoint => "ep",
                            models::SymbolType::Interface => "if",
                            models::SymbolType::TypeAlias => "ty",
                            models::SymbolType::Module => "mod",
                        },
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
                println!(
                    "{} Inspecting: {}\n",
                    "→".cyan(),
                    file_path.display().to_string().bold()
                );
                println!("Language: {}", language.as_str());
                println!("Size: {} bytes", file_info.size);
                println!("Symbols: {}\n", file_info.symbols.len());

                let symbol_refs: Vec<&models::Symbol> = file_info.symbols.iter().collect();
                let output = formatter.format_query(symbol_refs, false, show_body);
                println!("{}", output);
            }
        }
    }

    println!(
        "\n{} Parse time: {}ms",
        "→".cyan(),
        elapsed_ms.to_string().bold()
    );

    Ok(())
}

fn cmd_diff(
    commit: String,
    path: PathBuf,
    extensions: String,
    skip_anonymous: bool,
    format: OutputFormat,
) -> Result<()> {
    use std::time::Instant;

    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();

    eprintln!(
        "{} Computing symbol diff against {}...",
        "→".cyan(),
        commit.bold()
    );

    let start = Instant::now();

    let subpath = if path == PathBuf::from(".") {
        None
    } else {
        Some(path.as_path())
    };

    let mut result = diff::compute_diff(&std::env::current_dir()?, &commit, subpath, &ext_list)?;
    let elapsed_ms = start.elapsed().as_millis();

    // Filter anonymous if requested
    if skip_anonymous {
        result.symbols.retain(|s| s.name != "anonymous");
    }

    eprintln!(
        "{} Analyzed {} files in {}ms\n",
        "✓".green(),
        result.files_analyzed.to_string().bold(),
        elapsed_ms.to_string().bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_diff(&result);
    println!("{}", output);

    Ok(())
}

fn cmd_since(
    commit: String,
    path: PathBuf,
    extensions: String,
    breaking: bool,
    format: OutputFormat,
) -> Result<()> {
    use diff::ChangeType;
    use std::time::Instant;

    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();

    let label = if breaking {
        "breaking changes"
    } else {
        "changes"
    };
    eprintln!(
        "{} Finding {} since {}...",
        "→".cyan(),
        label,
        commit.bold()
    );

    let start = Instant::now();

    let subpath = if path == PathBuf::from(".") {
        None
    } else {
        Some(path.as_path())
    };

    let mut result = diff::compute_diff(&std::env::current_dir()?, &commit, subpath, &ext_list)?;
    let elapsed_ms = start.elapsed().as_millis();

    if breaking {
        result.symbols.retain(|s| {
            matches!(
                s.change_type,
                ChangeType::Deleted | ChangeType::SignatureChanged
            )
        });
    }

    eprintln!(
        "{} Analyzed {} files in {}ms\n",
        "✓".green(),
        result.files_analyzed.to_string().bold(),
        elapsed_ms.to_string().bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = if breaking {
        formatter.format_breaking(&result)
    } else {
        formatter.format_diff(&result)
    };
    println!("{}", output);

    Ok(())
}

fn normalize_qualified_name(name: &str) -> String {
    let trimmed = name.trim();

    if let Some((_, tail)) = trimmed.rsplit_once("::") {
        return tail.to_string();
    }
    if let Some((_, tail)) = trimmed.rsplit_once('.') {
        return tail.to_string();
    }

    trimmed.to_string()
}

fn qualifier_pattern(name: &str) -> Option<String> {
    let trimmed = name.trim();

    if let Some((prefix, tail)) = trimmed.rsplit_once("::") {
        if !prefix.is_empty() && !tail.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some((prefix, tail)) = trimmed.rsplit_once('.') {
        if !prefix.is_empty() && !tail.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

fn cmd_callers(
    symbol: String,
    path: PathBuf,
    fuzzy: bool,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    limit: Option<usize>,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let symbol = pipe::read_symbol_arg(symbol)?;
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

    let original_symbol = symbol;
    let symbol = normalize_qualified_name(&original_symbol);

    eprintln!(
        "{} Finding callers of '{}'...",
        "→".cyan(),
        original_symbol.bold()
    );

    let mut symbols = if fuzzy {
        index.fuzzy_search(&symbol)
    } else {
        index.query_symbol(&symbol)
    };

    if symbols.is_empty() {
        // allow Enum::Variant lookup via our variant indexing
        if let Some(pattern) = qualifier_pattern(&original_symbol) {
            symbols = if fuzzy {
                index.fuzzy_search(&pattern)
            } else {
                index.query_symbol(&pattern)
            };
        }
    }

    if symbols.is_empty() {
        println!(
            "{} Symbol '{}' not found in codebase",
            "✗".yellow(),
            symbol.bold()
        );
        return Ok(());
    }

    let start = Instant::now();
    let mut callers = callgraph::find_callers(&index, &original_symbol, fuzzy)?;
    let elapsed_ms = start.elapsed().as_millis();

    if callers.is_empty() {
        println!("{} No callers found for '{}'", "✗".yellow(), symbol.bold());
        return Ok(());
    }

    let total_count = callers.len();
    let truncated = if let Some(lim) = limit {
        if callers.len() > lim {
            callers.truncate(lim);
            true
        } else {
            false
        }
    } else {
        false
    };

    eprintln!(
        "{} Found {} call site(s) in {}ms{}\n",
        "✓".green(),
        total_count.to_string().bold(),
        elapsed_ms.to_string().bold(),
        if truncated {
            format!(" (showing first {})", limit.unwrap())
        } else {
            String::new()
        }
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_callers(&callers, &original_symbol);
    println!("{}", output);

    Ok(())
}

fn cmd_callees(
    symbol: String,
    path: PathBuf,
    fuzzy: bool,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    limit: Option<usize>,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let symbol = pipe::read_symbol_arg(symbol)?;
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

    let original_symbol = symbol;
    let symbol = normalize_qualified_name(&original_symbol);

    eprintln!(
        "{} Finding callees of '{}'...",
        "→".cyan(),
        original_symbol.bold()
    );

    let mut symbols = if fuzzy {
        index.fuzzy_search(&symbol)
    } else {
        index.query_symbol(&symbol)
    };

    if symbols.is_empty() {
        if let Some(pattern) = qualifier_pattern(&original_symbol) {
            symbols = if fuzzy {
                index.fuzzy_search(&pattern)
            } else {
                index.query_symbol(&pattern)
            };
        }
    }

    if symbols.is_empty() {
        println!(
            "{} Symbol '{}' not found in codebase",
            "✗".yellow(),
            symbol.bold()
        );
        return Ok(());
    }

    let start = Instant::now();
    let mut callees = callgraph::find_callees(&index, &symbol, fuzzy)?;
    let elapsed_ms = start.elapsed().as_millis();

    if callees.is_empty() {
        println!("{} No callees found for '{}'", "✗".yellow(), symbol.bold());
        return Ok(());
    }

    let total_count = callees.len();
    let truncated = if let Some(lim) = limit {
        if callees.len() > lim {
            callees.truncate(lim);
            true
        } else {
            false
        }
    } else {
        false
    };

    eprintln!(
        "{} Found {} callee(s) in {}ms{}\n",
        "✓".green(),
        total_count.to_string().bold(),
        elapsed_ms.to_string().bold(),
        if truncated {
            format!(" (showing first {})", limit.unwrap())
        } else {
            String::new()
        }
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_callees(&callees, &original_symbol);
    println!("{}", output);

    Ok(())
}

fn cmd_tests(
    symbol: String,
    path: PathBuf,
    fuzzy: bool,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let symbol = pipe::read_symbol_arg(symbol)?;
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

    let original_symbol = symbol;
    let symbol = normalize_qualified_name(&original_symbol);

    eprintln!(
        "{} Finding tests for '{}'...",
        "→".cyan(),
        original_symbol.bold()
    );

    let start = Instant::now();
    let tests = callgraph::find_tests(&index, &original_symbol, fuzzy)?;
    let elapsed_ms = start.elapsed().as_millis();

    if tests.is_empty() {
        println!("{} No tests found for '{}'", "✗".yellow(), symbol.bold());
        return Ok(());
    }

    eprintln!(
        "{} Found {} test(s) in {}ms\n",
        "✓".green(),
        tests.len().to_string().bold(),
        elapsed_ms.to_string().bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_tests(&tests, &original_symbol);
    println!("{}", output);

    Ok(())
}

fn cmd_untested(
    path: PathBuf,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

    eprintln!("{} Finding untested symbols...", "→".cyan());

    let start = Instant::now();
    let untested = callgraph::find_untested(&index)?;
    let elapsed_ms = start.elapsed().as_millis();

    let total_symbols = index.total_symbols();
    let tested_count = total_symbols.saturating_sub(untested.len());
    let coverage_pct = if total_symbols > 0 {
        (tested_count as f64 / total_symbols as f64) * 100.0
    } else {
        100.0
    };

    eprintln!(
        "{} Analyzed {} symbols in {}ms (coverage: {:.1}%)\n",
        "✓".green(),
        total_symbols.to_string().bold(),
        elapsed_ms.to_string().bold(),
        coverage_pct
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_untested(&untested, total_symbols);
    println!("{}", output);

    Ok(())
}

fn cmd_entrypoints(
    path: PathBuf,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

    eprintln!(
        "{} Finding entrypoints (uncalled exported symbols)...",
        "→".cyan()
    );

    let start = Instant::now();
    let entrypoints = callgraph::find_entrypoints(&index)?;
    let elapsed_ms = start.elapsed().as_millis();

    if entrypoints.is_empty() {
        println!(
            "{} No entrypoints found (all exported symbols have internal callers)",
            "✓".green()
        );
        return Ok(());
    }

    let main_count = entrypoints
        .iter()
        .filter(|e| e.category == callgraph::EntrypointCategory::MainEntry)
        .count();
    let api_count = entrypoints
        .iter()
        .filter(|e| e.category == callgraph::EntrypointCategory::ApiFunction)
        .count();
    let unused_count = entrypoints
        .iter()
        .filter(|e| e.category == callgraph::EntrypointCategory::PossiblyUnused)
        .count();

    eprintln!(
        "{} Found {} entrypoint(s) in {}ms ({} main, {} API, {} possibly unused)\n",
        "✓".green(),
        entrypoints.len().to_string().bold(),
        elapsed_ms.to_string().bold(),
        main_count,
        api_count,
        unused_count
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_entrypoints(&entrypoints);
    println!("{}", output);

    Ok(())
}

fn cmd_trace(
    from: String,
    to: String,
    path: PathBuf,
    fuzzy: bool,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let from = pipe::read_symbol_arg(from)?;
    let to = pipe::read_symbol_arg(to)?;
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

    let match_type = if fuzzy { " (fuzzy)" } else { "" };
    eprintln!(
        "{} Tracing call path from '{}' to '{}'{}...",
        "→".cyan(),
        from.bold(),
        to.bold(),
        match_type
    );

    let start = Instant::now();
    let trace = callgraph::trace_path(&index, &from, &to, fuzzy)?;
    let elapsed_ms = start.elapsed().as_millis();

    if !trace.found {
        println!(
            "{} No call path found from '{}' to '{}'",
            "✗".yellow(),
            from.bold(),
            to.bold()
        );
        return Ok(());
    }

    eprintln!(
        "{} Found path with {} step(s) in {}ms\n",
        "✓".green(),
        trace.steps.len().to_string().bold(),
        elapsed_ms.to_string().bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_trace(&trace, &from, &to);
    println!("{}", output);

    Ok(())
}

fn cmd_test_deps(
    test_file: PathBuf,
    path: PathBuf,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    if pipe::is_stdin_path(&test_file) {
        for file in pipe::read_path_arg(test_file)? {
            cmd_test_deps(
                file,
                path.clone(),
                extensions.clone(),
                no_cache,
                rebuild_cache,
                format,
                cache_dir,
            )?;
        }
        return Ok(());
    }

    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();

    let abs_test_file = if test_file.is_absolute() {
        test_file.clone()
    } else {
        std::env::current_dir()?.join(&test_file)
    };

    if !abs_test_file.exists() {
        anyhow::bail!("Test file not found: {}", test_file.display());
    }

    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

    eprintln!(
        "{} Analyzing test file '{}'...",
        "→".cyan(),
        test_file.display().to_string().bold()
    );

    let start = Instant::now();
    let deps = callgraph::find_test_deps(&index, &abs_test_file)?;
    let elapsed_ms = start.elapsed().as_millis();

    if deps.is_empty() {
        println!(
            "{} No production symbols found in '{}'",
            "✗".yellow(),
            test_file.display().to_string().bold()
        );
        return Ok(());
    }

    eprintln!(
        "{} Found {} production symbol(s) in {}ms\n",
        "✓".green(),
        deps.len().to_string().bold(),
        elapsed_ms.to_string().bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_test_deps(&deps, &test_file.display().to_string());
    println!("{}", output);

    Ok(())
}

fn cmd_blame(symbol: String, file: PathBuf, format: OutputFormat) -> Result<()> {
    let symbol = pipe::read_symbol_arg(symbol)?;
    eprintln!(
        "{} Finding last modification of '{}'...",
        "→".cyan(),
        symbol.bold()
    );

    let start = Instant::now();
    let cwd = std::env::current_dir()?;
    let result = blame::blame_symbol(&cwd, &file, &symbol)?;
    let elapsed_ms = start.elapsed().as_millis();

    eprintln!(
        "{} Found blame info in {}ms\n",
        "✓".green(),
        elapsed_ms.to_string().bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_blame(&result);
    println!("{}", output);

    Ok(())
}

fn cmd_history(symbol: String, file: PathBuf, format: OutputFormat) -> Result<()> {
    let symbol = pipe::read_symbol_arg(symbol)?;
    eprintln!("{} Tracing history of '{}'...", "→".cyan(), symbol.bold());

    let start = Instant::now();
    let cwd = std::env::current_dir()?;
    let history = blame::history_symbol(&cwd, &file, &symbol)?;
    let elapsed_ms = start.elapsed().as_millis();

    if history.is_empty() {
        println!("{} No history found for '{}'", "✗".yellow(), symbol.bold());
        return Ok(());
    }

    eprintln!(
        "{} Found {} version(s) in {}ms\n",
        "✓".green(),
        history.len().to_string().bold(),
        elapsed_ms.to_string().bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_history(&history, &symbol);
    println!("{}", output);

    Ok(())
}

fn cmd_implements(
    interface: String,
    path: PathBuf,
    fuzzy: bool,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    trait_only: bool,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let interface = pipe::read_symbol_arg(interface)?;
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

    let match_type = if fuzzy { " (fuzzy)" } else { "" };
    eprintln!(
        "{} Finding implementations of '{}'{}...",
        "→".cyan(),
        interface.bold(),
        match_type
    );

    let start = Instant::now();
    let implementations = implements::find_implementations(&index, &interface, fuzzy, trait_only)?;
    let elapsed_ms = start.elapsed().as_millis();

    if implementations.is_empty() {
        println!(
            "{} No implementations found for '{}'",
            "✗".yellow(),
            interface.bold()
        );
        return Ok(());
    }

    eprintln!(
        "{} Found {} implementation(s) in {}ms\n",
        "✓".green(),
        implementations.len().to_string().bold(),
        elapsed_ms.to_string().bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_implements(&implementations, &interface);
    println!("{}", output);

    Ok(())
}

fn cmd_types(
    symbol: String,
    path: PathBuf,
    fuzzy: bool,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let symbol = pipe::read_symbol_arg(symbol)?;
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

    let match_type = if fuzzy { " (fuzzy)" } else { "" };
    eprintln!(
        "{} Analyzing types for '{}'{}...",
        "→".cyan(),
        symbol.bold(),
        match_type
    );

    let start = Instant::now();
    let types_info = types::analyze_types(&index, &symbol, fuzzy)?;
    let elapsed_ms = start.elapsed().as_millis();

    if types_info.is_empty() {
        println!("{} No symbols found for '{}'", "✗".yellow(), symbol.bold());
        return Ok(());
    }

    eprintln!(
        "{} Analyzed {} symbol(s) in {}ms\n",
        "✓".green(),
        types_info.len().to_string().bold(),
        elapsed_ms.to_string().bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_types(&types_info);
    println!("{}", output);

    Ok(())
}

fn cmd_schema(
    symbol: String,
    path: PathBuf,
    fuzzy: bool,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let symbol = pipe::read_symbol_arg(symbol)?;
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;

    let match_type = if fuzzy { " (fuzzy)" } else { "" };
    eprintln!(
        "{} Analyzing schema for '{}'{}...",
        "→".cyan(),
        symbol.bold(),
        match_type
    );

    let start = Instant::now();
    let schemas = schema::analyze_schema(&index, &symbol, fuzzy)?;
    let elapsed_ms = start.elapsed().as_millis();

    if schemas.is_empty() {
        println!(
            "{} No class/struct found for '{}'",
            "✗".yellow(),
            symbol.bold()
        );
        return Ok(());
    }

    let total_fields: usize = schemas.iter().map(|s| s.fields.len()).sum();
    eprintln!(
        "{} Found {} schema(s) with {} field(s) in {}ms\n",
        "✓".green(),
        schemas.len().to_string().bold(),
        total_fields.to_string().bold(),
        elapsed_ms.to_string().bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_schema(&schemas);
    println!("{}", output);

    Ok(())
}

fn cmd_snapshot(
    name: Option<String>,
    path: PathBuf,
    list: bool,
    delete: Option<String>,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let abs_path = if path.is_absolute() {
        path.clone()
    } else {
        std::env::current_dir()?.join(&path)
    };

    if list {
        let snapshots = snapshot::list_snapshots(&abs_path, cache_dir)?;
        let formatter = OutputFormatter::new(format);
        let output = formatter.format_snapshot_list(&snapshots);
        println!("{}", output);
        return Ok(());
    }

    if let Some(ref snap_name) = delete {
        snapshot::delete_snapshot(snap_name, &abs_path, cache_dir)?;
        eprintln!("{} Deleted snapshot '{}'", "✓".green(), snap_name.bold());
        return Ok(());
    }

    let snap_name = name.unwrap_or_else(|| "unnamed".to_string());
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();

    eprintln!("{} Creating snapshot '{}'...", "→".cyan(), snap_name.bold());

    let start = Instant::now();
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;
    let saved = snapshot::save_snapshot(&index, &snap_name, &abs_path, cache_dir)?;
    let elapsed_ms = start.elapsed().as_millis();

    eprintln!(
        "{} Saved {} symbols from {} files in {}ms\n",
        "✓".green(),
        saved.symbol_count.to_string().bold(),
        saved.file_count.to_string().bold(),
        elapsed_ms.to_string().bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_snapshot_saved(&saved);
    println!("{}", output);

    Ok(())
}

fn cmd_compare(
    snapshot_name: String,
    path: PathBuf,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
    cache_dir: Option<&Path>,
) -> Result<()> {
    let abs_path = if path.is_absolute() {
        path.clone()
    } else {
        std::env::current_dir()?.join(&path)
    };

    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();

    eprintln!(
        "{} Comparing to snapshot '{}'...",
        "→".cyan(),
        snapshot_name.bold()
    );

    let start = Instant::now();

    let saved_snapshot = snapshot::load_snapshot(&snapshot_name, &abs_path, cache_dir)?;
    let index = try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache, cache_dir)?;
    let result = snapshot::compare_to_snapshot(&index, &saved_snapshot);

    let elapsed_ms = start.elapsed().as_millis();

    eprintln!(
        "{} Found {} change(s) in {}ms\n",
        "✓".green(),
        result.symbols.len().to_string().bold(),
        elapsed_ms.to_string().bold()
    );

    let formatter = OutputFormatter::new(format);
    let output = formatter.format_diff(&result);
    println!("{}", output);

    Ok(())
}

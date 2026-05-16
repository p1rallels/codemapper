# CodeMapper Rust - Fast Code Indexing and Mapping Tool

**CodeMapper (cm)** is a high-performance code analysis tool that indexes and maps your codebase in milliseconds. Built in Rust for maximum speed with no database overhead—everything runs in-memory using tree-sitter for accurate AST parsing.

## 🚀 Performance

- **Small projects (< 100 files)**: < 20ms instant
- **Medium projects (100-1000 files)**: Cached, ~0.5s load
- **Large projects (1000+ files)**: Fast mode auto-enabled (10-100x speedup)
- **Incremental rebuilds**: 45-55x faster than full reindex
- **Binary size**: ~2MB (stripped)

Compared to Python version: **10-50x faster** cold start performance.

### ⚡ Fast Mode

For large codebases, CodeMapper automatically plans fast searches with a ripgrep-style text prefilter and AST validation:

| Codebase Size | Before | After | Speedup |
|---------------|--------|-------|---------|
| 18,457 files | 76s | 1.2s | **63x** |
| 17,005 files | 122s | 9.6s | **12x** |
| < 1000 files | Normal mode (no overhead) | | |

**How it works:**
1. **Stage 1**: Lightning-fast text search finds candidate files (milliseconds)
2. **Stage 2**: AST validation ensures 100% accuracy (only parses candidates)
3. **Fallback**: Automatically uses full scan if no text matches found

## ✨ Features

- **Smart Caching**: Auto-enabled for projects ≥300ms to parse; small projects stay fast with no `.codemapper/` clutter
- **Fast Mode**: Automatic text prefilter planning with 10-100x speedups on large searches
- **Tree-sitter Parsing**: Accurate AST-based symbol extraction
- **Multi-language**: Python, JavaScript, TypeScript, Rust, Java, Go, C, Swift, Ruby, Markdown
- **Parallel Processing**: Uses rayon for concurrent file parsing
- **Fuzzy Search**: Case-insensitive matching by default; use `cm grep` or `cm query --grep` for symbol substrings
- **Call Graph Analysis**: callers, callees, trace, tests, entrypoints
- **Git Integration**: diff, since, blame, history commands
- **Type Analysis**: types, implements, schema commands
- **3 Output Formats**: ai (default), human (tables), default (markdown)

## 📦 Installation

### Build from source:
```bash
git clone https://github.com/p1rallels/codemapper.git
cd codemapper
cargo build --release
```

Binary location: `target/release/cm`

## 🎯 Quick Start

```bash
# 1. Get the lay of the land
cm stats .                           # Project overview

# 2. See file structure
cm map . --level 2 --format ai       # File listing with symbol counts

# 3. Find and explore
cm query authenticate                # Fuzzy search (default)
cm query Parser --show-body          # See implementation
cm inspect ./src/auth.py             # All symbols in a file

# 4. Understand code flow
cm callers process_payment           # Who calls this?
cm callees process_payment           # What does it call?
cm trace main process_payment        # Call path from A to B

# 5. Git analysis
cm diff main                         # Changes vs main branch
cm since v1.0 --breaking             # Breaking changes since release
cm blame authenticate ./auth.py      # Who last touched it?
```

## 📋 Commands

### Discovery (Start Here)

| Command | Description |
|---------|-------------|
| `stats` | Project size and composition |
| `map` | File listing with symbol counts (3 detail levels) |
| `query` | Find symbols by name (main search tool) |
| `inspect` | List all symbols in one file |
| `deps` | Track imports and usage |

### Call Graph

| Command | Description |
|---------|-------------|
| `callers` | WHO calls this function? (reverse dependencies) |
| `callees` | What DOES this function call? (forward dependencies) |
| `trace` | CALL PATH from A → B (shortest route) |
| `entrypoints` | Public APIs with no internal callers |
| `tests` | Which tests call this symbol? |
| `untested` | Find symbols not called by any test |
| `test-deps` | What production code does a test touch? |
| `impact` | Quick breakage report (definition + callers + tests) |

### Git History

| Command | Description |
|---------|-------------|
| `diff` | Symbol-level changes vs a commit |
| `since` | Breaking changes since commit |
| `blame` | Who last touched this symbol? |
| `history` | Full evolution of a symbol |

### Type Analysis

| Command | Description |
|---------|-------------|
| `types` | Parameter types and return type |
| `implements` | Find all implementations of an interface |
| `schema` | Field structure (structs, classes, dataclasses) |

### Snapshots

| Command | Description |
|---------|-------------|
| `snapshot` | Save current state (named checkpoint) |
| `compare` | Diff current vs saved snapshot |

## 🔍 Search Modes

Fuzzy matching is **enabled by default** for more forgiving searches:

```bash
# Default: fuzzy/case-insensitive
cm query auth                    # Matches authenticate, Authorization, etc.

# Grep-style symbol substring search
cm grep Parser                   # Case-sensitive symbol search
cm query Parser --grep           # Same search through query

# Raw text search
cm grep --text 'TODO|FIXME' .    # rg -n style line output
```

## 📊 Output Formats

```bash
cm query Parser                    # Compact AI output by default
cm query Parser --format human     # Tables (terminal viewing, pretty)
cm query Parser --format default   # Markdown (documentation, readable)
```

## 💾 Caching

Smart caching behavior:
- **Small repos (< 300ms to parse)**: No cache created—always fast, no `.codemapper/` clutter
- **Large repos (≥ 300ms)**: Cache created on first run, loads instantly after
- **File changes**: Auto-detected, only modified files re-parsed

### Cache Location

By default, cache is stored in `.codemapper/` in the project root. Override with:

```bash
# Using --cache-dir flag
cm stats . --cache-dir /custom/cache/path

# Using environment variable
export CODEMAPPER_CACHE_DIR=/custom/cache/path
cm stats .
```

**Priority**: `--cache-dir` flag > `CODEMAPPER_CACHE_DIR` env var > default

**Use cases**: Git worktrees, multi-repo projects, keeping cache in a central location.

### Cache Flags

```bash
--no-cache           # Skip cache, always reindex
--rebuild-cache      # Force cache rebuild
```

## 🎯 Typical Workflows

### Exploring Unknown Code
```bash
cm stats .                           # Size and composition
cm map . --level 2 --format ai       # File structure
cm query <symbol>                    # Find code
cm inspect ./path/to/file            # Deep dive
```

### Finding a Bug
```bash
cm query <suspected_function> --show-body   # See implementation
cm callers <function>                       # Who calls this?
cm trace <entry_point> <suspected_function> # How does bug get triggered?
cm tests <function>                         # Are there tests?
```

### Before Refactoring
```bash
cm callers <function>              # Impact radius
cm callees <function>              # What does it depend on?
cm tests <function>                # Verify coverage exists
cm since main --breaking           # (After refactor) Did we break anything?
```

### Understanding an API
```bash
cm entrypoints .                   # What's exported?
cm implements <interface>          # Find implementations
cm schema <DataClass>              # Field structure
```

### Validating Code Health
```bash
cm untested .                      # What's not tested?
cm since <last_release> --breaking # Breaking changes?
```

## 🎯 Supported Languages

| Language | Extensions | Extracts |
|----------|------------|----------|
| Python | .py | Functions, classes, methods, imports |
| JavaScript | .js, .jsx | Functions, classes, methods, imports |
| TypeScript | .ts, .tsx | Functions, classes, methods, interfaces, types, enums |
| Rust | .rs | Functions, structs, impl blocks, traits, enums |
| Java | .java | Classes, interfaces, methods, enums, javadoc |
| Go | .go | Functions, structs, methods, interfaces |
| C | .c, .h | Functions, structs, includes |
| Ruby | .rb, .rbi, .rake, .gemspec, .ru, Gemfile, Rakefile | Classes, modules, methods, generated attr/delegate/Rails DSL methods, mixins, callback refs, constants, requires |
| Markdown | .md | Headings, code blocks |

## 🏗️ Architecture

### Core Components

- **models.rs**: Data structures (Symbol, FileInfo, Language, etc.)
- **index.rs**: In-memory CodeIndex with HashMap-based lookups
- **parser/**: Language-specific parsers using tree-sitter
- **indexer.rs**: File walking, hashing, parallel processing
- **callgraph.rs**: Call graph analysis (callers, callees, trace)
- **fast_search.rs**: Text prefiltering and raw grep-style search
- **cache.rs**: Smart caching with incremental updates
- **output.rs**: Three output formatters
- **main.rs**: CLI interface using clap

### Design Principles

- **Functional Style**:
  - Immutable by default
  - Iterator chains over loops
  - Pure functions where possible
  - Minimal OOP

## 🔧 Development

```bash
cargo test                                  # Run tests
cargo check                                 # Check code
cargo clippy --all-targets --all-features  # Lint
cargo build --release                       # Build release
```

### Ignored Directories

The indexer automatically skips:
- `.codemapper`, `.git`, `.hg`, `.svn`
- `node_modules`, `__pycache__`, `venv`, `.venv`
- `target`, `dist`, `build`, `.cache`

## 🛠️ Common Flags

```
--grep               Case-sensitive symbol substring search
--text               Raw rg-style text search for cm grep
--limit 0            Return all results (default is first 50 for query/grep)
--format <format>    Output: ai (default), human (tables), default (markdown)
--show-body          Include actual code (not just signatures)
--exports-only       Public symbols only (pub, export, etc.)
--full               Include anonymous/lambda functions
--context minimal    Signatures only (default)
--context full       Include docstrings and metadata
--no-cache           Skip cache, always reindex
--rebuild-cache      Force cache rebuild
--extensions py,rs   Comma-separated file types (swift and ruby included by default)
--cache-dir <path>   Override cache location
```

## 📝 License

Part of the CodeMapper project.

## 🚀 Future Enhancements

- LSP protocol support
- Daemon mode with file watching
- More advanced call graph visualizations
- Cross-language call tracking

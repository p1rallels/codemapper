# CodeMapper Rust - Fast Code Indexing and Mapping Tool

**CodeMapper Rust** is a high-performance code analysis tool that indexes and maps your codebase in milliseconds. Built in Rust for maximum speed with no database overhead—everything runs in-memory using tree-sitter for accurate AST parsing.

## 🚀 Performance

- **3 files**: 1-2ms total (170ms wall time including startup)
- **Small projects**: <5ms
- **Medium projects (50 files)**: <20ms
- **Binary size**: 1.5MB (stripped)

Compared to Python version: **10-50x faster** cold start performance.

### ⚡ Fast Mode (NEW!)

For large codebases (1000+ files), CodeMapper automatically enables **Fast Mode**—a ripgrep-powered two-stage search that provides **10-100x speedup**:

**Performance Results:**
- **18,457 files**: 76s → 1.2s (**63x faster**)
- **17,005 files**: 122s → 9.6s (**12x faster**)
- **Small codebases** (< 1000 files): Uses normal mode (no overhead)

**How it works:**
1. **Stage 1**: Lightning-fast text search finds candidate files (milliseconds)
2. **Stage 2**: AST validation ensures 100% accuracy (only parses candidates)
3. **Fallback**: Automatically uses full scan if no text matches found

**Usage:**
```bash
# Explicit fast mode
cm query authenticate /huge/monorepo --fast

# Auto-enabled for 1000+ files (no flag needed!)
cm query MyClass /large/project --fuzzy

# Works with all query options
cm query validate . --fuzzy --fast --show-body --format ai
```

## ✨ Features

- **Fast Mode**: Ripgrep-powered search with 10-100x speedup (auto-enabled for 1000+ files)
- **No Database**: Pure in-memory indexing for instant startup
- **Tree-sitter Parsing**: Accurate AST-based symbol extraction
- **Multi-language**: Python, JavaScript, TypeScript, Rust, Java, Go, C, Markdown support
- **Parallel Processing**: Uses rayon for concurrent file parsing
- **Fuzzy Search**: Levenshtein distance-based symbol matching
- **3 Output Formats**: Default (markdown), Human (tables), AI (token-efficient)

## 📦 Installation

### Build from source:
```bash
git clone https://github.com/auland2vs/codemapper.git
cd codemapper
cargo build --release
```

Binary location: `target/release/cm`

## 🎯 Usage

### Commands

#### `map` - Project Overview
```bash
# Level 1: High-level overview
cm map /path/to/project --level 1

# Level 2: File summaries
cm map /path/to/project --level 2

# Level 3: Detailed signatures
cm map /path/to/project --level 3
```

#### `query` - Search Symbols
```bash
# Exact match
cm query process_payment /path/to/project

# Fuzzy search
cm query procpayment /path/to/project --fuzzy

# With full context (docstrings)
cm query MyClass /path/to/project --context full

# Fast mode (explicit or auto-enabled for 1000+ files)
cm query authenticate /large/monorepo --fast
cm query parse . --fuzzy --fast --format ai
```

#### `stats` - Codebase Statistics
```bash
cm stats /path/to/project
```

#### `deps` - Dependency Analysis
```bash
# Show imports for a file
cm deps /path/to/file.py /path/to/project

# Show what imports this file
cm deps /path/to/file.py /path/to/project --direction used-by
```

#### `index` - Index & Validate
```bash
# Index all supported files
cm index /path/to/project

# Index specific extensions
cm index /path/to/project --extensions py,js,ts
```

### Output Formats

#### Default (Markdown)
```bash
cm query MyClass /path/to/project --format default
```

#### Human (Pretty Tables)
```bash
cm map /path/to/project --format human
```

#### AI (Token-Efficient)
```bash
cm stats /path/to/project --format ai
```

## 📊 Examples

### Query a Function
```bash
$ cm query process_payment example_project
Found 1 symbols

## process_payment
- Type: method
- File: example_project/payment.py
- Lines: 22-61
- Signature: (self, amount: float, currency: str = "USD")
```

### Project Overview
```bash
$ cm map example_project --level 1
# Project Overview

## Languages
- python: 3 files

## Statistics
- Total files: 3
- Total symbols: 16
  - Functions: 4
  - Classes: 3
  - Methods: 9
```

### Statistics
```bash
$ cm stats example_project
# Codebase Statistics

## Files by Language
- python: 3

## Symbols by Type
- Functions: 4
- Classes: 3
- Methods: 9

## Totals
- Total Files: 3
- Total Symbols: 16
- Total Bytes: 5346

→ Parse time: 1ms
```

## 🏗️ Architecture

### Core Components

- **models.rs**: Data structures (Symbol, FileInfo, Language, etc.)
- **index.rs**: In-memory CodeIndex with HashMap-based lookups
- **parser/**: Language-specific parsers using tree-sitter
  - `python.rs`: Python AST parsing
  - `javascript.rs`: JS/TS parsing
- **indexer.rs**: File walking, hashing, parallel processing
- **output.rs**: Three output formatters
- **main.rs**: CLI interface using clap

### Design Principles

- **MISRA/Power of 10 Compliance**:
  - No unwrap() - proper error handling with anyhow
  - No recursion - iterative tree-sitter queries
  - Bounds checking on all array access
  - Static analysis with clippy

- **Functional Style**:
  - Immutable by default
  - Iterator chains over loops
  - Pure functions where possible
  - Minimal OOP

## 🔧 Development

### Run Tests
```bash
cargo test
```

### Check Code
```bash
cargo check
cargo clippy --all-targets --all-features
```

### Build Release
```bash
cargo build --release
```

## 🎯 Supported Languages

- ✅ Python (.py)
- ✅ JavaScript (.js, .jsx)
- ✅ TypeScript (.ts, .tsx)
- ✅ Rust (.rs)
- ✅ Java (.java)
- ✅ Go (.go)
- ✅ C (.c, .h)
- ✅ Markdown (.md)

## 📈 Performance Comparison

| Metric | Rust | Python | Speedup |
|--------|------|--------|---------|
| 3 files | 1ms | ~50ms | 50x |
| Parse time | O(n) | O(n) | - |
| Memory | In-memory | SQLite | Lower |
| Binary size | 1.5MB | N/A | - |

## 🛠️ Technical Details

### Dependencies

- **clap**: CLI framework with derive macros
- **tree-sitter**: Parser generator tool
- **rayon**: Data parallelism library
- **walkdir**: Directory traversal
- **comfy-table**: Pretty table formatting
- **colored**: Terminal colors
- **md5**: File hashing
- **anyhow**: Error handling

### Ignored Directories

The indexer automatically skips:
- `.codemapper`, `.git`, `.hg`, `.svn`
- `node_modules`, `__pycache__`, `venv`, `.venv`
- `target`, `dist`, `build`, `.cache`

## 📝 License

Part of the CodeMapper project.

## 🚀 Future Enhancements

- Optional disk cache for instant startup on large projects
- More languages (Go, C/C++, Java)
- Call graph analysis
- LSP protocol support
- Daemon mode with file watching

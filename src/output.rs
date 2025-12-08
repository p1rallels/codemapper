use crate::blame::{BlameResult, HistoryEntry};
use crate::callgraph::{CallInfo, EntrypointCategory, EntrypointInfo, TestDep, TestInfo, TracePath, UntestedInfo};
use crate::diff::{ChangeType, DiffResult, SymbolDiff};
use crate::implements::Implementation;
use crate::index::CodeIndex;
use crate::models::{Symbol, SymbolType};
use crate::types::SymbolTypes;
use colored::*;
use comfy_table::{modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, Table};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Default,
    Human,
    AI,
}

impl OutputFormat {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "default" => Some(Self::Default),
            "human" => Some(Self::Human),
            "ai" => Some(Self::AI),
            _ => None,
        }
    }
}

pub struct OutputFormatter {
    format: OutputFormat,
}

/// Read specific lines from a file (1-indexed line numbers)
fn read_file_lines(path: &Path, start_line: usize, end_line: usize) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = content.lines().collect();

    if start_line == 0 || start_line > lines.len() || end_line > lines.len() {
        return None;
    }

    // Convert to 0-indexed and extract the range
    let start_idx = start_line - 1;
    let end_idx = end_line;

    if start_idx >= end_idx {
        return None;
    }

    let selected_lines = &lines[start_idx..end_idx];
    let mut result = String::new();

    for (i, line) in selected_lines.iter().enumerate() {
        let line_num = start_line + i;
        result.push_str(&format!("{:4} | {}\n", line_num, line));
    }

    Some(result)
}

impl OutputFormatter {
    pub fn new(format: OutputFormat) -> Self {
        Self { format }
    }

    pub fn format_map(&self, index: &CodeIndex, level: u8) -> String {
        match self.format {
            OutputFormat::Default => self.format_map_default(index, level),
            OutputFormat::Human => self.format_map_human(index, level),
            OutputFormat::AI => self.format_map_ai(index, level),
        }
    }

    fn format_map_default(&self, index: &CodeIndex, level: u8) -> String {
        let mut output = String::new();
        output.push_str("# Project Overview\n\n");

        let mut lang_counts: HashMap<&str, usize> = HashMap::new();
        for file in index.files() {
            *lang_counts.entry(file.language.as_str()).or_insert(0) += 1;
        }

        output.push_str("## Languages\n");
        for (lang, count) in &lang_counts {
            output.push_str(&format!("- {}: {} files\n", lang, count));
        }

        output.push_str(&format!("\n## Statistics\n"));
        output.push_str(&format!("- Total files: {}\n", index.total_files()));
        output.push_str(&format!("- Total symbols: {}\n", index.total_symbols()));
        output.push_str(&format!("  - Functions: {}\n", index.symbols_by_type(SymbolType::Function)));
        output.push_str(&format!("  - Classes: {}\n", index.symbols_by_type(SymbolType::Class)));
        output.push_str(&format!("  - Methods: {}\n", index.symbols_by_type(SymbolType::Method)));
        output.push_str(&format!("  - Enums: {}\n", index.symbols_by_type(SymbolType::Enum)));
        output.push_str(&format!("  - Static Fields: {}\n", index.symbols_by_type(SymbolType::StaticField)));
        output.push_str(&format!("  - Headings: {}\n", index.symbols_by_type(SymbolType::Heading)));
        output.push_str(&format!("  - Code Blocks: {}\n", index.symbols_by_type(SymbolType::CodeBlock)));

        if level >= 2 {
            output.push_str("\n## Files\n\n");
            for file in index.files() {
                output.push_str(&format!("### {}\n", file.path.display()));
                output.push_str(&format!("- Language: {}\n", file.language.as_str()));
                output.push_str(&format!("- Size: {} bytes\n", file.size));
                
                let symbols = index.get_file_symbols(&file.path);
                if !symbols.is_empty() {
                    output.push_str(&format!("- Symbols: {}\n", symbols.len()));
                    
                    if level >= 3 {
                        for symbol in symbols {
                            output.push_str(&format!("  - {} {} (lines {}-{})",
                                symbol.symbol_type.as_str(),
                                symbol.name,
                                symbol.line_start,
                                symbol.line_end
                            ));
                            if let Some(sig) = &symbol.signature {
                                output.push_str(&format!("{}", sig));
                            }
                            output.push('\n');
                            if let Some(doc) = &symbol.docstring {
                                output.push_str(&format!("    \"{}\"\n", doc));
                            }
                        }
                    }
                }
                output.push('\n');
            }
        }

        output
    }

    fn format_map_human(&self, index: &CodeIndex, level: u8) -> String {
        let mut output = String::new();
        
        output.push_str(&format!("{}\n\n", "Project Overview".bold().green()));

        let mut lang_table = Table::new();
        lang_table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Language", "Files"]);

        let mut lang_counts: HashMap<&str, usize> = HashMap::new();
        for file in index.files() {
            *lang_counts.entry(file.language.as_str()).or_insert(0) += 1;
        }

        for (lang, count) in &lang_counts {
            lang_table.add_row(vec![lang.to_string(), count.to_string()]);
        }

        output.push_str(&format!("{}\n\n", lang_table));

        let mut stats_table = Table::new();
        stats_table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Metric", "Count"]);

        stats_table.add_row(vec!["Total Files", &index.total_files().to_string()]);
        stats_table.add_row(vec!["Total Symbols", &index.total_symbols().to_string()]);
        stats_table.add_row(vec!["Functions", &index.symbols_by_type(SymbolType::Function).to_string()]);
        stats_table.add_row(vec!["Classes", &index.symbols_by_type(SymbolType::Class).to_string()]);
        stats_table.add_row(vec!["Methods", &index.symbols_by_type(SymbolType::Method).to_string()]);
        stats_table.add_row(vec!["Enums", &index.symbols_by_type(SymbolType::Enum).to_string()]);
        stats_table.add_row(vec!["Static Fields", &index.symbols_by_type(SymbolType::StaticField).to_string()]);
        stats_table.add_row(vec!["Headings", &index.symbols_by_type(SymbolType::Heading).to_string()]);
        stats_table.add_row(vec!["Code Blocks", &index.symbols_by_type(SymbolType::CodeBlock).to_string()]);

        output.push_str(&format!("{}\n", stats_table));

        if level >= 2 {
            output.push_str(&format!("\n{}\n\n", "Files".bold().green()));
            
            let mut file_table = Table::new();
            file_table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS);

            if level >= 3 {
                file_table.set_header(vec!["File", "Language", "Size", "Symbols"]);
            } else {
                file_table.set_header(vec!["File", "Language", "Size", "Symbol Count"]);
            }

            for file in index.files() {
                let symbols = index.get_file_symbols(&file.path);
                let symbol_info = if level >= 3 {
                    symbols.iter()
                        .map(|s| format!("{}:{}", s.symbol_type.as_str(), s.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                } else {
                    symbols.len().to_string()
                };

                file_table.add_row(vec![
                    file.path.display().to_string(),
                    file.language.as_str().to_string(),
                    format!("{} bytes", file.size),
                    symbol_info,
                ]);
            }

            output.push_str(&format!("{}\n", file_table));
        }

        output
    }

    fn format_map_ai(&self, index: &CodeIndex, level: u8) -> String {
        let mut output = String::new();
        output.push_str("[PROJECT]\n");

        let mut lang_counts: HashMap<&str, usize> = HashMap::new();
        for file in index.files() {
            *lang_counts.entry(file.language.as_str()).or_insert(0) += 1;
        }

        output.push_str("LANGS:");
        for (lang, count) in &lang_counts {
            output.push_str(&format!(" {}:{}", lang, count));
        }
        output.push('\n');

        output.push_str(&format!("FILES:{} SYMBOLS:{} FUNCTIONS:{} CLASSES:{} METHODS:{} ENUMS:{} STATICS:{} HEADINGS:{} CODE BLOCKS:{}\n",
            index.total_files(),
            index.total_symbols(),
            index.symbols_by_type(SymbolType::Function),
            index.symbols_by_type(SymbolType::Class),
            index.symbols_by_type(SymbolType::Method),
            index.symbols_by_type(SymbolType::Enum),
            index.symbols_by_type(SymbolType::StaticField),
            index.symbols_by_type(SymbolType::Heading),
            index.symbols_by_type(SymbolType::CodeBlock)
        ));

        if level >= 2 {
            output.push_str("\n[FILES]\n");
            for file in index.files() {
                output.push_str(&format!("{}|{}|{}", 
                    file.path.display(),
                    file.language.as_str(),
                    file.size
                ));
                
                let symbols = index.get_file_symbols(&file.path);
                if !symbols.is_empty() && level >= 3 {
                    output.push_str("|");
                    for (i, symbol) in symbols.iter().enumerate() {
                        if i > 0 { output.push(','); }
                        output.push_str(&format!("{}:{}@{}-{}",
                            match symbol.symbol_type {
                                SymbolType::Function => "f",
                                SymbolType::Class => "c",
                                SymbolType::Method => "m",
                                SymbolType::Enum => "e",
                                SymbolType::StaticField => "s",
                                SymbolType::Heading => "h",
                                SymbolType::CodeBlock => "cb",
                            },
                            symbol.name,
                            symbol.line_start,
                            symbol.line_end
                        ));
                    }
                }
                output.push('\n');
            }
        }

        output
    }

    pub fn format_query(&self, symbols: Vec<&Symbol>, context: bool, show_body: bool) -> String {
        match self.format {
            OutputFormat::Default => self.format_query_default(symbols, context, show_body),
            OutputFormat::Human => self.format_query_human(symbols, context, show_body),
            OutputFormat::AI => self.format_query_ai(symbols, context, show_body),
        }
    }

    fn format_query_default(&self, symbols: Vec<&Symbol>, context: bool, show_body: bool) -> String {
        let mut output = String::new();
        output.push_str(&format!("Found {} symbols\n\n", symbols.len()));

        for symbol in symbols {
            output.push_str(&format!("## {}\n", symbol.name));
            output.push_str(&format!("- Type: {}\n", symbol.symbol_type.as_str()));
            output.push_str(&format!("- File: {}\n", symbol.file_path.display()));
            output.push_str(&format!("- Lines: {}-{}\n", symbol.line_start, symbol.line_end));

            if let Some(sig) = &symbol.signature {
                output.push_str(&format!("- Signature: {}\n", sig));
            }

            if context {
                if let Some(doc) = &symbol.docstring {
                    output.push_str(&format!("- Documentation: {}\n", doc));
                }
            }

            // Show code body if requested (limit to symbols with <= 50 lines)
            if show_body {
                let line_count = symbol.line_end - symbol.line_start + 1;
                if line_count <= 50 {
                    if let Some(body) = read_file_lines(&symbol.file_path, symbol.line_start, symbol.line_end) {
                        output.push_str("\nCode:\n");
                        output.push_str(&body);
                    }
                } else {
                    output.push_str(&format!("\n(Code body omitted: {} lines, use --context full to see more details)\n", line_count));
                }
            }

            output.push('\n');
        }

        output
    }

    fn format_query_human(&self, symbols: Vec<&Symbol>, context: bool, show_body: bool) -> String {
        let mut output = String::new();
        output.push_str(&format!("{} {}\n\n", "Found".green(), format!("{} symbols", symbols.len()).bold()));

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS);

        if context {
            table.set_header(vec!["Name", "Type", "File", "Lines", "Signature", "Documentation"]);
            for symbol in &symbols {
                table.add_row(vec![
                    symbol.name.clone(),
                    symbol.symbol_type.as_str().to_string(),
                    symbol.file_path.display().to_string(),
                    format!("{}-{}", symbol.line_start, symbol.line_end),
                    symbol.signature.as_deref().unwrap_or("-").to_string(),
                    symbol.docstring.as_deref().unwrap_or("-").to_string(),
                ]);
            }
        } else {
            table.set_header(vec!["Name", "Type", "File", "Lines"]);
            for symbol in &symbols {
                table.add_row(vec![
                    symbol.name.clone(),
                    symbol.symbol_type.as_str().to_string(),
                    symbol.file_path.display().to_string(),
                    format!("{}-{}", symbol.line_start, symbol.line_end),
                ]);
            }
        }

        output.push_str(&format!("{}\n", table));

        // Show code bodies after the table if requested
        if show_body {
            output.push_str("\n");
            for symbol in &symbols {
                let line_count = symbol.line_end - symbol.line_start + 1;
                if line_count <= 50 {
                    if let Some(body) = read_file_lines(&symbol.file_path, symbol.line_start, symbol.line_end) {
                        output.push_str(&format!("{} {}\n", "Code for".cyan(), symbol.name.bold()));
                        output.push_str(&body);
                        output.push_str("\n");
                    }
                }
            }
        }

        output
    }

    fn format_query_ai(&self, symbols: Vec<&Symbol>, context: bool, show_body: bool) -> String {
        let mut output = String::new();
        output.push_str(&format!("[RESULTS:{}]\n", symbols.len()));

        for symbol in symbols {
            output.push_str(&format!("{}|{}|{}|{}-{}",
                symbol.name,
                match symbol.symbol_type {
                    SymbolType::Function => "f",
                    SymbolType::Class => "c",
                    SymbolType::Method => "m",
                    SymbolType::Enum => "e",
                    SymbolType::StaticField => "s",
                    SymbolType::Heading => "h",
                    SymbolType::CodeBlock => "cb",
                },
                symbol.file_path.display(),
                symbol.line_start,
                symbol.line_end
            ));

            if context {
                if let Some(sig) = &symbol.signature {
                    output.push_str(&format!("|sig:{}", sig));
                }
                if let Some(doc) = &symbol.docstring {
                    output.push_str(&format!("|doc:{}", doc));
                }
            }

            if show_body {
                let line_count = symbol.line_end - symbol.line_start + 1;
                if line_count <= 50 {
                    if let Some(body) = read_file_lines(&symbol.file_path, symbol.line_start, symbol.line_end) {
                        // Compact format: include body on separate lines with indentation
                        output.push_str("|body:");
                        for line in body.lines() {
                            output.push_str(&format!("\n  {}", line));
                        }
                    }
                }
            }

            output.push('\n');
        }

        output
    }

    pub fn format_deps(&self, target: &str, deps: Vec<String>, direction: &str) -> String {
        match self.format {
            OutputFormat::Default => self.format_deps_default(target, deps, direction),
            OutputFormat::Human => self.format_deps_human(target, deps, direction),
            OutputFormat::AI => self.format_deps_ai(target, deps, direction),
        }
    }

    fn format_deps_default(&self, target: &str, deps: Vec<String>, direction: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("# Dependencies for {}\n\n", target));
        output.push_str(&format!("Direction: {}\n\n", direction));
        
        for dep in deps {
            output.push_str(&format!("- {}\n", dep));
        }

        output
    }

    fn format_deps_human(&self, target: &str, deps: Vec<String>, direction: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("{} {}\n", "Dependencies for".green(), target.bold()));
        output.push_str(&format!("{}: {}\n\n", "Direction".cyan(), direction));

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Dependency"]);

        for dep in deps {
            table.add_row(vec![dep]);
        }

        output.push_str(&format!("{}\n", table));
        output
    }

    fn format_deps_ai(&self, target: &str, deps: Vec<String>, direction: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("[DEPS:{}|{}]\n", target, direction));
        
        for dep in deps {
            output.push_str(&format!("{}\n", dep));
        }

        output
    }

    pub fn format_stats(&self, index: &CodeIndex) -> String {
        match self.format {
            OutputFormat::Default => self.format_stats_default(index),
            OutputFormat::Human => self.format_stats_human(index),
            OutputFormat::AI => self.format_stats_ai(index),
        }
    }

    fn format_stats_default(&self, index: &CodeIndex) -> String {
        let mut output = String::new();
        output.push_str("# Codebase Statistics\n\n");

        let mut lang_counts: HashMap<&str, usize> = HashMap::new();
        let mut total_loc = 0u64;
        
        for file in index.files() {
            *lang_counts.entry(file.language.as_str()).or_insert(0) += 1;
            total_loc += file.size;
        }

        output.push_str("## Files by Language\n");
        for (lang, count) in &lang_counts {
            output.push_str(&format!("- {}: {}\n", lang, count));
        }

        output.push_str("\n## Symbols by Type\n");
        output.push_str(&format!("- Functions: {}\n", index.symbols_by_type(SymbolType::Function)));
        output.push_str(&format!("- Classes: {}\n", index.symbols_by_type(SymbolType::Class)));
        output.push_str(&format!("- Methods: {}\n", index.symbols_by_type(SymbolType::Method)));
        output.push_str(&format!("- Enums: {}\n", index.symbols_by_type(SymbolType::Enum)));
        output.push_str(&format!("- Static Fields: {}\n", index.symbols_by_type(SymbolType::StaticField)));
        output.push_str(&format!("- Headings: {}\n", index.symbols_by_type(SymbolType::Heading)));
        output.push_str(&format!("- Code Blocks: {}\n", index.symbols_by_type(SymbolType::CodeBlock)));

        output.push_str(&format!("\n## Totals\n"));
        output.push_str(&format!("- Total Files: {}\n", index.total_files()));
        output.push_str(&format!("- Total Symbols: {}\n", index.total_symbols()));
        output.push_str(&format!("- Total Bytes: {}\n", total_loc));

        output
    }

    fn format_stats_human(&self, index: &CodeIndex) -> String {
        let mut output = String::new();
        output.push_str(&format!("{}\n\n", "Codebase Statistics".bold().green()));

        let mut lang_table = Table::new();
        lang_table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Language", "Files"]);

        let mut lang_counts: HashMap<&str, usize> = HashMap::new();
        let mut total_loc = 0u64;
        
        for file in index.files() {
            *lang_counts.entry(file.language.as_str()).or_insert(0) += 1;
            total_loc += file.size;
        }

        for (lang, count) in &lang_counts {
            lang_table.add_row(vec![lang.to_string(), count.to_string()]);
        }

        output.push_str(&format!("{}\n\n", "Files by Language".cyan()));
        output.push_str(&format!("{}\n\n", lang_table));

        let mut symbol_table = Table::new();
        symbol_table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Type", "Count"]);

        symbol_table.add_row(vec!["Functions", &index.symbols_by_type(SymbolType::Function).to_string()]);
        symbol_table.add_row(vec!["Classes", &index.symbols_by_type(SymbolType::Class).to_string()]);
        symbol_table.add_row(vec!["Methods", &index.symbols_by_type(SymbolType::Method).to_string()]);
        symbol_table.add_row(vec!["Enums", &index.symbols_by_type(SymbolType::Enum).to_string()]);
        symbol_table.add_row(vec!["Static Fields", &index.symbols_by_type(SymbolType::StaticField).to_string()]);
        symbol_table.add_row(vec!["Headings", &index.symbols_by_type(SymbolType::Heading).to_string()]);
        symbol_table.add_row(vec!["Code Blocks", &index.symbols_by_type(SymbolType::CodeBlock).to_string()]);

        output.push_str(&format!("{}\n", "Symbols by Type".cyan()));
        output.push_str(&format!("{}\n\n", symbol_table));

        let mut totals_table = Table::new();
        totals_table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Metric", "Value"]);

        totals_table.add_row(vec!["Total Files", &index.total_files().to_string()]);
        totals_table.add_row(vec!["Total Symbols", &index.total_symbols().to_string()]);
        totals_table.add_row(vec!["Total Bytes", &total_loc.to_string()]);

        output.push_str(&format!("{}\n", "Totals".cyan()));
        output.push_str(&format!("{}\n", totals_table));

        output
    }

    fn format_stats_ai(&self, index: &CodeIndex) -> String {
        let mut output = String::new();
        output.push_str("[STATS]\n");

        let mut lang_counts: HashMap<&str, usize> = HashMap::new();
        let mut total_loc = 0u64;
        
        for file in index.files() {
            *lang_counts.entry(file.language.as_str()).or_insert(0) += 1;
            total_loc += file.size;
        }

        output.push_str("LANGS:");
        for (lang, count) in &lang_counts {
            output.push_str(&format!(" {}:{}", lang, count));
        }
        output.push('\n');

        output.push_str(&format!("SYMS: f:{} c:{} m:{} e:{} s:{} h:{} cb:{}\n",
            index.symbols_by_type(SymbolType::Function),
            index.symbols_by_type(SymbolType::Class),
            index.symbols_by_type(SymbolType::Method),
            index.symbols_by_type(SymbolType::Enum),
            index.symbols_by_type(SymbolType::StaticField),
            index.symbols_by_type(SymbolType::Heading),
            index.symbols_by_type(SymbolType::CodeBlock)
        ));

        output.push_str(&format!("TOTALS: files:{} syms:{} bytes:{}\n",
            index.total_files(),
            index.total_symbols(),
            total_loc
        ));

        output
    }

    pub fn format_diff(&self, result: &DiffResult) -> String {
        match self.format {
            OutputFormat::Default => self.format_diff_default(result),
            OutputFormat::Human => self.format_diff_human(result),
            OutputFormat::AI => self.format_diff_ai(result),
        }
    }

    fn format_diff_default(&self, result: &DiffResult) -> String {
        let mut output = String::new();
        output.push_str(&format!("# Symbol Diff\n\n"));
        output.push_str(&format!("Comparing HEAD to commit: `{}`\n\n", &result.commit[..8.min(result.commit.len())]));
        output.push_str(&format!("Files analyzed: {}\n", result.files_analyzed));
        output.push_str(&format!("Symbol changes: {}\n\n", result.symbols.len()));

        if result.symbols.is_empty() {
            output.push_str("No symbol changes detected.\n");
            return output;
        }

        let mut by_type: HashMap<ChangeType, Vec<&SymbolDiff>> = HashMap::new();
        for sym in &result.symbols {
            by_type.entry(sym.change_type).or_default().push(sym);
        }

        for change_type in [ChangeType::Added, ChangeType::Deleted, ChangeType::Modified, ChangeType::SignatureChanged] {
            if let Some(symbols) = by_type.get(&change_type) {
                output.push_str(&format!("## {} ({})\n\n", change_type.as_str(), symbols.len()));
                for sym in symbols {
                    output.push_str(&format!("- **{}** ({}) in `{}`",
                        sym.name,
                        sym.symbol_type.as_str(),
                        sym.file_path.display()
                    ));
                    if let Some((start, end)) = sym.new_lines {
                        output.push_str(&format!(" @ lines {}-{}", start, end));
                    } else if let Some((start, end)) = sym.old_lines {
                        output.push_str(&format!(" @ lines {}-{} (deleted)", start, end));
                    }
                    output.push('\n');
                    
                    if change_type == ChangeType::SignatureChanged {
                        if let Some(ref old_sig) = sym.old_signature {
                            output.push_str(&format!("  - Old: `{}`\n", old_sig));
                        }
                        if let Some(ref new_sig) = sym.new_signature {
                            output.push_str(&format!("  - New: `{}`\n", new_sig));
                        }
                    }
                }
                output.push('\n');
            }
        }

        output
    }

    fn format_diff_human(&self, result: &DiffResult) -> String {
        let mut output = String::new();
        output.push_str(&format!("{}\n\n", "Symbol Diff".bold().green()));
        output.push_str(&format!("Commit: {}\n", result.commit[..8.min(result.commit.len())].cyan()));
        output.push_str(&format!("Files analyzed: {}\n", result.files_analyzed.to_string().bold()));
        output.push_str(&format!("Symbol changes: {}\n\n", result.symbols.len().to_string().bold()));

        if result.symbols.is_empty() {
            output.push_str(&format!("{}\n", "No symbol changes detected.".yellow()));
            return output;
        }

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Change", "Name", "Type", "File", "Lines"]);

        for sym in &result.symbols {
            let change_str = match sym.change_type {
                ChangeType::Added => format!("{}", "+".green()),
                ChangeType::Deleted => format!("{}", "-".red()),
                ChangeType::Modified => format!("{}", "~".yellow()),
                ChangeType::SignatureChanged => format!("{}", "!".magenta()),
            };
            
            let lines = if let Some((start, end)) = sym.new_lines {
                format!("{}-{}", start, end)
            } else if let Some((start, end)) = sym.old_lines {
                format!("{}-{} (del)", start, end)
            } else {
                "-".to_string()
            };

            table.add_row(vec![
                change_str,
                sym.name.clone(),
                sym.symbol_type.as_str().to_string(),
                sym.file_path.display().to_string(),
                lines,
            ]);
        }

        output.push_str(&format!("{}\n", table));

        let sig_changes: Vec<&SymbolDiff> = result.symbols.iter()
            .filter(|s| s.change_type == ChangeType::SignatureChanged)
            .collect();
        
        if !sig_changes.is_empty() {
            output.push_str(&format!("\n{}\n", "Signature Changes:".cyan()));
            for sym in sig_changes {
                output.push_str(&format!("  {} {}\n", "→".cyan(), sym.name.bold()));
                if let Some(ref old_sig) = sym.old_signature {
                    output.push_str(&format!("    Old: {}\n", old_sig.red()));
                }
                if let Some(ref new_sig) = sym.new_signature {
                    output.push_str(&format!("    New: {}\n", new_sig.green()));
                }
            }
        }

        output.push_str(&format!("\n{}: {} {} {} {} {} {} {} {}\n",
            "Legend".cyan(),
            "+".green(), "added".dimmed(),
            "-".red(), "deleted".dimmed(),
            "~".yellow(), "modified".dimmed(),
            "!".magenta(), "signature".dimmed()
        ));

        output
    }

    fn format_diff_ai(&self, result: &DiffResult) -> String {
        let mut output = String::new();
        output.push_str(&format!("[DIFF:{}]\n", &result.commit[..8.min(result.commit.len())]));
        output.push_str(&format!("FILES:{} CHANGES:{}\n", result.files_analyzed, result.symbols.len()));

        if result.symbols.is_empty() {
            output.push_str("NO_CHANGES\n");
            return output;
        }

        for sym in &result.symbols {
            output.push_str(&format!("{}|{}|{}|{}",
                sym.change_type.short(),
                sym.name,
                match sym.symbol_type {
                    SymbolType::Function => "f",
                    SymbolType::Class => "c",
                    SymbolType::Method => "m",
                    SymbolType::Enum => "e",
                    SymbolType::StaticField => "s",
                    SymbolType::Heading => "h",
                    SymbolType::CodeBlock => "cb",
                },
                sym.file_path.display()
            ));

            if let Some((start, end)) = sym.new_lines {
                output.push_str(&format!("|{}-{}", start, end));
            } else if let Some((start, end)) = sym.old_lines {
                output.push_str(&format!("|{}-{}(del)", start, end));
            }

            if sym.change_type == ChangeType::SignatureChanged {
                if let Some(ref old_sig) = sym.old_signature {
                    output.push_str(&format!("|old:{}", old_sig));
                }
                if let Some(ref new_sig) = sym.new_signature {
                    output.push_str(&format!("|new:{}", new_sig));
                }
            }

            output.push('\n');
        }

        output
    }

    pub fn format_breaking(&self, result: &DiffResult) -> String {
        match self.format {
            OutputFormat::Default => self.format_breaking_default(result),
            OutputFormat::Human => self.format_breaking_human(result),
            OutputFormat::AI => self.format_breaking_ai(result),
        }
    }

    fn format_breaking_default(&self, result: &DiffResult) -> String {
        let mut output = String::new();
        output.push_str("# Breaking Changes\n\n");
        output.push_str(&format!("Since commit: `{}`\n\n", &result.commit[..8.min(result.commit.len())]));

        if result.symbols.is_empty() {
            output.push_str("No breaking changes detected.\n");
            return output;
        }

        output.push_str(&format!("**{} breaking change(s) found**\n\n", result.symbols.len()));

        let deleted: Vec<&SymbolDiff> = result.symbols.iter()
            .filter(|s| s.change_type == ChangeType::Deleted)
            .collect();
        
        let sig_changed: Vec<&SymbolDiff> = result.symbols.iter()
            .filter(|s| s.change_type == ChangeType::SignatureChanged)
            .collect();

        if !deleted.is_empty() {
            output.push_str("## REMOVED (callers will break)\n\n");
            for sym in &deleted {
                output.push_str(&format!("- **{}** ({}) in `{}`",
                    sym.name,
                    sym.symbol_type.as_str(),
                    sym.file_path.display()
                ));
                if let Some((start, end)) = sym.old_lines {
                    output.push_str(&format!(" @ lines {}-{}", start, end));
                }
                output.push('\n');
                if let Some(ref sig) = sym.old_signature {
                    output.push_str(&format!("  Was: `{}`\n", sig));
                }
            }
            output.push('\n');
        }

        if !sig_changed.is_empty() {
            output.push_str("## SIGNATURE CHANGED (callers may need updates)\n\n");
            for sym in &sig_changed {
                output.push_str(&format!("- **{}** ({}) in `{}`\n",
                    sym.name,
                    sym.symbol_type.as_str(),
                    sym.file_path.display()
                ));
                if let Some(ref old_sig) = sym.old_signature {
                    output.push_str(&format!("  Old: `{}`\n", old_sig));
                }
                if let Some(ref new_sig) = sym.new_signature {
                    output.push_str(&format!("  New: `{}`\n", new_sig));
                }
            }
            output.push('\n');
        }

        output
    }

    fn format_breaking_human(&self, result: &DiffResult) -> String {
        let mut output = String::new();
        output.push_str(&format!("{}\n\n", "Breaking Changes".bold().red()));
        output.push_str(&format!("Since commit: {}\n\n", result.commit[..8.min(result.commit.len())].cyan()));

        if result.symbols.is_empty() {
            output.push_str(&format!("{}\n", "No breaking changes detected.".green()));
            return output;
        }

        output.push_str(&format!("{} breaking change(s) found\n\n", 
            result.symbols.len().to_string().bold().red()));

        let deleted: Vec<&SymbolDiff> = result.symbols.iter()
            .filter(|s| s.change_type == ChangeType::Deleted)
            .collect();
        
        let sig_changed: Vec<&SymbolDiff> = result.symbols.iter()
            .filter(|s| s.change_type == ChangeType::SignatureChanged)
            .collect();

        if !deleted.is_empty() {
            output.push_str(&format!("{}\n\n", "REMOVED (callers will break)".bold().red()));
            
            let mut table = Table::new();
            table
                .load_preset(UTF8_FULL)
                .apply_modifier(UTF8_ROUND_CORNERS)
                .set_header(vec!["Symbol", "Type", "File", "Was"]);

            for sym in &deleted {
                let was_sig = sym.old_signature.as_deref().unwrap_or("-");
                table.add_row(vec![
                    sym.name.clone(),
                    sym.symbol_type.as_str().to_string(),
                    sym.file_path.display().to_string(),
                    was_sig.to_string(),
                ]);
            }
            output.push_str(&format!("{}\n\n", table));
        }

        if !sig_changed.is_empty() {
            output.push_str(&format!("{}\n\n", "SIGNATURE CHANGED (callers may need updates)".bold().yellow()));

            for sym in &sig_changed {
                output.push_str(&format!("  {} {} ({})\n", "→".cyan(), sym.name.bold(), sym.symbol_type.as_str()));
                output.push_str(&format!("    File: {}\n", sym.file_path.display()));
                if let Some(ref old_sig) = sym.old_signature {
                    output.push_str(&format!("    Old: {}\n", old_sig.red()));
                }
                if let Some(ref new_sig) = sym.new_signature {
                    output.push_str(&format!("    New: {}\n", new_sig.green()));
                }
                output.push('\n');
            }
        }

        output
    }

    fn format_breaking_ai(&self, result: &DiffResult) -> String {
        let mut output = String::new();
        output.push_str(&format!("[BREAKING:{}]\n", &result.commit[..8.min(result.commit.len())]));
        output.push_str(&format!("COUNT:{}\n", result.symbols.len()));

        if result.symbols.is_empty() {
            output.push_str("NO_BREAKING_CHANGES\n");
            return output;
        }

        for sym in &result.symbols {
            let change_marker = match sym.change_type {
                ChangeType::Deleted => "REMOVED",
                ChangeType::SignatureChanged => "SIG_CHANGED",
                _ => continue,
            };
            
            output.push_str(&format!("{}|{}|{}|{}",
                change_marker,
                sym.name,
                match sym.symbol_type {
                    SymbolType::Function => "f",
                    SymbolType::Class => "c",
                    SymbolType::Method => "m",
                    SymbolType::Enum => "e",
                    SymbolType::StaticField => "s",
                    SymbolType::Heading => "h",
                    SymbolType::CodeBlock => "cb",
                },
                sym.file_path.display()
            ));

            if sym.change_type == ChangeType::Deleted {
                if let Some(ref old_sig) = sym.old_signature {
                    output.push_str(&format!("|was:{}", old_sig));
                }
            } else if sym.change_type == ChangeType::SignatureChanged {
                if let Some(ref old_sig) = sym.old_signature {
                    output.push_str(&format!("|old:{}", old_sig));
                }
                if let Some(ref new_sig) = sym.new_signature {
                    output.push_str(&format!("|new:{}", new_sig));
                }
            }

            output.push('\n');
        }

        output
    }

    pub fn format_callers(&self, callers: &[CallInfo], symbol_name: &str) -> String {
        match self.format {
            OutputFormat::Default => self.format_callers_default(callers, symbol_name),
            OutputFormat::Human => self.format_callers_human(callers, symbol_name),
            OutputFormat::AI => self.format_callers_ai(callers, symbol_name),
        }
    }

    fn format_callers_default(&self, callers: &[CallInfo], symbol_name: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("# Callers of `{}`\n\n", symbol_name));
        output.push_str(&format!("Found {} call site(s)\n\n", callers.len()));

        for caller in callers {
            output.push_str(&format!("## {} ({})\n", caller.caller_name, caller.caller_type.as_str()));
            output.push_str(&format!("- File: {}:{}\n", caller.file_path, caller.line));
            if !caller.context.is_empty() {
                output.push_str(&format!("- Context: `{}`\n", caller.context));
            }
            output.push('\n');
        }

        output
    }

    fn format_callers_human(&self, callers: &[CallInfo], symbol_name: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("{} {}\n\n", "Callers of".green(), symbol_name.bold()));

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Caller", "Type", "Location", "Context"]);

        for caller in callers {
            table.add_row(vec![
                caller.caller_name.clone(),
                caller.caller_type.as_str().to_string(),
                format!("{}:{}", caller.file_path, caller.line),
                if caller.context.len() > 60 {
                    format!("{}...", &caller.context[..57])
                } else {
                    caller.context.clone()
                },
            ]);
        }

        output.push_str(&format!("{}\n", table));
        output
    }

    fn format_callers_ai(&self, callers: &[CallInfo], symbol_name: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("[CALLERS:{}|{}]\n", symbol_name, callers.len()));

        for caller in callers {
            output.push_str(&format!("{}|{}|{}:{}",
                caller.caller_name,
                match caller.caller_type {
                    SymbolType::Function => "f",
                    SymbolType::Class => "c",
                    SymbolType::Method => "m",
                    SymbolType::Enum => "e",
                    SymbolType::StaticField => "s",
                    SymbolType::Heading => "h",
                    SymbolType::CodeBlock => "cb",
                },
                caller.file_path,
                caller.line
            ));
            output.push('\n');
        }

        output
    }

    pub fn format_callees(&self, callees: &[CallInfo], symbol_name: &str) -> String {
        match self.format {
            OutputFormat::Default => self.format_callees_default(callees, symbol_name),
            OutputFormat::Human => self.format_callees_human(callees, symbol_name),
            OutputFormat::AI => self.format_callees_ai(callees, symbol_name),
        }
    }

    fn format_callees_default(&self, callees: &[CallInfo], symbol_name: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("# Functions called by `{}`\n\n", symbol_name));
        output.push_str(&format!("Found {} callee(s)\n\n", callees.len()));

        for callee in callees {
            output.push_str(&format!("## {} ({})\n", callee.caller_name, callee.caller_type.as_str()));
            if callee.file_path != "<external>" {
                output.push_str(&format!("- Definition: {}:{}\n", callee.file_path, callee.line));
            } else {
                output.push_str("- External/built-in function\n");
            }
            if !callee.context.is_empty() {
                output.push_str(&format!("- Signature: `{}`\n", callee.context));
            }
            output.push('\n');
        }

        output
    }

    fn format_callees_human(&self, callees: &[CallInfo], symbol_name: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("{} {}\n\n", "Functions called by".green(), symbol_name.bold()));

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Callee", "Type", "Definition", "Signature"]);

        for callee in callees {
            let location = if callee.file_path != "<external>" {
                format!("{}:{}", callee.file_path, callee.line)
            } else {
                "<external>".to_string()
            };

            table.add_row(vec![
                callee.caller_name.clone(),
                callee.caller_type.as_str().to_string(),
                location,
                if callee.context.len() > 40 {
                    format!("{}...", &callee.context[..37])
                } else {
                    callee.context.clone()
                },
            ]);
        }

        output.push_str(&format!("{}\n", table));
        output
    }

    fn format_callees_ai(&self, callees: &[CallInfo], symbol_name: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("[CALLEES:{}|{}]\n", symbol_name, callees.len()));

        for callee in callees {
            output.push_str(&format!("{}|{}|{}:{}",
                callee.caller_name,
                match callee.caller_type {
                    SymbolType::Function => "f",
                    SymbolType::Class => "c",
                    SymbolType::Method => "m",
                    SymbolType::Enum => "e",
                    SymbolType::StaticField => "s",
                    SymbolType::Heading => "h",
                    SymbolType::CodeBlock => "cb",
                },
                callee.file_path,
                callee.line
            ));
            if !callee.context.is_empty() {
                output.push_str(&format!("|sig:{}", callee.context));
            }
            output.push('\n');
        }

        output
    }

    pub fn format_tests(&self, tests: &[TestInfo], symbol_name: &str) -> String {
        match self.format {
            OutputFormat::Default => self.format_tests_default(tests, symbol_name),
            OutputFormat::Human => self.format_tests_human(tests, symbol_name),
            OutputFormat::AI => self.format_tests_ai(tests, symbol_name),
        }
    }

    fn format_tests_default(&self, tests: &[TestInfo], symbol_name: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("# Tests covering `{}`\n\n", symbol_name));
        output.push_str(&format!("Found {} test(s)\n\n", tests.len()));

        for test in tests {
            output.push_str(&format!("## {} ({})\n", test.test_name, test.test_type.as_str()));
            output.push_str(&format!("- Test definition: {}:{}\n", test.file_path, test.line));
            output.push_str(&format!("- Calls symbol at: line {}\n", test.call_line));
            if !test.context.is_empty() {
                output.push_str(&format!("- Context: `{}`\n", test.context));
            }
            output.push('\n');
        }

        output
    }

    fn format_tests_human(&self, tests: &[TestInfo], symbol_name: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("{} {}\n\n", "Tests covering".green(), symbol_name.bold()));

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Test", "Type", "Location", "Call Line", "Context"]);

        for test in tests {
            table.add_row(vec![
                test.test_name.clone(),
                test.test_type.as_str().to_string(),
                format!("{}:{}", test.file_path, test.line),
                test.call_line.to_string(),
                if test.context.len() > 50 {
                    format!("{}...", &test.context[..47])
                } else {
                    test.context.clone()
                },
            ]);
        }

        output.push_str(&format!("{}\n", table));
        output
    }

    fn format_tests_ai(&self, tests: &[TestInfo], symbol_name: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("[TESTS:{}|{}]\n", symbol_name, tests.len()));

        for test in tests {
            output.push_str(&format!("{}|{}|{}:{}|call:{}",
                test.test_name,
                match test.test_type {
                    SymbolType::Function => "f",
                    SymbolType::Class => "c",
                    SymbolType::Method => "m",
                    SymbolType::Enum => "e",
                    SymbolType::StaticField => "s",
                    SymbolType::Heading => "h",
                    SymbolType::CodeBlock => "cb",
                },
                test.file_path,
                test.line,
                test.call_line
            ));
            output.push('\n');
        }

        output
    }

    pub fn format_test_deps(&self, deps: &[TestDep], test_file: &str) -> String {
        match self.format {
            OutputFormat::Default => self.format_test_deps_default(deps, test_file),
            OutputFormat::Human => self.format_test_deps_human(deps, test_file),
            OutputFormat::AI => self.format_test_deps_ai(deps, test_file),
        }
    }

    fn format_test_deps_default(&self, deps: &[TestDep], test_file: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("# Production dependencies of `{}`\n\n", test_file));
        output.push_str(&format!("Found {} production symbol(s) called\n\n", deps.len()));

        let mut current_file = String::new();
        for dep in deps {
            if dep.file_path != current_file {
                current_file = dep.file_path.clone();
                output.push_str(&format!("\n## {}\n\n", current_file));
            }
            
            output.push_str(&format!("- **{}** ({}) @ line {} (called from test line {})\n",
                dep.name,
                dep.symbol_type.as_str(),
                dep.line,
                dep.called_from_line
            ));
        }

        output
    }

    fn format_test_deps_human(&self, deps: &[TestDep], test_file: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("{} {}\n\n", "Production dependencies of".green(), test_file.bold()));
        output.push_str(&format!("Found {} production symbol(s)\n\n", deps.len().to_string().bold()));

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Symbol", "Type", "Location", "Called From"]);

        for dep in deps {
            table.add_row(vec![
                dep.name.clone(),
                dep.symbol_type.as_str().to_string(),
                format!("{}:{}", dep.file_path, dep.line),
                format!("line {}", dep.called_from_line),
            ]);
        }

        output.push_str(&format!("{}\n", table));
        output
    }

    fn format_test_deps_ai(&self, deps: &[TestDep], test_file: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("[TEST_DEPS:{}|{}]\n", test_file, deps.len()));

        for dep in deps {
            output.push_str(&format!("{}|{}|{}:{}|from:{}",
                dep.name,
                match dep.symbol_type {
                    SymbolType::Function => "f",
                    SymbolType::Class => "c",
                    SymbolType::Method => "m",
                    SymbolType::Enum => "e",
                    SymbolType::StaticField => "s",
                    SymbolType::Heading => "h",
                    SymbolType::CodeBlock => "cb",
                },
                dep.file_path,
                dep.line,
                dep.called_from_line
            ));
            output.push('\n');
        }

        output
    }

    pub fn format_untested(&self, untested: &[UntestedInfo], total_symbols: usize) -> String {
        match self.format {
            OutputFormat::Default => self.format_untested_default(untested, total_symbols),
            OutputFormat::Human => self.format_untested_human(untested, total_symbols),
            OutputFormat::AI => self.format_untested_ai(untested, total_symbols),
        }
    }

    fn format_untested_default(&self, untested: &[UntestedInfo], total_symbols: usize) -> String {
        let mut output = String::new();
        output.push_str("# Untested Symbols\n\n");
        
        let tested_count = total_symbols.saturating_sub(untested.len());
        let coverage_pct = if total_symbols > 0 {
            (tested_count as f64 / total_symbols as f64) * 100.0
        } else {
            100.0
        };
        
        output.push_str(&format!("**Coverage**: {:.1}% ({} of {} symbols tested)\n", 
            coverage_pct, tested_count, total_symbols));
        output.push_str(&format!("**Untested**: {} symbols\n\n", untested.len()));

        let mut current_file = String::new();
        for info in untested {
            if info.file_path != current_file {
                current_file = info.file_path.clone();
                output.push_str(&format!("\n## {}\n\n", current_file));
            }
            
            output.push_str(&format!("- **{}** ({}) @ line {}", 
                info.name, 
                info.symbol_type.as_str(),
                info.line
            ));
            if let Some(ref sig) = info.signature {
                output.push_str(&format!(": `{}`", sig));
            }
            output.push('\n');
        }

        output
    }

    fn format_untested_human(&self, untested: &[UntestedInfo], total_symbols: usize) -> String {
        let mut output = String::new();
        output.push_str(&format!("{}\n\n", "Untested Symbols".bold().red()));

        let tested_count = total_symbols.saturating_sub(untested.len());
        let coverage_pct = if total_symbols > 0 {
            (tested_count as f64 / total_symbols as f64) * 100.0
        } else {
            100.0
        };

        let coverage_color = if coverage_pct >= 80.0 {
            format!("{:.1}%", coverage_pct).green()
        } else if coverage_pct >= 50.0 {
            format!("{:.1}%", coverage_pct).yellow()
        } else {
            format!("{:.1}%", coverage_pct).red()
        };

        output.push_str(&format!("{}: {} ({} of {} symbols tested)\n",
            "Coverage".cyan(),
            coverage_color,
            tested_count,
            total_symbols
        ));
        output.push_str(&format!("{}: {}\n\n",
            "Untested".cyan(),
            untested.len().to_string().bold()
        ));

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Symbol", "Type", "File", "Line", "Signature"]);

        for info in untested {
            table.add_row(vec![
                info.name.clone(),
                info.symbol_type.as_str().to_string(),
                info.file_path.clone(),
                info.line.to_string(),
                info.signature.as_deref().unwrap_or("-").to_string(),
            ]);
        }

        output.push_str(&format!("{}\n", table));
        output
    }

    fn format_untested_ai(&self, untested: &[UntestedInfo], total_symbols: usize) -> String {
        let mut output = String::new();
        
        let tested_count = total_symbols.saturating_sub(untested.len());
        let coverage_pct = if total_symbols > 0 {
            (tested_count as f64 / total_symbols as f64) * 100.0
        } else {
            100.0
        };

        output.push_str(&format!("[UNTESTED:{}|{:.1}%]\n", untested.len(), coverage_pct));
        output.push_str(&format!("TESTED:{} TOTAL:{}\n", tested_count, total_symbols));

        for info in untested {
            output.push_str(&format!("{}|{}|{}:{}",
                info.name,
                match info.symbol_type {
                    SymbolType::Function => "f",
                    SymbolType::Class => "c",
                    SymbolType::Method => "m",
                    SymbolType::Enum => "e",
                    SymbolType::StaticField => "s",
                    SymbolType::Heading => "h",
                    SymbolType::CodeBlock => "cb",
                },
                info.file_path,
                info.line
            ));
            if let Some(ref sig) = info.signature {
                output.push_str(&format!("|sig:{}", sig));
            }
            output.push('\n');
        }

        output
    }

    pub fn format_entrypoints(&self, entrypoints: &[EntrypointInfo]) -> String {
        match self.format {
            OutputFormat::Default => self.format_entrypoints_default(entrypoints),
            OutputFormat::Human => self.format_entrypoints_human(entrypoints),
            OutputFormat::AI => self.format_entrypoints_ai(entrypoints),
        }
    }

    fn format_entrypoints_default(&self, entrypoints: &[EntrypointInfo]) -> String {
        let mut output = String::new();
        output.push_str("# Entrypoints (Uncalled Exported Symbols)\n\n");
        output.push_str(&format!("Found {} entrypoint(s)\n\n", entrypoints.len()));

        let main_entries: Vec<_> = entrypoints.iter()
            .filter(|e| e.category == EntrypointCategory::MainEntry)
            .collect();
        let api_funcs: Vec<_> = entrypoints.iter()
            .filter(|e| e.category == EntrypointCategory::ApiFunction)
            .collect();
        let unused: Vec<_> = entrypoints.iter()
            .filter(|e| e.category == EntrypointCategory::PossiblyUnused)
            .collect();

        if !main_entries.is_empty() {
            output.push_str("## Main Entrypoints\n\n");
            for entry in &main_entries {
                output.push_str(&format!("- **{}** ({}) @ `{}:{}`",
                    entry.name, entry.symbol_type.as_str(), entry.file_path, entry.line));
                if let Some(ref sig) = entry.signature {
                    output.push_str(&format!("\n  `{}`", sig));
                }
                output.push('\n');
            }
            output.push('\n');
        }

        if !api_funcs.is_empty() {
            output.push_str("## API Functions\n\n");
            for entry in &api_funcs {
                output.push_str(&format!("- **{}** ({}) @ `{}:{}`",
                    entry.name, entry.symbol_type.as_str(), entry.file_path, entry.line));
                if let Some(ref sig) = entry.signature {
                    output.push_str(&format!("\n  `{}`", sig));
                }
                output.push('\n');
            }
            output.push('\n');
        }

        if !unused.is_empty() {
            output.push_str("## Possibly Unused\n\n");
            for entry in &unused {
                output.push_str(&format!("- **{}** ({}) @ `{}:{}`",
                    entry.name, entry.symbol_type.as_str(), entry.file_path, entry.line));
                if let Some(ref sig) = entry.signature {
                    output.push_str(&format!("\n  `{}`", sig));
                }
                output.push('\n');
            }
            output.push('\n');
        }

        output
    }

    fn format_entrypoints_human(&self, entrypoints: &[EntrypointInfo]) -> String {
        let mut output = String::new();
        output.push_str(&format!("{}\n\n", "Entrypoints (Uncalled Exported Symbols)".bold().green()));

        let main_count = entrypoints.iter().filter(|e| e.category == EntrypointCategory::MainEntry).count();
        let api_count = entrypoints.iter().filter(|e| e.category == EntrypointCategory::ApiFunction).count();
        let unused_count = entrypoints.iter().filter(|e| e.category == EntrypointCategory::PossiblyUnused).count();

        output.push_str(&format!("{}: {} total ({} main, {} API, {} possibly unused)\n\n",
            "Found".cyan(),
            entrypoints.len().to_string().bold(),
            main_count.to_string().green(),
            api_count.to_string().cyan(),
            unused_count.to_string().yellow()
        ));

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Category", "Symbol", "Type", "File", "Line", "Signature"]);

        for entry in entrypoints {
            let category_str = match entry.category {
                EntrypointCategory::MainEntry => "Main".green().to_string(),
                EntrypointCategory::ApiFunction => "API".cyan().to_string(),
                EntrypointCategory::PossiblyUnused => "Unused?".yellow().to_string(),
            };
            table.add_row(vec![
                category_str,
                entry.name.clone(),
                entry.symbol_type.as_str().to_string(),
                entry.file_path.clone(),
                entry.line.to_string(),
                entry.signature.as_deref().unwrap_or("-").to_string(),
            ]);
        }

        output.push_str(&format!("{}\n", table));
        output
    }

    fn format_entrypoints_ai(&self, entrypoints: &[EntrypointInfo]) -> String {
        let mut output = String::new();
        
        let main_count = entrypoints.iter().filter(|e| e.category == EntrypointCategory::MainEntry).count();
        let api_count = entrypoints.iter().filter(|e| e.category == EntrypointCategory::ApiFunction).count();
        let unused_count = entrypoints.iter().filter(|e| e.category == EntrypointCategory::PossiblyUnused).count();

        output.push_str(&format!("[ENTRYPOINTS:{}]\n", entrypoints.len()));
        output.push_str(&format!("MAIN:{} API:{} UNUSED:{}\n", main_count, api_count, unused_count));

        for entry in entrypoints {
            let cat_short = match entry.category {
                EntrypointCategory::MainEntry => "main",
                EntrypointCategory::ApiFunction => "api",
                EntrypointCategory::PossiblyUnused => "unused",
            };
            output.push_str(&format!("{}|{}|{}|{}:{}",
                cat_short,
                entry.name,
                match entry.symbol_type {
                    SymbolType::Function => "f",
                    SymbolType::Class => "c",
                    SymbolType::Method => "m",
                    SymbolType::Enum => "e",
                    SymbolType::StaticField => "s",
                    SymbolType::Heading => "h",
                    SymbolType::CodeBlock => "cb",
                },
                entry.file_path,
                entry.line
            ));
            if let Some(ref sig) = entry.signature {
                output.push_str(&format!("|sig:{}", sig));
            }
            output.push('\n');
        }

        output
    }

    pub fn format_blame(&self, result: &BlameResult) -> String {
        match self.format {
            OutputFormat::Default => self.format_blame_default(result),
            OutputFormat::Human => self.format_blame_human(result),
            OutputFormat::AI => self.format_blame_ai(result),
        }
    }

    fn format_blame_default(&self, result: &BlameResult) -> String {
        let mut output = String::new();
        output.push_str(&format!("# Blame for `{}`\n\n", result.symbol_name));
        output.push_str(&format!("Type: {}\n", result.symbol_type.as_str()));
        output.push_str(&format!("Lines: {}-{}\n\n", result.current_lines.0, result.current_lines.1));
        
        output.push_str("## Last Modification\n\n");
        output.push_str(&format!("- Commit: `{}`\n", result.last_commit.short_hash));
        output.push_str(&format!("- Author: {}\n", result.last_commit.author));
        output.push_str(&format!("- Date: {}\n", result.last_commit.date));
        output.push_str(&format!("- Message: {}\n", result.last_commit.message));
        
        if result.old_signature.is_some() || result.new_signature.is_some() {
            output.push_str("\n## Signature Change\n\n");
            if let Some(ref old_sig) = result.old_signature {
                output.push_str(&format!("- Old: `{}`\n", old_sig));
            }
            if let Some(ref new_sig) = result.new_signature {
                output.push_str(&format!("- New: `{}`\n", new_sig));
            }
        }
        
        output
    }

    fn format_blame_human(&self, result: &BlameResult) -> String {
        let mut output = String::new();
        output.push_str(&format!("{} {}\n\n", "Blame for".green(), result.symbol_name.bold()));
        output.push_str(&format!("Type: {} | Lines: {}-{}\n\n", 
            result.symbol_type.as_str().cyan(),
            result.current_lines.0,
            result.current_lines.1
        ));

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Commit", "Author", "Date", "Message"]);

        table.add_row(vec![
            result.last_commit.short_hash.clone(),
            result.last_commit.author.clone(),
            result.last_commit.date.clone(),
            if result.last_commit.message.len() > 50 {
                format!("{}...", &result.last_commit.message[..47])
            } else {
                result.last_commit.message.clone()
            },
        ]);

        output.push_str(&format!("{}\n", table));

        if result.old_signature.is_some() || result.new_signature.is_some() {
            output.push_str(&format!("\n{}\n", "Signature Change:".cyan()));
            if let Some(ref old_sig) = result.old_signature {
                output.push_str(&format!("  Old: {}\n", old_sig.red()));
            }
            if let Some(ref new_sig) = result.new_signature {
                output.push_str(&format!("  New: {}\n", new_sig.green()));
            }
        }

        output
    }

    fn format_blame_ai(&self, result: &BlameResult) -> String {
        let mut output = String::new();
        output.push_str(&format!("[BLAME:{}|{}]\n", result.symbol_name, result.symbol_type.as_str()));
        output.push_str(&format!("LINES:{}-{}\n", result.current_lines.0, result.current_lines.1));
        output.push_str(&format!("COMMIT:{}|{}|{}|{}\n",
            result.last_commit.short_hash,
            result.last_commit.author,
            result.last_commit.date,
            result.last_commit.message
        ));
        
        if let Some(ref old_sig) = result.old_signature {
            output.push_str(&format!("OLD_SIG:{}\n", old_sig));
        }
        if let Some(ref new_sig) = result.new_signature {
            output.push_str(&format!("NEW_SIG:{}\n", new_sig));
        }

        output
    }

    pub fn format_history(&self, history: &[HistoryEntry], symbol_name: &str) -> String {
        match self.format {
            OutputFormat::Default => self.format_history_default(history, symbol_name),
            OutputFormat::Human => self.format_history_human(history, symbol_name),
            OutputFormat::AI => self.format_history_ai(history, symbol_name),
        }
    }

    fn format_history_default(&self, history: &[HistoryEntry], symbol_name: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("# History for `{}`\n\n", symbol_name));
        output.push_str(&format!("Found {} version(s)\n\n", history.len()));

        for (i, entry) in history.iter().enumerate() {
            let version_num = history.len() - i;
            let status = if entry.existed { "exists" } else { "deleted" };
            
            output.push_str(&format!("## Version {} ({})\n\n", version_num, status));
            output.push_str(&format!("- Commit: `{}`\n", entry.commit.short_hash));
            output.push_str(&format!("- Author: {}\n", entry.commit.author));
            output.push_str(&format!("- Date: {}\n", entry.commit.date));
            output.push_str(&format!("- Message: {}\n", entry.commit.message));
            
            if let Some((start, end)) = entry.lines {
                output.push_str(&format!("- Lines: {}-{}\n", start, end));
            }
            if let Some(ref sig) = entry.signature {
                output.push_str(&format!("- Signature: `{}`\n", sig));
            }
            output.push('\n');
        }

        output
    }

    fn format_history_human(&self, history: &[HistoryEntry], symbol_name: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("{} {}\n\n", "History for".green(), symbol_name.bold()));
        output.push_str(&format!("Found {} version(s)\n\n", history.len().to_string().bold()));

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["#", "Commit", "Date", "Status", "Signature"]);

        for (i, entry) in history.iter().enumerate() {
            let version_num = history.len() - i;
            let status = if entry.existed { 
                "✓".green().to_string() 
            } else { 
                "✗".red().to_string() 
            };
            
            let sig = entry.signature.as_deref().unwrap_or("-");
            let sig_display = if sig.len() > 40 {
                format!("{}...", &sig[..37])
            } else {
                sig.to_string()
            };

            table.add_row(vec![
                version_num.to_string(),
                entry.commit.short_hash.clone(),
                entry.commit.date.clone(),
                status,
                sig_display,
            ]);
        }

        output.push_str(&format!("{}\n", table));
        output
    }

    fn format_history_ai(&self, history: &[HistoryEntry], symbol_name: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("[HISTORY:{}|{}]\n", symbol_name, history.len()));

        for entry in history {
            let status = if entry.existed { "+" } else { "-" };
            output.push_str(&format!("{}|{}|{}|{}",
                status,
                entry.commit.short_hash,
                entry.commit.date,
                entry.commit.message
            ));
            
            if let Some((start, end)) = entry.lines {
                output.push_str(&format!("|{}-{}", start, end));
            }
            if let Some(ref sig) = entry.signature {
                output.push_str(&format!("|sig:{}", sig));
            }
            output.push('\n');
        }

        output
    }

    pub fn format_trace(&self, trace: &TracePath, from: &str, to: &str) -> String {
        match self.format {
            OutputFormat::Default => self.format_trace_default(trace, from, to),
            OutputFormat::Human => self.format_trace_human(trace, from, to),
            OutputFormat::AI => self.format_trace_ai(trace, from, to),
        }
    }

    fn format_trace_default(&self, trace: &TracePath, from: &str, to: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("# Call Path: {} → {}\n\n", from, to));

        if !trace.found {
            output.push_str("No call path found between these symbols.\n");
            return output;
        }

        output.push_str(&format!("**Path length**: {} step(s)\n\n", trace.steps.len()));
        output.push_str("## Call Chain\n\n");
        output.push_str("```\n");

        for (i, step) in trace.steps.iter().enumerate() {
            if i > 0 {
                output.push_str("    ↓\n");
            }
            output.push_str(&format!("[{}] {} ({})\n", 
                i + 1,
                step.symbol_name,
                step.symbol_type.as_str()
            ));
            output.push_str(&format!("    {}:{}\n", step.file_path, step.line));
        }

        output.push_str("```\n");
        output
    }

    fn format_trace_human(&self, trace: &TracePath, from: &str, to: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("{} {} {} {}\n\n", 
            "Call Path:".green(),
            from.bold(),
            "→".cyan(),
            to.bold()
        ));

        if !trace.found {
            output.push_str(&format!("{} No call path found between these symbols.\n", "✗".yellow()));
            return output;
        }

        output.push_str(&format!("{}: {} step(s)\n\n", "Path length".cyan(), trace.steps.len()));

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Step", "Symbol", "Type", "Location"]);

        for (i, step) in trace.steps.iter().enumerate() {
            let step_marker = if i == 0 {
                "→".green().to_string()
            } else if i == trace.steps.len() - 1 {
                "◉".cyan().to_string()
            } else {
                "↓".white().to_string()
            };

            table.add_row(vec![
                step_marker,
                step.symbol_name.clone(),
                step.symbol_type.as_str().to_string(),
                format!("{}:{}", step.file_path, step.line),
            ]);
        }

        output.push_str(&format!("{}\n", table));

        output.push_str(&format!("\n{}\n", "Call Chain:".cyan()));
        let names: Vec<&str> = trace.steps.iter().map(|s| s.symbol_name.as_str()).collect();
        output.push_str(&format!("  {}\n", names.join(" → ")));

        output
    }

    fn format_trace_ai(&self, trace: &TracePath, from: &str, to: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("[TRACE:{}->{}]\n", from, to));

        if !trace.found {
            output.push_str("FOUND:false\n");
            return output;
        }

        output.push_str(&format!("FOUND:true STEPS:{}\n", trace.steps.len()));

        let names: Vec<&str> = trace.steps.iter().map(|s| s.symbol_name.as_str()).collect();
        output.push_str(&format!("PATH:{}\n", names.join("|")));

        for step in &trace.steps {
            output.push_str(&format!("{}|{}|{}:{}\n",
                step.symbol_name,
                match step.symbol_type {
                    SymbolType::Function => "f",
                    SymbolType::Class => "c",
                    SymbolType::Method => "m",
                    SymbolType::Enum => "e",
                    SymbolType::StaticField => "s",
                    SymbolType::Heading => "h",
                    SymbolType::CodeBlock => "cb",
                },
                step.file_path,
                step.line
            ));
        }

        output
    }

    pub fn format_implements(&self, implementations: &[Implementation], interface: &str) -> String {
        match self.format {
            OutputFormat::Default => self.format_implements_default(implementations, interface),
            OutputFormat::Human => self.format_implements_human(implementations, interface),
            OutputFormat::AI => self.format_implements_ai(implementations, interface),
        }
    }

    fn format_implements_default(&self, implementations: &[Implementation], interface: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("# Implementations of `{}`\n\n", interface));
        output.push_str(&format!("Found {} implementation(s)\n\n", implementations.len()));

        for imp in implementations {
            output.push_str(&format!("## {}\n", imp.implementor_name));
            output.push_str(&format!("- Interface: {}\n", imp.interface_name));
            output.push_str(&format!("- Kind: {}\n", imp.kind.as_str()));
            output.push_str(&format!("- Language: {}\n", imp.language.as_str()));
            output.push_str(&format!("- Location: `{}:{}`\n", imp.file_path.display(), imp.line));
            output.push('\n');
        }

        output
    }

    fn format_implements_human(&self, implementations: &[Implementation], interface: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("{} {}\n\n", "Implementations of".green(), interface.bold()));
        output.push_str(&format!("{} {} implementation(s)\n\n", "Found".cyan(), implementations.len().to_string().bold()));

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_header(vec!["Implementor", "Interface", "Kind", "Language", "Location"]);

        for imp in implementations {
            table.add_row(vec![
                imp.implementor_name.clone(),
                imp.interface_name.clone(),
                imp.kind.as_str().to_string(),
                imp.language.as_str().to_string(),
                format!("{}:{}", imp.file_path.display(), imp.line),
            ]);
        }

        output.push_str(&format!("{}\n", table));
        output
    }

    fn format_implements_ai(&self, implementations: &[Implementation], interface: &str) -> String {
        let mut output = String::new();
        output.push_str(&format!("[IMPLEMENTS:{}|{}]\n", interface, implementations.len()));

        for imp in implementations {
            output.push_str(&format!("{}|{}|{}|{}|{}:{}\n",
                imp.implementor_name,
                imp.interface_name,
                imp.kind.as_str(),
                imp.language.as_str(),
                imp.file_path.display(),
                imp.line
            ));
        }

        output
    }

    pub fn format_types(&self, types_info: &[SymbolTypes]) -> String {
        match self.format {
            OutputFormat::Default => self.format_types_default(types_info),
            OutputFormat::Human => self.format_types_human(types_info),
            OutputFormat::AI => self.format_types_ai(types_info),
        }
    }

    fn format_types_default(&self, types_info: &[SymbolTypes]) -> String {
        let mut output = String::new();
        output.push_str("# Type Analysis\n\n");
        output.push_str(&format!("Found {} symbol(s)\n\n", types_info.len()));

        for symbol in types_info {
            output.push_str(&format!("## {}\n\n", symbol.symbol_name));
            output.push_str(&format!("- Type: {}\n", symbol.symbol_type.as_str()));
            output.push_str(&format!("- Location: `{}:{}`\n", symbol.file_path, symbol.line));
            
            if let Some(ref sig) = symbol.signature {
                output.push_str(&format!("- Signature: `{}`\n", sig));
            }
            output.push('\n');

            if !symbol.params.is_empty() {
                output.push_str("### Parameters\n\n");
                output.push_str("| Name | Type | Defined In |\n");
                output.push_str("|------|------|------------|\n");
                for param in &symbol.params {
                    let defined = param.defined_in.as_deref().unwrap_or("-");
                    output.push_str(&format!("| {} | `{}` | {} |\n", 
                        param.name, param.type_name, defined));
                }
                output.push('\n');
            }

            if let Some(ref ret) = symbol.return_type {
                output.push_str("### Return Type\n\n");
                let defined = ret.defined_in.as_deref().unwrap_or("-");
                output.push_str(&format!("- `{}` (defined in: {})\n\n", ret.type_name, defined));
            }
        }

        output
    }

    fn format_types_human(&self, types_info: &[SymbolTypes]) -> String {
        let mut output = String::new();
        output.push_str(&format!("{}\n\n", "Type Analysis".bold().green()));
        output.push_str(&format!("{} {} symbol(s)\n\n", "Found".cyan(), types_info.len().to_string().bold()));

        for symbol in types_info {
            output.push_str(&format!("{} {} ({})\n", 
                "→".cyan(), 
                symbol.symbol_name.bold(),
                symbol.symbol_type.as_str()
            ));
            output.push_str(&format!("  Location: {}:{}\n", symbol.file_path, symbol.line));
            
            if let Some(ref sig) = symbol.signature {
                output.push_str(&format!("  Signature: {}\n", sig.cyan()));
            }
            output.push('\n');

            if !symbol.params.is_empty() || symbol.return_type.is_some() {
                let mut table = Table::new();
                table
                    .load_preset(UTF8_FULL)
                    .apply_modifier(UTF8_ROUND_CORNERS)
                    .set_header(vec!["Kind", "Name", "Type", "Defined In"]);

                for param in &symbol.params {
                    let defined = param.defined_in.as_deref().unwrap_or("-");
                    table.add_row(vec![
                        param.kind.as_str().to_string(),
                        param.name.clone(),
                        param.type_name.clone(),
                        defined.to_string(),
                    ]);
                }

                if let Some(ref ret) = symbol.return_type {
                    let defined = ret.defined_in.as_deref().unwrap_or("-");
                    table.add_row(vec![
                        ret.kind.as_str().to_string(),
                        "-".to_string(),
                        ret.type_name.clone(),
                        defined.to_string(),
                    ]);
                }

                output.push_str(&format!("{}\n\n", table));
            }
        }

        output
    }

    fn format_types_ai(&self, types_info: &[SymbolTypes]) -> String {
        let mut output = String::new();
        output.push_str(&format!("[TYPES:{}]\n", types_info.len()));

        for symbol in types_info {
            output.push_str(&format!("SYM:{}|{}|{}:{}\n", 
                symbol.symbol_name,
                match symbol.symbol_type {
                    SymbolType::Function => "f",
                    SymbolType::Class => "c",
                    SymbolType::Method => "m",
                    SymbolType::Enum => "e",
                    SymbolType::StaticField => "s",
                    SymbolType::Heading => "h",
                    SymbolType::CodeBlock => "cb",
                },
                symbol.file_path,
                symbol.line
            ));

            if let Some(ref sig) = symbol.signature {
                output.push_str(&format!("SIG:{}\n", sig));
            }

            for param in &symbol.params {
                let defined = param.defined_in.as_deref().unwrap_or("-");
                output.push_str(&format!("P:{}|{}|{}\n", 
                    param.name, param.type_name, defined));
            }

            if let Some(ref ret) = symbol.return_type {
                let defined = ret.defined_in.as_deref().unwrap_or("-");
                output.push_str(&format!("R:{}|{}\n", ret.type_name, defined));
            }
        }

        output
    }
}

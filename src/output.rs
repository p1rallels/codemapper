use crate::callgraph::{CallInfo, TestInfo};
use crate::diff::{ChangeType, DiffResult, SymbolDiff};
use crate::index::CodeIndex;
use crate::models::{Symbol, SymbolType};
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
}

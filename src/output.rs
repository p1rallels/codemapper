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
}

use crate::callgraph;
use crate::models::SymbolType;
use crate::output::{OutputFormat, OutputFormatter};
use anyhow::Result;
use std::path::PathBuf;

pub fn cmd_impact(
    symbol: String,
    path: PathBuf,
    exact: bool,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
) -> Result<()> {
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = crate::try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache)?;

    let symbols = if exact {
        index.query_symbol(&symbol)
    } else {
        index.fuzzy_search(&symbol)
    };

    if symbols.is_empty() {
        println!("{} Symbol '{}' not found in codebase", "✗", symbol);
        return Ok(());
    }

    let formatter = OutputFormatter::new(format);

    // v1: choose best match (same behavior as other commands)
    let target = symbols[0];

    let callers = callgraph::find_callers(&index, &target.name, !exact)?;
    let tests = callgraph::find_tests(&index, &target.name, !exact)?;

    let is_untested = matches!(
        target.symbol_type,
        SymbolType::Function | SymbolType::Method
    ) && tests.is_empty();

    let mut out = String::new();
    out.push_str(&format!("# Impact: `{}`\n\n", target.name));
    out.push_str(&format!("- Type: {}\n", target.symbol_type.as_str()));
    out.push_str(&format!(
        "- Location: {}:{}\n",
        target.file_path.display(),
        target.line_start
    ));
    if let Some(sig) = &target.signature {
        out.push_str(&format!("- Signature: `{}`\n", sig));
    }
    out.push_str(&format!("- Callers: {}\n", callers.len()));
    out.push_str(&format!("- Tests: {}\n", tests.len()));
    if is_untested {
        out.push_str("- Untested: true\n");
    }
    out.push('\n');

    out.push_str(&formatter.format_callers(&callers, &target.name));
    out.push('\n');
    out.push_str(&formatter.format_tests(&tests, &target.name));

    println!("{}", out);
    Ok(())
}

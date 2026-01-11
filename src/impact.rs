use crate::callgraph;
use crate::models::SymbolType;
use crate::output::{OutputFormat, OutputFormatter};
use anyhow::Result;
use std::path::PathBuf;

pub fn cmd_impact(
    symbol: String,
    path: PathBuf,
    exact: bool,
    include_docs: bool,
    limit: Option<usize>,
    all: bool,
    extensions: String,
    no_cache: bool,
    rebuild_cache: bool,
    format: OutputFormat,
) -> Result<()> {
    let ext_list: Vec<&str> = extensions.split(',').map(|s| s.trim()).collect();
    let index = crate::try_load_or_rebuild(&path, &ext_list, no_cache, rebuild_cache)?;

    let symbol = normalize_qualified_name(&symbol);

    let raw_matches = if exact {
        index.query_symbol(&symbol)
    } else {
        index.fuzzy_search(&symbol)
    };

    let matches: Vec<_> = raw_matches
        .into_iter()
        .filter(|s| {
            if include_docs {
                true
            } else {
                !matches!(s.symbol_type, SymbolType::Heading | SymbolType::CodeBlock)
            }
        })
        .collect();

    if matches.is_empty() {
        println!("{} Symbol '{}' not found in codebase", "✗", symbol);
        return Ok(());
    }

    let show_limit = if all { None } else { Some(limit.unwrap_or(10)) };

    // default behavior should not require flags:
    // - if fuzzy match yields multiple, show shortlist and pick best match (first)
    // - print only top-n callers/tests by default
    let target = matches[0];

    let formatter = OutputFormatter::new(format);

    let mut callers = callgraph::find_callers(&index, &target.name, !exact)?;
    let mut tests = callgraph::find_tests(&index, &target.name, !exact)?;

    // reduce self/noise by default: drop entries where caller == target
    callers.retain(|c| c.caller_name != target.name);

    let is_untested = matches!(
        target.symbol_type,
        SymbolType::Function | SymbolType::Method
    ) && tests.is_empty();

    let (callers_total, callers_truncated) = truncate_vec(&mut callers, show_limit);
    let (tests_total, tests_truncated) = truncate_vec(&mut tests, show_limit);

    let mut out = String::new();

    if !exact && matches.len() > 1 {
        out.push_str(&format!("[MATCHES:{}]\n", matches.len()));
        for (i, s) in matches.iter().take(5).enumerate() {
            out.push_str(&format!(
                "{}|{}|{}:{}\n",
                i + 1,
                s.symbol_type.as_str(),
                s.file_path.display(),
                s.line_start
            ));
        }
        out.push('\n');
    }

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

    out.push_str(&format!("- Callers: {}\n", callers_total));
    if callers_truncated {
        out.push_str(&format!("  (showing first {})\n", show_limit.unwrap_or(0)));
    }

    out.push_str(&format!("- Tests: {}\n", tests_total));
    if tests_truncated {
        out.push_str(&format!("  (showing first {})\n", show_limit.unwrap_or(0)));
    }

    if is_untested {
        out.push_str("- Untested: true\n");
    }

    out.push('\n');

    // only print sections if there's something to show, to avoid noisy empty headers
    if callers_total > 0 {
        out.push_str(&formatter.format_callers(&callers, &target.name));
        out.push('\n');
    }

    if tests_total > 0 {
        out.push_str(&formatter.format_tests(&tests, &target.name));
    }

    println!("{}", out);
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

fn truncate_vec<T>(items: &mut Vec<T>, limit: Option<usize>) -> (usize, bool) {
    let total = items.len();
    if let Some(lim) = limit {
        if items.len() > lim {
            items.truncate(lim);
            return (total, true);
        }
    }
    (total, false)
}

use crate::models::{Symbol, SymbolType};
use anyhow::Result;
use std::collections::{HashMap, HashSet};

pub fn heading_level(symbol: &Symbol) -> Option<usize> {
    if symbol.symbol_type != SymbolType::Heading {
        return None;
    }
    let sig = symbol.signature.as_deref()?;
    let sig = sig.trim();
    if !sig.starts_with('h') {
        return None;
    }
    let digits: String = sig
        .chars()
        .skip(1)
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<usize>().ok()
}

pub fn heading_leaf(name: &str) -> &str {
    name.rsplit(" > ").next().unwrap_or(name)
}

fn normalized_section_key(value: &str) -> String {
    let mut normalized = String::new();
    let mut pending_space = false;

    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() {
            if pending_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push(ch);
            pending_space = false;
        } else {
            pending_space = true;
        }
    }

    normalized
}

fn resolve_section_matches(
    symbols: &[Symbol],
    section: &str,
    matches: Vec<usize>,
) -> Result<usize> {
    match matches.len() {
        0 => anyhow::bail!("section not found: {}", section),
        1 => Ok(matches[0]),
        _ => {
            let mut names: Vec<String> = matches.iter().map(|i| symbols[*i].name.clone()).collect();
            names.sort();
            anyhow::bail!(
                "section is ambiguous: {}\n\nmatched headings:\n{}",
                section,
                names
                    .into_iter()
                    .map(|n| format!("- {}", n))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }
    }
}

pub fn compute_heading_end_lines(
    headings: &[(usize, usize, usize)],
    total_lines: usize,
) -> HashMap<usize, usize> {
    let mut end_lines = HashMap::<usize, usize>::new();
    let mut stack: Vec<(usize, usize, usize)> = Vec::new();

    for (idx, level, line_start) in headings {
        while let Some((prev_level, prev_idx, _)) = stack.last().copied() {
            if *level > prev_level {
                break;
            }
            stack.pop();
            let end = line_start.saturating_sub(1).max(1);
            end_lines.insert(prev_idx, end);
        }
        stack.push((*level, *idx, *line_start));
    }

    while let Some((_level, idx, _line_start)) = stack.pop() {
        end_lines.insert(idx, total_lines.max(1));
    }

    end_lines
}

pub fn select_section_heading(symbols: &[Symbol], section: &str) -> Result<usize> {
    let section = section.trim();
    if section.is_empty() {
        anyhow::bail!("--section cannot be empty");
    }

    let exact: Vec<usize> = symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.symbol_type == SymbolType::Heading)
        .filter(|(_, s)| s.name == section)
        .map(|(i, _)| i)
        .collect();
    if exact.len() == 1 {
        return Ok(exact[0]);
    }

    let leaf_matches: Vec<usize> = symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.symbol_type == SymbolType::Heading)
        .filter(|(_, s)| heading_leaf(&s.name) == section)
        .map(|(i, _)| i)
        .collect();
    if !leaf_matches.is_empty() {
        return resolve_section_matches(symbols, section, leaf_matches);
    }

    let section_key = normalized_section_key(section);
    let normalized_matches: Vec<usize> = symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.symbol_type == SymbolType::Heading)
        .filter(|(_, s)| {
            normalized_section_key(heading_leaf(&s.name)) == section_key
                || normalized_section_key(&s.name) == section_key
        })
        .map(|(i, _)| i)
        .collect();

    resolve_section_matches(symbols, section, normalized_matches)
}

pub fn is_descendant(symbols: &[Symbol], symbol_idx: usize, root_idx: usize) -> bool {
    if symbol_idx == root_idx {
        return true;
    }

    let mut cur = symbols.get(symbol_idx).and_then(|s| s.parent_id);
    let mut seen = HashSet::<usize>::new();

    while let Some(pid) = cur {
        if !seen.insert(pid) {
            break;
        }
        if pid == root_idx {
            return true;
        }
        cur = symbols.get(pid).and_then(|s| s.parent_id);
    }

    false
}

pub fn nearest_printable_heading_ancestor(
    symbols: &[Symbol],
    symbol_idx: usize,
    printable_headings: &HashSet<usize>,
) -> Option<usize> {
    let mut cur = symbols.get(symbol_idx).and_then(|s| s.parent_id);
    let mut seen = HashSet::<usize>::new();

    while let Some(pid) = cur {
        if !seen.insert(pid) {
            break;
        }
        if printable_headings.contains(&pid) {
            return Some(pid);
        }
        cur = symbols.get(pid).and_then(|s| s.parent_id);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Symbol;
    use std::path::PathBuf;

    fn heading(name: &str, level: usize, line: usize) -> Symbol {
        Symbol {
            name: name.to_string(),
            symbol_type: SymbolType::Heading,
            signature: Some(format!("h{} (#)", level)),
            docstring: None,
            line_start: line,
            line_end: line,
            parent_id: None,
            file_path: PathBuf::from("x.md"),
            is_exported: false,
        }
    }

    #[test]
    fn test_heading_level_parse() {
        let s = heading("A", 2, 10);
        assert_eq!(heading_level(&s), Some(2));
    }

    #[test]
    fn test_compute_end_lines() {
        let syms = vec![
            heading("A", 1, 1),
            heading("A > B", 2, 3),
            heading("C", 1, 10),
        ];
        let headings: Vec<(usize, usize, usize)> = syms
            .iter()
            .enumerate()
            .map(|(i, s)| (i, heading_level(s).unwrap(), s.line_start))
            .collect();
        let end = compute_heading_end_lines(&headings, 20);
        assert_eq!(end.get(&0).copied(), Some(9));
        assert_eq!(end.get(&1).copied(), Some(9));
        assert_eq!(end.get(&2).copied(), Some(20));
    }

    #[test]
    fn test_select_section_heading_leaf_match() -> Result<()> {
        let syms = vec![heading("Orders", 1, 1), heading("Foo > Orders", 2, 10)];
        assert_eq!(select_section_heading(&syms, "Orders")?, 0);
        Ok(())
    }

    #[test]
    fn test_select_section_heading_ignores_decorative_punctuation() -> Result<()> {
        let syms = vec![
            heading("Root", 1, 1),
            heading("Root > 🗺️ Project Maps", 2, 10),
        ];

        assert_eq!(select_section_heading(&syms, "Project Maps")?, 1);
        Ok(())
    }
}

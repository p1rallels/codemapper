use anyhow::{anyhow, Result};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const SYMBOL_TAGS: &[&str] = &["f", "c", "m", "e", "s", "h", "cb", "ep", "if", "ty", "mod"];
const ENTRYPOINT_KINDS: &[&str] = &["main", "api", "unused"];

pub fn is_stdin_path(path: &Path) -> bool {
    path == Path::new("-")
}

pub fn read_symbol_arg(arg: String) -> Result<String> {
    if arg.trim() == "-" {
        let symbols = read_stdin_symbols()?;
        Ok(symbols.join("|"))
    } else {
        Ok(arg)
    }
}

pub fn read_path_arg(path: PathBuf) -> Result<Vec<PathBuf>> {
    if is_stdin_path(&path) {
        read_stdin_paths()
    } else {
        Ok(vec![path])
    }
}

fn read_stdin_symbols() -> Result<Vec<String>> {
    let symbols = extract_symbols(&read_stdin_text()?);
    if symbols.is_empty() {
        Err(anyhow!("stdin did not contain any cm symbols"))
    } else {
        Ok(symbols)
    }
}

fn read_stdin_paths() -> Result<Vec<PathBuf>> {
    let paths: Vec<PathBuf> = extract_paths(&read_stdin_text()?)
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if paths.is_empty() {
        Err(anyhow!("stdin did not contain any cm file paths"))
    } else {
        Ok(paths)
    }
}

fn read_stdin_text() -> Result<String> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    if input.trim().is_empty() {
        Err(anyhow!("stdin was empty"))
    } else {
        Ok(input)
    }
}

fn extract_symbols(input: &str) -> Vec<String> {
    unique(input.lines().filter_map(symbol_from_line))
}

fn extract_paths(input: &str) -> Vec<String> {
    unique(input.lines().filter_map(path_from_line))
}

fn symbol_from_line(line: &str) -> Option<String> {
    let trimmed = usable_line(line)?;
    let fields: Vec<&str> = trimmed.split('|').map(str::trim).collect();

    if fields.len() >= 4
        && ENTRYPOINT_KINDS.contains(&fields[0])
        && SYMBOL_TAGS.contains(&fields[2])
    {
        return non_empty(fields[1]);
    }

    if fields.len() >= 3 && SYMBOL_TAGS.contains(&fields[1]) {
        return non_empty(fields[0]);
    }

    if trimmed.contains('|') {
        return None;
    }

    non_empty(trimmed)
}

fn path_from_line(line: &str) -> Option<String> {
    let trimmed = usable_line(line)?;

    if let Some(file) = trimmed
        .strip_prefix("[FILE:")
        .and_then(|rest| rest.strip_suffix(']'))
    {
        return non_empty(file);
    }

    trimmed
        .split('|')
        .map(str::trim)
        .find_map(path_from_token)
        .or_else(|| path_from_token(trimmed))
}

fn path_from_token(token: &str) -> Option<String> {
    let candidate = strip_line_suffix(token.trim());
    let pathish = candidate.starts_with('/')
        || candidate.starts_with("./")
        || candidate.starts_with("../")
        || candidate.starts_with('~')
        || candidate.contains('/')
        || candidate.contains('\\');

    if pathish {
        non_empty(candidate)
    } else {
        None
    }
}

fn strip_line_suffix(token: &str) -> &str {
    if let Some((path, _line, _rest)) = split_rg_line(token) {
        return path;
    }

    if let Some((path, suffix)) = token.rsplit_once(':') {
        if suffix.chars().all(|c| c.is_ascii_digit() || c == '-') {
            return path;
        }
    }
    token
}

fn split_rg_line(token: &str) -> Option<(&str, &str, &str)> {
    let (path_and_line, suffix) = token.rsplit_once(':')?;
    let (path, line) = path_and_line.rsplit_once(':')?;
    if !path.is_empty() && line.chars().all(|c| c.is_ascii_digit()) {
        Some((path, line, suffix))
    } else {
        None
    }
}

fn usable_line(line: &str) -> Option<&str> {
    if line.starts_with(' ') || line.starts_with('\t') {
        return None;
    }

    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('[') && !trimmed.starts_with("[FILE:")
        || trimmed.starts_with('→')
        || trimmed.starts_with('✓')
        || trimmed.starts_with('✗')
    {
        None
    } else {
        Some(trimmed)
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn unique(values: impl Iterator<Item = String>) -> Vec<String> {
    values.fold(Vec::new(), |mut acc, value| {
        if !acc.iter().any(|seen| seen == &value) {
            acc.push(value);
        }
        acc
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_symbols_from_ai_query() {
        let input = "[RESULTS:2]\nCodeIndex|c|/repo/src/index.rs|7-13|exp\nquery_symbol|m|/repo/src/index.rs|145-174\n";
        assert_eq!(extract_symbols(input), vec!["CodeIndex", "query_symbol"]);
    }

    #[test]
    fn extracts_symbols_from_entrypoints() {
        let input = "[ENTRYPOINTS:2]\napi|CacheManager|c|/repo/src/cache.rs:81\nunused|IGNORED_DIRS|s|/repo/src/ignore.rs:1|sig:&[&str]\n";
        assert_eq!(extract_symbols(input), vec!["CacheManager", "IGNORED_DIRS"]);
    }

    #[test]
    fn extracts_paths_from_cm_outputs() {
        let input = "[FILE:/repo/src/main.rs]\nCodeIndex|c|/repo/src/index.rs|7-13|exp\napi|CacheManager|c|/repo/src/cache.rs:81\n/repo/src/output.rs|rust|105454\n";
        assert_eq!(
            extract_paths(input),
            vec![
                "/repo/src/main.rs",
                "/repo/src/index.rs",
                "/repo/src/cache.rs",
                "/repo/src/output.rs"
            ]
        );
    }

    #[test]
    fn extracts_paths_from_text_grep_lines() {
        let input = "src/main.rs:10:fn main() {}\n./src/lib.rs:20:mod tests;\n";
        assert_eq!(extract_paths(input), vec!["src/main.rs", "./src/lib.rs"]);
    }

    #[test]
    fn extracts_windows_paths_from_text_grep_lines() {
        let input = r"C:\repo\src\main.rs:10:fn main() {}";
        assert_eq!(extract_paths(input), vec![r"C:\repo\src\main.rs"]);
    }

    #[test]
    fn extracts_raw_symbol_lines() {
        assert_eq!(extract_symbols("foo\nbar\nfoo\n"), vec!["foo", "bar"]);
    }
}

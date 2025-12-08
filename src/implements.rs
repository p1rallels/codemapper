use anyhow::Result;
use regex::Regex;
use std::fs;
use std::path::PathBuf;

use crate::index::CodeIndex;
use crate::models::Language;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImplementsKind {
    Implements,
    Extends,
    Impl,
    Inherits,
}

impl ImplementsKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImplementsKind::Implements => "implements",
            ImplementsKind::Extends => "extends",
            ImplementsKind::Impl => "impl",
            ImplementsKind::Inherits => "inherits",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Implementation {
    pub implementor_name: String,
    pub interface_name: String,
    pub file_path: PathBuf,
    pub line: usize,
    pub kind: ImplementsKind,
    pub language: Language,
}

pub fn find_implementations(
    index: &CodeIndex,
    interface: &str,
    fuzzy: bool,
) -> Result<Vec<Implementation>> {
    let mut results = Vec::new();
    let interface_lower = interface.to_lowercase();

    for file in index.files() {
        let content = fs::read_to_string(&file.path).unwrap_or_default();
        if content.is_empty() {
            continue;
        }

        let file_impls = match file.language {
            Language::Rust => find_rust_implementations(&content, interface, fuzzy, &interface_lower),
            Language::Python => find_python_implementations(&content, interface, fuzzy, &interface_lower),
            Language::TypeScript | Language::JavaScript => {
                find_ts_implementations(&content, interface, fuzzy, &interface_lower)
            }
            Language::Java => find_java_implementations(&content, interface, fuzzy, &interface_lower),
            Language::Go => find_go_implementations(&content, interface, fuzzy, &interface_lower),
            _ => Vec::new(),
        };

        for (implementor, iface, line, kind) in file_impls {
            results.push(Implementation {
                implementor_name: implementor,
                interface_name: iface,
                file_path: file.path.clone(),
                line,
                kind,
                language: file.language,
            });
        }
    }

    results.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then_with(|| a.line.cmp(&b.line))
    });

    Ok(results)
}

fn matches_interface(name: &str, interface: &str, fuzzy: bool, interface_lower: &str) -> bool {
    if fuzzy {
        name.to_lowercase().contains(interface_lower)
    } else {
        name == interface
    }
}

fn find_rust_implementations(
    content: &str,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let mut results = Vec::new();

    // Pattern: impl Trait for Type
    let impl_for_re = Regex::new(r"impl\s+(?:<[^>]*>\s*)?(\w+)(?:<[^>]*>)?\s+for\s+(\w+)")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    // Pattern: impl Type (inherent impl - type implements its own methods)
    let impl_self_re = Regex::new(r"impl\s+(?:<[^>]*>\s*)?(\w+)(?:<[^>]*>)?\s*\{")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Check impl Trait for Type
        if let Some(caps) = impl_for_re.captures(trimmed) {
            let trait_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let type_name = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            if matches_interface(trait_name, interface, fuzzy, interface_lower) {
                results.push((
                    type_name.to_string(),
                    trait_name.to_string(),
                    line_num + 1,
                    ImplementsKind::Impl,
                ));
            }
        }

        // Check impl Type (searching for types that match the interface name)
        if let Some(caps) = impl_self_re.captures(trimmed) {
            if !trimmed.contains(" for ") {
                let type_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
                if matches_interface(type_name, interface, fuzzy, interface_lower) {
                    results.push((
                        type_name.to_string(),
                        type_name.to_string(),
                        line_num + 1,
                        ImplementsKind::Impl,
                    ));
                }
            }
        }
    }

    results
}

fn find_python_implementations(
    content: &str,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let mut results = Vec::new();

    // Pattern: class ClassName(ParentClass, AnotherParent):
    let class_re = Regex::new(r"class\s+(\w+)\s*\(([^)]+)\)\s*:")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        if let Some(caps) = class_re.captures(trimmed) {
            let class_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let parents_str = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            // Split parents by comma, handling potential generics
            for parent in parents_str.split(',') {
                let parent = parent.trim();
                // Remove generic parameters like [T] or typing stuff
                let parent_name = parent
                    .split('[')
                    .next()
                    .unwrap_or(parent)
                    .split('.')
                    .last()
                    .unwrap_or(parent)
                    .trim();

                if !parent_name.is_empty()
                    && matches_interface(parent_name, interface, fuzzy, interface_lower)
                {
                    results.push((
                        class_name.to_string(),
                        parent_name.to_string(),
                        line_num + 1,
                        ImplementsKind::Inherits,
                    ));
                }
            }
        }
    }

    results
}

fn find_ts_implementations(
    content: &str,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let mut results = Vec::new();

    // Pattern: class ClassName implements Interface1, Interface2
    let implements_re = Regex::new(r"class\s+(\w+)(?:<[^>]*>)?(?:\s+extends\s+\w+(?:<[^>]*>)?)?\s+implements\s+([^{]+)")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    // Pattern: class ClassName extends ParentClass
    let extends_re = Regex::new(r"class\s+(\w+)(?:<[^>]*>)?\s+extends\s+(\w+)")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Check implements
        if let Some(caps) = implements_re.captures(trimmed) {
            let class_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let interfaces_str = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            for iface in interfaces_str.split(',') {
                let iface = iface.trim().split('<').next().unwrap_or("").trim();
                if !iface.is_empty()
                    && matches_interface(iface, interface, fuzzy, interface_lower)
                {
                    results.push((
                        class_name.to_string(),
                        iface.to_string(),
                        line_num + 1,
                        ImplementsKind::Implements,
                    ));
                }
            }
        }

        // Check extends
        if let Some(caps) = extends_re.captures(trimmed) {
            let class_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let parent_name = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            if matches_interface(parent_name, interface, fuzzy, interface_lower) {
                results.push((
                    class_name.to_string(),
                    parent_name.to_string(),
                    line_num + 1,
                    ImplementsKind::Extends,
                ));
            }
        }
    }

    results
}

fn find_java_implementations(
    content: &str,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let mut results = Vec::new();

    // Pattern: class ClassName implements Interface1, Interface2
    let implements_re = Regex::new(r"class\s+(\w+)(?:<[^>]*>)?(?:\s+extends\s+\w+(?:<[^>]*>)?)?\s+implements\s+([^{]+)")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    // Pattern: class ClassName extends ParentClass
    let extends_re = Regex::new(r"class\s+(\w+)(?:<[^>]*>)?\s+extends\s+(\w+)")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    // Pattern: interface InterfaceName extends ParentInterface
    let interface_extends_re = Regex::new(r"interface\s+(\w+)(?:<[^>]*>)?\s+extends\s+([^{]+)")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Check class implements
        if let Some(caps) = implements_re.captures(trimmed) {
            let class_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let interfaces_str = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            for iface in interfaces_str.split(',') {
                let iface = iface.trim().split('<').next().unwrap_or("").trim();
                if !iface.is_empty()
                    && matches_interface(iface, interface, fuzzy, interface_lower)
                {
                    results.push((
                        class_name.to_string(),
                        iface.to_string(),
                        line_num + 1,
                        ImplementsKind::Implements,
                    ));
                }
            }
        }

        // Check class extends
        if let Some(caps) = extends_re.captures(trimmed) {
            let class_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let parent_name = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            if matches_interface(parent_name, interface, fuzzy, interface_lower) {
                results.push((
                    class_name.to_string(),
                    parent_name.to_string(),
                    line_num + 1,
                    ImplementsKind::Extends,
                ));
            }
        }

        // Check interface extends
        if let Some(caps) = interface_extends_re.captures(trimmed) {
            let iface_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            let parents_str = caps.get(2).map(|m| m.as_str()).unwrap_or_default();

            for parent in parents_str.split(',') {
                let parent = parent.trim().split('<').next().unwrap_or("").trim();
                if !parent.is_empty()
                    && matches_interface(parent, interface, fuzzy, interface_lower)
                {
                    results.push((
                        iface_name.to_string(),
                        parent.to_string(),
                        line_num + 1,
                        ImplementsKind::Extends,
                    ));
                }
            }
        }
    }

    results
}

fn find_go_implementations(
    content: &str,
    interface: &str,
    fuzzy: bool,
    interface_lower: &str,
) -> Vec<(String, String, usize, ImplementsKind)> {
    let mut results = Vec::new();

    // Go uses implicit interface implementation
    // We look for:
    // 1. type TypeName struct that embeds the interface
    // 2. func (receiver TypeName) MethodName patterns that match interface methods

    // Pattern: type Name struct { embedded Interface }
    let struct_embed_re = Regex::new(r"type\s+(\w+)\s+struct\s*\{")
        .unwrap_or_else(|_| Regex::new(r"^$").expect("fallback regex"));

    // We'll track struct definitions and look for embedded interfaces
    let mut current_struct: Option<(String, usize)> = None;
    let mut brace_depth = 0;

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();

        // Track struct definitions
        if let Some(caps) = struct_embed_re.captures(trimmed) {
            let struct_name = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
            current_struct = Some((struct_name.to_string(), line_num + 1));
            brace_depth = 1;
            continue;
        }

        // Track brace depth for struct scope
        if current_struct.is_some() {
            brace_depth += trimmed.matches('{').count() as i32;
            brace_depth -= trimmed.matches('}').count() as i32;

            if brace_depth <= 0 {
                current_struct = None;
                brace_depth = 0;
                continue;
            }

            // Look for embedded interface (just the interface name on its own line)
            let field = trimmed.split_whitespace().next().unwrap_or("");
            if matches_interface(field, interface, fuzzy, interface_lower) {
                if let Some((ref struct_name, struct_line)) = current_struct {
                    results.push((
                        struct_name.clone(),
                        field.to_string(),
                        struct_line,
                        ImplementsKind::Implements,
                    ));
                }
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rust_impl_for() {
        let content = r#"
impl Display for MyType {
    fn fmt(&self, f: &mut Formatter) -> Result {
    }
}
"#;
        let results = find_rust_implementations(content, "Display", false, "display");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "MyType");
        assert_eq!(results[0].1, "Display");
    }

    #[test]
    fn test_python_inheritance() {
        let content = r#"
class MyService(BaseService):
    pass
"#;
        let results = find_python_implementations(content, "BaseService", false, "baseservice");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "MyService");
    }

    #[test]
    fn test_ts_implements() {
        let content = r#"
class UserRepository implements Repository {
    async find(id: string) {}
}
"#;
        let results = find_ts_implementations(content, "Repository", false, "repository");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "UserRepository");
    }

    #[test]
    fn test_java_implements() {
        let content = r#"
public class ArrayList implements List, Serializable {
}
"#;
        let results = find_java_implementations(content, "List", false, "list");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "ArrayList");
    }

    #[test]
    fn test_fuzzy_search() {
        let content = r#"
impl Iterator for MyIterator {}
impl IntoIterator for MyCollection {}
"#;
        let results = find_rust_implementations(content, "iter", true, "iter");
        assert_eq!(results.len(), 2);
    }
}

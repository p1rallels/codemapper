use crate::index::CodeIndex;
use crate::models::{Language, Symbol, SymbolType};
use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};

/// What kind of type reference this is
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeKind {
    Parameter,
    Return,
    Field,
    Generic,
}

impl TypeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TypeKind::Parameter => "param",
            TypeKind::Return => "return",
            TypeKind::Field => "field",
            TypeKind::Generic => "generic",
        }
    }
}

/// Information about a single type reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeInfo {
    /// Name of the parameter/field (empty for return type)
    pub name: String,
    /// What kind of type reference (param, return, field, generic)
    pub kind: TypeKind,
    /// The actual type name (e.g., "String", "Vec<T>", "User")
    pub type_name: String,
    /// Where this type is defined (file:line), if found
    pub defined_in: Option<String>,
}

/// All type information for a symbol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolTypes {
    /// The symbol name
    pub symbol_name: String,
    /// Symbol type (function, method, class, etc.)
    pub symbol_type: SymbolType,
    /// File path where symbol is defined
    pub file_path: String,
    /// Line where symbol starts
    pub line: usize,
    /// Full signature if available
    pub signature: Option<String>,
    /// Parameter types
    pub params: Vec<TypeInfo>,
    /// Return type (if any)
    pub return_type: Option<TypeInfo>,
}

/// Analyze types for a symbol by name
pub fn analyze_types(
    index: &CodeIndex,
    symbol_name: &str,
    fuzzy: bool,
) -> Result<Vec<SymbolTypes>> {
    let symbols: Vec<&Symbol> = if fuzzy {
        index.fuzzy_search(symbol_name)
    } else {
        index.query_symbol(symbol_name)
    };

    let mut results = Vec::new();

    for symbol in symbols {
        // Skip non-callable symbols
        if matches!(
            symbol.symbol_type,
            SymbolType::Heading | SymbolType::CodeBlock
        ) {
            continue;
        }

        let signature = symbol.signature.as_deref().unwrap_or_default();
        let language = detect_language_from_path(&symbol.file_path.to_string_lossy());

        let (params, return_type) = parse_signature(signature, language);

        // Resolve type definitions
        let resolved_params = resolve_types(index, params);
        let resolved_return = return_type.map(|rt| resolve_type(index, rt));

        results.push(SymbolTypes {
            symbol_name: symbol.name.clone(),
            symbol_type: symbol.symbol_type,
            file_path: symbol.file_path.display().to_string(),
            line: symbol.line_start,
            signature: symbol.signature.clone(),
            params: resolved_params,
            return_type: resolved_return,
        });
    }

    Ok(results)
}

/// Detect language from file path extension
fn detect_language_from_path(path: &str) -> Language {
    let ext = path.rsplit('.').next().unwrap_or_default().to_lowercase();
    Language::from_extension(&ext)
}

/// Parse signature to extract parameter types and return type
fn parse_signature(signature: &str, language: Language) -> (Vec<TypeInfo>, Option<TypeInfo>) {
    match language {
        Language::Rust => parse_rust_signature(signature),
        Language::Python => parse_python_signature(signature),
        Language::TypeScript | Language::JavaScript => parse_typescript_signature(signature),
        Language::Go => parse_go_signature(signature),
        Language::Java => parse_java_signature(signature),
        Language::C => parse_c_signature(signature),
        _ => (Vec::new(), None),
    }
}

/// Parse Rust signature: `fn name(x: Type, y: Type) -> RetType`
fn parse_rust_signature(signature: &str) -> (Vec<TypeInfo>, Option<TypeInfo>) {
    let mut params = Vec::new();
    let mut return_type = None;

    // Extract return type: `-> Type` or `-> Result<Type, Error>`
    let return_re = Regex::new(r"->\s*(.+?)\s*(?:\{|$|\))")
        .unwrap_or_else(|_| Regex::new(r"$").unwrap_or_else(|_| panic!("Failed to compile regex")));

    if let Some(cap) = return_re.captures(signature) {
        let ret_type = cap.get(1).map(|m| m.as_str().trim()).unwrap_or_default();
        if !ret_type.is_empty() {
            return_type = Some(TypeInfo {
                name: String::new(),
                kind: TypeKind::Return,
                type_name: clean_type_name(ret_type),
                defined_in: None,
            });
        }
    }

    // Extract parameters: look for content between ( and )
    let param_re = Regex::new(r"\(([^)]*)\)")
        .unwrap_or_else(|_| Regex::new(r"$").unwrap_or_else(|_| panic!("Failed to compile regex")));

    if let Some(cap) = param_re.captures(signature) {
        let param_str = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        params = parse_rust_params(param_str);
    }

    (params, return_type)
}

/// Parse Rust parameters: `x: Type, y: Type`
fn parse_rust_params(param_str: &str) -> Vec<TypeInfo> {
    let mut params = Vec::new();

    // Split by comma, but handle nested generics
    let parts = split_by_comma_respecting_brackets(param_str);

    for part in parts {
        let part = part.trim();
        if part.is_empty() || part == "self" || part == "&self" || part == "&mut self" {
            continue;
        }

        // Match `name: Type` pattern
        if let Some(colon_pos) = part.find(':') {
            let name = part[..colon_pos].trim();
            let type_name = part[colon_pos + 1..].trim();

            // Skip lifetime-only parameters
            if !type_name.starts_with('\'') {
                params.push(TypeInfo {
                    name: name.to_string(),
                    kind: TypeKind::Parameter,
                    type_name: clean_type_name(type_name),
                    defined_in: None,
                });
            }
        }
    }

    params
}

/// Parse Python signature: `def name(x: Type, y: Type) -> RetType:`
fn parse_python_signature(signature: &str) -> (Vec<TypeInfo>, Option<TypeInfo>) {
    let mut params = Vec::new();
    let mut return_type = None;

    // Extract return type: `-> Type:`
    let return_re = Regex::new(r"->\s*([^:]+)")
        .unwrap_or_else(|_| Regex::new(r"$").unwrap_or_else(|_| panic!("Failed to compile regex")));

    if let Some(cap) = return_re.captures(signature) {
        let ret_type = cap.get(1).map(|m| m.as_str().trim()).unwrap_or_default();
        if !ret_type.is_empty() && ret_type != "None" {
            return_type = Some(TypeInfo {
                name: String::new(),
                kind: TypeKind::Return,
                type_name: clean_type_name(ret_type),
                defined_in: None,
            });
        }
    }

    // Extract parameters
    let param_re = Regex::new(r"\(([^)]*)\)")
        .unwrap_or_else(|_| Regex::new(r"$").unwrap_or_else(|_| panic!("Failed to compile regex")));

    if let Some(cap) = param_re.captures(signature) {
        let param_str = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        params = parse_python_params(param_str);
    }

    (params, return_type)
}

/// Parse Python parameters: `x: Type, y: Type = default`
fn parse_python_params(param_str: &str) -> Vec<TypeInfo> {
    let mut params = Vec::new();

    let parts = split_by_comma_respecting_brackets(param_str);

    for part in parts {
        let part = part.trim();
        if part.is_empty() || part == "self" || part == "cls" {
            continue;
        }

        // Remove default values: `x: Type = default` -> `x: Type`
        let part = part.split('=').next().unwrap_or(part).trim();

        // Match `name: Type` pattern
        if let Some(colon_pos) = part.find(':') {
            let name = part[..colon_pos].trim();
            let type_name = part[colon_pos + 1..].trim();

            if !type_name.is_empty() {
                params.push(TypeInfo {
                    name: name.to_string(),
                    kind: TypeKind::Parameter,
                    type_name: clean_type_name(type_name),
                    defined_in: None,
                });
            }
        }
    }

    params
}

/// Parse TypeScript signature: `function name(x: Type, y: Type): RetType`
fn parse_typescript_signature(signature: &str) -> (Vec<TypeInfo>, Option<TypeInfo>) {
    let mut params = Vec::new();
    let mut return_type = None;

    // Extract return type: `): Type {` or `): Type`
    let return_re = Regex::new(r"\)\s*:\s*([^{=]+)")
        .unwrap_or_else(|_| Regex::new(r"$").unwrap_or_else(|_| panic!("Failed to compile regex")));

    if let Some(cap) = return_re.captures(signature) {
        let ret_type = cap.get(1).map(|m| m.as_str().trim()).unwrap_or_default();
        if !ret_type.is_empty() && ret_type != "void" {
            return_type = Some(TypeInfo {
                name: String::new(),
                kind: TypeKind::Return,
                type_name: clean_type_name(ret_type),
                defined_in: None,
            });
        }
    }

    // Extract parameters
    let param_re = Regex::new(r"\(([^)]*)\)")
        .unwrap_or_else(|_| Regex::new(r"$").unwrap_or_else(|_| panic!("Failed to compile regex")));

    if let Some(cap) = param_re.captures(signature) {
        let param_str = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        params = parse_typescript_params(param_str);
    }

    (params, return_type)
}

/// Parse TypeScript parameters: `x: Type, y?: Type`
fn parse_typescript_params(param_str: &str) -> Vec<TypeInfo> {
    let mut params = Vec::new();

    let parts = split_by_comma_respecting_brackets(param_str);

    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        // Handle optional params: `x?: Type`
        let part = part.replace('?', "");

        // Remove default values
        let part = part.split('=').next().unwrap_or(&part).trim();

        // Match `name: Type` pattern
        if let Some(colon_pos) = part.find(':') {
            let name = part[..colon_pos].trim();
            let type_name = part[colon_pos + 1..].trim();

            if !type_name.is_empty() {
                params.push(TypeInfo {
                    name: name.to_string(),
                    kind: TypeKind::Parameter,
                    type_name: clean_type_name(type_name),
                    defined_in: None,
                });
            }
        }
    }

    params
}

/// Parse Go signature: `func name(x Type, y Type) RetType`
fn parse_go_signature(signature: &str) -> (Vec<TypeInfo>, Option<TypeInfo>) {
    let mut params = Vec::new();
    let mut return_type = None;

    // Extract return type: `) Type {` or `) (Type, error) {`
    let return_re = Regex::new(r"\)\s*([^{]+)")
        .unwrap_or_else(|_| Regex::new(r"$").unwrap_or_else(|_| panic!("Failed to compile regex")));

    if let Some(cap) = return_re.captures(signature) {
        let ret_type = cap.get(1).map(|m| m.as_str().trim()).unwrap_or_default();
        if !ret_type.is_empty() && ret_type != "error" {
            return_type = Some(TypeInfo {
                name: String::new(),
                kind: TypeKind::Return,
                type_name: clean_type_name(ret_type),
                defined_in: None,
            });
        }
    }

    // Extract parameters
    let param_re = Regex::new(r"\(([^)]*)\)")
        .unwrap_or_else(|_| Regex::new(r"$").unwrap_or_else(|_| panic!("Failed to compile regex")));

    if let Some(cap) = param_re.captures(signature) {
        let param_str = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        params = parse_go_params(param_str);
    }

    (params, return_type)
}

/// Parse Go parameters: `x Type, y Type` or `x, y Type`
fn parse_go_params(param_str: &str) -> Vec<TypeInfo> {
    let mut params = Vec::new();

    let parts = split_by_comma_respecting_brackets(param_str);

    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        // Go syntax: `name Type` (space-separated)
        let tokens: Vec<&str> = part.split_whitespace().collect();
        if tokens.len() >= 2 {
            let name = tokens[0];
            let type_name = tokens[1..].join(" ");

            params.push(TypeInfo {
                name: name.to_string(),
                kind: TypeKind::Parameter,
                type_name: clean_type_name(&type_name),
                defined_in: None,
            });
        }
    }

    params
}

/// Parse Java signature: `RetType name(Type x, Type y)`
fn parse_java_signature(signature: &str) -> (Vec<TypeInfo>, Option<TypeInfo>) {
    let mut params = Vec::new();
    let mut return_type = None;

    // Java: return type is before function name
    // Pattern: `public? static? RetType name(`
    let return_re = Regex::new(r"(?:public\s+)?(?:private\s+)?(?:protected\s+)?(?:static\s+)?(?:final\s+)?(\w+(?:<[^>]+>)?)\s+\w+\s*\(")
        .unwrap_or_else(|_| Regex::new(r"$").unwrap_or_else(|_| panic!("Failed to compile regex")));

    if let Some(cap) = return_re.captures(signature) {
        let ret_type = cap.get(1).map(|m| m.as_str().trim()).unwrap_or_default();
        if !ret_type.is_empty() && ret_type != "void" {
            return_type = Some(TypeInfo {
                name: String::new(),
                kind: TypeKind::Return,
                type_name: clean_type_name(ret_type),
                defined_in: None,
            });
        }
    }

    // Extract parameters
    let param_re = Regex::new(r"\(([^)]*)\)")
        .unwrap_or_else(|_| Regex::new(r"$").unwrap_or_else(|_| panic!("Failed to compile regex")));

    if let Some(cap) = param_re.captures(signature) {
        let param_str = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        params = parse_java_params(param_str);
    }

    (params, return_type)
}

/// Parse Java parameters: `Type x, Type y`
fn parse_java_params(param_str: &str) -> Vec<TypeInfo> {
    let mut params = Vec::new();

    let parts = split_by_comma_respecting_brackets(param_str);

    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        // Remove annotations like @NotNull
        let part = Regex::new(r"@\w+\s*")
            .map(|re| re.replace_all(part, ""))
            .unwrap_or_else(|_| part.into());
        let part = part.trim();

        // Java syntax: `Type name` or `final Type name`
        let part = part.strip_prefix("final").unwrap_or(part).trim();

        let tokens: Vec<&str> = part.split_whitespace().collect();
        if tokens.len() >= 2 {
            let type_name = tokens[..tokens.len() - 1].join(" ");
            let name = tokens[tokens.len() - 1];

            params.push(TypeInfo {
                name: name.to_string(),
                kind: TypeKind::Parameter,
                type_name: clean_type_name(&type_name),
                defined_in: None,
            });
        }
    }

    params
}

/// Parse C signature: `RetType name(Type x, Type y)`
fn parse_c_signature(signature: &str) -> (Vec<TypeInfo>, Option<TypeInfo>) {
    let mut params = Vec::new();
    let mut return_type = None;

    // C: return type is before function name
    let return_re =
        Regex::new(r"^\s*(?:static\s+)?(?:inline\s+)?(?:const\s+)?(\w+\s*\*?)\s+\w+\s*\(")
            .unwrap_or_else(|_| {
                Regex::new(r"$").unwrap_or_else(|_| panic!("Failed to compile regex"))
            });

    if let Some(cap) = return_re.captures(signature) {
        let ret_type = cap.get(1).map(|m| m.as_str().trim()).unwrap_or_default();
        if !ret_type.is_empty() && ret_type != "void" {
            return_type = Some(TypeInfo {
                name: String::new(),
                kind: TypeKind::Return,
                type_name: clean_type_name(ret_type),
                defined_in: None,
            });
        }
    }

    // Extract parameters
    let param_re = Regex::new(r"\(([^)]*)\)")
        .unwrap_or_else(|_| Regex::new(r"$").unwrap_or_else(|_| panic!("Failed to compile regex")));

    if let Some(cap) = param_re.captures(signature) {
        let param_str = cap.get(1).map(|m| m.as_str()).unwrap_or_default();
        params = parse_c_params(param_str);
    }

    (params, return_type)
}

/// Parse C parameters: `Type x, Type y`
fn parse_c_params(param_str: &str) -> Vec<TypeInfo> {
    let mut params = Vec::new();

    let parts = split_by_comma_respecting_brackets(param_str);

    for part in parts {
        let part = part.trim();
        if part.is_empty() || part == "void" {
            continue;
        }

        // C syntax: `Type name` or `const Type *name`
        let tokens: Vec<&str> = part.split_whitespace().collect();
        if tokens.len() >= 2 {
            let name = tokens[tokens.len() - 1].trim_start_matches('*');
            let type_name = tokens[..tokens.len() - 1].join(" ");

            // Check if name has pointer
            let type_with_ptr = if tokens[tokens.len() - 1].starts_with('*') {
                format!("{}*", type_name)
            } else {
                type_name
            };

            params.push(TypeInfo {
                name: name.to_string(),
                kind: TypeKind::Parameter,
                type_name: clean_type_name(&type_with_ptr),
                defined_in: None,
            });
        }
    }

    params
}

/// Split string by comma, respecting angle brackets and parentheses
fn split_by_comma_respecting_brackets(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth: usize = 0;

    for c in s.chars() {
        match c {
            '<' | '(' | '[' | '{' => {
                depth += 1;
                current.push(c);
            }
            '>' | ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if depth == 0 => {
                parts.push(current.trim().to_string());
                current = String::new();
            }
            _ => {
                current.push(c);
            }
        }
    }

    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }

    parts
}

/// Clean up type name (remove leading &, mut, etc. for base type lookup)
fn clean_type_name(type_name: &str) -> String {
    type_name
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim()
        .to_string()
}

/// Extract the base type name for lookup (e.g., "Vec<User>" -> "User", "Option<String>" -> "String")
fn extract_base_types(type_name: &str) -> Vec<String> {
    let mut types = Vec::new();

    // Get the main type
    let main_type = type_name
        .split('<')
        .next()
        .unwrap_or(type_name)
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim_end_matches('*')
        .to_string();

    if !main_type.is_empty() && !is_primitive_type(&main_type) {
        types.push(main_type);
    }

    // Extract generic parameters
    if let Some(start) = type_name.find('<') {
        if let Some(end) = type_name.rfind('>') {
            let generics = &type_name[start + 1..end];
            let generic_parts = split_by_comma_respecting_brackets(generics);
            for part in generic_parts {
                let part = part
                    .trim()
                    .trim_start_matches('&')
                    .trim_start_matches("mut ");
                if !part.is_empty() && !is_primitive_type(part) {
                    types.push(part.to_string());
                }
            }
        }
    }

    types
}

/// Check if a type is a primitive/built-in type
fn is_primitive_type(type_name: &str) -> bool {
    let primitives = [
        // Rust
        "i8",
        "i16",
        "i32",
        "i64",
        "i128",
        "isize",
        "u8",
        "u16",
        "u32",
        "u64",
        "u128",
        "usize",
        "f32",
        "f64",
        "bool",
        "char",
        "str",
        "String",
        "Vec",
        "Option",
        "Result",
        "Box",
        "Rc",
        "Arc",
        "HashMap",
        "HashSet",
        "BTreeMap",
        "BTreeSet",
        // Python
        "int",
        "float",
        "str",
        "bool",
        "list",
        "dict",
        "set",
        "tuple",
        "List",
        "Dict",
        "Set",
        "Tuple",
        "Optional",
        "Union",
        "Any",
        // TypeScript/JavaScript
        "string",
        "number",
        "boolean",
        "void",
        "undefined",
        "null",
        "Array",
        "object",
        "Object",
        "Promise",
        "Map",
        "Set",
        // Go
        "error",
        "byte",
        "rune",
        // Java
        "byte",
        "short",
        "int",
        "long",
        "float",
        "double",
        "boolean",
        "char",
        "Byte",
        "Short",
        "Integer",
        "Long",
        "Float",
        "Double",
        "Boolean",
        "Character",
        // C
        "void",
        "int",
        "long",
        "short",
        "char",
        "float",
        "double",
        "unsigned",
        "signed",
    ];

    primitives.contains(&type_name)
}

/// Resolve types by searching the index for their definitions
fn resolve_types(index: &CodeIndex, types: Vec<TypeInfo>) -> Vec<TypeInfo> {
    types.into_iter().map(|t| resolve_type(index, t)).collect()
}

/// Resolve a single type by searching for its definition
fn resolve_type(index: &CodeIndex, mut type_info: TypeInfo) -> TypeInfo {
    let base_types = extract_base_types(&type_info.type_name);

    for base_type in base_types {
        // Search for class/struct/type definition
        let symbols = index.query_symbol(&base_type);

        for symbol in symbols {
            // Look for class, struct (Class type), or enum definitions
            if matches!(symbol.symbol_type, SymbolType::Class | SymbolType::Enum) {
                type_info.defined_in = Some(format!(
                    "{}:{}",
                    symbol.file_path.display(),
                    symbol.line_start
                ));
                return type_info;
            }
        }

        // Try fuzzy search if exact match fails
        let fuzzy_symbols = index.fuzzy_search(&base_type);
        for symbol in fuzzy_symbols {
            if matches!(symbol.symbol_type, SymbolType::Class | SymbolType::Enum) {
                if symbol.name.to_lowercase() == base_type.to_lowercase() {
                    type_info.defined_in = Some(format!(
                        "{}:{}",
                        symbol.file_path.display(),
                        symbol.line_start
                    ));
                    return type_info;
                }
            }
        }
    }

    type_info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rust_signature() {
        let sig = "fn process(user: User, count: i32) -> Result<String, Error>";
        let (params, ret) = parse_rust_signature(sig);

        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "user");
        assert_eq!(params[0].type_name, "User");
        assert_eq!(params[1].name, "count");
        assert_eq!(params[1].type_name, "i32");

        assert!(ret.is_some());
        assert_eq!(
            ret.as_ref().map(|r| r.type_name.as_str()),
            Some("Result<String, Error>")
        );
    }

    #[test]
    fn test_parse_python_signature() {
        let sig = "def process(user: User, count: int = 0) -> Optional[str]:";
        let (params, ret) = parse_python_signature(sig);

        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "user");
        assert_eq!(params[0].type_name, "User");
        assert_eq!(params[1].name, "count");
        assert_eq!(params[1].type_name, "int");

        assert!(ret.is_some());
        assert_eq!(
            ret.as_ref().map(|r| r.type_name.as_str()),
            Some("Optional[str]")
        );
    }

    #[test]
    fn test_parse_typescript_signature() {
        let sig = "function process(user: User, count?: number): Promise<string>";
        let (params, ret) = parse_typescript_signature(sig);

        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "user");
        assert_eq!(params[0].type_name, "User");
        assert_eq!(params[1].name, "count");
        assert_eq!(params[1].type_name, "number");

        assert!(ret.is_some());
        assert_eq!(
            ret.as_ref().map(|r| r.type_name.as_str()),
            Some("Promise<string>")
        );
    }

    #[test]
    fn test_extract_base_types() {
        assert_eq!(extract_base_types("Vec<User>"), vec!["User"]);
        assert_eq!(extract_base_types("Option<String>"), vec![] as Vec<String>);
        assert_eq!(extract_base_types("HashMap<String, User>"), vec!["User"]);
        assert_eq!(extract_base_types("User"), vec!["User"]);
    }

    #[test]
    fn test_split_by_comma_respecting_brackets() {
        let input = "a: Vec<String, u32>, b: HashMap<K, V>";
        let parts = split_by_comma_respecting_brackets(input);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], "a: Vec<String, u32>");
        assert_eq!(parts[1], "b: HashMap<K, V>");
    }
}

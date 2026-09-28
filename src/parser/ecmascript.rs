use std::path::Path;

/// Keep symbol queries and call extraction on the same ECMAScript dialect.
#[derive(Clone, Copy)]
pub(crate) enum Dialect {
    JavaScript,
    TypeScript,
    Tsx,
}

impl Dialect {
    pub(crate) fn typescript(path: &Path) -> Self {
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("tsx"))
        {
            Self::Tsx
        } else {
            Self::TypeScript
        }
    }

    pub(crate) fn language(self) -> tree_sitter::Language {
        match self {
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }
}

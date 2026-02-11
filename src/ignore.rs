pub const IGNORED_DIRS: &[&str] = &[
    ".codemapper",
    ".git",
    "node_modules",
    "__pycache__",
    ".venv",
    ".build",
    "DerivedData",
    "target",
    "dist",
    "build",
];

pub fn is_ignored_dir(name: &str) -> bool {
    IGNORED_DIRS.iter().any(|d| *d == name)
}

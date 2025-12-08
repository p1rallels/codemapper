use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn is_git_repo(path: &Path) -> bool {
    let output = Command::new("git")
        .args(["-C", path.to_string_lossy().as_ref(), "rev-parse", "--git-dir"])
        .output();
    
    match output {
        Ok(o) => o.status.success(),
        Err(_) => false,
    }
}

pub fn get_repo_root(path: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["-C", path.to_string_lossy().as_ref(), "rev-parse", "--show-toplevel"])
        .output()
        .context("Failed to execute git command")?;
    
    if !output.status.success() {
        anyhow::bail!("Not a git repository: {}", path.display());
    }
    
    let root = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();
    
    Ok(PathBuf::from(root))
}

pub fn resolve_commit(path: &Path, commit_ref: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["-C", path.to_string_lossy().as_ref(), "rev-parse", "--verify", commit_ref])
        .output()
        .context("Failed to execute git rev-parse")?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Invalid commit reference '{}': {}", commit_ref, stderr.trim());
    }
    
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn get_file_at_commit(repo_path: &Path, file_path: &Path, commit: &str) -> Result<Option<String>> {
    let repo_root = get_repo_root(repo_path)?;
    
    let relative_path = if file_path.is_absolute() {
        file_path.strip_prefix(&repo_root)
            .unwrap_or(file_path)
    } else {
        file_path
    };
    
    let git_path = format!("{}:{}", commit, relative_path.display());
    
    let output = Command::new("git")
        .args(["-C", repo_root.to_string_lossy().as_ref(), "show", &git_path])
        .output()
        .context("Failed to execute git show")?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("does not exist") || stderr.contains("exists on disk, but not in") {
            return Ok(None);
        }
        anyhow::bail!("git show failed: {}", stderr.trim());
    }
    
    Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()))
}

pub fn list_files_at_commit(repo_path: &Path, commit: &str, subpath: Option<&Path>) -> Result<Vec<PathBuf>> {
    let repo_root = get_repo_root(repo_path)?;
    
    let mut args = vec![
        "-C".to_string(),
        repo_root.to_string_lossy().to_string(),
        "ls-tree".to_string(),
        "-r".to_string(),
        "--name-only".to_string(),
        commit.to_string(),
    ];
    
    if let Some(sp) = subpath {
        let relative = if sp.is_absolute() {
            sp.strip_prefix(&repo_root).unwrap_or(sp)
        } else {
            sp
        };
        if relative != Path::new(".") && relative != Path::new("") {
            args.push("--".to_string());
            args.push(relative.to_string_lossy().to_string());
        }
    }
    
    let output = Command::new("git")
        .args(&args)
        .output()
        .context("Failed to execute git ls-tree")?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git ls-tree failed: {}", stderr.trim());
    }
    
    let files: Vec<PathBuf> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| repo_root.join(line))
        .collect();
    
    Ok(files)
}

pub fn get_changed_files(repo_path: &Path, commit: &str, subpath: Option<&Path>) -> Result<ChangedFiles> {
    let repo_root = get_repo_root(repo_path)?;
    
    let mut args = vec![
        "-C".to_string(),
        repo_root.to_string_lossy().to_string(),
        "diff".to_string(),
        "--name-status".to_string(),
        commit.to_string(),
        "HEAD".to_string(),
    ];
    
    if let Some(sp) = subpath {
        let relative = if sp.is_absolute() {
            sp.strip_prefix(&repo_root).unwrap_or(sp)
        } else {
            sp
        };
        if relative != Path::new(".") && relative != Path::new("") {
            args.push("--".to_string());
            args.push(relative.to_string_lossy().to_string());
        }
    }
    
    let output = Command::new("git")
        .args(&args)
        .output()
        .context("Failed to execute git diff")?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff failed: {}", stderr.trim());
    }
    
    let mut changed = ChangedFiles {
        added: Vec::new(),
        deleted: Vec::new(),
        modified: Vec::new(),
    };
    
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() != 2 {
            continue;
        }
        
        let status = parts[0];
        let file_path = repo_root.join(parts[1]);
        
        match status.chars().next() {
            Some('A') => changed.added.push(file_path),
            Some('D') => changed.deleted.push(file_path),
            Some('M') => changed.modified.push(file_path),
            Some('R') => changed.modified.push(file_path),
            _ => {}
        }
    }
    
    Ok(changed)
}

#[derive(Debug, Clone)]
pub struct ChangedFiles {
    pub added: Vec<PathBuf>,
    pub deleted: Vec<PathBuf>,
    pub modified: Vec<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_is_git_repo() {
        let current_dir = std::env::current_dir().unwrap_or_default();
        assert!(is_git_repo(&current_dir));
    }
}

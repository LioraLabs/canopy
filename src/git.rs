use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Find the git root directory from the current or given path
pub fn find_root(from: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(from)
        .output()
        .context("Failed to run git")?;
    if !output.status.success() {
        anyhow::bail!("Not a git repository: {}", from.display());
    }
    let root = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(PathBuf::from(root))
}

/// Get the current HEAD SHA
pub fn head_sha(repo: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .context("Failed to get HEAD SHA")?;
    if !output.status.success() {
        anyhow::bail!("Failed to get HEAD SHA");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Get the current branch name
pub fn current_branch(repo: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo)
        .output()
        .context("Failed to get current branch")?;
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

#[derive(Debug)]
pub enum FileChange {
    Added(String),
    Modified(String),
    Deleted(String),
}

/// Get list of changed files between two SHAs
pub fn diff_files(repo: &Path, from_sha: &str, to_sha: &str) -> Result<Vec<FileChange>> {
    let output = Command::new("git")
        .args(["diff", "--name-status", from_sha, to_sha])
        .current_dir(repo)
        .output()
        .context("Failed to run git diff")?;
    if !output.status.success() {
        anyhow::bail!("git diff failed");
    }
    let text = String::from_utf8(output.stdout)?;
    let mut changes = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() != 2 {
            continue;
        }
        let change = match parts[0] {
            "A" => FileChange::Added(parts[1].to_string()),
            "M" => FileChange::Modified(parts[1].to_string()),
            "D" => FileChange::Deleted(parts[1].to_string()),
            s if s.starts_with('R') => {
                // Rename: treat as delete old + add new
                let names: Vec<&str> = parts[1].splitn(2, '\t').collect();
                if names.len() == 2 {
                    changes.push(FileChange::Deleted(names[0].to_string()));
                    FileChange::Added(names[1].to_string())
                } else {
                    FileChange::Modified(parts[1].to_string())
                }
            }
            _ => FileChange::Modified(parts[1].to_string()),
        };
        changes.push(change);
    }
    Ok(changes)
}

/// List all tracked files in the repo (for full reindex)
pub fn all_tracked_files(repo: &Path) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(["ls-files"])
        .current_dir(repo)
        .output()
        .context("Failed to list tracked files")?;
    let text = String::from_utf8(output.stdout)?;
    Ok(text.lines().map(|l| l.to_string()).collect())
}

const HOOK_MARKER: &str = "# canopy-hook";

/// Install post-commit and post-merge hooks
pub fn install_hooks(repo: &Path) -> Result<()> {
    let hooks_dir = repo.join(".git/hooks");
    std::fs::create_dir_all(&hooks_dir)?;

    let canopy_block = format!(
        r#"
{HOOK_MARKER}
if [ "$(git rev-parse --abbrev-ref HEAD)" = "main" ] || [ "$(git rev-parse --abbrev-ref HEAD)" = "master" ]; then
    canopy index >/dev/null 2>&1 &
fi
"#
    );

    for hook_name in ["post-commit", "post-merge"] {
        let hook_path = hooks_dir.join(hook_name);
        let existing = std::fs::read_to_string(&hook_path).unwrap_or_default();

        if existing.contains(HOOK_MARKER) {
            continue; // Already installed
        }

        let content = if existing.is_empty() {
            format!("#!/bin/sh\n{canopy_block}")
        } else {
            format!("{existing}\n{canopy_block}")
        };

        std::fs::write(&hook_path, content)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))?;
        }
    }

    Ok(())
}

/// Remove canopy hook blocks from post-commit and post-merge hooks
pub fn uninstall_hooks(repo: &Path) -> Result<()> {
    let hooks_dir = repo.join(".git/hooks");

    for hook_name in ["post-commit", "post-merge"] {
        let hook_path = hooks_dir.join(hook_name);
        let existing = std::fs::read_to_string(&hook_path).unwrap_or_default();

        if !existing.contains(HOOK_MARKER) {
            continue;
        }

        // Remove the canopy block: from HOOK_MARKER line to the next blank line or EOF
        let mut lines: Vec<&str> = existing.lines().collect();
        let mut i = 0;
        while i < lines.len() {
            if lines[i].contains(HOOK_MARKER) {
                let start = if i > 0 && lines[i - 1].is_empty() { i - 1 } else { i };
                let mut end = i + 1;
                while end < lines.len() && !lines[end].is_empty() {
                    end += 1;
                }
                lines.drain(start..end);
                i = start;
            } else {
                i += 1;
            }
        }

        let content = lines.join("\n");
        if content.trim() == "#!/bin/sh" || content.trim().is_empty() {
            // Hook file is now empty — remove it
            let _ = std::fs::remove_file(&hook_path);
        } else {
            std::fs::write(&hook_path, content)?;
        }
    }

    Ok(())
}

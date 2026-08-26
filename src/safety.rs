use std::path::{Path, PathBuf};
use crate::bindings::exports::carapace::plugin::tool::ToolContext;
use crate::config::PluginConfig;

pub struct SafetyChecker;

impl SafetyChecker {
    /// Resolve and validate repository path against allowed roots.
    pub fn resolve_repo_path(
        repo_path: Option<&str>,
        config: &PluginConfig,
    ) -> Result<PathBuf, String> {
        let raw_path = match repo_path {
            Some(p) if !p.trim().is_empty() => p.trim(),
            _ => {
                return Err(
                    "Missing required parameter: 'repo_path' (repository path must be explicitly provided)"
                        .to_string(),
                );
            }
        };

        let path = PathBuf::from(raw_path);
        let base_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let abs_path = if path.is_relative() {
            base_dir.join(&path)
        } else {
            path
        };

        // Canonicalize if path exists; if not, canonicalize closest existing ancestor
        let resolved = if let Ok(canonical) = abs_path.canonicalize() {
            canonical
        } else {
            // Find closest existing ancestor to resolve any symlinks along the path
            let mut current = abs_path.as_path();
            let mut uncreated = Vec::new();
            while !current.exists() && current.parent().is_some() {
                if let Some(file_name) = current.file_name() {
                    uncreated.push(file_name);
                }
                current = current.parent().unwrap();
            }

            let mut resolved_base = current.canonicalize().unwrap_or_else(|_| normalize_path(current));
            for segment in uncreated.into_iter().rev() {
                resolved_base.push(segment);
            }
            normalize_path(&resolved_base)
        };

        // If allowed_roots is specified, enforce that resolved path starts with an allowed root
        if !config.allowed_roots.is_empty() {
            let mut is_allowed = false;
            for root_str in &config.allowed_roots {
                let root_path = PathBuf::from(root_str);
                let abs_root = if root_path.is_relative() {
                    base_dir.join(&root_path)
                } else {
                    root_path
                };
                let canonical_root = abs_root.canonicalize().unwrap_or_else(|_| normalize_path(&abs_root));
                if resolved.starts_with(&canonical_root) {
                    is_allowed = true;
                    break;
                }
            }
            if !is_allowed {
                return Err(format!(
                    "Access denied: repository path '{}' is outside configured allowed roots: {:?}",
                    resolved.display(),
                    config.allowed_roots
                ));
            }
        }

        Ok(resolved)
    }

    /// Check if target branch name is in the configured protected branches list
    pub fn is_branch_protected(branch_name: &str, config: &PluginConfig) -> bool {
        let normalized = Self::normalize_branch_name(branch_name);
        config
            .protected_branches
            .iter()
            .any(|b| b.trim().eq_ignore_ascii_case(&normalized))
    }

    /// Normalize branch name by removing prefixes like refs/heads/, refs/remotes/<remote>/, etc.
    pub fn normalize_branch_name(branch_name: &str) -> String {
        let trimmed = branch_name.trim();
        if let Some(rest) = trimmed.strip_prefix("refs/heads/") {
            return rest.to_string();
        }
        if let Some(rest) = trimmed.strip_prefix("heads/") {
            return rest.to_string();
        }
        if let Some(rest) = trimmed.strip_prefix("refs/remotes/") {
            if let Some((_, b)) = rest.split_once('/') {
                return b.to_string();
            }
            return rest.to_string();
        }
        if let Some(rest) = trimmed.strip_prefix("remotes/") {
            if let Some((_, b)) = rest.split_once('/') {
                return b.to_string();
            }
            return rest.to_string();
        }
        if let Some(rest) = trimmed.strip_prefix("origin/") {
            return rest.to_string();
        }
        trimmed.to_string()
    }

    /// Check if target branch is protected and whether the destructive action is allowed
    pub fn check_branch_protection(
        branch_name: &str,
        force: bool,
        config: &PluginConfig,
        ctx: &ToolContext,
    ) -> Result<(), String> {
        let normalized = Self::normalize_branch_name(branch_name);

        if Self::is_branch_protected(branch_name, config) {
            if ctx.sandboxed {
                return Err(format!(
                    "Security violation: protected branch '{}' cannot be modified in sandboxed agent mode.",
                    normalized
                ));
            }
            if !force {
                return Err(format!(
                    "Protected branch safeguard: '{}' is a protected branch. Modifying/force-updating requires explicit force: true parameter.",
                    normalized
                ));
            }
        }

        Ok(())
    }

    /// Verify whether a destructive action is permitted in the current invocation context
    pub fn verify_destructive_allowed(action_desc: &str, ctx: &ToolContext) -> Result<(), String> {
        if ctx.sandboxed {
            return Err(format!(
                "Permission denied: destructive operation '{}' is not permitted for sandboxed agents.",
                action_desc
            ));
        }
        Ok(())
    }
}

/// Normalizes a path by removing '.' and resolving '..' without filesystem access
pub fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut components = Vec::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                match components.last() {
                    Some(Component::Normal(_)) => {
                        components.pop();
                    }
                    Some(Component::RootDir) => {
                        // In POSIX root, /.. stays /
                    }
                    _ => {
                        components.push(component);
                    }
                }
            }
            _ => components.push(component),
        }
    }

    if components.is_empty() {
        PathBuf::from(".")
    } else {
        components.into_iter().collect()
    }
}

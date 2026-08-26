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
            _ => &config.default_repo_path,
        };

        let path = PathBuf::from(raw_path);
        
        // Canonicalize if the path exists, otherwise normalize
        let resolved = if let Ok(canonical) = path.canonicalize() {
            canonical
        } else {
            normalize_path(&path)
        };

        // If allowed_roots is specified, enforce that resolved path starts with an allowed root
        if !config.allowed_roots.is_empty() {
            let mut is_allowed = false;
            for root_str in &config.allowed_roots {
                let root_path = PathBuf::from(root_str);
                let canonical_root = root_path.canonicalize().unwrap_or_else(|_| normalize_path(&root_path));
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

    /// Check if target branch is protected and whether the destructive action is allowed
    pub fn check_branch_protection(
        branch_name: &str,
        force: bool,
        config: &PluginConfig,
        ctx: &ToolContext,
    ) -> Result<(), String> {
        let normalized = branch_name.trim_start_matches("refs/heads/").trim();
        let is_protected = config
            .protected_branches
            .iter()
            .any(|b| b.eq_ignore_ascii_case(normalized));

        if is_protected {
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
                if let Some(Component::Normal(_)) = components.last() {
                    components.pop();
                } else {
                    components.push(component);
                }
            }
            _ => components.push(component),
        }
    }

    components.into_iter().collect()
}

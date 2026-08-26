use serde::Deserialize;
use serde_json::json;
use crate::bindings::exports::carapace::plugin::tool::{ToolContext, ToolDefinition};
use crate::config::PluginConfig;

use crate::engine::GitEngine;
use crate::safety::SafetyChecker;
use crate::types::GitToolResult;

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "git_branch".to_string(),
            description: "List, create, delete, or rename branches."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "action": {
                        "type": "string",
                        "enum": ["list", "create", "delete", "rename"],
                        "description": "Branch action to perform: 'list', 'create', 'delete', or 'rename'."
                    },
                    "branch_name": {
                        "type": "string",
                        "description": "Name of the branch (required for create, delete, and rename)."
                    },
                    "new_name": {
                        "type": "string",
                        "description": "New branch name (required for rename action)."
                    },
                    "start_point": {
                        "type": "string",
                        "description": "Starting commit/branch for new branch (defaults to HEAD)."
                    },
                    "force": {
                        "type": "boolean",
                        "description": "Force branch creation or deletion (defaults to false)."
                    }
                },
                "required": ["repo_path", "action"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_checkout".to_string(),
            description: "Switch branches or create and checkout a new branch."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "branch_name": {
                        "type": "string",
                        "description": "Name of the branch to switch to or create."
                    },
                    "create_new": {
                        "type": "boolean",
                        "description": "If true, create and checkout new branch (-b behavior)."
                    },
                    "start_point": {
                        "type": "string",
                        "description": "Starting commit/branch if creating new branch (defaults to HEAD)."
                    }
                },
                "required": ["repo_path", "branch_name"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_merge".to_string(),
            description: "Join two or more development histories together into current HEAD."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "source_ref": {
                        "type": "string",
                        "description": "Branch name or commit to merge into current HEAD."
                    },
                    "message": {
                        "type": "string",
                        "description": "Optional custom merge commit message."
                    },
                    "no_ff": {
                        "type": "boolean",
                        "description": "Create a merge commit even when the merge resolves as a fast-forward."
                    },
                    "squash": {
                        "type": "boolean",
                        "description": "Produce working tree and index changes without creating a merge commit."
                    }
                },
                "required": ["repo_path", "source_ref"],
                "additionalProperties": false
            })
            .to_string(),
        },
    ]
}

#[derive(Deserialize)]
struct BranchParams {
    repo_path: Option<String>,
    action: String,
    branch_name: Option<String>,
    new_name: Option<String>,
    start_point: Option<String>,
    force: Option<bool>,
}

#[derive(Deserialize)]
struct CheckoutParams {
    repo_path: Option<String>,
    branch_name: String,
    create_new: Option<bool>,
    start_point: Option<String>,
}

#[derive(Deserialize)]
struct MergeParams {
    repo_path: Option<String>,
    source_ref: String,
    message: Option<String>,
    no_ff: Option<bool>,
    squash: Option<bool>,
}

pub fn handle(name: &str, params_json: &str, config: &PluginConfig, ctx: &ToolContext) -> GitToolResult {
    let raw_json = if params_json.trim().is_empty() { "{}" } else { params_json };
    match name {
        "git_branch" => {
            let params: BranchParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_branch: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };

            let force = params.force.unwrap_or(false);
            if params.action == "delete" {
                if let Err(e) = SafetyChecker::verify_destructive_allowed("git_branch (action: delete)", ctx) {
                    return GitToolResult::err(e);
                }
                if let Some(ref b) = params.branch_name {
                    if let Err(e) = SafetyChecker::check_branch_protection(b, force, config, ctx) {
                        return GitToolResult::err(e);
                    }
                }
            } else if params.action == "rename" {
                if let Some(ref b) = params.branch_name {
                    if SafetyChecker::is_branch_protected(b, config) {
                        if let Err(e) = SafetyChecker::verify_destructive_allowed("git_branch (rename protected branch)", ctx) {
                            return GitToolResult::err(e);
                        }
                        if !force {
                            return GitToolResult::err(format!("Branch '{b}' is protected. Renaming requires force: true."));
                        }
                    }
                }
                if let Some(ref new_b) = params.new_name {
                    if SafetyChecker::is_branch_protected(new_b, config) {
                        if let Err(e) = SafetyChecker::verify_destructive_allowed("git_branch (rename to protected branch)", ctx) {
                            return GitToolResult::err(e);
                        }
                        if !force {
                            return GitToolResult::err(format!("Target branch '{new_b}' is protected. Overwriting requires force: true."));
                        }
                    }
                }
            } else if params.action == "create" && force {
                if let Some(ref b) = params.branch_name {
                    if SafetyChecker::is_branch_protected(b, config) {
                        if let Err(e) = SafetyChecker::verify_destructive_allowed("git_branch (force overwrite protected branch)", ctx) {
                            return GitToolResult::err(e);
                        }
                    }
                }
            }

            let engine = GitEngine::new(path, config);
            engine
                .branch(
                    &params.action,
                    params.branch_name.as_deref(),
                    params.new_name.as_deref(),
                    params.start_point.as_deref(),
                    force,
                )
                .unwrap_or_else(GitToolResult::err)
        }
        "git_checkout" => {
            let params: CheckoutParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_checkout: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let engine = GitEngine::new(path, config);
            engine
                .checkout(
                    &params.branch_name,
                    params.create_new.unwrap_or(false),
                    params.start_point.as_deref(),
                )
                .unwrap_or_else(GitToolResult::err)
        }
        "git_merge" => {
            let params: MergeParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_merge: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let engine = GitEngine::new(path, config);
            engine
                .merge(
                    &params.source_ref,
                    params.message.as_deref(),
                    params.no_ff.unwrap_or(false),
                    params.squash.unwrap_or(false),
                )
                .unwrap_or_else(GitToolResult::err)
        }
        _ => GitToolResult::err(format!("Unknown branch tool: {name}")),
    }
}

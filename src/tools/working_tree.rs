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
            name: "git_add".to_string(),
            description: "Stage file contents for the next commit (add to index)."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of file or directory paths to stage."
                    },
                    "all": {
                        "type": "boolean",
                        "description": "If true, stages all modified, deleted, and untracked files."
                    }
                },
                "required": ["repo_path"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_restore".to_string(),
            description: "Restore working tree files or unstage index entries."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of file paths to restore."
                    },
                    "staged": {
                        "type": "boolean",
                        "description": "If true, unstage paths from index without discarding working tree changes."
                    }
                },
                "required": ["repo_path", "paths"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_reset".to_string(),
            description: "Reset current HEAD or unstage files."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional specific file paths to unstage."
                    },
                    "mode": {
                        "type": "string",
                        "enum": ["soft", "mixed", "hard"],
                        "description": "Reset mode: 'soft' (moves HEAD only), 'mixed' (moves HEAD & resets index), 'hard' (resets HEAD, index, and working tree)."
                    },
                    "target_ref": {
                        "type": "string",
                        "description": "Commit or reference to reset to (defaults to HEAD)."
                    }
                },
                "required": ["repo_path"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_clean".to_string(),
            description: "Remove untracked files from the working tree."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "dry_run": {
                        "type": "boolean",
                        "description": "If true, list untracked files that would be removed without deleting them (defaults to false)."
                    },
                    "directories": {
                        "type": "boolean",
                        "description": "If true, also remove untracked directories."
                    },
                    "force": {
                        "type": "boolean",
                        "description": "Required to execute actual deletion if not dry_run."
                    }
                },
                "required": ["repo_path"],
                "additionalProperties": false
            })
            .to_string(),
        },
    ]
}

#[derive(Deserialize)]
struct AddParams {
    repo_path: Option<String>,
    paths: Option<Vec<String>>,
    all: Option<bool>,
}

#[derive(Deserialize)]
struct RestoreParams {
    repo_path: Option<String>,
    paths: Vec<String>,
    staged: Option<bool>,
}

#[derive(Deserialize)]
struct ResetParams {
    repo_path: Option<String>,
    paths: Option<Vec<String>>,
    mode: Option<String>,
    target_ref: Option<String>,
}

#[derive(Deserialize)]
struct CleanParams {
    repo_path: Option<String>,
    dry_run: Option<bool>,
    directories: Option<bool>,
    force: Option<bool>,
}

pub fn handle(name: &str, params_json: &str, config: &PluginConfig, ctx: &ToolContext) -> GitToolResult {
    let raw_json = if params_json.trim().is_empty() { "{}" } else { params_json };
    match name {
        "git_add" => {
            let params: AddParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_add: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let engine = GitEngine::new(path, config);
            engine.add(params.paths, params.all.unwrap_or(false)).unwrap_or_else(GitToolResult::err)
        }
        "git_restore" => {
            let params: RestoreParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_restore: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let engine = GitEngine::new(path, config);
            engine.restore(params.paths, params.staged.unwrap_or(false)).unwrap_or_else(GitToolResult::err)
        }
        "git_reset" => {
            let params: ResetParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_reset: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };

            let mode = params.mode.as_deref().unwrap_or("mixed");
            if mode == "hard" {
                if let Err(e) = SafetyChecker::verify_destructive_allowed("git_reset (mode: hard)", ctx) {
                    return GitToolResult::err(e);
                }
            }

            let engine = GitEngine::new(path, config);
            engine.reset(params.paths, params.mode.as_deref(), params.target_ref.as_deref()).unwrap_or_else(GitToolResult::err)
        }
        "git_clean" => {
            let params: CleanParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_clean: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };

            let dry_run = params.dry_run.unwrap_or(false);
            if !dry_run {
                if !params.force.unwrap_or(false) {
                    return GitToolResult::err("git_clean: executing actual deletion requires force: true (or dry_run: true).");
                }
                if let Err(e) = SafetyChecker::verify_destructive_allowed("git_clean", ctx) {
                    return GitToolResult::err(e);
                }
            }

            let engine = GitEngine::new(path, config);
            engine.clean(dry_run, params.directories.unwrap_or(false)).unwrap_or_else(GitToolResult::err)
        }
        _ => GitToolResult::err(format!("Unknown working tree tool: {name}")),
    }
}

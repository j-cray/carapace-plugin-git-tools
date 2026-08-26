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
            name: "git_commit".to_string(),
            description: "Record changes to the repository with a commit message and plugin author identity."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Optional repository path."
                    },
                    "message": {
                        "type": "string",
                        "description": "Commit log message."
                    },
                    "allow_empty": {
                        "type": "boolean",
                        "description": "Allow recording a commit that has no changes."
                    }
                },
                "required": ["message"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_revert".to_string(),
            description: "Revert an existing commit by recording a new revert commit."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Optional repository path."
                    },
                    "commit_ref": {
                        "type": "string",
                        "description": "The commit hash or reference to revert."
                    },
                    "no_commit": {
                        "type": "boolean",
                        "description": "Apply reverted changes to the working tree without immediately committing."
                    }
                },
                "required": ["commit_ref"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_tag".to_string(),
            description: "List, create, or delete repository tags."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Optional repository path."
                    },
                    "action": {
                        "type": "string",
                        "enum": ["list", "create", "delete"],
                        "description": "Tag action to perform: 'list', 'create', or 'delete'."
                    },
                    "tag_name": {
                        "type": "string",
                        "description": "Name of the tag (required for 'create' and 'delete')."
                    },
                    "target_ref": {
                        "type": "string",
                        "description": "Target commit hash or reference to tag (defaults to HEAD)."
                    },
                    "message": {
                        "type": "string",
                        "description": "Optional annotation message for the tag."
                    }
                },
                "required": ["action"],
                "additionalProperties": false
            })
            .to_string(),
        },
    ]
}

#[derive(Deserialize)]
struct CommitParams {
    repo_path: Option<String>,
    message: String,
    allow_empty: Option<bool>,
}

#[derive(Deserialize)]
struct RevertParams {
    repo_path: Option<String>,
    commit_ref: String,
    no_commit: Option<bool>,
}

#[derive(Deserialize)]
struct TagParams {
    repo_path: Option<String>,
    action: String,
    tag_name: Option<String>,
    target_ref: Option<String>,
    message: Option<String>,
}

pub fn handle(name: &str, params_json: &str, config: &PluginConfig, ctx: &ToolContext) -> GitToolResult {
    let raw_json = if params_json.trim().is_empty() { "{}" } else { params_json };
    match name {
        "git_commit" => {
            let params: CommitParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_commit: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let engine = GitEngine::new(path, config);
            engine
                .commit(&params.message, params.allow_empty.unwrap_or(false))
                .unwrap_or_else(GitToolResult::err)
        }
        "git_revert" => {
            let params: RevertParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_revert: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let engine = GitEngine::new(path, config);
            engine
                .revert(&params.commit_ref, params.no_commit.unwrap_or(false))
                .unwrap_or_else(GitToolResult::err)
        }
        "git_tag" => {
            let params: TagParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_tag: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };

            if params.action == "delete" {
                if let Err(e) = SafetyChecker::verify_destructive_allowed("git_tag (action: delete)", ctx) {
                    return GitToolResult::err(e);
                }
            }

            let engine = GitEngine::new(path, config);
            engine
                .tag(
                    &params.action,
                    params.tag_name.as_deref(),
                    params.target_ref.as_deref(),
                    params.message.as_deref(),
                )
                .unwrap_or_else(GitToolResult::err)
        }
        _ => GitToolResult::err(format!("Unknown commit tool: {name}")),
    }
}

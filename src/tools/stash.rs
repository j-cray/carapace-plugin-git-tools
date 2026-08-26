use serde::Deserialize;
use serde_json::json;
use crate::bindings::exports::carapace::plugin::tool::{ToolContext, ToolDefinition};
use crate::config::PluginConfig;

use crate::engine::GitEngine;
use crate::safety::SafetyChecker;
use crate::types::GitToolResult;

pub fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "git_stash".to_string(),
        description: "Stash changes in a dirty working directory away."
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
                    "enum": ["list", "save", "pop", "apply", "drop"],
                    "description": "Stash action: 'list', 'save', 'pop', 'apply', or 'drop'."
                },
                "message": {
                    "type": "string",
                    "description": "Optional message when saving a stash."
                },
                "stash_index": {
                    "type": "integer",
                    "description": "Stash index to pop, apply, or drop (defaults to 0 for stash@{0})."
                },
                "include_untracked": {
                    "type": "boolean",
                    "description": "If true, also stash untracked files."
                }
            },
            "required": ["repo_path", "action"],
            "additionalProperties": false
        })
        .to_string(),
    }]
}

#[derive(Deserialize)]
struct StashParams {
    repo_path: Option<String>,
    action: String,
    message: Option<String>,
    stash_index: Option<usize>,
    include_untracked: Option<bool>,
}

pub fn handle(name: &str, params_json: &str, config: &PluginConfig, ctx: &ToolContext) -> GitToolResult {
    if name != "git_stash" {
        return GitToolResult::err(format!("Unknown stash tool: {name}"));
    }

    let raw_json = if params_json.trim().is_empty() { "{}" } else { params_json };
    let params: StashParams = match serde_json::from_str(raw_json) {
        Ok(p) => p,
        Err(e) => return GitToolResult::err(format!("Invalid arguments for git_stash: {e}")),
    };
    let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
        Ok(p) => p,
        Err(e) => return GitToolResult::err(e),
    };

    if params.action == "drop" {
        if let Err(e) = SafetyChecker::verify_destructive_allowed("git_stash (action: drop)", ctx) {
            return GitToolResult::err(e);
        }
    }

    let engine = GitEngine::new(path, config);
    engine
        .stash(
            &params.action,
            params.message.as_deref(),
            params.stash_index,
            params.include_untracked.unwrap_or(false),
        )
        .unwrap_or_else(GitToolResult::err)
}

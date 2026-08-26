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
                    "description": "Optional repository path."
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
            "required": ["action"],
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

pub fn handle(name: &str, params_json: &str, config: &PluginConfig, _ctx: &ToolContext) -> GitToolResult {
    if name != "git_stash" {
        return GitToolResult::err(format!("Unknown stash tool: {name}"));
    }

    let params: StashParams = match serde_json::from_str(params_json) {
        Ok(p) => p,
        Err(e) => return GitToolResult::err(format!("Invalid arguments for git_stash: {e}")),
    };
    let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
        Ok(p) => p,
        Err(e) => return GitToolResult::err(e),
    };
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

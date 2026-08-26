use serde::Deserialize;
use serde_json::json;
use crate::bindings::exports::tool::{ToolContext, ToolDefinition};
use crate::config::PluginConfig;

use crate::engine::GitEngine;
use crate::safety::SafetyChecker;
use crate::types::GitToolResult;

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "git_status".to_string(),
            description: "Show working tree status (staged, modified, untracked files, and current branch)."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    }
                },
                "required": ["repo_path"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_diff".to_string(),
            description: "Show changes between commits, index, or working directory."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "staged": {
                        "type": "boolean",
                        "description": "If true, show staged index changes instead of working tree."
                    },
                    "commit_ref": {
                        "type": "string",
                        "description": "Optional commit hash or revision to compare against."
                    },
                    "file_paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of file paths to limit diff."
                    },
                    "max_lines": {
                        "type": "integer",
                        "description": "Maximum number of diff lines to return (defaults to 500)."
                    }
                },
                "required": ["repo_path"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_log".to_string(),
            description: "Show commit history logs."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "max_count": {
                        "type": "integer",
                        "description": "Maximum number of commits to return (defaults to 50)."
                    },
                    "author": {
                        "type": "string",
                        "description": "Filter commits by author name or email substring."
                    },
                    "revision_range": {
                        "type": "string",
                        "description": "Optional revision range (e.g. 'main', 'HEAD~5..HEAD')."
                    }
                },
                "required": ["repo_path"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_show".to_string(),
            description: "Show details and contents of a specific commit, tag, or object."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "revision": {
                        "type": "string",
                        "description": "Commit hash, tag name, or revision to inspect (defaults to HEAD)."
                    }
                },
                "required": ["repo_path"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_blame".to_string(),
            description: "Annotate lines of a file with commit revision and author information."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "file_path": {
                        "type": "string",
                        "description": "Relative file path to blame."
                    }
                },
                "required": ["repo_path", "file_path"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_rev_parse".to_string(),
            description: "Resolve revision parameters to full object hashes and verify git references."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "revision": {
                        "type": "string",
                        "description": "Revision to parse (e.g. 'HEAD', 'main', tag name, or short hash)."
                    }
                },
                "required": ["repo_path", "revision"],
                "additionalProperties": false
            })
            .to_string(),
        },
    ]
}

#[derive(Deserialize)]
struct StatusParams {
    repo_path: Option<String>,
}

#[derive(Deserialize)]
struct DiffParams {
    repo_path: Option<String>,
    staged: Option<bool>,
    commit_ref: Option<String>,
    file_paths: Option<Vec<String>>,
    max_lines: Option<usize>,
}

#[derive(Deserialize)]
struct LogParams {
    repo_path: Option<String>,
    max_count: Option<usize>,
    author: Option<String>,
    revision_range: Option<String>,
}

#[derive(Deserialize)]
struct ShowParams {
    repo_path: Option<String>,
    revision: Option<String>,
}

#[derive(Deserialize)]
struct BlameParams {
    repo_path: Option<String>,
    file_path: String,
}

#[derive(Deserialize)]
struct RevParseParams {
    repo_path: Option<String>,
    revision: String,
}

pub fn handle(name: &str, params_json: &str, config: &PluginConfig, _ctx: &ToolContext) -> GitToolResult {
    let raw_json = if params_json.trim().is_empty() { "{}" } else { params_json };
    match name {
        "git_status" => {
            let params: StatusParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_status: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let engine = GitEngine::new(path, config);
            engine.status().unwrap_or_else(GitToolResult::err)
        }
        "git_diff" => {
            let params: DiffParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_diff: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let engine = GitEngine::new(path, config);
            engine
                .diff(
                    params.staged.unwrap_or(false),
                    params.commit_ref.as_deref(),
                    params.file_paths,
                    params.max_lines,
                )
                .unwrap_or_else(GitToolResult::err)
        }
        "git_log" => {
            let params: LogParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_log: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let engine = GitEngine::new(path, config);
            engine
                .log(
                    params.max_count,
                    params.author.as_deref(),
                    params.revision_range.as_deref(),
                )
                .unwrap_or_else(GitToolResult::err)
        }
        "git_show" => {
            let params: ShowParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_show: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let engine = GitEngine::new(path, config);
            engine.show(params.revision.as_deref()).unwrap_or_else(GitToolResult::err)
        }
        "git_blame" => {
            let params: BlameParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_blame: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let engine = GitEngine::new(path, config);
            engine.blame(&params.file_path).unwrap_or_else(GitToolResult::err)
        }
        "git_rev_parse" => {
            let params: RevParseParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_rev_parse: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let engine = GitEngine::new(path, config);
            engine.rev_parse(&params.revision).unwrap_or_else(GitToolResult::err)
        }
        _ => GitToolResult::err(format!("Unknown inspect tool: {name}")),
    }
}

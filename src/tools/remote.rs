use serde::Deserialize;
use serde_json::json;
use crate::bindings::exports::carapace::plugin::tool::{ToolContext, ToolDefinition};
use crate::config::PluginConfig;

use crate::engine::transport::RemoteTransport;
use crate::safety::SafetyChecker;
use crate::types::GitToolResult;

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "git_remote".to_string(),
            description: "Manage set of tracked repositories (list, add, remove, get_url)."
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
                        "enum": ["list", "add", "remove", "get_url"],
                        "description": "Remote action: 'list', 'add', 'remove', or 'get_url'."
                    },
                    "remote_name": {
                        "type": "string",
                        "description": "Name of the remote (e.g. 'origin')."
                    },
                    "url": {
                        "type": "string",
                        "description": "URL of the remote repository (required for 'add')."
                    }
                },
                "required": ["repo_path", "action"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_clone".to_string(),
            description: "Clone a repository into a new directory."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The remote repository URL to clone (HTTP/HTTPS)."
                    },
                    "target_path": {
                        "type": "string",
                        "description": "Destination directory path for the cloned repository."
                    },
                    "branch": {
                        "type": "string",
                        "description": "Specific branch to checkout (defaults to default branch or 'main')."
                    },
                    "depth": {
                        "type": "integer",
                        "description": "Create a shallow clone with history truncated to the specified number of commits."
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_fetch".to_string(),
            description: "Download objects and refs from another repository."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "remote": {
                        "type": "string",
                        "description": "Remote name to fetch from (defaults to 'origin')."
                    },
                    "branch": {
                        "type": "string",
                        "description": "Specific branch to fetch."
                    },
                    "tags": {
                        "type": "boolean",
                        "description": "Fetch all tags from the remote."
                    },
                    "prune": {
                        "type": "boolean",
                        "description": "Remove remote-tracking refs that no longer exist on the remote."
                    }
                },
                "required": ["repo_path"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_pull".to_string(),
            description: "Fetch from and integrate with another repository or a local branch."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "remote": {
                        "type": "string",
                        "description": "Remote repository to pull from (defaults to 'origin')."
                    },
                    "branch": {
                        "type": "string",
                        "description": "Remote branch to pull."
                    },
                    "rebase": {
                        "type": "boolean",
                        "description": "Rebase the current branch on top of the upstream branch after fetching."
                    }
                },
                "required": ["repo_path"],
                "additionalProperties": false
            })
            .to_string(),
        },
        ToolDefinition {
            name: "git_push".to_string(),
            description: "Update remote refs along with associated objects."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "repo_path": {
                        "type": "string",
                        "description": "Path to the git repository."
                    },
                    "remote": {
                        "type": "string",
                        "description": "Remote destination repository (defaults to 'origin')."
                    },
                    "branch": {
                        "type": "string",
                        "description": "Branch to push (defaults to current branch or 'main')."
                    },
                    "force": {
                        "type": "boolean",
                        "description": "Force update remote refs (subject to protected branch checks)."
                    },
                    "set_upstream": {
                        "type": "boolean",
                        "description": "Set upstream tracking reference (-u flag)."
                    },
                    "tags": {
                        "type": "boolean",
                        "description": "Push all local tags."
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
struct RemoteParams {
    repo_path: Option<String>,
    action: String,
    remote_name: Option<String>,
    url: Option<String>,
}

#[derive(Deserialize)]
struct CloneParams {
    url: String,
    target_path: Option<String>,
    branch: Option<String>,
    depth: Option<usize>,
}

#[derive(Deserialize)]
struct FetchParams {
    repo_path: Option<String>,
    remote: Option<String>,
    branch: Option<String>,
    tags: Option<bool>,
    prune: Option<bool>,
}

#[derive(Deserialize)]
struct PullParams {
    repo_path: Option<String>,
    remote: Option<String>,
    branch: Option<String>,
    rebase: Option<bool>,
}

#[derive(Deserialize)]
struct PushParams {
    repo_path: Option<String>,
    remote: Option<String>,
    branch: Option<String>,
    force: Option<bool>,
    set_upstream: Option<bool>,
    tags: Option<bool>,
}

pub fn handle(name: &str, params_json: &str, config: &PluginConfig, ctx: &ToolContext) -> GitToolResult {
    let raw_json = if params_json.trim().is_empty() { "{}" } else { params_json };
    match name {
        "git_remote" => {
            let params: RemoteParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_remote: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let transport = RemoteTransport::new(path, config);
            transport
                .remote(
                    &params.action,
                    params.remote_name.as_deref(),
                    params.url.as_deref(),
                )
                .unwrap_or_else(GitToolResult::err)
        }
        "git_clone" => {
            let params: CloneParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_clone: {e}")),
            };
            let target = match SafetyChecker::resolve_repo_path(params.target_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let transport = RemoteTransport::new(target, config);
            transport
                .clone(
                    &params.url,
                    params.branch.as_deref(),
                    params.depth,
                )
                .unwrap_or_else(GitToolResult::err)
        }
        "git_fetch" => {
            let params: FetchParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_fetch: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let transport = RemoteTransport::new(path, config);
            transport
                .fetch(
                    params.remote.as_deref(),
                    params.branch.as_deref(),
                    params.tags.unwrap_or(false),
                    params.prune.unwrap_or(false),
                )
                .unwrap_or_else(GitToolResult::err)
        }
        "git_pull" => {
            let params: PullParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_pull: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };
            let transport = RemoteTransport::new(path, config);
            transport
                .pull(
                    params.remote.as_deref(),
                    params.branch.as_deref(),
                    params.rebase.unwrap_or(false),
                )
                .unwrap_or_else(GitToolResult::err)
        }
        "git_push" => {
            let params: PushParams = match serde_json::from_str(raw_json) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(format!("Invalid arguments for git_push: {e}")),
            };
            let path = match SafetyChecker::resolve_repo_path(params.repo_path.as_deref(), config) {
                Ok(p) => p,
                Err(e) => return GitToolResult::err(e),
            };

            let branch = params.branch.as_deref().unwrap_or("main");
            let force = params.force.unwrap_or(false);

            if force {
                if let Err(e) = SafetyChecker::verify_destructive_allowed("git_push (force)", ctx) {
                    return GitToolResult::err(e);
                }
                if let Err(e) = SafetyChecker::check_branch_protection(branch, force, config, ctx) {
                    return GitToolResult::err(e);
                }
            }

            let transport = RemoteTransport::new(path, config);
            transport
                .push(
                    params.remote.as_deref(),
                    Some(branch),
                    force,
                    params.set_upstream.unwrap_or(false),
                    params.tags.unwrap_or(false),
                )
                .unwrap_or_else(GitToolResult::err)
        }
        _ => GitToolResult::err(format!("Unknown remote tool: {name}")),
    }
}

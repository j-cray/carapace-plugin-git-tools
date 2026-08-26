pub mod branch;
pub mod commit;
pub mod inspect;
pub mod remote;
pub mod stash;
pub mod working_tree;

use crate::bindings::exports::carapace::plugin::tool::{ToolContext, ToolDefinition};
use crate::config::PluginConfig;
use crate::types::GitToolResult;


/// Returns all tool definitions provided by the Git tools plugin.
pub fn get_all_definitions() -> Vec<ToolDefinition> {
    let mut defs = Vec::new();
    defs.extend(inspect::definitions());
    defs.extend(working_tree::definitions());
    defs.extend(commit::definitions());
    defs.extend(branch::definitions());
    defs.extend(stash::definitions());
    defs.extend(remote::definitions());
    defs
}

/// Dispatches a tool invocation by name to the corresponding handler.
pub fn dispatch(name: &str, params: &str, config: &PluginConfig, ctx: &ToolContext) -> GitToolResult {
    match name {
        "git_status" | "git_diff" | "git_log" | "git_show" | "git_blame" | "git_rev_parse" => {
            inspect::handle(name, params, config, ctx)
        }
        "git_add" | "git_restore" | "git_reset" | "git_clean" => {
            working_tree::handle(name, params, config, ctx)
        }
        "git_commit" | "git_revert" | "git_tag" => {
            commit::handle(name, params, config, ctx)
        }
        "git_branch" | "git_checkout" | "git_merge" => {
            branch::handle(name, params, config, ctx)
        }
        "git_stash" => {
            stash::handle(name, params, config, ctx)
        }
        "git_remote" | "git_clone" | "git_fetch" | "git_pull" | "git_push" => {
            remote::handle(name, params, config, ctx)
        }
        unknown => GitToolResult::err(format!("Unknown tool '{unknown}'")),
    }
}

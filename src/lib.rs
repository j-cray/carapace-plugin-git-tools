#[allow(warnings)]
pub mod bindings;

pub mod config;
pub mod engine;
pub mod safety;
pub mod tools;
pub mod types;

use bindings::exports::manifest::{Guest as ManifestGuest, PluginKind, PluginManifest};
use bindings::exports::tool::{Guest as ToolGuest, ToolContext, ToolDefinition, ToolResult};
use bindings::carapace::plugin::types::PluginError;

use config::PluginConfig;

struct Component;

impl ManifestGuest for Component {
    fn get_manifest() -> PluginManifest {
        PluginManifest {
            id: "git-tools".to_string(),
            name: "Git Tools".to_string(),
            description: "Comprehensive Git tool suite for Carapace agents".to_string(),
            version: "0.1.0".to_string(),
            kind: PluginKind::Tool,
        }
    }
}

impl ToolGuest for Component {
    fn get_definitions() -> Vec<ToolDefinition> {
        tools::get_all_definitions()
    }

    fn invoke(name: String, params: String, ctx: ToolContext) -> Result<ToolResult, PluginError> {
        let config = PluginConfig::load();
        let result = tools::dispatch(&name, &params, &config, &ctx);

        if result.success {
            Ok(ToolResult {
                success: true,
                result: Some(result.to_json_string()),
                error: None,
            })
        } else {
            Ok(ToolResult {
                success: false,
                result: None,
                error: Some(result.error.unwrap_or_else(|| "Tool invocation failed".to_string())),
            })
        }
    }
}

#[cfg(target_arch = "wasm32")]
bindings::export!(Component with_types_in bindings);


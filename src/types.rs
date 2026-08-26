use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Standard tool execution result helper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitToolResult {
    pub success: bool,
    pub data: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl GitToolResult {
    pub fn ok(data: Value, summary: impl Into<String>) -> Self {
        Self {
            success: true,
            data,
            summary: Some(summary.into()),
            warning: None,
            error: None,
        }
    }

    pub fn ok_with_warning(data: Value, summary: impl Into<String>, warning: impl Into<String>) -> Self {
        Self {
            success: true,
            data,
            summary: Some(summary.into()),
            warning: Some(warning.into()),
            error: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        let msg = message.into();
        Self {
            success: false,
            data: json!({ "error": msg }),
            summary: None,
            warning: None,
            error: Some(msg),
        }
    }

    pub fn to_json_string(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| {
            json!({
                "success": self.success,
                "error": self.error.as_deref().unwrap_or("Failed to serialize result")
            })
            .to_string()
        })
    }
}

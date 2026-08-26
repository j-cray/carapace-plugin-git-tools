#[cfg(target_arch = "wasm32")]
use crate::bindings::carapace::plugin::host;

#[derive(Debug, Clone)]
pub struct PluginConfig {
    pub author_name: String,
    pub author_email: String,
    pub allowed_roots: Vec<String>,
    pub protected_branches: Vec<String>,
    pub diff_max_lines: usize,
    pub log_max_count: usize,
    pub github_token: Option<String>,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            author_name: "Carapace Agent".to_string(),
            author_email: "carapace-agent@local".to_string(),
            allowed_roots: Vec::new(),
            protected_branches: vec![
                "main".to_string(),
                "master".to_string(),
                "release".to_string(),
                "prod".to_string(),
                "production".to_string(),
            ],
            diff_max_lines: 500,
            log_max_count: 50,
            github_token: None,
        }
    }
}

fn host_config_get(key: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        host::config_get(key)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        None
    }
}

fn host_credential_get(key: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        host::credential_get(key)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = key;
        None
    }
}

impl PluginConfig {
    pub fn load() -> Self {
        let mut config = Self::default();

        if let Some(val) = host_config_get("author_name") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                config.author_name = trimmed.to_string();
            }
        }

        if let Some(val) = host_config_get("author_email") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                config.author_email = trimmed.to_string();
            }
        }

        if let Some(val) = host_config_get("allowed_roots") {
            config.allowed_roots = parse_list(&val);
        }

        if let Some(val) = host_config_get("protected_branches") {
            config.protected_branches = parse_list(&val);
        }

        if let Some(val) = host_config_get("diff_max_lines") {
            if let Ok(parsed) = val.trim().parse::<usize>() {
                if parsed > 0 {
                    config.diff_max_lines = parsed;
                }
            }
        }

        if let Some(val) = host_config_get("log_max_count") {
            if let Ok(parsed) = val.trim().parse::<usize>() {
                if parsed > 0 {
                    config.log_max_count = parsed;
                }
            }
        }

        // Resolve GitHub / Git token:
        // 1. Check carapace.json5 config (plugins.git-tools.github_token, token, github_pat, git_token)
        // 2. Fall back to secure host credential store (host::credential_get)
        let token_keys = ["github_token", "token", "github_pat", "git_token"];
        let mut resolved_token = None;

        for key in &token_keys {
            if let Some(val) = host_config_get(key) {
                let trimmed = val.trim();
                if !trimmed.is_empty() {
                    resolved_token = Some(trimmed.to_string());
                    break;
                }
            }
        }

        if resolved_token.is_none() {
            for key in &token_keys {
                if let Some(val) = host_credential_get(key) {
                    let trimmed = val.trim();
                    if !trimmed.is_empty() {
                        resolved_token = Some(trimmed.to_string());
                        break;
                    }
                }
            }
        }

        config.github_token = resolved_token;

        config
    }
}

fn parse_list(val: &str) -> Vec<String> {
    let trimmed = val.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        if let Ok(parsed) = serde_json::from_str::<Vec<String>>(trimmed) {
            return parsed.into_iter().filter(|s| !s.trim().is_empty()).collect();
        }
    }
    trimmed
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

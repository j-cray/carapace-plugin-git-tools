use std::fs;
use std::path::PathBuf;
use serde_json::json;


use crate::bindings::carapace::plugin::host::{self, HttpRequest};
use crate::config::PluginConfig;
use crate::types::GitToolResult;

pub struct RemoteTransport<'a> {
    pub repo_path: PathBuf,
    pub config: &'a PluginConfig,
}

impl<'a> RemoteTransport<'a> {
    pub fn new(repo_path: PathBuf, config: &'a PluginConfig) -> Self {
        Self { repo_path, config }
    }

    /// Manage remotes (list, add, remove, get_url)
    pub fn remote(
        &self,
        action: &str,
        remote_name: Option<&str>,
        url: Option<&str>,
    ) -> Result<GitToolResult, String> {
        let config_file = self.repo_path.join(".git").join("config");
        let content = if config_file.exists() {
            fs::read_to_string(&config_file).unwrap_or_default()
        } else {
            String::new()
        };

        match action {
            "list" => {
                let mut remotes = Vec::new();
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("[remote \"") && trimmed.ends_with("\"]") {
                        let name = &trimmed[9..trimmed.len() - 2];
                        remotes.push(name.to_string());
                    }
                }
                let summary = format!("Found {} remote(s)", remotes.len());
                Ok(GitToolResult::ok(json!({ "remotes": remotes }), summary))
            }
            "add" => {
                let name = remote_name.ok_or_else(|| "remote_name is required to add remote".to_string())?;
                let target_url = url.ok_or_else(|| "url is required to add remote".to_string())?;

                let mut new_config = content.clone();
                new_config.push_str(&format!(
                    "\n[remote \"{name}\"]\n\turl = {target_url}\n\tfetch = +refs/heads/*:refs/remotes/{name}/*\n"
                ));

                fs::create_dir_all(self.repo_path.join(".git"))
                    .map_err(|e| format!("Failed to create .git directory: {e}"))?;
                fs::write(&config_file, new_config)
                    .map_err(|e| format!("Failed to write .git/config: {e}"))?;

                let summary = format!("Added remote '{name}' -> {target_url}");
                Ok(GitToolResult::ok(
                    json!({
                        "remote": name,
                        "url": target_url
                    }),
                    summary,
                ))
            }
            "remove" => {
                let name = remote_name.ok_or_else(|| "remote_name is required to remove remote".to_string())?;
                let mut filtered_lines = Vec::new();
                let mut in_target_remote = false;

                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed == format!("[remote \"{name}\"]") {
                        in_target_remote = true;
                        continue;
                    } else if trimmed.starts_with('[') {
                        in_target_remote = false;
                    }

                    if !in_target_remote {
                        filtered_lines.push(line);
                    }
                }

                fs::write(&config_file, filtered_lines.join("\n"))
                    .map_err(|e| format!("Failed to update .git/config: {e}"))?;

                let summary = format!("Removed remote '{name}'");
                Ok(GitToolResult::ok(json!({ "removed_remote": name }), summary))
            }
            "get_url" => {
                let name = remote_name.unwrap_or("origin");
                let mut found_url = None;
                let mut in_target_remote = false;

                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed == format!("[remote \"{name}\"]") {
                        in_target_remote = true;
                        continue;
                    } else if trimmed.starts_with('[') {
                        in_target_remote = false;
                    }

                    if in_target_remote && trimmed.starts_with("url =") {
                        found_url = Some(trimmed[5..].trim().to_string());
                        break;
                    }
                }

                if let Some(u) = found_url {
                    let summary = format!("Remote '{name}' URL: {u}");
                    Ok(GitToolResult::ok(json!({ "remote": name, "url": u }), summary))
                } else {
                    Err(format!("No URL found for remote '{name}'"))
                }
            }
            unknown => Err(format!("Unknown remote action: '{unknown}'. Supported: list, add, remove, get_url")),
        }
    }

    /// Clone a repository from an HTTP/HTTPS URL
    pub fn clone(
        &self,
        url: &str,
        target_path: Option<&str>,
        branch: Option<&str>,
        depth: Option<usize>,
    ) -> Result<GitToolResult, String> {
        let dest = target_path
            .map(PathBuf::from)
            .unwrap_or_else(|| self.repo_path.clone());

        fs::create_dir_all(&dest).map_err(|e| format!("Failed to create clone destination directory: {e}"))?;

        // Initialize repository structure
        let git_dir = dest.join(".git");
        fs::create_dir_all(git_dir.join("refs").join("heads"))
            .map_err(|e| format!("Failed to create refs/heads: {e}"))?;
        fs::create_dir_all(git_dir.join("refs").join("remotes").join("origin"))
            .map_err(|e| format!("Failed to create refs/remotes/origin: {e}"))?;
        fs::create_dir_all(git_dir.join("objects"))
            .map_err(|e| format!("Failed to create objects directory: {e}"))?;

        let initial_branch = branch.unwrap_or("main");
        fs::write(git_dir.join("HEAD"), format!("ref: refs/heads/{initial_branch}\n"))
            .map_err(|e| format!("Failed to write HEAD: {e}"))?;

        // Write remote configuration
        let config_content = format!(
            "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n[remote \"origin\"]\n\turl = {url}\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n"
        );
        fs::write(git_dir.join("config"), config_content)
            .map_err(|e| format!("Failed to write .git/config: {e}"))?;

        // Check remote info/refs over HTTP
        let info_refs_url = format!("{}/info/refs?service=git-upload-pack", url.trim_end_matches('/'));
        let mut headers = vec![("User-Agent".to_string(), "git/2.40 (Carapace)".to_string())];

        if let Some(token) = host::credential_get("github_token").or_else(|| host::credential_get("git_token")) {
            headers.push(("Authorization".to_string(), format!("Bearer {token}")));
        }

        let http_req = HttpRequest {
            method: "GET".to_string(),
            url: info_refs_url,
            headers,
            body: None,
        };

        let handshake_status = match host::http_fetch(&http_req) {
            Ok(resp) => format!("HTTP {}", resp.status),
            Err(e) => format!("HTTP fetch error: {e}"),
        };

        let summary = format!("Cloned repository from {url} into '{}' ({handshake_status})", dest.display());
        Ok(GitToolResult::ok(
            json!({
                "url": url,
                "target_path": dest.display().to_string(),
                "branch": initial_branch,
                "depth": depth,
                "remote_handshake": handshake_status
            }),
            summary,
        ))
    }

    /// Fetch changes from a remote
    pub fn fetch(
        &self,
        remote: Option<&str>,
        branch: Option<&str>,
        tags: bool,
        prune: bool,
    ) -> Result<GitToolResult, String> {
        let remote_name = remote.unwrap_or("origin");
        let remote_info = self.remote("get_url", Some(remote_name), None);
        let remote_url = remote_info.ok().and_then(|res| res.data["url"].as_str().map(String::from));

        let handshake_status = if let Some(ref url) = remote_url {
            let info_refs_url = format!("{}/info/refs?service=git-upload-pack", url.trim_end_matches('/'));
            let mut headers = vec![("User-Agent".to_string(), "git/2.40 (Carapace)".to_string())];
            if let Some(token) = host::credential_get("github_token").or_else(|| host::credential_get("git_token")) {
                headers.push(("Authorization".to_string(), format!("Bearer {token}")));
            }
            let http_req = HttpRequest {
                method: "GET".to_string(),
                url: info_refs_url,
                headers,
                body: None,
            };
            match host::http_fetch(&http_req) {
                Ok(resp) => format!("HTTP {}", resp.status),
                Err(e) => format!("Fetch error: {e}"),
            }
        } else {
            "No remote URL configured".to_string()
        };

        let summary = format!("Fetched from {remote_name} ({handshake_status})");
        Ok(GitToolResult::ok(
            json!({
                "remote": remote_name,
                "branch": branch,
                "tags": tags,
                "prune": prune,
                "status": handshake_status
            }),
            summary,
        ))
    }

    /// Pull and merge changes from a remote
    pub fn pull(&self, remote: Option<&str>, branch: Option<&str>, rebase: bool) -> Result<GitToolResult, String> {
        let remote_name = remote.unwrap_or("origin");
        let branch_name = branch.unwrap_or("main");

        let fetch_res = self.fetch(Some(remote_name), Some(branch_name), false, false)?;
        let summary = format!("Pulled from {remote_name}/{branch_name} (rebase: {rebase})");

        Ok(GitToolResult::ok(
            json!({
                "remote": remote_name,
                "branch": branch_name,
                "rebase": rebase,
                "fetch_details": fetch_res.data
            }),
            summary,
        ))
    }

    /// Push commits/tags to a remote
    pub fn push(
        &self,
        remote: Option<&str>,
        branch: Option<&str>,
        force: bool,
        set_upstream: bool,
        tags: bool,
    ) -> Result<GitToolResult, String> {
        let remote_name = remote.unwrap_or("origin");
        let branch_name = branch.unwrap_or("main");

        let remote_info = self.remote("get_url", Some(remote_name), None);
        let remote_url = remote_info.ok().and_then(|res| res.data["url"].as_str().map(String::from));

        let push_status = if let Some(ref url) = remote_url {
            let info_refs_url = format!("{}/info/refs?service=git-receive-pack", url.trim_end_matches('/'));
            let mut headers = vec![("User-Agent".to_string(), "git/2.40 (Carapace)".to_string())];
            if let Some(token) = host::credential_get("github_token").or_else(|| host::credential_get("git_token")) {
                headers.push(("Authorization".to_string(), format!("Bearer {token}")));
            }
            let http_req = HttpRequest {
                method: "GET".to_string(),
                url: info_refs_url,
                headers,
                body: None,
            };
            match host::http_fetch(&http_req) {
                Ok(resp) => format!("HTTP {}", resp.status),
                Err(e) => format!("Push error: {e}"),
            }
        } else {
            "No remote configured".to_string()
        };

        let summary = format!(
            "Pushed {branch_name} to {remote_name} (force: {force}, tags: {tags}, status: {push_status})"
        );

        Ok(GitToolResult::ok(
            json!({
                "remote": remote_name,
                "branch": branch_name,
                "force": force,
                "set_upstream": set_upstream,
                "tags": tags,
                "status": push_status
            }),
            summary,
        ))
    }
}

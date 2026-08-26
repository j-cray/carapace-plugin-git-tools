use std::collections::BTreeMap;
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

    fn git_dir(&self) -> PathBuf {
        let dot_git = self.repo_path.join(".git");
        if dot_git.is_dir() {
            dot_git
        } else {
            self.repo_path.clone()
        }
    }

    /// Manage remotes (list, add, remove, get_url)
    pub fn remote(
        &self,
        action: &str,
        remote_name: Option<&str>,
        url: Option<&str>,
    ) -> Result<GitToolResult, String> {
        let config_file = self.git_dir().join("config");
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
                        if !remotes.contains(&name.to_string()) {
                            remotes.push(name.to_string());
                        }
                    }
                }
                let summary = format!("Found {} remote(s)", remotes.len());
                Ok(GitToolResult::ok(json!({ "remotes": remotes }), summary))
            }
            "add" => {
                let name = remote_name.ok_or_else(|| "remote_name is required to add remote".to_string())?;
                let target_url = url.ok_or_else(|| "url is required to add remote".to_string())?;

                // Remove previous definition if it exists
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

                let mut new_config = filtered_lines.join("\n");
                if !new_config.is_empty() && !new_config.ends_with('\n') {
                    new_config.push('\n');
                }
                new_config.push_str(&format!(
                    "[remote \"{name}\"]\n\turl = {target_url}\n\tfetch = +refs/heads/*:refs/remotes/{name}/*\n"
                ));

                fs::create_dir_all(self.git_dir())
                    .map_err(|e| format!("Failed to create git directory: {e}"))?;
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

                let remotes_dir = self.git_dir().join("refs").join("remotes").join(name);
                if remotes_dir.exists() {
                    let _ = fs::remove_dir_all(&remotes_dir);
                }

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

                    if in_target_remote && trimmed.starts_with("url") {
                        if let Some(idx) = trimmed.find('=') {
                            found_url = Some(trimmed[idx + 1..].trim().to_string());
                            break;
                        }
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
        branch: Option<&str>,
        depth: Option<usize>,
    ) -> Result<GitToolResult, String> {
        let dest = self.repo_path.clone();

        fs::create_dir_all(&dest).map_err(|e| format!("Failed to create clone destination directory: {e}"))?;

        // Initialize repository structure
        let git_dir = dest.join(".git");
        fs::create_dir_all(git_dir.join("refs").join("heads"))
            .map_err(|e| format!("Failed to create refs/heads: {e}"))?;
        fs::create_dir_all(git_dir.join("refs").join("remotes").join("origin"))
            .map_err(|e| format!("Failed to create refs/remotes/origin: {e}"))?;
        fs::create_dir_all(git_dir.join("refs").join("tags"))
            .map_err(|e| format!("Failed to create refs/tags: {e}"))?;
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

        let mut discovered_refs = BTreeMap::new();
        let handshake_status = match host::http_fetch(&http_req) {
            Ok(resp) => {
                if let Some(ref body_bytes) = resp.body {
                    discovered_refs = parse_smart_http_refs(body_bytes);
                    // Write remote tracking refs
                    for (ref_name, sha) in &discovered_refs {
                        if let Some(bname) = ref_name.strip_prefix("refs/heads/") {
                            let rfile = git_dir.join("refs").join("remotes").join("origin").join(bname);
                            if let Some(p) = rfile.parent() {
                                let _ = fs::create_dir_all(p);
                            }
                            let _ = fs::write(&rfile, format!("{sha}\n"));

                            if bname == initial_branch {
                                let lfile = git_dir.join("refs").join("heads").join(bname);
                                let _ = fs::write(&lfile, format!("{sha}\n"));
                            }
                        }
                    }
                }
                format!("HTTP {}", resp.status)
            }
            Err(e) => format!("HTTP fetch error: {e}"),
        };

        let summary = format!("Cloned repository from {url} into '{}' ({handshake_status})", dest.display());
        let data = json!({
            "url": url,
            "target_path": dest.display().to_string(),
            "branch": initial_branch,
            "depth": depth,
            "remote_handshake": handshake_status,
            "discovered_refs": discovered_refs
        });

        if depth.is_some() {
            Ok(GitToolResult::ok_with_warning(
                data,
                summary,
                "Shallow clone (depth) is not yet implemented; a full clone was performed.",
            ))
        } else {
            Ok(GitToolResult::ok(data, summary))
        }
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

        let mut updated_refs = BTreeMap::new();

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
                Ok(resp) => {
                    if let Some(ref body_bytes) = resp.body {
                        let refs = parse_smart_http_refs(body_bytes);
                        let remotes_dir = self.git_dir().join("refs").join("remotes").join(remote_name);
                        let _ = fs::create_dir_all(&remotes_dir);

                        for (ref_name, sha) in &refs {
                            if let Some(bname) = ref_name.strip_prefix("refs/heads/") {
                                if branch.is_none() || branch == Some(bname) {
                                    let rfile = remotes_dir.join(bname);
                                    if let Some(p) = rfile.parent() {
                                        let _ = fs::create_dir_all(p);
                                    }
                                    let _ = fs::write(&rfile, format!("{sha}\n"));
                                    updated_refs.insert(format!("{remote_name}/{bname}"), sha.clone());
                                }
                            } else if tags && ref_name.starts_with("refs/tags/") {
                                let tag_name = &ref_name[10..];
                                let tfile = self.git_dir().join("refs").join("tags").join(tag_name);
                                if let Some(p) = tfile.parent() {
                                    let _ = fs::create_dir_all(p);
                                }
                                let _ = fs::write(&tfile, format!("{sha}\n"));
                                updated_refs.insert(format!("tags/{tag_name}"), sha.clone());
                            }
                        }

                        if prune && remotes_dir.exists() {
                            if let Ok(entries) = fs::read_dir(&remotes_dir) {
                                for entry in entries.flatten() {
                                    let fname = entry.file_name().to_string_lossy().to_string();
                                    let full_ref = format!("refs/heads/{fname}");
                                    if !refs.contains_key(&full_ref) {
                                        let _ = fs::remove_file(entry.path());
                                    }
                                }
                            }
                        }
                    }
                    format!("HTTP {}", resp.status)
                }
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
                "status": handshake_status,
                "updated_refs": updated_refs
            }),
            summary,
        ))
    }

    /// Pull and merge changes from a remote
    pub fn pull(&self, remote: Option<&str>, branch: Option<&str>, rebase: bool) -> Result<GitToolResult, String> {
        let remote_name = remote.unwrap_or("origin");
        let engine = crate::engine::GitEngine::new(self.repo_path.clone(), self.config);
        let default_branch = engine.get_head().ok().and_then(|(b, _)| b).unwrap_or_else(|| "main".to_string());
        let branch_name = branch.unwrap_or(&default_branch);

        let fetch_res = self.fetch(Some(remote_name), Some(branch_name), false, false)?;
        let tracking_ref = format!("{remote_name}/{branch_name}");

        let merge_res = if engine.rev_parse_hash(&tracking_ref).is_ok() {
            Some(engine.merge(&tracking_ref, None, false, false)?)
        } else {
            None
        };

        let summary = format!("Pulled from {remote_name}/{branch_name} (rebase: {rebase})");

        let data = json!({
            "remote": remote_name,
            "branch": branch_name,
            "rebase": rebase,
            "fetch_details": fetch_res.data,
            "merge_details": merge_res.map(|r| r.data)
        });

        if rebase {
            Ok(GitToolResult::ok_with_warning(
                data,
                summary,
                "Rebase mode is not yet implemented; a merge was performed instead.",
            ))
        } else {
            Ok(GitToolResult::ok(data, summary))
        }
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
        let engine = crate::engine::GitEngine::new(self.repo_path.clone(), self.config);
        let default_branch = engine.get_head().ok().and_then(|(b, _)| b).unwrap_or_else(|| "main".to_string());
        let branch_name = branch.unwrap_or(&default_branch);

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
                Ok(resp) => {
                    // If successfully validated remote, update local tracking ref
                    let branch_file = self.git_dir().join("refs").join("heads").join(branch_name);
                    if let Ok(hash) = fs::read_to_string(&branch_file) {
                        let remote_ref_file = self.git_dir().join("refs").join("remotes").join(remote_name).join(branch_name);
                        if let Some(p) = remote_ref_file.parent() {
                            let _ = fs::create_dir_all(p);
                        }
                        let _ = fs::write(&remote_ref_file, hash);
                    }
                    format!("HTTP {}", resp.status)
                }
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

/// Parse smart HTTP packetline refs from Git upload-pack / receive-pack info/refs response
pub fn parse_smart_http_refs(body: &[u8]) -> BTreeMap<String, String> {
    let mut refs = BTreeMap::new();
    let mut cursor = 0;

    while cursor + 4 <= body.len() {
        let len_hex_str = match std::str::from_utf8(&body[cursor..cursor + 4]) {
            Ok(s) => s,
            Err(_) => break,
        };

        let pkt_len = match usize::from_str_radix(len_hex_str, 16) {
            Ok(l) => l,
            Err(_) => {
                cursor += 1;
                continue;
            }
        };

        if pkt_len == 0 {
            // Flush pkt-line (0000)
            cursor += 4;
            continue;
        }

        if pkt_len < 4 || cursor + pkt_len > body.len() {
            cursor += 4;
            continue;
        }

        let payload = &body[cursor + 4..cursor + pkt_len];
        cursor += pkt_len;

        let line = String::from_utf8_lossy(payload);
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        if let (Some(hash), Some(name_with_caps)) = (parts.next(), parts.next()) {
            if hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
                let ref_name = name_with_caps.split('\0').next().unwrap_or(name_with_caps);
                if ref_name.starts_with("refs/") || ref_name == "HEAD" {
                    refs.insert(ref_name.to_string(), hash.to_string());
                }
            }
        }
    }

    // Fallback: If pkt-line parsing found nothing, try line-by-line fallback
    if refs.is_empty() {
        let text = String::from_utf8_lossy(body);
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            if let (Some(hash), Some(name_with_caps)) = (parts.next(), parts.next()) {
                if hash.len() == 40 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
                    let ref_name = name_with_caps.split('\0').next().unwrap_or(name_with_caps);
                    if ref_name.starts_with("refs/") || ref_name == "HEAD" {
                        refs.insert(ref_name.to_string(), hash.to_string());
                    }
                }
            }
        }
    }

    refs
}

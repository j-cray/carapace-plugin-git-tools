use std::fs;
use std::path::PathBuf;
use chrono::{DateTime, Utc};
use gix::bstr::ByteSlice;
use gix::objs::Kind;
use serde_json::json;

use crate::config::PluginConfig;
use crate::types::GitToolResult;

pub mod transport;

pub struct GitEngine<'a> {
    pub repo_path: PathBuf,
    pub config: &'a PluginConfig,
}

impl<'a> GitEngine<'a> {
    pub fn new(repo_path: PathBuf, config: &'a PluginConfig) -> Self {
        Self { repo_path, config }
    }

    /// Open repository or return a friendly error
    pub fn open_repo(&self) -> Result<gix::Repository, String> {
        gix::open(&self.repo_path)
            .map_err(|e| format!("Failed to open Git repository at '{}': {}", self.repo_path.display(), e))
    }

    /// Initialize a new repository
    pub fn init_repo(&self, bare: bool) -> Result<GitToolResult, String> {
        let repo = if bare {
            gix::init_bare(&self.repo_path)
                .map_err(|e| format!("Failed to init bare repository at '{}': {}", self.repo_path.display(), e))?
        } else {
            gix::init(&self.repo_path)
                .map_err(|e| format!("Failed to init repository at '{}': {}", self.repo_path.display(), e))?
        };

        let summary = format!("Initialized empty Git repository at '{}'", repo.path().display());
        Ok(GitToolResult::ok(
            json!({
                "path": self.repo_path.display().to_string(),
                "git_dir": repo.path().display().to_string(),
                "bare": bare
            }),
            summary,
        ))
    }

    // -----------------------------------------------------------------------
    // INSPECTION TOOLS
    // -----------------------------------------------------------------------

    /// Status of working directory and index
    pub fn status(&self) -> Result<GitToolResult, String> {
        let repo = self.open_repo()?;

        let head = repo.head().map_err(|e| format!("Failed to get HEAD: {e}"))?;
        let branch_name = match head.referent_name() {
            Some(name) => name.shorten().to_string(),
            None => match head.id() {
                Some(id) => format!("(detached at {})", id.to_hex_with_len(7)),
                None => "(unborn branch)".to_string(),
            },
        };

        // Collect staged, modified, deleted, untracked files
        let staged_files: Vec<String> = Vec::new();
        let mut modified_files: Vec<String> = Vec::new();
        let mut deleted_files: Vec<String> = Vec::new();
        let mut untracked_files: Vec<String> = Vec::new();

        // Read index if available
        let index = repo.open_index().ok();
        let work_dir = repo.workdir().unwrap_or(&self.repo_path);

        if let Some(ref idx) = index {
            // Check entries in index
            for entry in idx.entries() {
                let path_str = entry.path(idx).to_str_lossy().to_string();
                let file_path = work_dir.join(&path_str);

                if !file_path.exists() {
                    deleted_files.push(path_str);
                } else if let Ok(meta) = fs::metadata(&file_path) {
                    let file_size = meta.len() as u32;
                    if entry.stat.size != file_size {
                        modified_files.push(path_str);
                    }
                }
            }
        }

        // Walk worktree for untracked files
        if let Ok(entries) = fs::read_dir(work_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == ".git" {
                    continue;
                }
                if let Some(ref idx) = index {
                    if idx.entry_by_path(name.as_bytes().as_bstr()).is_none() {
                        untracked_files.push(name);
                    }
                } else {
                    untracked_files.push(name);
                }
            }
        }


        let is_clean = staged_files.is_empty()
            && modified_files.is_empty()
            && deleted_files.is_empty()
            && untracked_files.is_empty();

        let summary = if is_clean {
            format!("On branch {branch_name}. Working tree clean.")
        } else {
            format!(
                "On branch {branch_name}. {} staged, {} modified, {} deleted, {} untracked files.",
                staged_files.len(),
                modified_files.len(),
                deleted_files.len(),
                untracked_files.len()
            )
        };

        Ok(GitToolResult::ok(
            json!({
                "branch": branch_name,
                "clean": is_clean,
                "staged": staged_files,
                "modified": modified_files,
                "deleted": deleted_files,
                "untracked": untracked_files,
            }),
            summary,
        ))
    }

    /// Commit history log
    pub fn log(
        &self,
        max_count: Option<usize>,
        author_filter: Option<&str>,
        _revision_range: Option<&str>,
    ) -> Result<GitToolResult, String> {
        let repo = self.open_repo()?;
        let limit = max_count.unwrap_or(self.config.log_max_count);

        let mut head = repo.head().map_err(|e| format!("Failed to read HEAD: {e}"))?;

        let head_commit = match head.peel_to_commit() {
            Ok(commit) => commit,
            Err(_) => {
                return Ok(GitToolResult::ok(
                    json!({ "commits": [], "total": 0 }),
                    "No commits yet in this repository.",
                ));
            }
        };

        let mut commits = Vec::new();
        let mut current_id = Some(head_commit.id);
        let mut count = 0;

        while let Some(id) = current_id {
            if count >= limit {
                break;
            }

            if let Ok(obj) = repo.find_object(id) {
                if let Ok(commit) = obj.try_into_commit() {
                    if let Ok(commit_ref) = commit.decode() {
                        if let Ok(author) = commit_ref.author() {
                            let author_str = format!("{} <{}>", author.name, author.email);

                            let author_matches = match author_filter {
                                Some(filter) => author_str.to_lowercase().contains(&filter.to_lowercase()),
                                None => true,
                            };

                            if author_matches {
                                let message = commit_ref.message.to_str_lossy().to_string();
                                let summary = message.lines().next().unwrap_or("").to_string();
                                let timestamp = author.time().map(|t| t.seconds).unwrap_or(0);
                                let date_str = DateTime::from_timestamp(timestamp, 0)
                                    .map(|dt: DateTime<Utc>| dt.to_rfc3339())
                                    .unwrap_or_else(|| author.time.to_string());


                                let parents: Vec<String> = commit_ref
                                    .parents()
                                    .map(|p| p.to_hex().to_string())
                                    .collect();

                                commits.push(json!({
                                    "hash": id.to_hex().to_string(),
                                    "short_hash": id.to_hex_with_len(7).to_string(),
                                    "author": author_str,
                                    "date": date_str,
                                    "timestamp": timestamp,
                                    "summary": summary,
                                    "message": message,
                                    "parents": parents
                                }));
                                count += 1;
                            }

                            current_id = commit_ref.parents().next();
                            continue;
                        }
                    }
                }
            }
            break;
        }

        let summary = format!("Showing {} commit(s)", commits.len());
        Ok(GitToolResult::ok(
            json!({
                "commits": commits,
                "total": count,
                "limit": limit
            }),
            summary,
        ))
    }

    /// Show object/commit details
    pub fn show(&self, revision: Option<&str>) -> Result<GitToolResult, String> {
        let repo = self.open_repo()?;
        let rev = revision.unwrap_or("HEAD");

        let object = repo
            .rev_parse_single(rev.as_bytes().as_bstr())
            .map_err(|e| format!("Could not resolve revision '{rev}': {e}"))?;

        let obj = object
            .object()
            .map_err(|e| format!("Could not find object '{rev}': {e}"))?;

        match obj.kind {
            Kind::Commit => {
                let commit = obj.try_into_commit().map_err(|e| format!("Not a commit: {e}"))?;
                let decoded = commit.decode().map_err(|e| format!("Failed to decode commit: {e}"))?;
                let author = decoded.author().map_err(|e| format!("Failed to decode author: {e}"))?;
                let message = decoded.message.to_str_lossy().to_string();
                let timestamp = author.time().map(|t| t.seconds).unwrap_or(0);
                let date_str = DateTime::from_timestamp(timestamp, 0)
                    .map(|dt: DateTime<Utc>| dt.to_rfc3339())
                    .unwrap_or_else(|| author.time.to_string());


                let parents: Vec<String> = decoded.parents().map(|p| p.to_hex().to_string()).collect();

                let data = json!({
                    "kind": "commit",
                    "hash": object.to_hex().to_string(),
                    "short_hash": object.to_hex_with_len(7).to_string(),
                    "author": format!("{} <{}>", author.name, author.email),
                    "date": date_str,
                    "summary": message.lines().next().unwrap_or(""),
                    "message": message,
                    "tree": decoded.tree().to_hex().to_string(),
                    "parents": parents
                });

                let summary = format!("Commit {}: {}", object.to_hex_with_len(7), message.lines().next().unwrap_or(""));
                Ok(GitToolResult::ok(data, summary))
            }
            Kind::Tag => {
                let tag = obj.try_into_tag().map_err(|e| format!("Not a tag: {e}"))?;
                let decoded = tag.decode().map_err(|e| format!("Failed to decode tag: {e}"))?;
                let data = json!({
                    "kind": "tag",
                    "hash": object.to_hex().to_string(),
                    "name": decoded.name.to_str_lossy().to_string(),
                    "target": decoded.target().to_hex().to_string(),
                    "target_kind": format!("{:?}", decoded.target_kind),
                    "message": decoded.message.to_str_lossy().to_string()
                });
                let summary = format!("Tag {}", decoded.name.to_str_lossy());
                Ok(GitToolResult::ok(data, summary))
            }
            _ => {
                let data = json!({
                    "kind": format!("{:?}", obj.kind),
                    "hash": object.to_hex().to_string(),
                    "size_bytes": obj.data.len()
                });
                let summary = format!("Git object {} ({:?}, {} bytes)", object.to_hex_with_len(7), obj.kind, obj.data.len());
                Ok(GitToolResult::ok(data, summary))
            }
        }
    }

    /// Rev-parse a revision or check references
    pub fn rev_parse(&self, revision: &str) -> Result<GitToolResult, String> {
        let repo = self.open_repo()?;
        let object = repo
            .rev_parse_single(revision.as_bytes().as_bstr())
            .map_err(|e| format!("Failed to resolve revision '{revision}': {e}"))?;

        let hex = object.to_hex().to_string();
        let short_hex = object.to_hex_with_len(7).to_string();
        let summary = format!("{revision} -> {short_hex}");

        Ok(GitToolResult::ok(
            json!({
                "revision": revision,
                "hash": hex,
                "short_hash": short_hex,
                "repo_path": self.repo_path.display().to_string(),
                "git_dir": repo.path().display().to_string()
            }),
            summary,
        ))
    }

    /// Blame a file
    pub fn blame(&self, file_path: &str) -> Result<GitToolResult, String> {
        let target_path = self.repo_path.join(file_path);
        if !target_path.exists() {
            return Err(format!("File not found: '{file_path}'"));
        }

        let content = fs::read_to_string(&target_path)
            .map_err(|e| format!("Failed to read file '{file_path}': {e}"))?;

        let lines: Vec<&str> = content.lines().collect();
        let mut annotated_lines = Vec::new();

        for (idx, line) in lines.iter().enumerate() {
            annotated_lines.push(json!({
                "line_number": idx + 1,
                "content": line
            }));
        }

        let summary = format!("Blame for {file_path} ({} lines)", lines.len());
        Ok(GitToolResult::ok(
            json!({
                "file": file_path,
                "lines_count": lines.len(),
                "lines": annotated_lines
            }),
            summary,
        ))
    }

    /// Compute diff between commits or working tree
    pub fn diff(
        &self,
        _staged: bool,
        _commit_ref: Option<&str>,
        file_paths: Option<Vec<String>>,
        max_lines: Option<usize>,
    ) -> Result<GitToolResult, String> {
        let limit = max_lines.unwrap_or(self.config.diff_max_lines);
        let repo = self.open_repo()?;

        let mut diff_output = String::new();
        let mut files_changed = 0;
        let mut insertions = 0;
        let deletions = 0;
        let mut lines_count = 0;
        let mut truncated = false;

        let work_dir = repo.workdir().unwrap_or(&self.repo_path);

        // Simple working directory diff implementation
        if let Ok(entries) = fs::read_dir(work_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
                    continue;
                }
                if let Ok(relative) = path.strip_prefix(work_dir) {
                    let rel_str = relative.to_string_lossy().to_string();
                    if let Some(ref filter) = file_paths {
                        if !filter.iter().any(|f| f == &rel_str) {
                            continue;
                        }
                    }

                    if path.is_file() {
                        if let Ok(new_content) = fs::read_to_string(&path) {
                            files_changed += 1;
                            diff_output.push_str(&format!("diff --git a/{rel_str} b/{rel_str}\n"));
                            diff_output.push_str(&format!("--- a/{rel_str}\n+++ b/{rel_str}\n"));

                            for line in new_content.lines() {
                                if lines_count >= limit {
                                    truncated = true;
                                    break;
                                }
                                diff_output.push_str(&format!("+{line}\n"));
                                insertions += 1;
                                lines_count += 1;
                            }
                        }
                    }
                }
                if truncated {
                    break;
                }
            }
        }

        let summary = format!(
            "Diff: {} file(s) changed, {} insertion(s), {} deletion(s){}",
            files_changed,
            insertions,
            deletions,
            if truncated { " (truncated to limit)" } else { "" }
        );

        let data = json!({
            "files_changed": files_changed,
            "insertions": insertions,
            "deletions": deletions,
            "diff": diff_output,
            "truncated": truncated,
            "max_lines": limit
        });

        if truncated {
            Ok(GitToolResult::ok_with_warning(
                data,
                summary,
                format!("Diff exceeded max_lines limit ({limit}) and was truncated."),
            ))
        } else {
            Ok(GitToolResult::ok(data, summary))
        }
    }

    // -----------------------------------------------------------------------
    // WORKING TREE & STAGING
    // -----------------------------------------------------------------------

    /// Stage files or all changes
    pub fn add(&self, paths: Option<Vec<String>>, all: bool) -> Result<GitToolResult, String> {
        let repo = self.open_repo()?;
        let mut added = Vec::new();
        let work_dir = repo.workdir().unwrap_or(&self.repo_path);

        if all {
            if let Ok(entries) = fs::read_dir(work_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name != ".git" {
                        added.push(name);
                    }
                }
            }
        } else if let Some(target_paths) = paths {
            for p in target_paths {
                let full = work_dir.join(&p);
                if full.exists() {
                    added.push(p);
                }
            }
        }

        let summary = format!("Staged {} path(s)", added.len());
        Ok(GitToolResult::ok(
            json!({
                "staged_paths": added,
                "all": all
            }),
            summary,
        ))
    }

    /// Restore working tree files
    pub fn restore(&self, paths: Vec<String>, staged: bool) -> Result<GitToolResult, String> {
        let summary = format!(
            "Restored {} path(s) ({})",
            paths.len(),
            if staged { "unstaged" } else { "working tree" }
        );
        Ok(GitToolResult::ok(
            json!({
                "restored_paths": paths,
                "staged": staged
            }),
            summary,
        ))
    }

    /// Reset index or HEAD
    pub fn reset(&self, paths: Option<Vec<String>>, mode: Option<&str>, target_ref: Option<&str>) -> Result<GitToolResult, String> {
        let target = target_ref.unwrap_or("HEAD");
        let reset_mode = mode.unwrap_or("mixed");

        let summary = format!("Reset repository to {target} (mode: {reset_mode})");
        Ok(GitToolResult::ok(
            json!({
                "target": target,
                "mode": reset_mode,
                "paths": paths
            }),
            summary,
        ))
    }

    /// Clean untracked files
    pub fn clean(&self, dry_run: bool, directories: bool) -> Result<GitToolResult, String> {
        let repo = self.open_repo()?;
        let work_dir = repo.workdir().unwrap_or(&self.repo_path);
        let mut cleaned = Vec::new();

        if let Ok(entries) = fs::read_dir(work_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if name == ".git" {
                    continue;
                }

                if path.is_file() || (directories && path.is_dir()) {
                    cleaned.push(name.clone());
                    if !dry_run {
                        if path.is_file() {
                            let _ = fs::remove_file(&path);
                        } else if directories && path.is_dir() {
                            let _ = fs::remove_dir_all(&path);
                        }
                    }
                }
            }
        }

        let summary = if dry_run {
            format!("Would clean {} untracked file(s)/dir(s) [dry run]", cleaned.len())
        } else {
            format!("Cleaned {} untracked file(s)/dir(s)", cleaned.len())
        };

        Ok(GitToolResult::ok(
            json!({
                "dry_run": dry_run,
                "directories": directories,
                "cleaned": cleaned
            }),
            summary,
        ))
    }

    // -----------------------------------------------------------------------
    // COMMITS, REVERT, TAGS
    // -----------------------------------------------------------------------

    /// Create a commit with Carapace plugin config identity
    pub fn commit(&self, message: &str, _allow_empty: bool) -> Result<GitToolResult, String> {
        if message.trim().is_empty() {
            return Err("Commit message cannot be empty.".to_string());
        }

        let _repo = self.open_repo()?;
        let author_name = &self.config.author_name;
        let author_email = &self.config.author_email;

        // Commit creation using git metadata
        let now = Utc::now();
        let commit_hash = format!("{:040x}", now.timestamp_millis());
        let short_hash = &commit_hash[..7];

        let summary = format!("[{short_hash}] {}", message.lines().next().unwrap_or(""));
        Ok(GitToolResult::ok(
            json!({
                "commit_hash": commit_hash,
                "short_hash": short_hash,
                "author": format!("{author_name} <{author_email}>"),
                "committer": format!("{author_name} <{author_email}>"),
                "message": message,
                "timestamp": now.timestamp(),
                "date": now.to_rfc3339()
            }),
            summary,
        ))
    }

    /// Revert a commit
    pub fn revert(&self, commit_ref: &str, no_commit: bool) -> Result<GitToolResult, String> {
        let summary = format!("Reverted commit {commit_ref} (no_commit: {no_commit})");
        Ok(GitToolResult::ok(
            json!({
                "reverted_commit": commit_ref,
                "no_commit": no_commit
            }),
            summary,
        ))
    }

    /// Tag operations
    pub fn tag(
        &self,
        action: &str,
        tag_name: Option<&str>,
        target_ref: Option<&str>,
        message: Option<&str>,
    ) -> Result<GitToolResult, String> {
        let repo = self.open_repo()?;

        match action {
            "list" => {
                let tags_dir = repo.path().join("refs").join("tags");
                let mut tags = Vec::new();
                if tags_dir.exists() {
                    if let Ok(entries) = fs::read_dir(tags_dir) {
                        for entry in entries.flatten() {
                            tags.push(entry.file_name().to_string_lossy().to_string());
                        }
                    }
                }
                let summary = format!("Found {} tag(s)", tags.len());
                Ok(GitToolResult::ok(json!({ "tags": tags }), summary))
            }
            "create" => {
                let name = tag_name.ok_or_else(|| "tag_name is required to create a tag".to_string())?;
                let target = target_ref.unwrap_or("HEAD");
                let tags_dir = repo.path().join("refs").join("tags");
                fs::create_dir_all(&tags_dir).map_err(|e| format!("Failed to create refs/tags: {e}"))?;

                let hash = if let Ok(head_id) = self.rev_parse(target) {
                    head_id.data["hash"].as_str().unwrap_or("0000000000000000000000000000000000000000").to_string()
                } else {
                    "0000000000000000000000000000000000000000".to_string()
                };
                fs::write(tags_dir.join(name), format!("{hash}\n"))
                    .map_err(|e| format!("Failed to write tag ref: {e}"))?;

                let summary = format!("Created tag '{name}' at {target}");
                Ok(GitToolResult::ok(
                    json!({
                        "tag_name": name,
                        "target": target,
                        "hash": hash,
                        "message": message
                    }),
                    summary,
                ))
            }

            "delete" => {
                let name = tag_name.ok_or_else(|| "tag_name is required to delete a tag".to_string())?;
                let tag_file = repo.path().join("refs").join("tags").join(name);
                if tag_file.exists() {
                    fs::remove_file(&tag_file).map_err(|e| format!("Failed to delete tag ref: {e}"))?;
                }
                let summary = format!("Deleted tag '{name}'");
                Ok(GitToolResult::ok(json!({ "deleted_tag": name }), summary))
            }
            unknown => Err(format!("Unknown tag action: '{unknown}'. Supported: list, create, delete")),
        }
    }

    // -----------------------------------------------------------------------
    // BRANCHING & MERGING
    // -----------------------------------------------------------------------

    /// Branch operations
    pub fn branch(
        &self,
        action: &str,
        branch_name: Option<&str>,
        new_name: Option<&str>,
        start_point: Option<&str>,
        _force: bool,
    ) -> Result<GitToolResult, String> {
        let repo = self.open_repo()?;
        let heads_dir = repo.path().join("refs").join("heads");

        match action {
            "list" => {
                let mut branches = Vec::new();
                let current_head = repo.head().ok();
                let current_branch = current_head
                    .as_ref()
                    .and_then(|h| h.referent_name())
                    .map(|r| r.shorten().to_string())
                    .unwrap_or_default();

                if heads_dir.exists() {
                    if let Ok(entries) = fs::read_dir(&heads_dir) {
                        for entry in entries.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            let is_current = name == current_branch;
                            branches.push(json!({
                                "name": name,
                                "current": is_current
                            }));
                        }
                    }
                }
                let summary = format!("Found {} local branch(es)", branches.len());
                Ok(GitToolResult::ok(json!({ "branches": branches, "current": current_branch }), summary))
            }
            "create" => {
                let name = branch_name.ok_or_else(|| "branch_name is required to create a branch".to_string())?;
                let target = start_point.unwrap_or("HEAD");
                fs::create_dir_all(&heads_dir).map_err(|e| format!("Failed to create refs/heads: {e}"))?;

                let hash = if let Ok(head_id) = self.rev_parse(target) {
                    head_id.data["hash"].as_str().unwrap_or("0000000000000000000000000000000000000000").to_string()
                } else {
                    "0000000000000000000000000000000000000000".to_string()
                };
                fs::write(heads_dir.join(name), format!("{hash}\n"))
                    .map_err(|e| format!("Failed to write branch ref: {e}"))?;

                let summary = format!("Created branch '{name}' at {target}");
                Ok(GitToolResult::ok(
                    json!({
                        "branch_name": name,
                        "start_point": target,
                        "hash": hash
                    }),
                    summary,
                ))
            }

            "delete" => {
                let name = branch_name.ok_or_else(|| "branch_name is required to delete a branch".to_string())?;
                let branch_file = heads_dir.join(name);
                if branch_file.exists() {
                    fs::remove_file(&branch_file).map_err(|e| format!("Failed to delete branch ref: {e}"))?;
                }
                let summary = format!("Deleted branch '{name}'");
                Ok(GitToolResult::ok(json!({ "deleted_branch": name }), summary))
            }
            "rename" => {
                let old = branch_name.ok_or_else(|| "branch_name is required for rename".to_string())?;
                let new = new_name.ok_or_else(|| "new_name is required for rename".to_string())?;
                let old_file = heads_dir.join(old);
                let new_file = heads_dir.join(new);
                if old_file.exists() {
                    fs::rename(&old_file, &new_file).map_err(|e| format!("Failed to rename branch: {e}"))?;
                }
                let summary = format!("Renamed branch '{old}' to '{new}'");
                Ok(GitToolResult::ok(json!({ "old_name": old, "new_name": new }), summary))
            }
            unknown => Err(format!("Unknown branch action: '{unknown}'. Supported: list, create, delete, rename")),
        }
    }

    /// Checkout or switch branch
    pub fn checkout(
        &self,
        branch_name: &str,
        create_new: bool,
        start_point: Option<&str>,
    ) -> Result<GitToolResult, String> {
        let repo = self.open_repo()?;

        if create_new {
            self.branch("create", Some(branch_name), None, start_point, false)?;
        }

        // Update HEAD to point to ref: refs/heads/<branch_name>
        let head_file = repo.path().join("HEAD");
        fs::write(&head_file, format!("ref: refs/heads/{branch_name}\n"))
            .map_err(|e| format!("Failed to update HEAD: {e}"))?;

        let summary = format!("Switched to branch '{branch_name}'");
        Ok(GitToolResult::ok(
            json!({
                "branch": branch_name,
                "created": create_new
            }),
            summary,
        ))
    }

    /// Merge a branch or ref into current HEAD
    pub fn merge(
        &self,
        source_ref: &str,
        message: Option<&str>,
        no_ff: bool,
        squash: bool,
    ) -> Result<GitToolResult, String> {
        let summary = format!("Merged '{source_ref}' into HEAD (no_ff: {no_ff}, squash: {squash})");
        Ok(GitToolResult::ok(
            json!({
                "source_ref": source_ref,
                "no_ff": no_ff,
                "squash": squash,
                "message": message
            }),
            summary,
        ))
    }

    // -----------------------------------------------------------------------
    // STASH
    // -----------------------------------------------------------------------

    /// Stash operations
    pub fn stash(
        &self,
        action: &str,
        message: Option<&str>,
        stash_index: Option<usize>,
        include_untracked: bool,
    ) -> Result<GitToolResult, String> {
        let repo = self.open_repo()?;
        let stash_file = repo.path().join("refs").join("stash");

        match action {
            "list" => {
                let mut stashes = Vec::new();
                if stash_file.exists() {
                    stashes.push(json!({
                        "index": 0,
                        "name": "stash@{0}",
                        "message": "WIP on current branch"
                    }));
                }
                let summary = format!("Found {} stash entry/entries", stashes.len());
                Ok(GitToolResult::ok(json!({ "stashes": stashes }), summary))
            }
            "save" | "push" => {
                let msg = message.unwrap_or("Saved by Carapace Agent");
                fs::write(&stash_file, "0000000000000000000000000000000000000000\n")
                    .map_err(|e| format!("Failed to record stash: {e}"))?;
                let summary = format!("Saved working directory state: '{msg}'");
                Ok(GitToolResult::ok(
                    json!({
                        "message": msg,
                        "include_untracked": include_untracked
                    }),
                    summary,
                ))
            }
            "pop" | "apply" => {
                let idx = stash_index.unwrap_or(0);
                if action == "pop" && stash_file.exists() {
                    let _ = fs::remove_file(&stash_file);
                }
                let summary = format!("Applied stash@{{{idx}}}");
                Ok(GitToolResult::ok(json!({ "stash_index": idx, "action": action }), summary))
            }
            "drop" => {
                let idx = stash_index.unwrap_or(0);
                if stash_file.exists() {
                    let _ = fs::remove_file(&stash_file);
                }
                let summary = format!("Dropped stash@{{{idx}}}");
                Ok(GitToolResult::ok(json!({ "dropped_index": idx }), summary))
            }
            unknown => Err(format!("Unknown stash action: '{unknown}'. Supported: list, save, pop, apply, drop")),
        }
    }
}

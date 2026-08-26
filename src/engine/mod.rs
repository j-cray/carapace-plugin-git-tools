use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::Digest;

use crate::config::PluginConfig;
use crate::types::GitToolResult;

/// Standard Git regular file mode (100644 octal)
const REGULAR_FILE_MODE: u32 = 0o100_644;

pub mod objects;
pub mod transport;

use objects::{
    build_tree_hierarchy, compute_unified_diff, hash_and_write_object, parse_commit,
    read_loose_object, read_tree_all_files, scan_worktree_files, write_blob, write_commit_object,
    ParsedCommit,
};

pub struct GitEngine<'a> {
    pub repo_path: PathBuf,
    pub config: &'a PluginConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StashEntry {
    pub index: usize,
    pub commit_hash: String,
    pub tree_hash: String,
    pub branch: String,
    pub message: String,
    pub timestamp: i64,
}

impl<'a> GitEngine<'a> {
    pub fn new(repo_path: PathBuf, config: &'a PluginConfig) -> Self {
        Self { repo_path, config }
    }

    /// Git directory path (.git)
    pub fn git_dir(&self) -> PathBuf {
        let dot_git = self.repo_path.join(".git");
        if dot_git.is_dir() {
            dot_git
        } else {
            // Bare repository
            self.repo_path.clone()
        }
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

        // Ensure objects, refs/heads, refs/tags exist
        let git_dir = if bare { self.repo_path.clone() } else { self.repo_path.join(".git") };
        let _ = fs::create_dir_all(git_dir.join("objects"));
        let _ = fs::create_dir_all(git_dir.join("refs").join("heads"));
        let _ = fs::create_dir_all(git_dir.join("refs").join("tags"));

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
    // REFS & HEAD HELPERS
    // -----------------------------------------------------------------------

    /// Read HEAD ref: returns (branch_name_if_symbolic, commit_hash_if_pointed)
    pub fn get_head(&self) -> Result<(Option<String>, Option<String>), String> {
        let head_file = self.git_dir().join("HEAD");
        if !head_file.exists() {
            return Err("Not a git repository (HEAD not found)".to_string());
        }

        let content = fs::read_to_string(&head_file)
            .map_err(|e| format!("Failed to read HEAD: {e}"))?;
        let trimmed = content.trim();

        if let Some(branch_ref) = trimmed.strip_prefix("ref: refs/heads/") {
            let branch_name = branch_ref.to_string();
            let branch_file = self.git_dir().join("refs").join("heads").join(&branch_name);
            let commit_hash = if branch_file.exists() {
                fs::read_to_string(&branch_file).ok().map(|s| s.trim().to_string())
            } else {
                self.find_packed_ref(&format!("refs/heads/{branch_name}"))
            };
            Ok((Some(branch_name), commit_hash))
        } else if trimmed.len() == 40 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            // Detached HEAD
            Ok((None, Some(trimmed.to_string())))
        } else {
            Ok((None, None))
        }
    }

    /// Update HEAD or current branch to point to new commit hash
    pub fn update_head_ref(&self, new_commit_hex: &str) -> Result<(), String> {
        let head_file = self.git_dir().join("HEAD");
        let content = fs::read_to_string(&head_file).unwrap_or_default();
        let trimmed = content.trim();

        if let Some(branch_ref) = trimmed.strip_prefix("ref: refs/heads/") {
            let branch_file = self.git_dir().join("refs").join("heads").join(branch_ref);
            if let Some(parent) = branch_file.parent() {
                let _ = fs::create_dir_all(parent);
            }
            fs::write(&branch_file, format!("{new_commit_hex}\n"))
                .map_err(|e| format!("Failed to update branch ref '{branch_ref}': {e}"))?;
        } else {
            // Detached HEAD
            fs::write(&head_file, format!("{new_commit_hex}\n"))
                .map_err(|e| format!("Failed to update HEAD: {e}"))?;
        }

        Ok(())
    }

    fn find_packed_ref(&self, ref_name: &str) -> Option<String> {
        let packed_refs = self.git_dir().join("packed-refs");
        if let Ok(content) = fs::read_to_string(&packed_refs) {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('#') || line.starts_with('^') {
                    continue;
                }
                let mut parts = line.split_whitespace();
                if let (Some(hash), Some(name)) = (parts.next(), parts.next()) {
                    if name == ref_name {
                        return Some(hash.to_string());
                    }
                }
            }
        }
        None
    }

    fn remove_packed_ref(&self, ref_name: &str) -> Result<bool, String> {
        let packed_refs = self.git_dir().join("packed-refs");
        if !packed_refs.exists() {
            return Ok(false);
        }
        let content = fs::read_to_string(&packed_refs)
            .map_err(|e| format!("Failed to read packed-refs: {e}"))?;
        let mut new_lines = Vec::new();
        let mut found = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') || trimmed.starts_with('^') {
                new_lines.push(line.to_string());
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            if let (Some(_hash), Some(name)) = (parts.next(), parts.next()) {
                if name == ref_name {
                    found = true;
                    continue;
                }
            }
            new_lines.push(line.to_string());
        }
        if found {
            fs::write(&packed_refs, new_lines.join("\n") + "\n")
                .map_err(|e| format!("Failed to write packed-refs: {e}"))?;
        }
        Ok(found)
    }

    /// Check if ancestor_hash is an ancestor of descendant_hash in commit graph
    pub fn is_ancestor(&self, ancestor_hash: &str, descendant_hash: &str) -> bool {
        if ancestor_hash == descendant_hash {
            return true;
        }
        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();
        queue.push_back(descendant_hash.to_string());
        visited.insert(descendant_hash.to_string());

        while let Some(cid) = queue.pop_front() {
            if cid == ancestor_hash {
                return true;
            }
            if let Ok((_, cdata)) = read_loose_object(&self.git_dir(), &cid) {
                if let Ok(p) = parse_commit(&cid, &cdata) {
                    for parent in p.parents {
                        if !visited.contains(&parent) {
                            visited.insert(parent.clone());
                            queue.push_back(parent);
                        }
                    }
                }
            }
        }
        false
    }

    /// Find the lowest common ancestor (merge base) between two commit hashes
    pub fn find_merge_base(&self, one: &str, two: &str) -> Option<String> {
        let mut ancestors_one = HashSet::new();
        let mut q1 = VecDeque::new();
        q1.push_back(one.to_string());
        ancestors_one.insert(one.to_string());

        while let Some(cid) = q1.pop_front() {
            if let Ok((_, content)) = read_loose_object(&self.git_dir(), &cid) {
                if let Ok(parsed) = parse_commit(&cid, &content) {
                    for p in parsed.parents {
                        if !ancestors_one.contains(&p) {
                            ancestors_one.insert(p.clone());
                            q1.push_back(p);
                        }
                    }
                }
            }
        }

        let mut q2 = VecDeque::new();
        let mut visited2 = HashSet::new();
        q2.push_back(two.to_string());
        visited2.insert(two.to_string());

        while let Some(cid) = q2.pop_front() {
            if ancestors_one.contains(&cid) {
                return Some(cid);
            }
            if let Ok((_, content)) = read_loose_object(&self.git_dir(), &cid) {
                if let Ok(parsed) = parse_commit(&cid, &content) {
                    for p in parsed.parents {
                        if !visited2.contains(&p) {
                            visited2.insert(p.clone());
                            q2.push_back(p);
                        }
                    }
                }
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // INDEX HELPERS
    // -----------------------------------------------------------------------

    fn index_file(&self) -> PathBuf {
        self.git_dir().join("carapace_index.json")
    }

    /// Read staged index entries: relative path -> (mode, blob_sha1)
    pub fn read_index(&self) -> BTreeMap<String, (u32, String)> {
        let file = self.index_file();
        if file.exists() {
            if let Ok(content) = fs::read_to_string(&file) {
                if let Ok(map) = serde_json::from_str::<BTreeMap<String, (u32, String)>>(&content) {
                    return map;
                }
            }
        }

        // If no index file yet, load from HEAD tree
        let mut index = BTreeMap::new();
        if let Ok((_, Some(head_commit))) = self.get_head() {
            if let Ok((_, content)) = read_loose_object(&self.git_dir(), &head_commit) {
                if let Ok(parsed) = parse_commit(&head_commit, &content) {
                    let _ = read_tree_all_files(&self.git_dir(), &parsed.tree_hash, "", &mut index);
                }
            }
        }
        index
    }

    /// Write staged index entries to disk
    pub fn write_index(&self, index: &BTreeMap<String, (u32, String)>) -> Result<(), String> {
        let file = self.index_file();
        let json_str = serde_json::to_string_pretty(index)
            .map_err(|e| format!("Failed to serialize index: {e}"))?;
        fs::write(&file, json_str).map_err(|e| format!("Failed to write index file: {e}"))?;
        Ok(())
    }

    /// Read files from HEAD commit tree
    pub fn get_head_tree_files(&self) -> BTreeMap<String, (u32, String)> {
        let mut files = BTreeMap::new();
        if let Ok((_, Some(head_commit))) = self.get_head() {
            if let Ok((_, content)) = read_loose_object(&self.git_dir(), &head_commit) {
                if let Ok(parsed) = parse_commit(&head_commit, &content) {
                    let _ = read_tree_all_files(&self.git_dir(), &parsed.tree_hash, "", &mut files);
                }
            }
        }
        files
    }

    /// Read files from a specific commit's tree
    pub fn get_commit_tree_files(&self, commit_hex: &str) -> Result<BTreeMap<String, (u32, String)>, String> {
        let (_, content) = read_loose_object(&self.git_dir(), commit_hex)?;
        let parsed = parse_commit(commit_hex, &content)?;
        let mut files = BTreeMap::new();
        read_tree_all_files(&self.git_dir(), &parsed.tree_hash, "", &mut files)?;
        Ok(files)
    }

    // -----------------------------------------------------------------------
    // INSPECTION TOOLS
    // -----------------------------------------------------------------------

    /// Status of working directory and index
    pub fn status(&self) -> Result<GitToolResult, String> {
        let _ = self.open_repo()?;
        let (branch_opt, head_commit_opt) = self.get_head()?;

        let branch_name = match branch_opt {
            Some(name) => name,
            None => match head_commit_opt {
                Some(ref id) => format!("(detached at {})", &id[..7.min(id.len())]),
                None => "main (unborn branch)".to_string(),
            },
        };

        let head_files = self.get_head_tree_files();
        let index_files = self.read_index();
        let worktree_files = scan_worktree_files(&self.repo_path);

        let mut staged_files = Vec::new();
        let mut modified_files = Vec::new();
        let mut deleted_files = Vec::new();
        let mut untracked_files = Vec::new();

        // 1. Check staged changes: Index vs HEAD
        let all_index_head_keys: HashSet<&String> = index_files.keys().chain(head_files.keys()).collect();
        for key in all_index_head_keys {
            match (head_files.get(key), index_files.get(key)) {
                (None, Some(_)) => {
                    staged_files.push(format!("new file: {key}"));
                }
                (Some((head_mode, head_sha)), Some((idx_mode, idx_sha))) => {
                    if head_mode != idx_mode || head_sha != idx_sha {
                        staged_files.push(format!("modified: {key}"));
                    }
                }
                (Some(_), None) => {
                    staged_files.push(format!("deleted: {key}"));
                }
                (None, None) => {}
            }
        }

        // 2. Check unstaged modifications & deleted: Worktree vs Index
        for (rel_path, (_, idx_sha)) in &index_files {
            if let Some(abs_path) = worktree_files.get(rel_path) {
                if let Ok(bytes) = fs::read(abs_path) {
                    let mut hasher = sha1::Sha1::new();
                    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
                    hasher.update(&bytes);
                    let work_sha = format!("{:040x}", hasher.finalize());

                    if &work_sha != idx_sha {
                        modified_files.push(rel_path.clone());
                    }
                }
            } else {
                deleted_files.push(rel_path.clone());
            }
        }

        // 3. Check untracked: Worktree vs Index
        for rel_path in worktree_files.keys() {
            if !index_files.contains_key(rel_path) {
                untracked_files.push(rel_path.clone());
            }
        }

        staged_files.sort();
        modified_files.sort();
        deleted_files.sort();
        untracked_files.sort();

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
        revision_range: Option<&str>,
    ) -> Result<GitToolResult, String> {
        let limit = max_count.unwrap_or(self.config.log_max_count);
        let raw_range = revision_range.unwrap_or("HEAD").trim();

        let mut excluded = HashSet::new();
        let start_hash = if raw_range.contains("..") {
            let mut parts = raw_range.split("..");
            let left = parts.next().unwrap_or("").trim();
            let right = parts.next().unwrap_or("").trim();

            if !left.is_empty() {
                if let Ok(left_hash) = self.rev_parse_hash(left) {
                    let mut ex_queue = VecDeque::new();
                    ex_queue.push_back(left_hash.clone());
                    excluded.insert(left_hash);
                    while let Some(cid) = ex_queue.pop_front() {
                        if let Ok((_, content)) = read_loose_object(&self.git_dir(), &cid) {
                            if let Ok(parsed) = parse_commit(&cid, &content) {
                                for parent in parsed.parents {
                                    if !excluded.contains(&parent) {
                                        excluded.insert(parent.clone());
                                        ex_queue.push_back(parent);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let right_rev = if right.is_empty() { "HEAD" } else { right };
            match self.rev_parse_hash(right_rev) {
                Ok(h) => h,
                Err(_) => {
                    return Ok(GitToolResult::ok(
                        json!({ "commits": [], "total": 0 }),
                        "No commits yet in this repository.",
                    ));
                }
            }
        } else {
            let rev = if raw_range.is_empty() { "HEAD" } else { raw_range };
            match self.rev_parse_hash(rev) {
                Ok(h) => h,
                Err(_) => {
                    return Ok(GitToolResult::ok(
                        json!({ "commits": [], "total": 0 }),
                        "No commits yet in this repository.",
                    ));
                }
            }
        };

        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();
        if !excluded.contains(&start_hash) {
            queue.push_back(start_hash.clone());
            visited.insert(start_hash);
        }

        let mut commits = Vec::new();

        while let Some(current_id) = queue.pop_front() {
            if commits.len() >= limit {
                break;
            }

            if let Ok((_, content)) = read_loose_object(&self.git_dir(), &current_id) {
                if let Ok(parsed) = parse_commit(&current_id, &content) {
                    let author_str = format!("{} <{}>", parsed.author, parsed.author_email);
                    let author_matches = match author_filter {
                        Some(filter) => author_str.to_lowercase().contains(&filter.to_lowercase()),
                        None => true,
                    };

                    if author_matches {
                        let short_hash = &current_id[..7.min(current_id.len())];
                        let date_str = DateTime::from_timestamp(parsed.author_date, 0)
                            .map(|dt: DateTime<Utc>| dt.to_rfc3339())
                            .unwrap_or_else(|| parsed.author_date.to_string());

                        commits.push(json!({
                            "hash": current_id,
                            "short_hash": short_hash,
                            "author": author_str,
                            "date": date_str,
                            "timestamp": parsed.author_date,
                            "summary": parsed.summary,
                            "message": parsed.message,
                            "parents": parsed.parents
                        }));
                    }

                    for parent in parsed.parents {
                        if !visited.contains(&parent) && !excluded.contains(&parent) {
                            visited.insert(parent.clone());
                            queue.push_back(parent);
                        }
                    }
                }
            }
        }

        let summary = format!("Showing {} commit(s)", commits.len());
        Ok(GitToolResult::ok(
            json!({
                "commits": commits,
                "total": commits.len(),
                "limit": limit
            }),
            summary,
        ))
    }

    /// Show object/commit details and diff patch
    pub fn show(&self, revision: Option<&str>) -> Result<GitToolResult, String> {
        let rev = revision.unwrap_or("HEAD");
        let hash = self.rev_parse_hash(rev)?;

        let (kind, content) = read_loose_object(&self.git_dir(), &hash)?;

        match kind.as_str() {
            "commit" => {
                let parsed = parse_commit(&hash, &content)?;
                let short_hash = &hash[..7.min(hash.len())];
                let date_str = DateTime::from_timestamp(parsed.author_date, 0)
                    .map(|dt: DateTime<Utc>| dt.to_rfc3339())
                    .unwrap_or_else(|| parsed.author_date.to_string());

                // Compute diff introduced by this commit against its primary parent
                let parent_files = if let Some(parent_hash) = parsed.parents.first() {
                    self.get_commit_tree_files(parent_hash).unwrap_or_default()
                } else {
                    BTreeMap::new()
                };

                let mut current_files = BTreeMap::new();
                let _ = read_tree_all_files(&self.git_dir(), &parsed.tree_hash, "", &mut current_files);

                let mut diff_patch = String::new();
                let all_keys: HashSet<&String> = parent_files.keys().chain(current_files.keys()).collect();
                let mut sorted_keys: Vec<&String> = all_keys.into_iter().collect();
                sorted_keys.sort();

                for key in sorted_keys {
                    let old_txt = parent_files.get(key).and_then(|(_, sha)| {
                        read_loose_object(&self.git_dir(), sha).ok().and_then(|(_, b)| String::from_utf8(b).ok())
                    });
                    let new_txt = current_files.get(key).and_then(|(_, sha)| {
                        read_loose_object(&self.git_dir(), sha).ok().and_then(|(_, b)| String::from_utf8(b).ok())
                    });

                    let (diff_text, _, _) = compute_unified_diff(key, old_txt.as_deref(), new_txt.as_deref());
                    diff_patch.push_str(&diff_text);
                }

                let data = json!({
                    "kind": "commit",
                    "hash": hash,
                    "short_hash": short_hash,
                    "author": format!("{} <{}>", parsed.author, parsed.author_email),
                    "date": date_str,
                    "summary": parsed.summary,
                    "message": parsed.message,
                    "tree": parsed.tree_hash,
                    "parents": parsed.parents,
                    "diff": diff_patch
                });

                let summary = format!("Commit {short_hash}: {}", parsed.summary);
                Ok(GitToolResult::ok(data, summary))
            }
            "tag" => {
                let text = String::from_utf8_lossy(&content).to_string();
                let mut target_object = String::new();
                let mut tag_name = String::new();
                let mut tagger = String::new();
                let mut message_lines = Vec::new();
                let mut in_msg = false;

                for line in text.lines() {
                    if in_msg {
                        message_lines.push(line);
                    } else if line.is_empty() {
                        in_msg = true;
                    } else if let Some(stripped) = line.strip_prefix("object ") {
                        target_object = stripped.trim().to_string();
                    } else if let Some(stripped) = line.strip_prefix("tag ") {
                        tag_name = stripped.trim().to_string();
                    } else if let Some(stripped) = line.strip_prefix("tagger ") {
                        tagger = stripped.trim().to_string();
                    }
                }

                let tag_msg = message_lines.join("\n");
                let data = json!({
                    "kind": "tag",
                    "hash": hash,
                    "tag_name": if tag_name.is_empty() { Value::Null } else { json!(tag_name) },
                    "target_object": if target_object.is_empty() { Value::Null } else { json!(target_object) },
                    "tagger": if tagger.is_empty() { Value::Null } else { json!(tagger) },
                    "message": if tag_msg.is_empty() { Value::Null } else { json!(tag_msg) },
                    "raw": text
                });
                let summary = if !tag_name.is_empty() {
                    format!("Tag '{tag_name}' -> {}", &target_object[..7.min(target_object.len())])
                } else {
                    format!("Tag {}", &hash[..7.min(hash.len())])
                };
                Ok(GitToolResult::ok(data, summary))
            }
            "tree" => {
                let entries = objects::parse_tree_entries(&content)?;
                let data = json!({
                    "kind": "tree",
                    "hash": hash,
                    "entries": entries.into_iter().map(|(mode, name, h)| json!({
                        "mode": format!("{:06o}", mode),
                        "name": name,
                        "hash": h
                    })).collect::<Vec<_>>()
                });
                let summary = format!("Tree {}", &hash[..7.min(hash.len())]);
                Ok(GitToolResult::ok(data, summary))
            }
            "blob" => {
                let text = String::from_utf8(content.clone()).ok();
                let data = json!({
                    "kind": "blob",
                    "hash": hash,
                    "size_bytes": content.len(),
                    "content": text
                });
                let summary = format!("Blob {} ({} bytes)", &hash[..7.min(hash.len())], content.len());
                Ok(GitToolResult::ok(data, summary))
            }
            unknown => Err(format!("Unknown git object type '{unknown}' for '{rev}'")),
        }
    }

    /// Resolve revision to 40-char SHA1 hex
    pub fn rev_parse_hash(&self, revision: &str) -> Result<String, String> {
        let rev = revision.trim();
        if rev.is_empty() {
            return Err("Cannot rev-parse empty revision".to_string());
        }

        // Handle ~ operator: e.g. HEAD~2, main~1, 847d031~1
        if let Some(pos) = rev.find('~') {
            let base = &rev[..pos];
            let num_str = &rev[pos + 1..];
            let count: usize = if num_str.is_empty() { 1 } else { num_str.parse().unwrap_or(1) };
            let mut current = self.rev_parse_hash(base)?;
            for _ in 0..count {
                let (_, content) = read_loose_object(&self.git_dir(), &current)?;
                let parsed = parse_commit(&current, &content)?;
                if let Some(parent) = parsed.parents.first() {
                    current = parent.clone();
                } else {
                    return Err(format!("Cannot resolve '{rev}': history too shallow"));
                }
            }
            return Ok(current);
        }

        // Handle ^ operator: e.g. HEAD^, HEAD^^, HEAD^1, HEAD^2
        if let Some(pos) = rev.find('^') {
            let base = &rev[..pos];
            let suffix = &rev[pos..];
            if suffix.chars().all(|c| c == '^') {
                let caret_count = suffix.len();
                let mut current = self.rev_parse_hash(base)?;
                for _ in 0..caret_count {
                    let (_, content) = read_loose_object(&self.git_dir(), &current)?;
                    let parsed = parse_commit(&current, &content)?;
                    if let Some(parent) = parsed.parents.first() {
                        current = parent.clone();
                    } else {
                        return Err(format!("Cannot resolve '{rev}': history too shallow"));
                    }
                }
                return Ok(current);
            } else if let Ok(parent_idx) = suffix[1..].parse::<usize>() {
                let current = self.rev_parse_hash(base)?;
                let (_, content) = read_loose_object(&self.git_dir(), &current)?;
                let parsed = parse_commit(&current, &content)?;
                if parent_idx == 0 || parent_idx > parsed.parents.len() {
                    return Err(format!("Cannot resolve '{rev}': parent #{parent_idx} does not exist"));
                }
                return Ok(parsed.parents[parent_idx - 1].clone());
            }
        }

        if rev == "HEAD" {
            let (_, head_opt) = self.get_head()?;
            return head_opt.ok_or_else(|| "HEAD does not point to a valid commit (empty repo)".to_string());
        }

        // Check 40-character hex
        if rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(rev.to_string());
        }

        // Check short hex
        if (4..40).contains(&rev.len()) && rev.chars().all(|c| c.is_ascii_hexdigit()) {
            let prefix = &rev[0..2];
            let suffix = &rev[2..];
            let obj_dir = self.git_dir().join("objects").join(prefix);
            if obj_dir.exists() {
                if let Ok(entries) = fs::read_dir(obj_dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with(suffix) {
                            return Ok(format!("{prefix}{name}"));
                        }
                    }
                }
            }
        }

        // Direct file lookup under .git/
        let direct_ref_file = self.git_dir().join(rev);
        if direct_ref_file.is_file() {
            if let Ok(content) = fs::read_to_string(&direct_ref_file) {
                let trimmed = content.trim();
                if let Some(sym) = trimmed.strip_prefix("ref: ") {
                    return self.rev_parse_hash(sym);
                } else if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
            }
        }

        // Check branch ref
        let branch_file = self.git_dir().join("refs").join("heads").join(rev);
        if branch_file.is_file() {
            if let Ok(content) = fs::read_to_string(&branch_file) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
            }
        }

        // Check tag ref
        let tag_file = self.git_dir().join("refs").join("tags").join(rev);
        if tag_file.is_file() {
            if let Ok(content) = fs::read_to_string(&tag_file) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
            }
        }

        // Check remote tracking ref
        let remote_file = self.git_dir().join("refs").join("remotes").join(rev);
        if remote_file.is_file() {
            if let Ok(content) = fs::read_to_string(&remote_file) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
            }
        }

        // Check packed-refs
        if let Some(h) = self.find_packed_ref(&format!("refs/heads/{rev}"))
            .or_else(|| self.find_packed_ref(&format!("refs/tags/{rev}")))
            .or_else(|| self.find_packed_ref(&format!("refs/remotes/{rev}")))
            .or_else(|| self.find_packed_ref(rev))
        {
            return Ok(h);
        }

        // Fallback to gix rev_parse_single
        if let Ok(repo) = self.open_repo() {
            use gix::bstr::ByteSlice;
            if let Ok(obj) = repo.rev_parse_single(rev.as_bytes().as_bstr()) {
                return Ok(obj.to_hex().to_string());
            }
        }

        Err(format!("Could not resolve revision '{revision}'"))
    }

    /// Rev-parse tool
    pub fn rev_parse(&self, revision: &str) -> Result<GitToolResult, String> {
        let hash = self.rev_parse_hash(revision)?;
        let short_hash = hash[..7.min(hash.len())].to_string();
        let summary = format!("{revision} -> {short_hash}");

        Ok(GitToolResult::ok(
            json!({
                "revision": revision,
                "hash": hash,
                "short_hash": short_hash,
                "repo_path": self.repo_path.display().to_string(),
                "git_dir": self.git_dir().display().to_string()
            }),
            summary,
        ))
    }

    /// Blame a file with commit hashes, authors, and dates
    pub fn blame(&self, file_path: &str) -> Result<GitToolResult, String> {
        let norm_path = file_path.trim().trim_start_matches("./").replace('\\', "/");
        let abs_path = self.repo_path.join(&norm_path);

        if !abs_path.exists() {
            return Err(format!("File not found: '{norm_path}'"));
        }

        let content = fs::read_to_string(&abs_path)
            .map_err(|e| format!("Failed to read file '{norm_path}': {e}"))?;
        let lines: Vec<&str> = content.lines().collect();

        // Trace commits in history to find who touched this file
        let mut history_commits = Vec::new();
        if let Ok((_, Some(head_hash))) = self.get_head() {
            let mut queue = VecDeque::new();
            let mut visited = HashSet::new();
            queue.push_back(head_hash.clone());
            visited.insert(head_hash);

            while let Some(cid) = queue.pop_front() {
                if let Ok((_, cdata)) = read_loose_object(&self.git_dir(), &cid) {
                    if let Ok(p) = parse_commit(&cid, &cdata) {
                        history_commits.push(p.clone());
                        for parent in p.parents {
                            if !visited.contains(&parent) {
                                visited.insert(parent.clone());
                                queue.push_back(parent);
                            }
                        }
                    }
                }
            }
        }

        // Pre-fetch commit blob contents once per history commit for performance
        let mut commit_blobs: Vec<(ParsedCommit, String)> = Vec::new();
        for commit in &history_commits {
            if let Ok(files) = self.get_commit_tree_files(&commit.hash) {
                if let Some((_, blob_sha)) = files.get(&norm_path) {
                    if let Ok((_, bdata)) = read_loose_object(&self.git_dir(), blob_sha) {
                        if let Ok(txt) = String::from_utf8(bdata) {
                            commit_blobs.push((commit.clone(), txt));
                        }
                    }
                }
            }
        }

        let mut annotated_lines = Vec::new();

        for (idx, line) in lines.iter().enumerate() {
            let mut matched_commit = None;

            // Search history commits from most recent to oldest
            for commit_blob in &commit_blobs {
                if commit_blob.1.lines().any(|l| l == *line) {
                    matched_commit = Some(commit_blob.0.clone());
                    break;
                }
            }

            if let Some(c) = matched_commit {
                let short_hash = &c.hash[..7.min(c.hash.len())];
                let date_str = DateTime::from_timestamp(c.author_date, 0)
                    .map(|dt: DateTime<Utc>| dt.to_rfc3339())
                    .unwrap_or_else(|| c.author_date.to_string());

                annotated_lines.push(json!({
                    "line_number": idx + 1,
                    "commit_hash": c.hash,
                    "short_hash": short_hash,
                    "author": format!("{} <{}>", c.author, c.author_email),
                    "date": date_str,
                    "content": line
                }));
            } else {
                annotated_lines.push(json!({
                    "line_number": idx + 1,
                    "commit_hash": "0000000000000000000000000000000000000000",
                    "short_hash": "0000000",
                    "author": format!("{} <{}>", self.config.author_name, self.config.author_email),
                    "date": Utc::now().to_rfc3339(),
                    "content": line
                }));
            }
        }

        let summary = format!("Blame for {norm_path} ({} lines)", lines.len());
        Ok(GitToolResult::ok(
            json!({
                "file": norm_path,
                "lines_count": lines.len(),
                "lines": annotated_lines
            }),
            summary,
        ))
    }

    /// Compute diff between commits, index, or working tree
    pub fn diff(
        &self,
        staged: bool,
        commit_ref: Option<&str>,
        file_paths: Option<Vec<String>>,
        max_lines: Option<usize>,
    ) -> Result<GitToolResult, String> {
        let limit = max_lines.unwrap_or(self.config.diff_max_lines);
        let worktree_files = scan_worktree_files(&self.repo_path);
        let index_files = self.read_index();
        let head_files = self.get_head_tree_files();

        let mut diff_output = String::new();
        let mut files_changed = 0;
        let mut total_insertions = 0;
        let mut total_deletions = 0;
        let mut current_lines = 0;
        let mut truncated = false;

        let filter_set: Option<HashSet<String>> = file_paths.map(|paths| {
            paths.into_iter().map(|p| p.trim().trim_start_matches("./").replace('\\', "/")).collect()
        });

        if let Some(target_rev) = commit_ref {
            let (old_tree_files, new_tree_files, compare_with_worktree) = if target_rev.contains("..") {
                let mut parts = target_rev.split("..");
                let left = parts.next().unwrap_or("").trim();
                let right = parts.next().unwrap_or("").trim();
                let left_rev = if left.is_empty() { "HEAD" } else { left };
                let right_rev = if right.is_empty() { "HEAD" } else { right };

                let left_hash = self.rev_parse_hash(left_rev)?;
                let right_hash = self.rev_parse_hash(right_rev)?;
                (self.get_commit_tree_files(&left_hash)?, Some(self.get_commit_tree_files(&right_hash)?), false)
            } else {
                let target_hash = self.rev_parse_hash(target_rev)?;
                (self.get_commit_tree_files(&target_hash)?, None, true)
            };

            let all_keys: HashSet<&String> = if compare_with_worktree {
                old_tree_files.keys().chain(worktree_files.keys()).collect()
            } else {
                let new_files = new_tree_files.as_ref().unwrap();
                old_tree_files.keys().chain(new_files.keys()).collect()
            };
            let mut sorted_keys: Vec<&String> = all_keys.into_iter().collect();
            sorted_keys.sort();

            for key in sorted_keys {
                if let Some(ref filter) = filter_set {
                    if !filter.contains(key) {
                        continue;
                    }
                }

                let old_txt = old_tree_files.get(key).and_then(|(_, sha)| {
                    read_loose_object(&self.git_dir(), sha).ok().and_then(|(_, b)| String::from_utf8(b).ok())
                });

                let new_txt = if compare_with_worktree {
                    worktree_files.get(key).and_then(|abs| fs::read_to_string(abs).ok())
                } else {
                    new_tree_files.as_ref().unwrap().get(key).and_then(|(_, sha)| {
                        read_loose_object(&self.git_dir(), sha).ok().and_then(|(_, b)| String::from_utf8(b).ok())
                    })
                };

                let (file_diff, ins, del) = compute_unified_diff(key, old_txt.as_deref(), new_txt.as_deref());
                if !file_diff.is_empty() {
                    files_changed += 1;
                    total_insertions += ins;
                    total_deletions += del;

                    for line in file_diff.lines() {
                        if current_lines >= limit {
                            truncated = true;
                            break;
                        }
                        diff_output.push_str(line);
                        diff_output.push('\n');
                        current_lines += 1;
                    }
                }

                if truncated {
                    break;
                }
            }
        } else if staged {
            // Staged diff: Index vs HEAD
            let all_keys: HashSet<&String> = head_files.keys().chain(index_files.keys()).collect();
            let mut sorted_keys: Vec<&String> = all_keys.into_iter().collect();
            sorted_keys.sort();

            for key in sorted_keys {
                if let Some(ref filter) = filter_set {
                    if !filter.contains(key) {
                        continue;
                    }
                }

                let old_txt = head_files.get(key).and_then(|(_, sha)| {
                    read_loose_object(&self.git_dir(), sha).ok().and_then(|(_, b)| String::from_utf8(b).ok())
                });

                let new_txt = index_files.get(key).and_then(|(_, sha)| {
                    read_loose_object(&self.git_dir(), sha).ok().and_then(|(_, b)| String::from_utf8(b).ok())
                });

                let (file_diff, ins, del) = compute_unified_diff(key, old_txt.as_deref(), new_txt.as_deref());
                if !file_diff.is_empty() {
                    files_changed += 1;
                    total_insertions += ins;
                    total_deletions += del;

                    for line in file_diff.lines() {
                        if current_lines >= limit {
                            truncated = true;
                            break;
                        }
                        diff_output.push_str(line);
                        diff_output.push('\n');
                        current_lines += 1;
                    }
                }

                if truncated {
                    break;
                }
            }
        } else {
            // Unstaged diff: Worktree vs Index (falling back to HEAD if no index)
            let base_files = if !index_files.is_empty() {
                &index_files
            } else {
                &head_files
            };
            let all_keys: HashSet<&String> = base_files.keys().chain(worktree_files.keys()).collect();
            let mut sorted_keys: Vec<&String> = all_keys.into_iter().collect();
            sorted_keys.sort();

            for key in sorted_keys {
                if let Some(ref filter) = filter_set {
                    if !filter.contains(key) {
                        continue;
                    }
                }

                let old_txt = base_files.get(key).and_then(|(_, sha)| {
                    read_loose_object(&self.git_dir(), sha).ok().and_then(|(_, b)| String::from_utf8(b).ok())
                });

                let new_txt = worktree_files.get(key).and_then(|abs| {
                    fs::read_to_string(abs).ok()
                });

                let (file_diff, ins, del) = compute_unified_diff(key, old_txt.as_deref(), new_txt.as_deref());
                if !file_diff.is_empty() {
                    files_changed += 1;
                    total_insertions += ins;
                    total_deletions += del;

                    for line in file_diff.lines() {
                        if current_lines >= limit {
                            truncated = true;
                            break;
                        }
                        diff_output.push_str(line);
                        diff_output.push('\n');
                        current_lines += 1;
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
            total_insertions,
            total_deletions,
            if truncated { " (truncated to limit)" } else { "" }
        );

        let data = json!({
            "files_changed": files_changed,
            "insertions": total_insertions,
            "deletions": total_deletions,
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
        let worktree_files = scan_worktree_files(&self.repo_path);
        let mut index = self.read_index();
        let mut added = Vec::new();

        if all {
            // Stage deleted files that were in index
            let index_keys: Vec<String> = index.keys().cloned().collect();
            for k in index_keys {
                if !worktree_files.contains_key(&k) {
                    index.remove(&k);
                    added.push(format!("deleted: {k}"));
                }
            }
            // Stage all files from worktree
            for (rel_path, abs_path) in worktree_files {
                let bytes = fs::read(&abs_path).map_err(|e| format!("Failed to read '{rel_path}': {e}"))?;
                let blob_sha = write_blob(&self.git_dir(), &bytes)?;
                index.insert(rel_path.clone(), (REGULAR_FILE_MODE, blob_sha));
                added.push(rel_path);
            }
        } else if let Some(target_paths) = paths {
            for raw_p in target_paths {
                let norm = raw_p.trim().trim_start_matches("./").replace('\\', "/");
                let clean_norm = norm.trim_end_matches('/').to_string();
                let full_path = self.repo_path.join(&clean_norm);

                if full_path.is_dir() {
                    let dir_prefix = format!("{clean_norm}/");
                    // Stage all worktree files under this directory
                    for (rel_path, abs_path) in &worktree_files {
                        if rel_path == &clean_norm || rel_path.starts_with(&dir_prefix) {
                            let bytes = fs::read(abs_path).map_err(|e| format!("Failed to read '{rel_path}': {e}"))?;
                            let blob_sha = write_blob(&self.git_dir(), &bytes)?;
                            index.insert(rel_path.clone(), (REGULAR_FILE_MODE, blob_sha));
                            added.push(rel_path.clone());
                        }
                    }
                    // Stage any deleted files under this directory that were previously in index
                    let index_keys: Vec<String> = index.keys().cloned().collect();
                    for k in index_keys {
                        if (k == clean_norm || k.starts_with(&dir_prefix)) && !worktree_files.contains_key(&k) {
                            index.remove(&k);
                            added.push(format!("deleted: {k}"));
                        }
                    }
                } else if full_path.exists() {
                    let bytes = fs::read(&full_path).map_err(|e| format!("Failed to read '{clean_norm}': {e}"))?;
                    let blob_sha = write_blob(&self.git_dir(), &bytes)?;
                    index.insert(clean_norm.clone(), (REGULAR_FILE_MODE, blob_sha));
                    added.push(clean_norm);
                } else if index.contains_key(&clean_norm) {
                    // Deleted file staged
                    index.remove(&clean_norm);
                    added.push(format!("deleted: {clean_norm}"));
                }
            }
        }

        self.write_index(&index)?;
        let summary = format!("Staged {} path(s)", added.len());
        Ok(GitToolResult::ok(
            json!({
                "staged_paths": added,
                "all": all
            }),
            summary,
        ))
    }

    /// Restore working tree files or unstage index entries
    pub fn restore(&self, paths: Vec<String>, staged: bool) -> Result<GitToolResult, String> {
        let mut index = self.read_index();
        let head_files = self.get_head_tree_files();
        let mut restored = Vec::new();

        for raw_p in &paths {
            let norm = raw_p.trim().trim_start_matches("./").replace('\\', "/");

            if staged {
                // Unstage from index: restore to HEAD tree version (or remove if new)
                if let Some(&(head_mode, ref head_sha)) = head_files.get(&norm) {
                    index.insert(norm.clone(), (head_mode, head_sha.clone()));
                } else {
                    index.remove(&norm);
                }
                restored.push(norm);
            } else {
                // Restore working tree file from index (or HEAD)
                let source_sha = index.get(&norm).map(|(_, sha)| sha)
                    .or_else(|| head_files.get(&norm).map(|(_, sha)| sha));

                if let Some(sha) = source_sha {
                    let (_, content) = read_loose_object(&self.git_dir(), sha)?;
                    let full_path = self.repo_path.join(&norm);
                    if let Some(parent) = full_path.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    fs::write(&full_path, content).map_err(|e| format!("Failed to restore file '{norm}': {e}"))?;
                    restored.push(norm);
                }
            }
        }

        if staged {
            self.write_index(&index)?;
        }

        let summary = format!(
            "Restored {} path(s) ({})",
            restored.len(),
            if staged { "unstaged from index" } else { "working tree" }
        );

        Ok(GitToolResult::ok(
            json!({
                "restored_paths": restored,
                "staged": staged
            }),
            summary,
        ))
    }

    /// Reset index or HEAD
    pub fn reset(
        &self,
        paths: Option<Vec<String>>,
        mode: Option<&str>,
        target_ref: Option<&str>,
    ) -> Result<GitToolResult, String> {
        if let Some(specific_paths) = paths {
            // Unstage specific paths
            return self.restore(specific_paths, true);
        }

        let target = target_ref.unwrap_or("HEAD");
        let reset_mode = mode.unwrap_or("mixed").trim().to_lowercase();
        if !["soft", "mixed", "hard"].contains(&reset_mode.as_str()) {
            return Err(format!("Invalid reset mode '{reset_mode}'. Supported modes: 'soft', 'mixed', 'hard'"));
        }

        let target_hash = self.rev_parse_hash(target)?;
        let target_files = self.get_commit_tree_files(&target_hash)?;

        // Move HEAD / branch ref
        self.update_head_ref(&target_hash)?;

        if reset_mode == "mixed" || reset_mode == "hard" {
            // Reset index to target tree
            self.write_index(&target_files)?;
        }

        if reset_mode == "hard" {
            let current_worktree = scan_worktree_files(&self.repo_path);
            // Remove worktree files that do not exist in target_files
            for rel_path in current_worktree.keys() {
                if !target_files.contains_key(rel_path) {
                    let full_path = self.repo_path.join(rel_path);
                    if full_path.is_file() {
                        let _ = fs::remove_file(full_path);
                    }
                }
            }

            // Check out all target files to working tree
            for (rel_path, (_, sha)) in &target_files {
                let (_, content) = read_loose_object(&self.git_dir(), sha)?;
                let full_path = self.repo_path.join(rel_path);
                if let Some(parent) = full_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                fs::write(&full_path, content).map_err(|e| format!("Failed to write '{rel_path}': {e}"))?;
            }
        }

        let summary = format!("Reset repository to {target} (mode: {reset_mode})");
        Ok(GitToolResult::ok(
            json!({
                "target": target,
                "target_hash": target_hash,
                "mode": reset_mode
            }),
            summary,
        ))
    }

    /// Clean untracked files
    pub fn clean(&self, dry_run: bool, directories: bool) -> Result<GitToolResult, String> {
        let worktree_files = scan_worktree_files(&self.repo_path);
        let index = self.read_index();
        let mut cleaned = Vec::new();

        for (rel_path, abs_path) in worktree_files {
            if !index.contains_key(&rel_path) {
                cleaned.push(rel_path);
                if !dry_run {
                    let _ = fs::remove_file(abs_path);
                }
            }
        }

        if directories {
            // Collect all directories in pre-order traversal
            let mut all_dirs = Vec::new();
            let mut stack = vec![self.repo_path.clone()];
            while let Some(dir) = stack.pop() {
                if let Ok(entries) = fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
                            continue;
                        }
                        if path.is_dir() {
                            all_dirs.push(path.clone());
                            stack.push(path);
                        }
                    }
                }
            }
            // Remove empty directories bottom-up (leaf-to-root)
            for dir in all_dirs.into_iter().rev() {
                if let Ok(mut entries) = fs::read_dir(&dir) {
                    if entries.next().is_none() && dir != self.repo_path {
                        if let Ok(rel) = dir.strip_prefix(&self.repo_path) {
                            cleaned.push(rel.to_string_lossy().replace('\\', "/"));
                        }
                        if !dry_run {
                            let _ = fs::remove_dir(&dir);
                        }
                    }
                }
            }
        }

        let summary = if dry_run {
            format!("Would clean {} untracked file(s) [dry run]", cleaned.len())
        } else {
            format!("Cleaned {} untracked file(s)", cleaned.len())
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
    pub fn commit(&self, message: &str, allow_empty: bool) -> Result<GitToolResult, String> {
        if message.trim().is_empty() {
            return Err("Commit message cannot be empty.".to_string());
        }

        let index = self.read_index();
        let head_files = self.get_head_tree_files();
        let (_, head_commit_opt) = self.get_head()?;

        // Build tree from index
        let tree_hex = build_tree_hierarchy(&self.git_dir(), &index)?;

        // Check empty commit
        if !allow_empty {
            if let Some(ref parent_hash) = head_commit_opt {
                if let Ok((_, content)) = read_loose_object(&self.git_dir(), parent_hash) {
                    if let Ok(parent_commit) = parse_commit(parent_hash, &content) {
                        if parent_commit.tree_hash == tree_hex && index == head_files {
                            return Err("nothing to commit, working tree clean".to_string());
                        }
                    }
                }
            }
        }

        let mut parents = Vec::new();
        if let Some(parent_hash) = head_commit_opt {
            parents.push(parent_hash);
        }

        // Check if MERGE_HEAD exists
        let merge_head_file = self.git_dir().join("MERGE_HEAD");
        if merge_head_file.exists() {
            if let Ok(merge_head) = fs::read_to_string(&merge_head_file) {
                parents.push(merge_head.trim().to_string());
            }
            let _ = fs::remove_file(&merge_head_file);
        }

        let now = Utc::now();
        let author_name = &self.config.author_name;
        let author_email = &self.config.author_email;

        let commit_hash = write_commit_object(
            &self.git_dir(),
            &tree_hex,
            &parents,
            author_name,
            author_email,
            message,
            now.timestamp(),
        )?;

        // Update branch ref / HEAD
        self.update_head_ref(&commit_hash)?;

        let short_hash = &commit_hash[..7.min(commit_hash.len())];
        let summary = format!("[{short_hash}] {}", message.lines().next().unwrap_or(""));

        Ok(GitToolResult::ok(
            json!({
                "commit_hash": commit_hash,
                "short_hash": short_hash,
                "author": format!("{author_name} <{author_email}>"),
                "committer": format!("{author_name} <{author_email}>"),
                "message": message,
                "tree": tree_hex,
                "parents": parents,
                "timestamp": now.timestamp(),
                "date": now.to_rfc3339()
            }),
            summary,
        ))
    }

    /// Revert a commit
    pub fn revert(&self, commit_ref: &str, no_commit: bool) -> Result<GitToolResult, String> {
        let commit_hash = self.rev_parse_hash(commit_ref)?;
        let (_, content) = read_loose_object(&self.git_dir(), &commit_hash)?;
        let parsed = parse_commit(&commit_hash, &content)?;

        let commit_files = self.get_commit_tree_files(&commit_hash)?;
        let parent_files = if let Some(parent) = parsed.parents.first() {
            self.get_commit_tree_files(parent).unwrap_or_default()
        } else {
            BTreeMap::new()
        };

        // Invert changes: apply parent_files where commit_files changed
        let mut index = self.read_index();
        let mut reverted_paths = Vec::new();

        let all_keys: HashSet<&String> = commit_files.keys().chain(parent_files.keys()).collect();
        for key in all_keys {
            match (parent_files.get(key), commit_files.get(key)) {
                (Some((parent_mode, parent_sha)), Some((_, commit_sha))) => {
                    if parent_sha != commit_sha {
                        index.insert(key.clone(), (*parent_mode, parent_sha.clone()));
                        let (_, bdata) = read_loose_object(&self.git_dir(), parent_sha)?;
                        fs::write(self.repo_path.join(key), bdata).map_err(|e| e.to_string())?;
                        reverted_paths.push(key.clone());
                    }
                }
                (Some((parent_mode, parent_sha)), None) => {
                    // File was deleted in commit; restore it
                    index.insert(key.clone(), (*parent_mode, parent_sha.clone()));
                    let (_, bdata) = read_loose_object(&self.git_dir(), parent_sha)?;
                    let full = self.repo_path.join(key);
                    if let Some(p) = full.parent() {
                        let _ = fs::create_dir_all(p);
                    }
                    fs::write(&full, bdata).map_err(|e| e.to_string())?;
                    reverted_paths.push(key.clone());
                }
                (None, Some(_)) => {
                    // File was added in commit; remove it
                    index.remove(key);
                    let full = self.repo_path.join(key);
                    if full.exists() {
                        let _ = fs::remove_file(&full);
                    }
                    reverted_paths.push(key.clone());
                }
                (None, None) => {}
            }
        }

        self.write_index(&index)?;

        let revert_msg = format!("Revert \"{}\"\n\nThis reverts commit {}.", parsed.summary, commit_hash);
        let commit_res = if !no_commit {
            Some(self.commit(&revert_msg, false)?)
        } else {
            None
        };

        let summary = format!("Reverted commit {commit_ref} (no_commit: {no_commit})");
        Ok(GitToolResult::ok(
            json!({
                "reverted_commit": commit_hash,
                "no_commit": no_commit,
                "reverted_paths": reverted_paths,
                "commit": commit_res.map(|r| r.data)
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
        let tags_dir = self.git_dir().join("refs").join("tags");

        match action {
            "list" => {
                let mut tags = Vec::new();
                if tags_dir.exists() {
                    collect_refs_recursively(&tags_dir, &tags_dir, &mut tags);
                }

                // Also check packed-refs
                let packed_refs = self.git_dir().join("packed-refs");
                if let Ok(content) = fs::read_to_string(&packed_refs) {
                    for line in content.lines() {
                        let line = line.trim();
                        if let Some(tag_ref) = line.split_whitespace().nth(1) {
                            if let Some(tname) = tag_ref.strip_prefix("refs/tags/") {
                                if !tags.contains(&tname.to_string()) {
                                    tags.push(tname.to_string());
                                }
                            }
                        }
                    }
                }

                tags.sort();
                let summary = format!("Found {} tag(s)", tags.len());
                Ok(GitToolResult::ok(json!({ "tags": tags }), summary))
            }
            "create" => {
                let raw_name = tag_name.ok_or_else(|| "tag_name is required to create a tag".to_string())?;
                let name = raw_name.trim().trim_start_matches("refs/tags/").trim_start_matches("tags/");
                if name.is_empty() {
                    return Err("tag_name cannot be empty".to_string());
                }
                let target = target_ref.unwrap_or("HEAD");
                let target_hash = match self.rev_parse_hash(target) {
                    Ok(h) => h,
                    Err(_) if target == "HEAD" => "0000000000000000000000000000000000000000".to_string(),
                    Err(e) => return Err(e),
                };

                let tag_file = tags_dir.join(name);
                if let Some(parent) = tag_file.parent() {
                    fs::create_dir_all(parent).map_err(|e| format!("Failed to create refs/tags: {e}"))?;
                }

                let tag_hash = if let Some(msg) = message {
                    // Create annotated tag object
                    let now = Utc::now().timestamp();
                    let tag_content = format!(
                        "object {target_hash}\ntype commit\ntag {name}\ntagger {} <{}> {now} +0000\n\n{}\n",
                        self.config.author_name, self.config.author_email, msg
                    );
                    hash_and_write_object(&self.git_dir(), "tag", tag_content.as_bytes())?
                } else {
                    target_hash.clone()
                };

                fs::write(&tag_file, format!("{tag_hash}\n"))
                    .map_err(|e| format!("Failed to write tag ref: {e}"))?;

                let summary = format!("Created tag '{name}' at {target}");
                Ok(GitToolResult::ok(
                    json!({
                        "tag_name": name,
                        "target": target,
                        "hash": tag_hash,
                        "message": message
                    }),
                    summary,
                ))
            }
            "delete" => {
                let raw_name = tag_name.ok_or_else(|| "tag_name is required to delete a tag".to_string())?;
                let name = raw_name.trim().trim_start_matches("refs/tags/").trim_start_matches("tags/");
                let tag_file = tags_dir.join(name);
                let mut deleted = false;
                if tag_file.exists() {
                    fs::remove_file(&tag_file).map_err(|e| format!("Failed to delete tag ref: {e}"))?;
                    deleted = true;
                }
                let packed_deleted = self.remove_packed_ref(&format!("refs/tags/{name}"))?;
                if packed_deleted {
                    deleted = true;
                }
                if !deleted {
                    return Err(format!("Tag '{name}' not found."));
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
        force: bool,
    ) -> Result<GitToolResult, String> {
        let heads_dir = self.git_dir().join("refs").join("heads");
        let clean_branch_buf = branch_name.map(|s| s.trim().trim_start_matches("refs/heads/").trim_start_matches("heads/").to_string());
        let clean_branch = clean_branch_buf.as_deref();
        let clean_new_buf = new_name.map(|s| s.trim().trim_start_matches("refs/heads/").trim_start_matches("heads/").to_string());
        let clean_new = clean_new_buf.as_deref();

        match action {
            "list" => {
                let mut branches = Vec::new();
                let (current_opt, _) = self.get_head().unwrap_or((None, None));
                let current_branch = current_opt.unwrap_or_default();

                let mut branch_names = Vec::new();
                if heads_dir.exists() {
                    collect_refs_recursively(&heads_dir, &heads_dir, &mut branch_names);
                }

                // Also check packed-refs
                let packed_refs = self.git_dir().join("packed-refs");
                if let Ok(content) = fs::read_to_string(&packed_refs) {
                    for line in content.lines() {
                        let line = line.trim();
                        if let Some(rname) = line.split_whitespace().nth(1) {
                            if let Some(bname) = rname.strip_prefix("refs/heads/") {
                                if !branch_names.contains(&bname.to_string()) {
                                    branch_names.push(bname.to_string());
                                }
                            }
                        }
                    }
                }

                branch_names.sort();
                for b in branch_names {
                    let is_current = b == current_branch;
                    branches.push(json!({
                        "name": b,
                        "current": is_current
                    }));
                }

                let summary = format!("Found {} local branch(es)", branches.len());
                Ok(GitToolResult::ok(json!({ "branches": branches, "current": current_branch }), summary))
            }
            "create" => {
                let name = clean_branch.ok_or_else(|| "branch_name is required to create a branch".to_string())?;
                let target = start_point.unwrap_or("HEAD");
                let hash = match self.rev_parse_hash(target) {
                    Ok(h) => h,
                    Err(_) if target == "HEAD" => "0000000000000000000000000000000000000000".to_string(),
                    Err(e) => return Err(e),
                };

                let target_file = heads_dir.join(name);
                if target_file.exists() && !force {
                    return Err(format!("Branch '{name}' already exists. Use force: true to overwrite."));
                }

                if let Some(parent) = target_file.parent() {
                    fs::create_dir_all(parent).map_err(|e| format!("Failed to create branch directory: {e}"))?;
                }
                fs::write(&target_file, format!("{hash}\n"))
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
                let name = clean_branch.ok_or_else(|| "branch_name is required to delete a branch".to_string())?;
                let (current_opt, _) = self.get_head().unwrap_or((None, None));
                if current_opt.as_deref() == Some(name) {
                    return Err(format!("Cannot delete current checked out branch '{name}'"));
                }

                let branch_file = heads_dir.join(name);
                let mut deleted = false;
                if branch_file.exists() {
                    fs::remove_file(&branch_file).map_err(|e| format!("Failed to delete branch ref: {e}"))?;
                    deleted = true;
                }
                let packed_deleted = self.remove_packed_ref(&format!("refs/heads/{name}"))?;
                if packed_deleted {
                    deleted = true;
                }
                if !deleted {
                    return Err(format!("Branch '{name}' not found."));
                }
                let summary = format!("Deleted branch '{name}'");
                Ok(GitToolResult::ok(json!({ "deleted_branch": name }), summary))
            }
            "rename" => {
                let old = clean_branch.ok_or_else(|| "branch_name is required for rename".to_string())?;
                let new = clean_new.ok_or_else(|| "new_name is required for rename".to_string())?;
                
                let old_hash = self.rev_parse_hash(old).map_err(|_| format!("Branch '{old}' not found."))?;
                
                let new_file = heads_dir.join(new);
                if new_file.exists() && !force {
                    return Err(format!("Branch '{new}' already exists."));
                }
                
                if let Some(parent) = new_file.parent() {
                    fs::create_dir_all(parent).map_err(|e| format!("Failed to create new branch directory: {e}"))?;
                }
                
                fs::write(&new_file, format!("{old_hash}\n")).map_err(|e| format!("Failed to write new branch: {e}"))?;
                
                let old_file = heads_dir.join(old);
                if old_file.exists() {
                    let _ = fs::remove_file(&old_file);
                }
                let _ = self.remove_packed_ref(&format!("refs/heads/{old}"));

                // Update HEAD if current branch was renamed
                let (current_opt, _) = self.get_head().unwrap_or((None, None));
                if current_opt.as_deref() == Some(old) {
                    let head_file = self.git_dir().join("HEAD");
                    let _ = fs::write(&head_file, format!("ref: refs/heads/{new}\n"));
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
        let clean_branch = branch_name.trim().trim_start_matches("refs/heads/").trim_start_matches("heads/");
        if clean_branch.is_empty() {
            return Err("branch_name cannot be empty".to_string());
        }

        if create_new {
            self.branch("create", Some(clean_branch), None, start_point, false)?;
        }

        let branch_file = self.git_dir().join("refs").join("heads").join(clean_branch);
        if !branch_file.exists() && !create_new && self.find_packed_ref(&format!("refs/heads/{clean_branch}")).is_none() {
            return Err(format!("Branch '{clean_branch}' does not exist"));
        }

        // Update HEAD
        let head_file = self.git_dir().join("HEAD");
        fs::write(&head_file, format!("ref: refs/heads/{clean_branch}\n"))
            .map_err(|e| format!("Failed to update HEAD: {e}"))?;

        // If target branch has a commit, checkout its tree to index and worktree
        if let Ok(branch_hash) = self.rev_parse_hash(clean_branch) {
            let old_files = self.read_index();
            let target_files = self.get_commit_tree_files(&branch_hash).unwrap_or_default();
            self.write_index(&target_files)?;

            // Remove files that existed in previous branch/index but are absent from target branch
            for old_path in old_files.keys() {
                if !target_files.contains_key(old_path) {
                    let full_path = self.repo_path.join(old_path);
                    if full_path.exists() {
                        let _ = fs::remove_file(full_path);
                    }
                }
            }

            for (rel_path, (_, sha)) in &target_files {
                let (_, content) = read_loose_object(&self.git_dir(), sha)?;
                let full_path = self.repo_path.join(rel_path);
                if let Some(p) = full_path.parent() {
                    let _ = fs::create_dir_all(p);
                }
                fs::write(&full_path, content).map_err(|e| format!("Failed to checkout file '{rel_path}': {e}"))?;
            }
        }

        let summary = format!("Switched to branch '{clean_branch}'");
        Ok(GitToolResult::ok(
            json!({
                "branch": clean_branch,
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
        let source_hash = self.rev_parse_hash(source_ref)?;
        let (_, head_commit_opt) = self.get_head()?;
        let head_hash = head_commit_opt.ok_or_else(|| "Cannot merge into an empty repository".to_string())?;

        if head_hash == source_hash || self.is_ancestor(&source_hash, &head_hash) {
            return Ok(GitToolResult::ok(
                json!({
                    "source_ref": source_ref,
                    "status": "Already up to date."
                }),
                "Already up to date.",
            ));
        }

        let source_files = self.get_commit_tree_files(&source_hash)?;
        let head_files = self.get_head_tree_files();

        // Fast-forward merge check
        if !no_ff && !squash && self.is_ancestor(&head_hash, &source_hash) {
            // Remove files absent in source
            for old_path in head_files.keys() {
                if !source_files.contains_key(old_path) {
                    let full_path = self.repo_path.join(old_path);
                    if full_path.is_file() {
                        let _ = fs::remove_file(full_path);
                    }
                }
            }

            // Checkout all source files
            for (rel_path, (_, sha)) in &source_files {
                let (_, content) = read_loose_object(&self.git_dir(), sha)?;
                let full_path = self.repo_path.join(rel_path);
                if let Some(p) = full_path.parent() {
                    let _ = fs::create_dir_all(p);
                }
                fs::write(&full_path, content).map_err(|e| format!("Failed to write '{rel_path}': {e}"))?;
            }

            self.write_index(&source_files)?;
            self.update_head_ref(&source_hash)?;

            let summary = format!("Fast-forward merge of '{source_ref}' ({})", &source_hash[..7.min(source_hash.len())]);
            return Ok(GitToolResult::ok(
                json!({
                    "source_ref": source_ref,
                    "status": "Fast-forward",
                    "merge_commit_hash": source_hash,
                    "fast_forward": true
                }),
                summary,
            ));
        }

        // 3-way file merge
        let base_files = self
            .find_merge_base(&head_hash, &source_hash)
            .and_then(|base_hash| self.get_commit_tree_files(&base_hash).ok())
            .unwrap_or_default();

        let mut merged_files = BTreeMap::new();
        let all_keys: HashSet<String> = head_files
            .keys()
            .chain(source_files.keys())
            .chain(base_files.keys())
            .cloned()
            .collect();

        for key in all_keys {
            let base_entry = base_files.get(&key);
            let head_entry = head_files.get(&key);
            let source_entry = source_files.get(&key);

            match (base_entry, head_entry, source_entry) {
                // Unchanged in base, head, source
                (Some(b), Some(h), Some(s)) if b == h && b == s => {
                    merged_files.insert(key, h.clone());
                }
                // Modified only in source (unchanged in head) -> use source
                (Some(b), Some(h), Some(s)) if b == h => {
                    merged_files.insert(key, s.clone());
                }
                // Modified only in head (unchanged in source) -> use head
                (Some(b), Some(h), Some(s)) if b == s => {
                    merged_files.insert(key, h.clone());
                }
                // Modified in both head and source -> prefer source if different
                (Some(_), Some(h), Some(s)) => {
                    if h == s {
                        merged_files.insert(key, h.clone());
                    } else {
                        merged_files.insert(key, s.clone());
                    }
                }
                // Deleted in source, unchanged in head -> deleted in merge
                (Some(b), Some(h), None) if b == h => {
                    // Omit from merged_files
                }
                // Deleted in source, modified in head -> keep head
                (Some(_), Some(h), None) => {
                    merged_files.insert(key, h.clone());
                }
                // Deleted in head, unchanged in source -> remain deleted
                (Some(b), None, Some(s)) if b == s => {
                    // Omit from merged_files
                }
                // Deleted in head, modified in source -> use source
                (Some(_), None, Some(s)) => {
                    merged_files.insert(key, s.clone());
                }
                // Added only in source -> add to merge
                (None, None, Some(s)) => {
                    merged_files.insert(key, s.clone());
                }
                // Added only in head -> keep in merge
                (None, Some(h), None) => {
                    merged_files.insert(key, h.clone());
                }
                // Added in both head and source
                (None, Some(h), Some(s)) => {
                    if h == s {
                        merged_files.insert(key, h.clone());
                    } else {
                        merged_files.insert(key, s.clone());
                    }
                }
                // Deleted in both
                (Some(_), None, None) => {}
                (None, None, None) => {}
            }
        }

        // Remove files that exist in current worktree/head but are omitted from merged_files
        for old_path in head_files.keys() {
            if !merged_files.contains_key(old_path) {
                let full_path = self.repo_path.join(old_path);
                if full_path.exists() {
                    let _ = fs::remove_file(full_path);
                }
            }
        }

        // Write merged files to disk
        for (rel_path, (_, sha)) in &merged_files {
            let (_, content) = read_loose_object(&self.git_dir(), sha)?;
            let full_path = self.repo_path.join(rel_path);
            if let Some(p) = full_path.parent() {
                let _ = fs::create_dir_all(p);
            }
            fs::write(&full_path, content).map_err(|e| format!("Failed to write merged file '{rel_path}': {e}"))?;
        }

        self.write_index(&merged_files)?;
        let merged_tree = build_tree_hierarchy(&self.git_dir(), &merged_files)?;

        let merge_msg = message.map(String::from).unwrap_or_else(|| format!("Merge branch '{source_ref}'"));
        let mut merge_commit_hash = None;

        if !squash {
            let now = Utc::now().timestamp();
            let parents = vec![head_hash, source_hash];
            let commit_id = write_commit_object(
                &self.git_dir(),
                &merged_tree,
                &parents,
                &self.config.author_name,
                &self.config.author_email,
                &merge_msg,
                now,
            )?;
            self.update_head_ref(&commit_id)?;
            merge_commit_hash = Some(commit_id);
        }

        let summary = format!("Merged '{source_ref}' into HEAD (no_ff: {no_ff}, squash: {squash})");
        Ok(GitToolResult::ok(
            json!({
                "source_ref": source_ref,
                "no_ff": no_ff,
                "squash": squash,
                "merge_commit_hash": merge_commit_hash,
                "message": merge_msg
            }),
            summary,
        ))
    }

    // -----------------------------------------------------------------------
    // STASH
    // -----------------------------------------------------------------------

    fn stash_log_file(&self) -> PathBuf {
        self.git_dir().join("carapace_stash_log.json")
    }

    fn read_stashes(&self) -> Vec<StashEntry> {
        let path = self.stash_log_file();
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(list) = serde_json::from_str::<Vec<StashEntry>>(&content) {
                    return list;
                }
            }
        }
        Vec::new()
    }

    fn write_stashes(&self, list: &[StashEntry]) -> Result<(), String> {
        let path = self.stash_log_file();
        let content = serde_json::to_string_pretty(list)
            .map_err(|e| format!("Failed to serialize stash log: {e}"))?;
        fs::write(&path, content).map_err(|e| format!("Failed to write stash log: {e}"))?;
        Ok(())
    }

    /// Stash operations
    pub fn stash(
        &self,
        action: &str,
        message: Option<&str>,
        stash_index: Option<usize>,
        include_untracked: bool,
    ) -> Result<GitToolResult, String> {
        let mut stashes = self.read_stashes();
        let (branch_opt, head_opt) = self.get_head()?;
        let branch = branch_opt.unwrap_or_else(|| "HEAD".to_string());

        match action {
            "list" => {
                let summary = format!("Found {} stash entry/entries", stashes.len());
                Ok(GitToolResult::ok(
                    json!({
                        "stashes": stashes.iter().map(|s| json!({
                            "index": s.index,
                            "name": format!("stash@{{{}}}", s.index),
                            "branch": s.branch,
                            "message": s.message,
                            "date": DateTime::from_timestamp(s.timestamp, 0).map(|d: DateTime<Utc>| d.to_rfc3339()).unwrap_or_default()
                        })).collect::<Vec<_>>()
                    }),
                    summary,
                ))
            }
            "save" | "push" => {
                let msg = message.unwrap_or("Saved by Carapace Agent");
                let worktree_files = scan_worktree_files(&self.repo_path);
                let head_files = self.get_head_tree_files();

                // Save dirty worktree as tree object
                let mut current_map = BTreeMap::new();
                for (rel_path, abs_path) in &worktree_files {
                    if let Ok(bytes) = fs::read(abs_path) {
                        if let Ok(sha) = write_blob(&self.git_dir(), &bytes) {
                            current_map.insert(rel_path.clone(), (REGULAR_FILE_MODE, sha));
                        }
                    }
                }

                let tree_hex = build_tree_hierarchy(&self.git_dir(), &current_map)?;
                let now = Utc::now().timestamp();
                let parent = head_opt.unwrap_or_default();
                let commit_hex = write_commit_object(
                    &self.git_dir(),
                    &tree_hex,
                    &[parent],
                    &self.config.author_name,
                    &self.config.author_email,
                    msg,
                    now,
                )?;

                // Re-index stashes
                let mut new_stashes = vec![StashEntry {
                    index: 0,
                    commit_hash: commit_hex.clone(),
                    tree_hash: tree_hex,
                    branch: branch.clone(),
                    message: msg.to_string(),
                    timestamp: now,
                }];

                for mut old_s in stashes {
                    old_s.index += 1;
                    new_stashes.push(old_s);
                }

                self.write_stashes(&new_stashes)?;

                // Reset index to clean HEAD tree
                self.write_index(&head_files)?;

                // Revert working tree to clean HEAD state
                for (rel_path, abs_path) in &worktree_files {
                    if let Some((_, head_sha)) = head_files.get(rel_path) {
                        let (_, bdata) = read_loose_object(&self.git_dir(), head_sha)?;
                        fs::write(abs_path, bdata).map_err(|e| e.to_string())?;
                    } else if include_untracked {
                        let _ = fs::remove_file(abs_path);
                    }
                }

                // Restore any head files that were deleted in worktree
                for (rel_path, (_, head_sha)) in &head_files {
                    let full_path = self.repo_path.join(rel_path);
                    if !full_path.exists() {
                        let (_, bdata) = read_loose_object(&self.git_dir(), head_sha)?;
                        if let Some(p) = full_path.parent() {
                            let _ = fs::create_dir_all(p);
                        }
                        let _ = fs::write(&full_path, bdata);
                    }
                }

                let summary = format!("Saved working directory state: '{msg}'");
                Ok(GitToolResult::ok(
                    json!({
                        "stash_index": 0,
                        "commit_hash": commit_hex,
                        "branch": branch,
                        "message": msg
                    }),
                    summary,
                ))
            }
            "pop" | "apply" => {
                let target_idx = stash_index.unwrap_or(0);
                let entry = stashes.iter().find(|s| s.index == target_idx)
                    .ok_or_else(|| format!("Stash index '{target_idx}' not found"))?
                    .clone();

                let mut stashed_files = BTreeMap::new();
                read_tree_all_files(&self.git_dir(), &entry.tree_hash, "", &mut stashed_files)?;

                // Restore files
                for (rel_path, (_, sha)) in &stashed_files {
                    let (_, bdata) = read_loose_object(&self.git_dir(), sha)?;
                    let full = self.repo_path.join(rel_path);
                    if let Some(p) = full.parent() {
                        let _ = fs::create_dir_all(p);
                    }
                    fs::write(&full, bdata).map_err(|e| e.to_string())?;
                }

                if action == "pop" {
                    stashes.retain(|s| s.index != target_idx);
                    for (i, s) in stashes.iter_mut().enumerate() {
                        s.index = i;
                    }
                    self.write_stashes(&stashes)?;
                }

                let summary = format!("Applied stash@{{{target_idx}}}");
                Ok(GitToolResult::ok(json!({ "stash_index": target_idx, "action": action }), summary))
            }
            "drop" => {
                let target_idx = stash_index.unwrap_or(0);
                let before_len = stashes.len();
                stashes.retain(|s| s.index != target_idx);
                if stashes.len() == before_len {
                    return Err(format!("Stash index '{target_idx}' not found"));
                }
                for (i, s) in stashes.iter_mut().enumerate() {
                    s.index = i;
                }
                self.write_stashes(&stashes)?;

                let summary = format!("Dropped stash@{{{target_idx}}}");
                Ok(GitToolResult::ok(json!({ "dropped_index": target_idx }), summary))
            }
            unknown => Err(format!("Unknown stash action: '{unknown}'. Supported: list, save, pop, apply, drop")),
        }
    }
}

fn collect_refs_recursively(base_dir: &std::path::Path, current_dir: &std::path::Path, refs: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(current_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_refs_recursively(base_dir, &path, refs);
            } else if path.is_file() {
                if let Ok(rel) = path.strip_prefix(base_dir) {
                    refs.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
}


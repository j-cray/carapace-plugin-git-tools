use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sha1::{Digest, Sha1};
use std::io::Read;

/// Loose object writing and formatting
pub fn hash_and_write_object(git_dir: &Path, obj_type: &str, data: &[u8]) -> Result<String, String> {
    let header = format!("{obj_type} {}\0", data.len());
    let mut hasher = Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(data);
    let hash_bytes = hasher.finalize();
    let hash_hex = format!("{:040x}", hash_bytes);

    let prefix = &hash_hex[0..2];
    let suffix = &hash_hex[2..];

    let obj_dir = git_dir.join("objects").join(prefix);
    fs::create_dir_all(&obj_dir).map_err(|e| format!("Failed to create object dir: {e}"))?;

    let obj_path = obj_dir.join(suffix);
    if !obj_path.exists() {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(header.as_bytes())
            .map_err(|e| format!("Zlib write header error: {e}"))?;
        encoder
            .write_all(data)
            .map_err(|e| format!("Zlib write data error: {e}"))?;
        let compressed = encoder
            .finish()
            .map_err(|e| format!("Zlib finish error: {e}"))?;

        fs::write(&obj_path, compressed).map_err(|e| format!("Failed to write loose object: {e}"))?;
    }

    Ok(hash_hex)
}

/// Read loose object from .git/objects/xx/yyy...
pub fn read_loose_object(git_dir: &Path, hex: &str) -> Result<(String, Vec<u8>), String> {
    if hex.len() < 40 {
        return Err(format!("Invalid object hash: {hex}"));
    }
    let prefix = &hex[0..2];
    let suffix = &hex[2..];
    let obj_path = git_dir.join("objects").join(prefix).join(suffix);

    if !obj_path.exists() {
        return Err(format!("Loose object '{hex}' not found"));
    }

    let compressed = fs::read(&obj_path).map_err(|e| format!("Failed to read loose object file: {e}"))?;
    let mut decoder = ZlibDecoder::new(&compressed[..]);
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| format!("Failed to decompress loose object '{hex}': {e}"))?;

    // Parse header: "<type> <len>\0<content>"
    let null_idx = decompressed
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| format!("Corrupt loose object '{hex}': no null byte in header"))?;

    let header_str = std::str::from_utf8(&decompressed[..null_idx])
        .map_err(|e| format!("Corrupt loose object header: {e}"))?;

    let mut parts = header_str.split_whitespace();
    let obj_type = parts.next().unwrap_or("unknown").to_string();
    let content = decompressed[null_idx + 1..].to_vec();

    Ok((obj_type, content))
}

/// Write a blob object
pub fn write_blob(git_dir: &Path, data: &[u8]) -> Result<String, String> {
    hash_and_write_object(git_dir, "blob", data)
}

/// Parse a tree object's raw content into entries: Vec<(mode, filename, sha1_hex)>
pub fn parse_tree_entries(content: &[u8]) -> Result<Vec<(u32, String, String)>, String> {
    let mut entries = Vec::new();
    let mut cursor = 0;

    while cursor < content.len() {
        let space_idx = content[cursor..]
            .iter()
            .position(|&b| b == b' ')
            .ok_or_else(|| "Corrupt tree entry: missing space".to_string())?
            + cursor;

        let mode_str = std::str::from_utf8(&content[cursor..space_idx])
            .map_err(|e| format!("Corrupt tree entry mode: {e}"))?;
        let mode = u32::from_str_radix(mode_str, 8).map_err(|e| format!("Invalid octal mode '{mode_str}': {e}"))?;

        cursor = space_idx + 1;

        let null_idx = content[cursor..]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| "Corrupt tree entry: missing null byte".to_string())?
            + cursor;

        let filename = std::str::from_utf8(&content[cursor..null_idx])
            .map_err(|e| format!("Corrupt tree entry filename: {e}"))?
            .to_string();

        cursor = null_idx + 1;

        if cursor + 20 > content.len() {
            return Err("Corrupt tree entry: truncated SHA1 hash".to_string());
        }

        let hash_bytes = &content[cursor..cursor + 20];
        let hash_hex: String = hash_bytes.iter().map(|b| format!("{:02x}", b)).collect();
        cursor += 20;

        entries.push((mode, filename, hash_hex));
    }

    Ok(entries)
}

/// Write a single tree object from sorted entries: (mode, name, sha1_hex)
pub fn write_tree(git_dir: &Path, mut entries: Vec<(u32, String, String)>) -> Result<String, String> {
    // Sort entries by name
    entries.sort_by(|a, b| a.1.cmp(&b.1));

    let mut buf = Vec::new();
    for (mode, name, hex) in entries {
        let mode_str = format!("{:o} {}\0", mode, name);
        buf.extend_from_slice(mode_str.as_bytes());

        let raw_sha = decode_hex_sha1(&hex)?;
        buf.extend_from_slice(&raw_sha);
    }

    hash_and_write_object(git_dir, "tree", &buf)
}

fn decode_hex_sha1(hex: &str) -> Result<[u8; 20], String> {
    if hex.len() != 40 {
        return Err(format!("Expected 40-char SHA1 hex, got length {}", hex.len()));
    }
    let mut out = [0u8; 20];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).map_err(|e| e.to_string())?;
        out[i] = u8::from_str_radix(s, 16).map_err(|e| format!("Invalid hex byte '{s}': {e}"))?;
    }
    Ok(out)
}

/// Recursively build hierarchical trees from a map of relative file path -> (mode, blob_hash)
pub fn build_tree_hierarchy(
    git_dir: &Path,
    files: &BTreeMap<String, (u32, String)>,
) -> Result<String, String> {
    // Tree node representing a directory or file in the tree
    enum Node {
        File(u32, String), // mode, sha1
        Dir(BTreeMap<String, Node>),
    }

    let mut root: BTreeMap<String, Node> = BTreeMap::new();

    for (rel_path, &(mode, ref sha)) in files {
        let parts: Vec<&str> = rel_path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            continue;
        }

        let mut current_dir = &mut root;
        for (i, &part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                current_dir.insert(part.to_string(), Node::File(mode, sha.clone()));
            } else {
                let entry = current_dir
                    .entry(part.to_string())
                    .or_insert_with(|| Node::Dir(BTreeMap::new()));
                match entry {
                    Node::Dir(ref mut sub) => current_dir = sub,
                    Node::File(_, _) => {
                        return Err(format!("Path conflict at '{}' in tree building", part));
                    }
                }
            }
        }
    }

    fn write_node_dir(git_dir: &Path, dir: &BTreeMap<String, Node>) -> Result<String, String> {
        let mut entries = Vec::new();
        for (name, node) in dir {
            match node {
                Node::File(mode, sha) => {
                    entries.push((*mode, name.clone(), sha.clone()));
                }
                Node::Dir(sub_dir) => {
                    let sub_tree_hash = write_node_dir(git_dir, sub_dir)?;
                    // Directory mode in git is 040000 (octal 040000 = 16384 decimal)
                    entries.push((0o040000, name.clone(), sub_tree_hash));
                }
            }
        }
        write_tree(git_dir, entries)
    }

    write_node_dir(git_dir, &root)
}

/// Recursively read all files from a tree object
pub fn read_tree_all_files(
    git_dir: &Path,
    tree_hash: &str,
    prefix: &str,
    out_files: &mut BTreeMap<String, (u32, String)>,
) -> Result<(), String> {
    let (_, content) = read_loose_object(git_dir, tree_hash)?;
    let entries = parse_tree_entries(&content)?;

    for (mode, name, hex) in entries {
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };

        if (mode & 0o170000) == 0o040000 {
            // Directory / subtree
            read_tree_all_files(git_dir, &hex, &path, out_files)?;
        } else {
            // Regular file or symlink or exec
            out_files.insert(path, (mode, hex));
        }
    }

    Ok(())
}

/// Write a commit object
pub fn write_commit_object(
    git_dir: &Path,
    tree_hex: &str,
    parent_hexes: &[String],
    author_name: &str,
    author_email: &str,
    message: &str,
    timestamp: i64,
) -> Result<String, String> {
    let mut commit_content = String::new();
    commit_content.push_str(&format!("tree {tree_hex}\n"));

    for parent in parent_hexes {
        if !parent.trim().is_empty() {
            commit_content.push_str(&format!("parent {}\n", parent.trim()));
        }
    }

    let ident = format!("{author_name} <{author_email}> {timestamp} +0000");
    commit_content.push_str(&format!("author {ident}\n"));
    commit_content.push_str(&format!("committer {ident}\n\n"));
    commit_content.push_str(message.trim_end());
    commit_content.push('\n');

    hash_and_write_object(git_dir, "commit", commit_content.as_bytes())
}

/// Parsed commit information
#[derive(Debug, Clone)]
pub struct ParsedCommit {
    pub hash: String,
    pub tree_hash: String,
    pub parents: Vec<String>,
    pub author: String,
    pub author_email: String,
    pub author_date: i64,
    pub committer: String,
    pub committer_email: String,
    pub committer_date: i64,
    pub message: String,
    pub summary: String,
}

/// Parse commit object content
pub fn parse_commit(hash: &str, content: &[u8]) -> Result<ParsedCommit, String> {
    let text = std::str::from_utf8(content).map_err(|e| format!("Invalid UTF-8 in commit '{hash}': {e}"))?;
    let mut lines = text.lines();
    let mut tree_hash = String::new();
    let mut parents = Vec::new();
    let mut author_raw = String::new();
    let mut committer_raw = String::new();
    let mut in_message = false;
    let mut message_lines = Vec::new();

    for line in lines.by_ref() {
        if in_message {
            message_lines.push(line);
        } else if line.is_empty() {
            in_message = true;
        } else if let Some(stripped) = line.strip_prefix("tree ") {
            tree_hash = stripped.trim().to_string();
        } else if let Some(stripped) = line.strip_prefix("parent ") {
            parents.push(stripped.trim().to_string());
        } else if let Some(stripped) = line.strip_prefix("author ") {
            author_raw = stripped.trim().to_string();
        } else if let Some(stripped) = line.strip_prefix("committer ") {
            committer_raw = stripped.trim().to_string();
        }
    }

    let (author, author_email, author_date) = parse_ident(&author_raw);
    let (committer, committer_email, committer_date) = parse_ident(&committer_raw);
    let message = message_lines.join("\n");
    let summary = message.lines().next().unwrap_or("").to_string();

    Ok(ParsedCommit {
        hash: hash.to_string(),
        tree_hash,
        parents,
        author,
        author_email,
        author_date,
        committer,
        committer_email,
        committer_date,
        message,
        summary,
    })
}

fn parse_ident(raw: &str) -> (String, String, i64) {
    // "Carapace Agent <agent@local> 1740528000 +0000"
    let mut name = String::new();
    let mut email = String::new();
    let mut timestamp = 0i64;

    if let Some(open) = raw.find('<') {
        name = raw[..open].trim().to_string();
        if let Some(close) = raw.find('>') {
            email = raw[open + 1..close].trim().to_string();
            let remainder = raw[close + 1..].trim();
            if let Some(first_word) = remainder.split_whitespace().next() {
                timestamp = first_word.parse::<i64>().unwrap_or(0);
            }
        }
    }

    if name.is_empty() {
        name = "Unknown".to_string();
    }
    (name, email, timestamp)
}

/// Compute line-by-line unified diff between old content and new content
pub fn compute_unified_diff(
    file_path: &str,
    old_content: Option<&str>,
    new_content: Option<&str>,
) -> (String, usize, usize) {
    let mut diff = String::new();
    let mut insertions = 0;
    let mut deletions = 0;

    match (old_content, new_content) {
        (None, Some(new_txt)) => {
            // Added file
            diff.push_str(&format!("diff --git a/{file_path} b/{file_path}\n"));
            diff.push_str("new file mode 100644\n");
            diff.push_str("--- /dev/null\n");
            diff.push_str(&format!("+++ b/{file_path}\n"));
            let lines: Vec<&str> = new_txt.lines().collect();
            diff.push_str(&format!("@@ -0,0 +1,{} @@\n", lines.len()));
            for line in lines {
                diff.push_str(&format!("+{line}\n"));
                insertions += 1;
            }
        }
        (Some(old_txt), None) => {
            // Deleted file
            diff.push_str(&format!("diff --git a/{file_path} b/{file_path}\n"));
            diff.push_str("deleted file mode 100644\n");
            diff.push_str(&format!("--- a/{file_path}\n"));
            diff.push_str("+++ /dev/null\n");
            let lines: Vec<&str> = old_txt.lines().collect();
            diff.push_str(&format!("@@ -1,{} +0,0 @@\n", lines.len()));
            for line in lines {
                diff.push_str(&format!("-{line}\n"));
                deletions += 1;
            }
        }
        (Some(old_txt), Some(new_txt)) => {
            if old_txt == new_txt {
                return (String::new(), 0, 0);
            }

            let old_lines: Vec<&str> = old_txt.lines().collect();
            let new_lines: Vec<&str> = new_txt.lines().collect();

            // Simple Myers diff line-by-line comparison
            let changes = diff_lines(&old_lines, &new_lines);
            if changes.is_empty() {
                return (String::new(), 0, 0);
            }

            diff.push_str(&format!("diff --git a/{file_path} b/{file_path}\n"));
            diff.push_str(&format!("--- a/{file_path}\n"));
            diff.push_str(&format!("+++ b/{file_path}\n"));
            diff.push_str(&format!(
                "@@ -1,{} +1,{} @@\n",
                old_lines.len().max(1),
                new_lines.len().max(1)
            ));

            for change in changes {
                match change {
                    DiffOp::Keep(line) => diff.push_str(&format!(" {line}\n")),
                    DiffOp::Insert(line) => {
                        diff.push_str(&format!("+{line}\n"));
                        insertions += 1;
                    }
                    DiffOp::Delete(line) => {
                        diff.push_str(&format!("-{line}\n"));
                        deletions += 1;
                    }
                }
            }
        }
        (None, None) => {}
    }

    (diff, insertions, deletions)
}

#[derive(Debug, Clone)]
enum DiffOp<'a> {
    Keep(&'a str),
    Insert(&'a str),
    Delete(&'a str),
}

fn diff_lines<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<DiffOp<'a>> {
    let n = old.len();
    let m = new.len();

    // Fast path: prefix and suffix matching
    let mut start = 0;
    while start < n && start < m && old[start] == new[start] {
        start += 1;
    }

    let mut old_end = n;
    let mut new_end = m;
    while old_end > start && new_end > start && old[old_end - 1] == new[new_end - 1] {
        old_end -= 1;
        new_end -= 1;
    }

    let mut ops = Vec::new();
    for &line in &old[..start] {
        ops.push(DiffOp::Keep(line));
    }

    // Middle changed section: dynamic programming longest common subsequence
    let middle_old = &old[start..old_end];
    let middle_new = &new[start..new_end];
    let sub_ops = lcs_diff(middle_old, middle_new);
    ops.extend(sub_ops);

    for &line in &old[old_end..] {
        ops.push(DiffOp::Keep(line));
    }

    ops
}

fn lcs_diff<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<DiffOp<'a>> {
    let n = old.len();
    let m = new.len();

    if n == 0 {
        return new.iter().map(|&s| DiffOp::Insert(s)).collect();
    }
    if m == 0 {
        return old.iter().map(|&s| DiffOp::Delete(s)).collect();
    }

    // DP table for LCS
    let mut dp = vec![vec![0u32; m + 1]; n + 1];

    for i in 0..n {
        for j in 0..m {
            if old[i] == new[j] {
                dp[i + 1][j + 1] = dp[i][j] + 1;
            } else {
                dp[i + 1][j + 1] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }

    // Backtrack to build diff
    let mut i = n;
    let mut j = m;
    let mut result = Vec::new();

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            result.push(DiffOp::Keep(old[i - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            result.push(DiffOp::Insert(new[j - 1]));
            j -= 1;
        } else if i > 0 {
            result.push(DiffOp::Delete(old[i - 1]));
            i -= 1;
        }
    }

    result.reverse();
    result
}

/// Recursively collect all tracked and untracked worktree files
pub fn scan_worktree_files(work_dir: &Path) -> BTreeMap<String, PathBuf> {
    let mut files = BTreeMap::new();
    let mut stack = vec![work_dir.to_path_buf()];

    while let Some(current_dir) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&current_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let file_name = entry.file_name().to_string_lossy().to_string();

                if file_name == ".git" {
                    continue;
                }

                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    if let Ok(rel) = path.strip_prefix(work_dir) {
                        let rel_str = rel.to_string_lossy().replace('\\', "/");
                        files.insert(rel_str, path);
                    }
                }
            }
        }
    }

    files
}

use std::path::PathBuf;
use tempfile::TempDir;

use git_tools::bindings::exports::tool::ToolContext;
use git_tools::config::PluginConfig;
use git_tools::engine::transport::{parse_smart_http_refs, RemoteTransport};
use git_tools::engine::vfs;
use git_tools::engine::GitEngine;
use git_tools::safety::{normalize_path, SafetyChecker};
use git_tools::tools;

#[test]
fn test_tool_definitions_schema_validity() {
    let defs = tools::get_all_definitions();
    assert!(!defs.is_empty(), "Tool definitions should not be empty");

    let expected_tools = [
        "git_status",
        "git_diff",
        "git_log",
        "git_show",
        "git_blame",
        "git_rev_parse",
        "git_add",
        "git_restore",
        "git_reset",
        "git_clean",
        "git_commit",
        "git_revert",
        "git_tag",
        "git_branch",
        "git_checkout",
        "git_merge",
        "git_stash",
        "git_remote",
        "git_clone",
        "git_fetch",
        "git_pull",
        "git_push",
    ];

    assert_eq!(defs.len(), 22, "Expected 22 dedicated git tools");

    for expected in expected_tools {
        let found = defs.iter().find(|d| d.name == expected);
        assert!(
            found.is_some(),
            "Expected tool '{}' to be present in tool definitions",
            expected
        );
        let def = found.unwrap();
        assert!(!def.description.is_empty(), "Description for '{}' should not be empty", expected);
        let schema: serde_json::Value = serde_json::from_str(&def.input_schema)
            .unwrap_or_else(|_| panic!("Input schema for '{}' should be valid JSON", expected));
        assert_eq!(schema["type"], "object", "Input schema for '{}' should be of type object", expected);

        let required = schema["required"].as_array().expect("Required array should be present");
        if expected == "git_clone" {
            assert!(required.iter().any(|r| r == "url"), "git_clone must require url");
        } else {
            assert!(
                required.iter().any(|r| r == "repo_path"),
                "Tool '{}' must require repo_path",
                expected
            );
        }
    }
}

#[test]
fn test_git_init_and_status() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let engine = GitEngine::new(repo_path.clone(), &config);

    // Init repo
    let init_res = engine.init_repo(false).expect("Failed to init repo");
    assert!(init_res.success);

    // Status on empty repo
    let status_res = engine.status().expect("Failed to get status");
    assert!(status_res.success);
    assert!(status_res.data["clean"].as_bool().unwrap());

    // Create a new file
    let test_file = repo_path.join("hello.txt");
    vfs::write(&test_file, "Hello, world!\n").expect("Failed to write test file");

    // Status should report untracked file
    let status_dirty = engine.status().expect("Failed to get dirty status");
    assert!(status_dirty.success);
    let untracked = status_dirty.data["untracked"].as_array().unwrap();
    assert_eq!(untracked.len(), 1);
    assert_eq!(untracked[0], "hello.txt");
}

#[test]
fn test_commit_log_show_blame_revparse() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig {
        author_name: "Carapace Test Agent".to_string(),
        author_email: "test@carapace.ai".to_string(),
        ..Default::default()
    };
    let engine = GitEngine::new(repo_path.clone(), &config);

    let _ = engine.init_repo(false).expect("Init failed");

    // Create nested directory and files
    let src_dir = repo_path.join("src");
    vfs::create_dir_all(&src_dir).unwrap();
    vfs::write(src_dir.join("lib.rs"), "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n").unwrap();
    vfs::write(repo_path.join("README.md"), "# Test Project\n").unwrap();

    // Stage all
    let add_res = engine.add(None, true).expect("Add all failed");
    assert!(add_res.success);

    // Status shows staged files
    let status_staged = engine.status().unwrap();
    assert!(status_staged.data["staged"].as_array().unwrap().len() >= 2);

    // Commit 1
    let commit1 = engine.commit("feat: initial library implementation", false).expect("Commit 1 failed");
    assert!(commit1.success);
    let hash1 = commit1.data["commit_hash"].as_str().unwrap().to_string();

    // Modify file
    vfs::write(src_dir.join("lib.rs"), "pub fn add(a: i32, b: i32) -> i32 {\n    // addition\n    a + b\n}\n").unwrap();
    let add_res2 = engine.add(Some(vec!["src/lib.rs".to_string()]), false).unwrap();
    assert!(add_res2.success);

    // Commit 2
    let commit2 = engine.commit("docs: add comment to add()", false).expect("Commit 2 failed");
    assert!(commit2.success);
    let hash2 = commit2.data["commit_hash"].as_str().unwrap().to_string();

    // Log
    let log_res = engine.log(Some(10), None, None).expect("Log failed");
    assert!(log_res.success);
    let commits = log_res.data["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0]["hash"], hash2);
    assert_eq!(commits[1]["hash"], hash1);

    // Log with author filter
    let log_author = engine.log(Some(10), Some("Carapace Test"), None).expect("Log with author filter failed");
    assert_eq!(log_author.data["commits"].as_array().unwrap().len(), 2);

    // Show commit
    let show_res = engine.show(Some(&hash2)).expect("Show commit failed");
    assert!(show_res.success);
    assert_eq!(show_res.data["kind"], "commit");
    assert!(show_res.data["diff"].as_str().unwrap().contains("+    // addition"));

    // Rev parse
    let rev_head = engine.rev_parse("HEAD").expect("Rev parse HEAD failed");
    assert_eq!(rev_head.data["hash"], hash2);

    let rev_head1 = engine.rev_parse("HEAD~1").expect("Rev parse HEAD~1 failed");
    assert_eq!(rev_head1.data["hash"], hash1);

    // Blame
    let blame_res = engine.blame("src/lib.rs").expect("Blame failed");
    assert!(blame_res.success);
    assert_eq!(blame_res.data["lines_count"], 4);
    let lines = blame_res.data["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0]["author"], "Carapace Test Agent <test@carapace.ai>");
}

#[test]
fn test_branch_checkout_merge_revert() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig {
        author_name: "Merge Test Agent".to_string(),
        author_email: "merge@test.ai".to_string(),
        ..Default::default()
    };
    let engine = GitEngine::new(repo_path.clone(), &config);

    let _ = engine.init_repo(false).expect("Init failed");

    // Commit on main
    vfs::write(repo_path.join("main.txt"), "base content\n").unwrap();
    engine.add(None, true).unwrap();
    let base_commit = engine.commit("initial on main", false).unwrap();
    assert!(base_commit.success);

    // Create and checkout feature branch
    let checkout_new = engine.checkout("feature-auth", true, None).unwrap();
    assert!(checkout_new.success);

    // Verify current branch in branch list
    let branch_list = engine.branch("list", None, None, None, false).unwrap();
    let branches = branch_list.data["branches"].as_array().unwrap();
    assert!(branches.iter().any(|b| b["name"] == "feature-auth" && b["current"] == true));

    // Commit on feature branch
    vfs::write(repo_path.join("auth.txt"), "auth module\n").unwrap();
    engine.add(None, true).unwrap();
    let feat_commit = engine.commit("feat: add auth", false).unwrap();
    assert!(feat_commit.success);

    // Switch back to main
    let checkout_main = engine.checkout("main", false, None).unwrap();
    assert!(checkout_main.success);
    assert!(!vfs::exists(&repo_path.join("auth.txt")), "auth.txt should not exist on main before merge");

    // Merge feature-auth into main
    let merge_res = engine.merge("feature-auth", Some("Merge feature-auth into main"), false, false).unwrap();
    assert!(merge_res.success);
    assert!(vfs::exists(&repo_path.join("auth.txt")), "auth.txt should exist on main after merge");

    // Revert the auth commit
    let revert_res = engine.revert("feature-auth", false).unwrap();
    assert!(revert_res.success);
    assert!(!vfs::exists(&repo_path.join("auth.txt")), "auth.txt should be removed after reverting auth commit");
}

#[test]
fn test_restore_reset_clean() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let engine = GitEngine::new(repo_path.clone(), &config);

    let _ = engine.init_repo(false).expect("Init failed");

    vfs::write(repo_path.join("data.txt"), "original data\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("initial commit", false).unwrap();

    // Modify data.txt
    vfs::write(repo_path.join("data.txt"), "modified data\n").unwrap();

    // Stage change
    engine.add(Some(vec!["data.txt".to_string()]), false).unwrap();
    let status1 = engine.status().unwrap();
    assert_eq!(status1.data["staged"].as_array().unwrap().len(), 1);

    // Restore staged (unstage)
    let restore_staged = engine.restore(vec!["data.txt".to_string()], true).unwrap();
    assert!(restore_staged.success);
    let status2 = engine.status().unwrap();
    assert_eq!(status2.data["staged"].as_array().unwrap().len(), 0);
    assert_eq!(status2.data["modified"].as_array().unwrap().len(), 1);

    // Restore working tree (discard modifications)
    let restore_work = engine.restore(vec!["data.txt".to_string()], false).unwrap();
    assert!(restore_work.success);
    assert_eq!(vfs::read_to_string(repo_path.join("data.txt")).unwrap(), "original data\n");

    // Test clean
    vfs::write(repo_path.join("temp1.tmp"), "scratch\n").unwrap();
    vfs::write(repo_path.join("temp2.tmp"), "scratch\n").unwrap();

    // Dry run clean
    let clean_dry = engine.clean(true, false).unwrap();
    assert_eq!(clean_dry.data["cleaned"].as_array().unwrap().len(), 2);
    assert!(vfs::exists(&repo_path.join("temp1.tmp")));

    // Actual clean
    let clean_real = engine.clean(false, false).unwrap();
    assert_eq!(clean_real.data["cleaned"].as_array().unwrap().len(), 2);
    assert!(!vfs::exists(&repo_path.join("temp1.tmp")));
}

#[test]
fn test_tags_and_stash_flow() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let engine = GitEngine::new(repo_path.clone(), &config);

    let _ = engine.init_repo(false).expect("Init failed");

    vfs::write(repo_path.join("file.txt"), "v1\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("v1 commit", false).unwrap();

    // Tag create
    let tag_create = engine.tag("create", Some("v0.1.0"), None, Some("release v0.1.0")).unwrap();
    assert!(tag_create.success);

    // Tag list
    let tag_list = engine.tag("list", None, None, None).unwrap();
    assert!(tag_list.success);
    let tags = tag_list.data["tags"].as_array().unwrap();
    assert!(tags.iter().any(|t| t == "v0.1.0"));

    // Modify file for stash
    vfs::write(repo_path.join("file.txt"), "v1 with experimental changes\n").unwrap();

    // Stash save
    let stash_save = engine.stash("save", Some("WIP experimental"), None, true).unwrap();
    assert!(stash_save.success);
    assert_eq!(vfs::read_to_string(repo_path.join("file.txt")).unwrap(), "v1\n");

    // Stash list
    let stash_list = engine.stash("list", None, None, false).unwrap();
    assert!(stash_list.success);
    let stashes = stash_list.data["stashes"].as_array().unwrap();
    assert_eq!(stashes.len(), 1);

    // Stash pop
    let stash_pop = engine.stash("pop", None, Some(0), false).unwrap();
    assert!(stash_pop.success);
    assert_eq!(vfs::read_to_string(repo_path.join("file.txt")).unwrap(), "v1 with experimental changes\n");

    // Tag delete
    let tag_del = engine.tag("delete", Some("v0.1.0"), None, None).unwrap();
    assert!(tag_del.success);
}

#[test]
fn test_remote_management_and_smart_http_parser() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let transport = RemoteTransport::new(repo_path.clone(), &config);

    // Add remote
    let add_res = transport.remote("add", Some("origin"), Some("https://github.com/puremachinery/carapace.git")).unwrap();
    assert!(add_res.success);

    // Get URL
    let url_res = transport.remote("get_url", Some("origin"), None).unwrap();
    assert!(url_res.success);
    assert_eq!(url_res.data["url"], "https://github.com/puremachinery/carapace.git");

    // List
    let list_res = transport.remote("list", None, None).unwrap();
    assert!(list_res.success);
    let remotes = list_res.data["remotes"].as_array().unwrap();
    assert!(remotes.iter().any(|r| r == "origin"));

    // Remove
    let remove_res = transport.remote("remove", Some("origin"), None).unwrap();
    assert!(remove_res.success);

    // Test smart HTTP packetline parser
    let mock_pktline = b"001e# service=git-upload-pack\n00000048847d031bf9e6cf6b1b72a9e3a6a9efc7e8e5efc9 refs/heads/main\0symref=HEAD:refs/heads/main\n003fa9b7829cd123456789abcdef0123456789abcdef refs/tags/v1.0.0\n0000";
    let parsed_refs = parse_smart_http_refs(mock_pktline);
    assert_eq!(parsed_refs.len(), 2);
    assert_eq!(parsed_refs.get("refs/heads/main").unwrap(), "847d031bf9e6cf6b1b72a9e3a6a9efc7e8e5efc9");
    assert_eq!(parsed_refs.get("refs/tags/v1.0.0").unwrap(), "a9b7829cd123456789abcdef0123456789abcdef");
}

#[test]
fn test_safety_path_containment_and_normalization() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let allowed_root = temp_dir.path().to_path_buf();

    let config = PluginConfig {
        allowed_roots: vec![allowed_root.display().to_string()],
        ..Default::default()
    };

    // Valid path inside allowed root
    let inside_path = allowed_root.join("sub-repo");
    vfs::create_dir_all(&inside_path).unwrap();
    let resolved = SafetyChecker::resolve_repo_path(Some(&inside_path.display().to_string()), &config);
    assert!(resolved.is_ok());

    // Invalid path outside allowed root
    let outside_path = PathBuf::from("/etc");
    let resolved_outside = SafetyChecker::resolve_repo_path(Some(&outside_path.display().to_string()), &config);
    assert!(resolved_outside.is_err());

    // Path normalization tests
    assert_eq!(normalize_path(&PathBuf::from("/foo/bar/../baz")), PathBuf::from("/foo/baz"));
    assert_eq!(normalize_path(&PathBuf::from("/foo/../../bar")), PathBuf::from("/bar"));
    assert_eq!(normalize_path(&PathBuf::from("./a/b/../c")), PathBuf::from("a/c"));

    // Missing or empty repo_path should error
    let missing_path = SafetyChecker::resolve_repo_path(None, &config);
    assert!(missing_path.is_err());
    assert!(missing_path.unwrap_err().contains("repo_path"));

    let empty_path = SafetyChecker::resolve_repo_path(Some("   "), &config);
    assert!(empty_path.is_err());
    assert!(empty_path.unwrap_err().contains("repo_path"));
}

#[test]
fn test_safety_protected_branch_and_sandboxing() {
    let config = PluginConfig::default();

    let normal_ctx = ToolContext {
        agent_id: Some("agent-1".to_string()),
        session_key: Some("main".to_string()),
        message_channel: None,
        sandboxed: false,
    };

    let sandboxed_ctx = ToolContext {
        agent_id: Some("agent-2".to_string()),
        session_key: Some("sandbox".to_string()),
        message_channel: None,
        sandboxed: true,
    };

    // Normal context: protected branch blocked without force
    let check_unforced = SafetyChecker::check_branch_protection("main", false, &config, &normal_ctx);
    assert!(check_unforced.is_err());

    // Normal context: protected branch allowed with force
    let check_forced = SafetyChecker::check_branch_protection("main", true, &config, &normal_ctx);
    assert!(check_forced.is_ok());

    // Sandboxed context: protected branch blocked even with force
    let check_sandboxed = SafetyChecker::check_branch_protection("main", true, &config, &sandboxed_ctx);
    assert!(check_sandboxed.is_err());

    // Destructive operations blocked in sandboxed mode
    assert!(SafetyChecker::verify_destructive_allowed("git_clean", &normal_ctx).is_ok());
    assert!(SafetyChecker::verify_destructive_allowed("git_clean", &sandboxed_ctx).is_err());
}

#[test]
fn test_dispatch_integration() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let repo_str = repo_path.display().to_string();
    let config = PluginConfig::default();
    let ctx = ToolContext {
        agent_id: Some("test-agent".to_string()),
        session_key: Some("test-session".to_string()),
        message_channel: None,
        sandboxed: false,
    };

    // Init via engine
    let engine = GitEngine::new(repo_path.clone(), &config);
    engine.init_repo(false).unwrap();

    // Create file
    vfs::write(repo_path.join("app.rs"), "fn run() {}\n").unwrap();

    // Test git_status dispatch with repo_path
    let res_status = tools::dispatch("git_status", &format!(r#"{{"repo_path": "{repo_str}"}}"#), &config, &ctx);
    assert!(res_status.success);

    // Test git_add dispatch
    let res_add = tools::dispatch("git_add", &format!(r#"{{"repo_path": "{repo_str}", "all": true}}"#), &config, &ctx);
    assert!(res_add.success);

    // Test git_commit dispatch
    let res_commit = tools::dispatch("git_commit", &format!(r#"{{"repo_path": "{repo_str}", "message": "feat: init app"}}"#), &config, &ctx);
    assert!(res_commit.success);

    // Test git_log dispatch
    let res_log = tools::dispatch("git_log", &format!(r#"{{"repo_path": "{repo_str}", "max_count": 5}}"#), &config, &ctx);
    assert!(res_log.success);

    // Test git_branch dispatch
    let res_branch = tools::dispatch("git_branch", &format!(r#"{{"repo_path": "{repo_str}", "action": "list"}}"#), &config, &ctx);
    assert!(res_branch.success);

    // Test git_rev_parse dispatch
    let res_rev = tools::dispatch("git_rev_parse", &format!(r#"{{"repo_path": "{repo_str}", "revision": "HEAD"}}"#), &config, &ctx);
    assert!(res_rev.success);

    // Test git_remote dispatch
    let res_remote = tools::dispatch("git_remote", &format!(r#"{{"repo_path": "{repo_str}", "action": "list"}}"#), &config, &ctx);
    assert!(res_remote.success);

    // Test unknown tool dispatch
    let res_unknown = tools::dispatch("git_nonexistent", "{}", &config, &ctx);
    assert!(!res_unknown.success);

    // Test dispatch without repo_path should fail with error
    let res_missing_param = tools::dispatch("git_status", "{}", &config, &ctx);
    assert!(!res_missing_param.success);
    assert!(res_missing_param.error.unwrap().contains("repo_path"));

    // Test dispatch with empty parameters string should fail
    let res_empty_param = tools::dispatch("git_status", "", &config, &ctx);
    assert!(!res_empty_param.success);
    assert!(res_empty_param.error.unwrap().contains("repo_path"));
}

#[test]
fn test_revision_ranges_and_add_deletions_and_reset_hard() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig {
        author_name: "Test Agent".to_string(),
        author_email: "agent@test.com".to_string(),
        ..Default::default()
    };
    let ctx = ToolContext {
        agent_id: Some("agent-1".to_string()),
        session_key: Some("test-session".to_string()),
        message_channel: None,
        sandboxed: false,
    };
    let engine = GitEngine::new(repo_path.clone(), &config);
    engine.init_repo(false).unwrap();

    // Commit 1: create file1.txt and file2.txt
    vfs::write(repo_path.join("file1.txt"), "line 1\n").unwrap();
    vfs::write(repo_path.join("file2.txt"), "hello\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("commit 1", false).unwrap();

    // Commit 2: update file1.txt, add file3.txt
    vfs::write(repo_path.join("file1.txt"), "line 1\nline 2\n").unwrap();
    vfs::write(repo_path.join("file3.txt"), "third file\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("commit 2", false).unwrap();

    // Commit 3: update file3.txt
    vfs::write(repo_path.join("file3.txt"), "third file modified\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("commit 3", false).unwrap();

    // Test git_log with revision range HEAD~1..HEAD (should only show commit 3)
    let log_range = engine.log(None, None, Some("HEAD~1..HEAD")).unwrap();
    assert_eq!(log_range.data["total"].as_u64().unwrap(), 1);
    assert_eq!(log_range.data["commits"][0]["summary"].as_str().unwrap(), "commit 3");

    // Test git_log with revision range HEAD~2..HEAD (should show commit 3 and commit 2)
    let log_range2 = engine.log(None, None, Some("HEAD~2..HEAD")).unwrap();
    assert_eq!(log_range2.data["total"].as_u64().unwrap(), 2);

    // Test git_diff with commit revision range HEAD~1..HEAD
    let diff_range = engine.diff(false, Some("HEAD~1..HEAD"), None, None).unwrap();
    assert_eq!(diff_range.data["files_changed"].as_u64().unwrap(), 1);
    assert!(diff_range.data["diff"].as_str().unwrap().contains("file3.txt"));

    // Test git_add with deleted file
    vfs::remove_file(repo_path.join("file2.txt")).unwrap();
    let add_del = engine.add(None, true).unwrap();
    let staged_paths = add_del.data["staged_paths"].as_array().unwrap();
    assert!(staged_paths.iter().any(|p| p.as_str() == Some("deleted: file2.txt")));

    // Commit deletion
    engine.commit("commit 4: remove file2", false).unwrap();

    // Test reset --hard clean up of extra untracked files
    vfs::write(repo_path.join("untracked_extra.txt"), "should be deleted").unwrap();
    let reset_res = engine.reset(None, Some("hard"), Some("HEAD~1")).unwrap();
    assert!(reset_res.success);
    assert!(!vfs::exists(&repo_path.join("untracked_extra.txt")), "reset --hard should remove untracked working-tree file");
    assert!(vfs::exists(&repo_path.join("file2.txt")), "file2.txt should be restored after resetting to HEAD~1");

    // Test branch rename protection
    let rename_protected = tools::dispatch(
        "git_branch",
        &format!(r#"{{"repo_path": "{}", "action": "rename", "branch_name": "main", "new_name": "renamed_main"}}"#, repo_path.display()),
        &config,
        &ctx,
    );
    assert!(!rename_protected.success, "Renaming protected branch without force should fail");
}

#[test]
fn test_stash_cleanliness_and_packed_refs() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig {
        author_name: "Stash Agent".to_string(),
        author_email: "stash@agent.ai".to_string(),
        ..Default::default()
    };
    let engine = GitEngine::new(repo_path.clone(), &config);
    engine.init_repo(false).unwrap();

    vfs::write(repo_path.join("clean.txt"), "clean base\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("base commit", false).unwrap();

    // Dirty worktree and dirty staged index
    vfs::write(repo_path.join("clean.txt"), "dirty modification\n").unwrap();
    vfs::write(repo_path.join("dirty_new.txt"), "dirty new file\n").unwrap();
    engine.add(Some(vec!["clean.txt".to_string()]), false).unwrap();

    // Stash save with include_untracked: true
    let stash_save = engine.stash("save", Some("WIP stash"), None, true).unwrap();
    assert!(stash_save.success);

    // Verify working tree is restored to clean base state
    let content_after = vfs::read_to_string(repo_path.join("clean.txt")).unwrap();
    assert_eq!(content_after, "clean base\n");
    assert!(!vfs::exists(&repo_path.join("dirty_new.txt")));

    // Verify status is completely clean
    let status = engine.status().unwrap();
    assert!(status.data["clean"].as_bool().unwrap(), "Status must be clean after stash save");

    // Stash pop
    let stash_pop = engine.stash("pop", None, Some(0), false).unwrap();
    assert!(stash_pop.success);
    let content_popped = vfs::read_to_string(repo_path.join("clean.txt")).unwrap();
    assert_eq!(content_popped, "dirty modification\n");

    // Test packed-refs deletion and rev-parse
    let packed_refs_file = repo_path.join(".git").join("packed-refs");
    let commit_hash = engine.rev_parse_hash("HEAD").unwrap();
    vfs::write(&packed_refs_file, format!("# packed-refs with: peeled fully-peeled sorted\n{commit_hash} refs/tags/v1.0.0-packed\n{commit_hash} refs/heads/packed-feature\n")).unwrap();

    // Verify rev-parse finds packed tag and branch
    assert_eq!(engine.rev_parse_hash("v1.0.0-packed").unwrap(), commit_hash);
    assert_eq!(engine.rev_parse_hash("packed-feature").unwrap(), commit_hash);

    // Delete packed tag
    let del_tag = engine.tag("delete", Some("v1.0.0-packed"), None, None).unwrap();
    assert!(del_tag.success);
    assert!(engine.rev_parse_hash("v1.0.0-packed").is_err());

    // Delete packed branch
    let del_branch = engine.branch("delete", Some("packed-feature"), None, None, false).unwrap();
    assert!(del_branch.success);
    assert!(engine.rev_parse_hash("packed-feature").is_err());
}

#[test]
fn test_rev_parse_caret_syntax_and_merge_parents() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let engine = GitEngine::new(repo_path.clone(), &config);
    engine.init_repo(false).unwrap();

    // Commit 1 on main
    vfs::write(repo_path.join("base.txt"), "base\n").unwrap();
    engine.add(None, true).unwrap();
    let commit1 = engine.commit("commit 1 on main", false).unwrap();
    let hash1 = commit1.data["commit_hash"].as_str().unwrap().to_string();

    // Commit 2 on main
    vfs::write(repo_path.join("main_extra.txt"), "extra\n").unwrap();
    engine.add(None, true).unwrap();
    let commit2 = engine.commit("commit 2 on main", false).unwrap();
    let hash2 = commit2.data["commit_hash"].as_str().unwrap().to_string();

    // Create and checkout branch feat
    engine.checkout("feat", true, Some(&hash1)).unwrap();
    vfs::write(repo_path.join("feat.txt"), "feat content\n").unwrap();
    engine.add(None, true).unwrap();
    let feat_commit = engine.commit("feat commit", false).unwrap();
    let feat_hash = feat_commit.data["commit_hash"].as_str().unwrap().to_string();

    // Switch back to main and merge feat with no_ff: true
    engine.checkout("main", false, None).unwrap();
    let merge_res = engine.merge("feat", Some("Merge feat into main"), true, false).unwrap();
    assert!(merge_res.success);
    let merge_hash = merge_res.data["merge_commit_hash"].as_str().unwrap().to_string();

    // Verify HEAD is merge commit
    assert_eq!(engine.rev_parse_hash("HEAD").unwrap(), merge_hash);

    // Verify HEAD^ and HEAD^1 resolve to 1st parent (commit2 on main)
    assert_eq!(engine.rev_parse_hash("HEAD^").unwrap(), hash2);
    assert_eq!(engine.rev_parse_hash("HEAD^1").unwrap(), hash2);

    // Verify HEAD^2 resolves to 2nd parent (feat_commit on feat branch)
    assert_eq!(engine.rev_parse_hash("HEAD^2").unwrap(), feat_hash);

    // Verify HEAD^^ resolves to grandparent (commit1)
    assert_eq!(engine.rev_parse_hash("HEAD^^").unwrap(), hash1);
}

#[test]
fn test_directory_staging_isolation_and_deletion() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let engine = GitEngine::new(repo_path.clone(), &config);
    engine.init_repo(false).unwrap();

    // Create src/lib.rs and src_extra/other.rs
    vfs::create_dir_all(repo_path.join("src")).unwrap();
    vfs::create_dir_all(repo_path.join("src_extra")).unwrap();
    vfs::write(repo_path.join("src").join("lib.rs"), "pub fn run() {}\n").unwrap();
    vfs::write(repo_path.join("src_extra").join("other.rs"), "pub fn extra() {}\n").unwrap();

    // Stage only "src" directory
    let add_res = engine.add(Some(vec!["src".to_string()]), false).unwrap();
    assert!(add_res.success);
    let staged = add_res.data["staged_paths"].as_array().unwrap();
    assert!(staged.iter().any(|p| p.as_str() == Some("src/lib.rs")));
    assert!(!staged.iter().any(|p| p.as_str() == Some("src_extra/other.rs")), "src_extra should not be staged when staging src");

    // Commit src/lib.rs
    engine.commit("initial src commit", false).unwrap();

    // Delete src/lib.rs and stage "src" directory
    vfs::remove_file(repo_path.join("src").join("lib.rs")).unwrap();
    let add_del = engine.add(Some(vec!["src".to_string()]), false).unwrap();
    let staged_del = add_del.data["staged_paths"].as_array().unwrap();
    assert!(staged_del.iter().any(|p| p.as_str() == Some("deleted: src/lib.rs")));
}

#[test]
fn test_three_way_merge_file_deletion_and_reconciliation() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let engine = GitEngine::new(repo_path.clone(), &config);
    engine.init_repo(false).unwrap();

    // Initial commit on main: file1.txt and file2.txt
    vfs::write(repo_path.join("file1.txt"), "original file1\n").unwrap();
    vfs::write(repo_path.join("file2.txt"), "original file2 to be deleted\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("base commit", false).unwrap();

    // Feature branch: delete file2.txt, modify file1.txt, add file3.txt
    engine.checkout("feat-changes", true, None).unwrap();
    vfs::remove_file(repo_path.join("file2.txt")).unwrap();
    vfs::write(repo_path.join("file1.txt"), "modified file1 on feat\n").unwrap();
    vfs::write(repo_path.join("file3.txt"), "new file3 on feat\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("feat updates", false).unwrap();

    // Switch back to main: add file4.txt
    engine.checkout("main", false, None).unwrap();
    vfs::write(repo_path.join("file4.txt"), "file4 on main\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("main update", false).unwrap();

    // Merge feat-changes into main
    let merge_res = engine.merge("feat-changes", Some("Merge feat-changes into main"), false, false).unwrap();
    assert!(merge_res.success);

    // Verify 3-way reconciliation:
    // file2.txt should be deleted
    assert!(!vfs::exists(&repo_path.join("file2.txt")), "file2.txt should be removed after merge");
    // file1.txt should have feature modifications
    assert_eq!(vfs::read_to_string(repo_path.join("file1.txt")).unwrap(), "modified file1 on feat\n");
    // file3.txt and file4.txt should both exist
    assert!(vfs::exists(&repo_path.join("file3.txt")), "file3.txt from feat should exist");
    assert!(vfs::exists(&repo_path.join("file4.txt")), "file4.txt from main should exist");
}

#[test]
fn test_clean_nested_empty_directories() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let engine = GitEngine::new(repo_path.clone(), &config);
    engine.init_repo(false).unwrap();

    // Initial commit
    vfs::write(repo_path.join("tracked.txt"), "tracked\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("init", false).unwrap();

    // Create deep nested directory tree with an untracked file
    let deep_dir = repo_path.join("deep").join("nested").join("folder");
    vfs::create_dir_all(&deep_dir).unwrap();
    vfs::write(deep_dir.join("scratch.tmp"), "temporary data\n").unwrap();

    // Clean with directories: true
    let clean_res = engine.clean(false, true).unwrap();
    assert!(clean_res.success);

    // Deep directory structure should be completely pruned
    assert!(!vfs::exists(&repo_path.join("deep")), "Empty nested directory tree should be pruned completely");
}

#[test]
fn test_branch_rename_target_protection() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let normal_ctx = ToolContext {
        agent_id: Some("agent-1".to_string()),
        session_key: Some("main".to_string()),
        message_channel: None,
        sandboxed: false,
    };
    let sandboxed_ctx = ToolContext {
        agent_id: Some("agent-2".to_string()),
        session_key: Some("sandbox".to_string()),
        message_channel: None,
        sandboxed: true,
    };
    let engine = GitEngine::new(repo_path.clone(), &config);
    engine.init_repo(false).unwrap();

    vfs::write(repo_path.join("app.txt"), "app\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("init", false).unwrap();

    // Create branch temp-feat
    engine.branch("create", Some("temp-feat"), None, None, false).unwrap();

    // Renaming temp-feat to protected branch "main" without force should fail
    let res_rename_unforced = tools::dispatch(
        "git_branch",
        &format!(r#"{{"repo_path": "{}", "action": "rename", "branch_name": "temp-feat", "new_name": "main"}}"#, repo_path.display()),
        &config,
        &normal_ctx,
    );
    assert!(!res_rename_unforced.success);

    // Renaming temp-feat to "main" in sandboxed mode with force: true should fail
    let res_rename_sandboxed = tools::dispatch(
        "git_branch",
        &format!(r#"{{"repo_path": "{}", "action": "rename", "branch_name": "temp-feat", "new_name": "main", "force": true}}"#, repo_path.display()),
        &config,
        &sandboxed_ctx,
    );
    assert!(!res_rename_sandboxed.success);
}

#[test]
fn test_annotated_tag_show_inspection() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig {
        author_name: "Tag Inspector".to_string(),
        author_email: "tagger@test.ai".to_string(),
        ..Default::default()
    };
    let engine = GitEngine::new(repo_path.clone(), &config);
    engine.init_repo(false).unwrap();

    vfs::write(repo_path.join("version.txt"), "1.0.0\n").unwrap();
    engine.add(None, true).unwrap();
    let commit = engine.commit("release commit", false).unwrap();
    let commit_hash = commit.data["commit_hash"].as_str().unwrap().to_string();

    // Create annotated tag
    let tag_res = engine.tag("create", Some("v1.0.0"), Some("HEAD"), Some("Official Release 1.0.0")).unwrap();
    assert!(tag_res.success);

    // Inspect tag with show
    let show_res = engine.show(Some("v1.0.0")).unwrap();
    assert!(show_res.success);
    assert_eq!(show_res.data["kind"], "tag");
    assert_eq!(show_res.data["tag_name"], "v1.0.0");
    assert_eq!(show_res.data["message"], "Official Release 1.0.0");
    assert_eq!(show_res.data["target_object"], commit_hash);
}

#[test]
fn test_reset_invalid_mode() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let engine = GitEngine::new(repo_path.clone(), &config);
    engine.init_repo(false).unwrap();

    vfs::write(repo_path.join("file.txt"), "hello\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("init", false).unwrap();

    let invalid_reset = engine.reset(None, Some("unsupported_mode"), None);
    assert!(invalid_reset.is_err());
}

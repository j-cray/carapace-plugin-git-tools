use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use carapace_plugin_git_tools::bindings::exports::carapace::plugin::tool::ToolContext;
use carapace_plugin_git_tools::config::PluginConfig;
use carapace_plugin_git_tools::engine::transport::{parse_smart_http_refs, RemoteTransport};
use carapace_plugin_git_tools::engine::GitEngine;
use carapace_plugin_git_tools::safety::{normalize_path, SafetyChecker};
use carapace_plugin_git_tools::tools;

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
    fs::write(&test_file, "Hello, world!\n").expect("Failed to write test file");

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
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(src_dir.join("lib.rs"), "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n").unwrap();
    fs::write(repo_path.join("README.md"), "# Test Project\n").unwrap();

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
    fs::write(src_dir.join("lib.rs"), "pub fn add(a: i32, b: i32) -> i32 {\n    // addition\n    a + b\n}\n").unwrap();
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
    fs::write(repo_path.join("main.txt"), "base content\n").unwrap();
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
    fs::write(repo_path.join("auth.txt"), "auth module\n").unwrap();
    engine.add(None, true).unwrap();
    let feat_commit = engine.commit("feat: add auth", false).unwrap();
    assert!(feat_commit.success);

    // Switch back to main
    let checkout_main = engine.checkout("main", false, None).unwrap();
    assert!(checkout_main.success);
    assert!(!repo_path.join("auth.txt").exists(), "auth.txt should not exist on main before merge");

    // Merge feature-auth into main
    let merge_res = engine.merge("feature-auth", Some("Merge feature-auth into main"), false, false).unwrap();
    assert!(merge_res.success);
    assert!(repo_path.join("auth.txt").exists(), "auth.txt should exist on main after merge");

    // Revert the auth commit
    let revert_res = engine.revert("feature-auth", false).unwrap();
    assert!(revert_res.success);
    assert!(!repo_path.join("auth.txt").exists(), "auth.txt should be removed after reverting auth commit");
}

#[test]
fn test_restore_reset_clean() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let engine = GitEngine::new(repo_path.clone(), &config);

    let _ = engine.init_repo(false).expect("Init failed");

    fs::write(repo_path.join("data.txt"), "original data\n").unwrap();
    engine.add(None, true).unwrap();
    engine.commit("initial commit", false).unwrap();

    // Modify data.txt
    fs::write(repo_path.join("data.txt"), "modified data\n").unwrap();

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
    assert_eq!(fs::read_to_string(repo_path.join("data.txt")).unwrap(), "original data\n");

    // Test clean
    fs::write(repo_path.join("temp1.tmp"), "scratch\n").unwrap();
    fs::write(repo_path.join("temp2.tmp"), "scratch\n").unwrap();

    // Dry run clean
    let clean_dry = engine.clean(true, false).unwrap();
    assert_eq!(clean_dry.data["cleaned"].as_array().unwrap().len(), 2);
    assert!(repo_path.join("temp1.tmp").exists());

    // Actual clean
    let clean_real = engine.clean(false, false).unwrap();
    assert_eq!(clean_real.data["cleaned"].as_array().unwrap().len(), 2);
    assert!(!repo_path.join("temp1.tmp").exists());
}

#[test]
fn test_tags_and_stash_flow() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let engine = GitEngine::new(repo_path.clone(), &config);

    let _ = engine.init_repo(false).expect("Init failed");

    fs::write(repo_path.join("file.txt"), "v1\n").unwrap();
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
    fs::write(repo_path.join("file.txt"), "v1 with experimental changes\n").unwrap();

    // Stash save
    let stash_save = engine.stash("save", Some("WIP experimental"), None, true).unwrap();
    assert!(stash_save.success);
    assert_eq!(fs::read_to_string(repo_path.join("file.txt")).unwrap(), "v1\n");

    // Stash list
    let stash_list = engine.stash("list", None, None, false).unwrap();
    assert!(stash_list.success);
    let stashes = stash_list.data["stashes"].as_array().unwrap();
    assert_eq!(stashes.len(), 1);

    // Stash pop
    let stash_pop = engine.stash("pop", None, Some(0), false).unwrap();
    assert!(stash_pop.success);
    assert_eq!(fs::read_to_string(repo_path.join("file.txt")).unwrap(), "v1 with experimental changes\n");

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
    fs::create_dir_all(&inside_path).unwrap();
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
    let config = PluginConfig {
        default_repo_path: repo_path.display().to_string(),
        ..Default::default()
    };
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
    fs::write(repo_path.join("app.rs"), "fn run() {}\n").unwrap();

    // Test git_status dispatch
    let res_status = tools::dispatch("git_status", "{}", &config, &ctx);
    assert!(res_status.success);

    // Test git_add dispatch
    let res_add = tools::dispatch("git_add", r#"{"all": true}"#, &config, &ctx);
    assert!(res_add.success);

    // Test git_commit dispatch
    let res_commit = tools::dispatch("git_commit", r#"{"message": "feat: init app"}"#, &config, &ctx);
    assert!(res_commit.success);

    // Test git_log dispatch
    let res_log = tools::dispatch("git_log", r#"{"max_count": 5}"#, &config, &ctx);
    assert!(res_log.success);

    // Test git_branch dispatch
    let res_branch = tools::dispatch("git_branch", r#"{"action": "list"}"#, &config, &ctx);
    assert!(res_branch.success);

    // Test git_rev_parse dispatch
    let res_rev = tools::dispatch("git_rev_parse", r#"{"revision": "HEAD"}"#, &config, &ctx);
    assert!(res_rev.success);

    // Test git_remote dispatch
    let res_remote = tools::dispatch("git_remote", r#"{"action": "list"}"#, &config, &ctx);
    assert!(res_remote.success);

    // Test unknown tool dispatch
    let res_unknown = tools::dispatch("git_nonexistent", "{}", &config, &ctx);
    assert!(!res_unknown.success);
}

use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

use carapace_plugin_git_tools::bindings::exports::carapace::plugin::tool::ToolContext;
use carapace_plugin_git_tools::config::PluginConfig;
use carapace_plugin_git_tools::engine::transport::RemoteTransport;
use carapace_plugin_git_tools::engine::GitEngine;
use carapace_plugin_git_tools::safety::SafetyChecker;
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
fn test_branch_and_commit_flow() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig {
        author_name: "Carapace Test Agent".to_string(),
        author_email: "test@carapace.ai".to_string(),
        ..Default::default()
    };
    let engine = GitEngine::new(repo_path.clone(), &config);

    let _ = engine.init_repo(false).expect("Init failed");

    // Create and checkout branch
    let branch_res = engine.branch("create", Some("feature-1"), None, None, false).expect("Branch create failed");
    assert!(branch_res.success);

    let checkout_res = engine.checkout("feature-1", false, None).expect("Checkout failed");
    assert!(checkout_res.success);

    // Commit
    let commit_res = engine.commit("feat: initial commit", false).expect("Commit failed");
    assert!(commit_res.success);
    assert_eq!(commit_res.data["author"], "Carapace Test Agent <test@carapace.ai>");
}

#[test]
fn test_staging_and_diff_flow() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let engine = GitEngine::new(repo_path.clone(), &config);

    let _ = engine.init_repo(false).expect("Init failed");

    let file_path = repo_path.join("sample.rs");
    fs::write(&file_path, "fn main() {\n    println!(\"hi\");\n}\n").unwrap();

    // Stage
    let add_res = engine.add(Some(vec!["sample.rs".to_string()]), false).unwrap();
    assert!(add_res.success);

    // Diff
    let diff_res = engine.diff(false, None, None, Some(100)).unwrap();
    assert!(diff_res.success);
    assert_eq!(diff_res.data["files_changed"], 1);

    // Blame
    let blame_res = engine.blame("sample.rs").unwrap();
    assert!(blame_res.success);
    assert_eq!(blame_res.data["lines_count"], 3);
}

#[test]
fn test_tags_and_stash_flow() {
    let temp_dir = TempDir::new().expect("Failed to create tempdir");
    let repo_path = temp_dir.path().to_path_buf();
    let config = PluginConfig::default();
    let engine = GitEngine::new(repo_path.clone(), &config);

    let _ = engine.init_repo(false).expect("Init failed");

    // Tag create
    let tag_create = engine.tag("create", Some("v0.1.0"), None, Some("release v0.1.0")).unwrap();
    assert!(tag_create.success);

    // Tag list
    let tag_list = engine.tag("list", None, None, None).unwrap();
    assert!(tag_list.success);
    let tags = tag_list.data["tags"].as_array().unwrap();
    assert!(tags.iter().any(|t| t == "v0.1.0"));

    // Stash save & pop
    let stash_save = engine.stash("save", Some("WIP test stash"), None, true).unwrap();
    assert!(stash_save.success);

    let stash_list = engine.stash("list", None, None, false).unwrap();
    assert!(stash_list.success);

    let stash_pop = engine.stash("pop", None, Some(0), false).unwrap();
    assert!(stash_pop.success);
}

#[test]
fn test_remote_management() {
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
}

#[test]
fn test_safety_path_containment() {
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

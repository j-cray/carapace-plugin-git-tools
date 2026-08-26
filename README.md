# Carapace Git Tools Plugin (`carapace-plugin-git-tools`)

A WebAssembly (WASM) tool plugin for [Carapace](https://github.com/puremachinery/carapace) providing agents with a comprehensive, sandboxed Git toolkit.

## Features

- **Granular Dedicated Tools**: 22 dedicated tools with strict JSON Schema draft-07 definitions.
- **Rich Structured Output**: Structured JSON data coupled with human-readable diffs and status summaries.
- **Pure-Rust Git Engine**: Built with Gitoxide (`gix`) for memory safety and zero external C dependencies.
- **Remote Synchronization**: Clone, fetch, pull, and push support over HTTP/HTTPS leveraging Carapace host capabilities and credential storage.
- **Enterprise Safety Policies**:
  - **Path Containment**: Restricts repository operations to explicitly configured root paths.
  - **Protected Branch Defense**: Safeguards critical branches (`main`, `master`, `release`, etc.) from accidental deletion or unforced modifications.
  - **Sandboxed Agent Protections**: Disables destructive actions when executed by sandboxed agents.
  - **Bounded Outputs**: Configurable pagination and truncation safeguards to prevent LLM context overflows.

---

## Tool Catalog

| Tool | Category | Description |
|---|---|---|
| `git_status` | Inspection | Show working tree status (staged, modified, untracked, branch) |
| `git_diff` | Inspection | Show differences between commits, index, or working directory |
| `git_log` | Inspection | View commit history logs with author and range filters |
| `git_show` | Inspection | Show details and contents of a specific commit, tag, or object |
| `git_blame` | Inspection | Annotate lines of a file with commit revision and author |
| `git_rev_parse` | Inspection | Resolve revisions and verify commit/tag/branch references |
| `git_add` | Working Tree | Stage file contents into the index |
| `git_restore` | Working Tree | Discard working tree modifications or unstage index entries |
| `git_reset` | Working Tree | Reset current HEAD or unstage files (`soft`, `mixed`, `hard`) |
| `git_clean` | Working Tree | Remove untracked files with dry-run support |
| `git_commit` | History | Record changes with commit message and plugin author identity |
| `git_revert` | History | Revert an existing commit by recording a new revert commit |
| `git_tag` | History | List, create, or delete repository tags |
| `git_branch` | Branching | List, create, delete, or rename branches |
| `git_checkout` | Branching | Switch branches or create and checkout a new branch |
| `git_merge` | Branching | Merge branches into current HEAD |
| `git_stash` | Stash | Save, pop, apply, drop, or list stashed changes |
| `git_remote` | Remote | Manage tracked remotes (list, add, remove, get_url) |
| `git_clone` | Remote | Clone a remote repository over HTTP/HTTPS |
| `git_fetch` | Remote | Download objects and refs from another repository |
| `git_pull` | Remote | Fetch and merge changes from a remote branch |
| `git_push` | Remote | Push local commits/tags to a remote repository |

---

## Configuration

Configure the plugin in your Carapace configuration file (e.g. `carapace.json` or `config.json5`):

```json5
{
  plugins: {
    enabled: true,
    load: {
      paths: [
        "/path/to/carapace-plugin-git-tools/target/wasm32-wasip1/release",
      ],
    },
    "git-tools": {
      // Default repository path if not supplied in tool calls
      default_repo_path: "/home/user/projects/my-app",

      // Author metadata for AI commits
      author_name: "Carapace AI",
      author_email: "carapace-ai@example.com",

      // Allowed repository root directories (prevents directory traversal)
      allowed_roots: [
        "/home/user/projects",
        "/workspace"
      ],

      // Protected branches requiring force: true to modify
      protected_branches: [
        "main",
        "master",
        "release",
        "production"
      ],

      // Output limits
      diff_max_lines: 500,
      log_max_count: 50
    }
  }
}
```

---

## Development & Build

This repository includes a Nix Flake development shell providing all required tooling (`cargo-component`, `wasm-tools`, `wit-bindgen`, and Rust toolchain with WASM targets).

### Build the WASM Component
```bash
cargo component build --release --target wasm32-wasip1
```
The compiled component will be located at:
`target/wasm32-wasip1/release/carapace_plugin_git_tools.wasm`

### Run Automated Tests
```bash
cargo test
```

### Inspect Component WIT Interface
```bash
wasm-tools component wit target/wasm32-wasip1/release/carapace_plugin_git_tools.wasm
```

## License
MIT OR Apache-2.0

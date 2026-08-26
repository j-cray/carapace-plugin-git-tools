# Agent Instructions

## Project Overview
This project is **`carapace-plugin-git-tools`**, a comprehensive Git tools plugin for **Carapace**.

### What is Carapace?
[Carapace](https://github.com/puremachinery/carapace) is a security-focused, open-source personal AI assistant written in Rust that runs locally on the user's machine. It connects to multiple messaging channels (such as Matrix, Signal, Telegram, Discord, Slack, webhooks, and console), supports multiple LLM providers, and provides an extensible, sandboxed WebAssembly (WASM) plugin runtime (supporting `tool-plugin`, `webhook-plugin`, `service-plugin`, and `channel-plugin`) with strict capability permissions and resource limits.

### Plugin Purpose & Capabilities
`carapace-plugin-git-tools` targets the **`tool-plugin`** WIT world in `carapace:plugin@1.0.0`. It provides a suite of 22 dedicated, granular Git tools designed specifically for LLM agents:

1. **Inspection & History**:
   - `git_status`: Working tree status (staged, modified, untracked, branch name).
   - `git_diff`: Show file/commit diffs with bounded line limits.
   - `git_log`: Commit history with author and range filters.
   - `git_show`: Inspect commit, tag, or object details and patches.
   - `git_blame`: Line-by-line file annotation with commit revisions.
   - `git_rev_parse`: Resolve revisions and verify commit/tag/branch hashes.
2. **Working Tree & Staging**:
   - `git_add`: Stage files or all changes into the index.
   - `git_restore`: Discard modifications or unstage index changes.
   - `git_reset`: Unstage files or move HEAD (`soft`, `mixed`, `hard`).
   - `git_clean`: Clean untracked files with dry-run protection.
3. **Commits, Reverts & Tags**:
   - `git_commit`: Record commits with configured plugin author identity.
   - `git_revert`: Create revert commits for existing changes.
   - `git_tag`: List, create, or delete repository tags.
4. **Branching & Merging**:
   - `git_branch`: List, create, delete, or rename branches.
   - `git_checkout`: Switch branches or create and checkout new branches.
   - `git_merge`: Merge branches into current HEAD.
5. **Stash Management**:
   - `git_stash`: Save, pop, apply, drop, or list stashed changes.
6. **Remote Synchronization**:
   - `git_remote`: Manage tracked remotes (list, add, remove, get_url).
   - `git_clone`: Clone remote repositories over HTTP/HTTPS.
   - `git_fetch`: Download objects/refs from remote repositories.
   - `git_pull`: Fetch and merge changes from remote tracking branches.
   - `git_push`: Push local commits/tags to remote repositories.

### Configuration (`plugins.git-tools.*`)
- `default_repo_path`: Default repository directory (defaults to current directory).
- `author_name`: Author name for commits (default: `"Carapace Agent"`).
- `author_email`: Author email for commits (default: `"carapace-agent@local"`).
- `allowed_roots`: Comma-separated or JSON list of directory path prefixes allowed for repository access.
- `protected_branches`: Branches protected from destructive actions without `force: true` (default: `"main,master,release,prod,production"`).
- `diff_max_lines`: Maximum lines returned in diff output (default: `500`).
- `log_max_count`: Default maximum commits returned by `git_log` (default: `50`).

### Security & Safety Model
- **Path Containment**: Prevents path traversal and restricts repository operations to configured `allowed_roots`.
- **Protected Branch Defense**: Guards `main`/`master` against accidental deletion or force updates.
- **Sandboxed Agent Restrictions**: Disables destructive operations (`git_clean`, `git_reset --hard`, `git_push --force`, protected branch modifications) when invoked in `ctx.sandboxed` mode.

## Project Maintenance & Documentation
- **Keep Descriptions Updated**: As this project evolves, agents **MUST** maintain accurate descriptions and usage instructions in both [`GEMINI.md`](GEMINI.md) and [`README.md`](README.md).

## Submodule Restriction
- **NEVER** make changes, create files, or modify anything inside the `carapace/` submodule directory.
- The `carapace/` submodule is strictly read-only and provided for reference only.
- All development and changes must take place exclusively within the main repository.

## Documentation & Development Guidelines
- Always refer to [`docs/plugin-development.md`](docs/plugin-development.md) for plugin development guidelines, WIT specifications, architecture details, and runtime workflows.

## Managing Nix Flake Dependencies
- The repository provides a Nix flake ([`flake.nix`](flake.nix)) and [`.envrc`](.envrc) that define and automatically load the development shell with all required tooling (`cargo-component`, `wasm-tools`, `wit-bindgen`, Rust toolchain with `wasm32-wasip1` / `wasm32-unknown-unknown` targets, etc.).
- Build the component:
  ```bash
  cargo component build --release --target wasm32-wasip1
  ```
- Run tests:
  ```bash
  cargo test
  ```

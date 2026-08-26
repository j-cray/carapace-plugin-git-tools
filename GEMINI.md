# Agent Instructions

## Project Overview
This project is a plugin for **Carapace**.

### What is Carapace?
[Carapace](https://github.com/puremachinery/carapace) is a security-focused, open-source personal AI assistant written in Rust that runs locally on the user's machine. It connects to multiple messaging channels (such as Matrix, Signal, Telegram, Discord, Slack, webhooks, and console), supports multiple LLM providers, and provides an extensible, sandboxed WebAssembly (WASM) plugin runtime (supporting `tool-plugin`, `webhook-plugin`, `service-plugin`, and `channel-plugin`) with strict capability permissions and resource limits.

## Project Maintenance & Documentation
- **Keep Descriptions Updated**: As this project evolves or is fleshed out into a specific plugin implementation, agents **MUST** update the project description, purpose, and usage instructions in both [`GEMINI.md`](GEMINI.md) and [`README.md`](README.md) to accurately reflect the current state, WIT targets, and functionality of the plugin.

## Submodule Restriction
- **NEVER** make changes, create files, or modify anything inside the `carapace/` submodule directory.
- The `carapace/` submodule is strictly read-only and provided for reference only.
- All development and changes must take place exclusively within the main repository.

## Documentation & Development Guidelines
- Always refer to [`docs/plugin-development.md`](docs/plugin-development.md) for plugin development guidelines, WIT specifications, architecture details, and runtime workflows.

## Managing Nix Flake Dependencies
- The repository provides a Nix flake ([`flake.nix`](flake.nix)) and [`.envrc`](.envrc) that define and automatically load the development shell with all required tooling (`cargo-component`, `wasm-tools`, `wit-bindgen`, Rust toolchain with `wasm32-wasip1` / `wasm32-unknown-unknown` targets, etc.).
- When adding new tools, libraries, build dependencies, or compiler targets/extensions required by the project:
  - Update [`flake.nix`](flake.nix) in the workspace root by adding new packages to `devShells.default.buildInputs` or configuring `rustToolchain`.
  - Ensure any new or modified files are staged in Git (`git add flake.nix flake.lock`) so Nix can evaluate them in pure mode.
  - Verify the shell evaluates and builds cleanly with `nix develop` or `direnv reload`.
  - Always prefer maintaining dependencies declaratively in [`flake.nix`](flake.nix) over relying on unmanaged host-level installations.

{
  description = "Development environment for Carapace WASM plugins";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          targets = [
            "wasm32-wasip1"
            "wasm32-unknown-unknown"
          ];
          extensions = [
            "rust-src"
            "rust-analyzer"
            "clippy"
            "rustfmt"
          ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          name = "carapace-plugin-dev";

          buildInputs = with pkgs; [
            # Rust toolchain with WASM targets & developer tools
            rustToolchain

            # WebAssembly & Component Model tooling
            cargo-component
            wasm-tools
            wit-bindgen

            # Build utilities & helpers
            pkg-config
            openssl
            curl
            jq
          ];

          shellHook = ''
            export RUST_SRC_PATH="${rustToolchain}/lib/rustlib/src/rust/library"
          '';
        };
      }
    );
}

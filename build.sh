#!/usr/bin/env bash
set -euo pipefail

echo "==> Building carapace-plugin-git-tools for wasm32-unknown-unknown..."
cargo component build --release --target wasm32-unknown-unknown

TARGET_DIR="target/wasm32-unknown-unknown/release"
cp "${TARGET_DIR}/git_tools.wasm" "${TARGET_DIR}/git-tools.wasm"
cp "${TARGET_DIR}/git_tools.wasm" "./git-tools.wasm"

echo "==> Successfully built component artifact:"
ls -lh "${TARGET_DIR}/git-tools.wasm" "./git-tools.wasm"

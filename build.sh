#!/usr/bin/env bash
set -euo pipefail

echo "==> Building carapace-plugin-git-tools for wasm32-unknown-unknown..."
cargo component build --release --target wasm32-unknown-unknown

TARGET_DIR="target/wasm32-unknown-unknown/release"
if [ -f "${TARGET_DIR}/git_tools.wasm" ]; then
    mv -f "${TARGET_DIR}/git_tools.wasm" "${TARGET_DIR}/git-tools.wasm"
fi

echo "==> Successfully built component artifact:"
ls -lh "${TARGET_DIR}/git-tools.wasm"

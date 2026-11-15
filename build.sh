#!/usr/bin/env bash
# Build Glass (release). Native Rust orchestrator; capture runs in a child process.
set -euo pipefail
cd "$(dirname "$0")"

export TC="$HOME/.local/toolchains"
export PATH="$HOME/.cargo/bin:$TC/go/go/bin:$TC/lua/bin:$TC/node/bin:$HOME/.local/bin:$PATH"

cargo build --release
echo "Built target/release/glass"
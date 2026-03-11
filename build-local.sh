#!/usr/bin/env zsh
source ~/.zshrc 2>/dev/null || true
export CMAKE_POLICY_VERSION_MINIMUM=3.5
cd "$(dirname "$0")"
bun run tauri build

#!/bin/bash
set -e

# Build the release binary
cargo build --release

# Link to ~/.local/bin
mkdir -p "$HOME/.local/bin"
ln -sf "$(pwd)/target/release/gi" "$HOME/.local/bin/gi"

echo "✓ Gi installed to ~/.local/bin/gi"

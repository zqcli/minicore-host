#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
./scripts/check-whitespace.sh
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task059_projection_test.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/task058_rss_test.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo test --workspace --doc --locked
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked

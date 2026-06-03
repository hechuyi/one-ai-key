#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)

cd "${REPO_ROOT}"

cargo fmt -- --check
cargo check --locked
cargo clippy --locked -- -D warnings
cargo test --locked

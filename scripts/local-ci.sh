#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)

cd "${REPO_ROOT}"

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  export CARGO_TARGET_DIR="${TMPDIR:-/tmp}/one-ai-key-cargo-target"
fi

mkdir -p "${CARGO_TARGET_DIR}"
CARGO_TARGET_DIR_ABS=$(CDPATH= cd -- "${CARGO_TARGET_DIR}" && pwd -P)
case "${CARGO_TARGET_DIR_ABS}" in
  "${REPO_ROOT}"|"${REPO_ROOT}"/*)
    echo "CARGO_TARGET_DIR must be outside the repository: ${CARGO_TARGET_DIR_ABS}" >&2
    exit 1
    ;;
esac

cargo fmt -- --check
cargo check --locked
cargo clippy --locked -- -D warnings
cargo test --locked

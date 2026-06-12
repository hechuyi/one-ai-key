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
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR_ABS}"

ensure_no_repository_target_dir() {
  if [[ -d "${REPO_ROOT}/target" ]]; then
    echo "repository-local target/ exists; remove it before running local CI" >&2
    exit 1
  fi
}

ensure_no_repository_target_dir
scripts/check-staged-denylist.sh --self-test
scripts/check-staged-denylist.sh --check-public-plans
git diff --check
cargo fmt -- --check
cargo check --locked
cargo clippy --locked -- -D warnings
cargo test --locked
ensure_no_repository_target_dir

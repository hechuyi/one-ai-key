#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)
NIX_STORE_VOLUME=${ONE_AI_KEY_NIX_STORE_VOLUME:-one-ai-key-nix-amd64}
CARGO_TARGET_VOLUME=${ONE_AI_KEY_CARGO_TARGET_VOLUME:-one-ai-key-cargo-target-amd64}

if ! command -v docker >/dev/null 2>&1; then
  printf 'error: docker is required to run the x86_64 Linux release container\n' >&2
  exit 1
fi

docker run --rm \
  --platform linux/amd64 \
  -v "${REPO_ROOT}:/work" \
  -v "${NIX_STORE_VOLUME}:/nix" \
  -v "${CARGO_TARGET_VOLUME}:/cargo-target" \
  -e CARGO_TARGET_DIR=/cargo-target \
  -w /work \
  nixos/nix:latest \
  nix --extra-experimental-features "nix-command flakes" shell \
    nixpkgs#cargo \
    nixpkgs#rustc \
    nixpkgs#gcc \
    nixpkgs#pkg-config \
    nixpkgs#openssl \
    --command bash -lc '/work/scripts/build-release-x86_64-linux.sh'

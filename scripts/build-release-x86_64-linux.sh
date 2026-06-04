#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)

OS_NAME=$(uname -s)
ARCH_NAME=$(uname -m)

if [[ "${OS_NAME}" != "Linux" ]]; then
  printf 'error: release builds must run inside the local x86_64 Linux Nix container; got OS %s\n' "${OS_NAME}" >&2
  exit 1
fi

if [[ "${ARCH_NAME}" != "x86_64" ]]; then
  printf 'error: release builds must run on x86_64; got architecture %s\n' "${ARCH_NAME}" >&2
  exit 1
fi

if ! command -v nix >/dev/null 2>&1 || ! nix --version >/dev/null 2>&1; then
  printf 'error: release builds must run in a local x86_64 Linux container with Nix available\n' >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1 || ! cargo --version >/dev/null 2>&1; then
  printf 'error: release builds require cargo from the active Nix toolchain\n' >&2
  exit 1
fi

if ! command -v rustc >/dev/null 2>&1 || ! rustc --version >/dev/null 2>&1; then
  printf 'error: release builds require rustc from the active Nix toolchain\n' >&2
  exit 1
fi

if ! command -v gzip >/dev/null 2>&1 || ! gzip --version >/dev/null 2>&1; then
  printf 'error: release builds require gzip for deterministic archive compression\n' >&2
  exit 1
fi

cd "${REPO_ROOT}"

PACKAGE_ID=$(cargo pkgid --locked)
PACKAGE_SPEC=${PACKAGE_ID##*#}
if [[ "${PACKAGE_SPEC}" != *@* ]]; then
  printf 'error: could not parse cargo package id: %s\n' "${PACKAGE_ID}" >&2
  exit 1
fi
PACKAGE_NAME=${PACKAGE_SPEC%@*}
VERSION=${PACKAGE_SPEC##*@}
if [[ -z "${PACKAGE_NAME}" || -z "${VERSION}" ]]; then
  printf 'error: could not parse package name/version from cargo package id: %s\n' "${PACKAGE_ID}" >&2
  exit 1
fi
TARGET=${TARGET:-x86_64-unknown-linux-gnu}
DIST_DIR="${REPO_ROOT}/dist"
BUILD_BIN="${REPO_ROOT}/target/${TARGET}/release/${PACKAGE_NAME}"
ARCHIVE_NAME="${PACKAGE_NAME}-${VERSION}-${TARGET}.tar.gz"
ARCHIVE_PATH="${DIST_DIR}/${ARCHIVE_NAME}"
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-0}
STAGING_DIR=$(mktemp -d)

cleanup() {
  rm -rf "${STAGING_DIR}"
}
trap cleanup EXIT

cargo build --release --locked --target "${TARGET}"

mkdir -p "${DIST_DIR}"
install -m 0755 -p "${BUILD_BIN}" "${STAGING_DIR}/${PACKAGE_NAME}"
touch -h -d "@${SOURCE_DATE_EPOCH}" "${STAGING_DIR}" "${STAGING_DIR}/${PACKAGE_NAME}"
tar -C "${STAGING_DIR}" \
  --sort=name \
  --mtime="@${SOURCE_DATE_EPOCH}" \
  --owner=0 \
  --group=0 \
  --numeric-owner \
  -cf - "${PACKAGE_NAME}" | gzip -n -9 > "${ARCHIVE_PATH}"
(cd "${DIST_DIR}" && sha256sum "${ARCHIVE_NAME}" > "${ARCHIVE_NAME}.sha256")

printf 'wrote %s\n' "${ARCHIVE_PATH}"
printf 'wrote %s\n' "${ARCHIVE_PATH}.sha256"

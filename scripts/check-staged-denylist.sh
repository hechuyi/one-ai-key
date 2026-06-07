#!/usr/bin/env bash
set -euo pipefail

deny_regex='^(dist|target|config|data|db|logs|keys|key-pool-router)/|(^|/)AGENTS\.md$|(^|/)docker-compose\.override\.yml$|(^|/)\.env($|\.)|(^|/)scripts/one_ai_key_keys\.py$|(\.sqlite3?|\.db|\.db-[^/]*|\.wal|\.shm|\.keys|\.key|\.pem|\.log|\.pid|\.tmp)$|(^|/)[^/]*(key|keys|secret|secrets|token|tokens)[^/]*\.txt$'

violations=()
while IFS= read -r path; do
  if [[ "${path}" =~ ${deny_regex} ]]; then
    violations+=("${path}")
  fi
done < <(git diff --cached --name-only)

if ((${#violations[@]} > 0)); then
  printf 'staged denylist violation:\n' >&2
  printf '  %s\n' "${violations[@]}" >&2
  exit 1
fi

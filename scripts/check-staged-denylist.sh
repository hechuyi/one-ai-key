#!/usr/bin/env bash
set -euo pipefail

PATH_DENY_REGEX='^(dist|target|config|data|db|logs|keys|key-pool-router)/|(^|/)AGENTS\.md$|(^|/)docker-compose\.override\.yml$|(^|/)\.env($|\.)|(^|/)scripts/one_ai_key_keys\.py$|(\.sqlite3?|\.db|\.db-[^/]*|\.wal|\.shm|\.keys|\.key|\.pem|\.log|\.pid|\.tmp)$|(^|/)[^/]*(key|keys|secret|secrets|token|tokens)[^/]*\.txt$'
INTERNAL_TRACE_REGEX='(For agentic workers|[Aa]gentic workers?|[Ss]ubagents?|[Ss]uperpowers:|plan-status|implementation workers?|main controller|子代理|主控|多轮[[:space:]]*质询|相互[[:space:]]*质询|内部审议|内部审查)'
SECRET_MATERIAL_REGEX='(sk-[A-Za-z0-9_-]{20,}|github_pat_[A-Za-z0-9_]{20,}|ghp_[A-Za-z0-9_]{20,}|-----BEGIN (RSA |OPENSSH |EC |DSA )?PRIVATE KEY-----)'
PROMO_INJECTION_REGEX='(邀请码|拉群|备用网址|欢迎加入|购买套餐|低价[[:space:]]*API|公益.*换[[:space:]]*key)'
CONTENT_DENY_REGEX="(${INTERNAL_TRACE_REGEX}|${SECRET_MATERIAL_REGEX}|${PROMO_INJECTION_REGEX})"

is_denied_path() {
  local path=$1
  [[ "${path}" =~ ${PATH_DENY_REGEX} ]]
}

is_denied_added_line() {
  local line=$1
  [[ "${line}" =~ ${CONTENT_DENY_REGEX} ]]
}

run_self_test() {
  local failed=0

  is_denied_path "target/debug/app" || failed=1
  is_denied_path "AGENTS.md" || failed=1
  is_denied_path "docs/architecture.md" && failed=1
  is_denied_path "docs/plans/product-improvement-roadmap.md" && failed=1

  is_denied_added_line "For agentic workers: use superpowers:executing-plans" || failed=1
  is_denied_added_line "A subagent may own this slice" || failed=1
  is_denied_added_line "token = sk-abcdefghijklmnopqrstuvwxyz123456" || failed=1
  is_denied_added_line "欢迎加入测试群" || failed=1
  is_denied_added_line "This roadmap defines a public operator contract." && failed=1

  if ((failed)); then
    printf 'staged denylist self-test failed\n' >&2
    return 1
  fi
}

if [[ "${1:-}" == "--self-test" ]]; then
  run_self_test
  exit 0
fi

violations=()
while IFS= read -r path; do
  if is_denied_path "${path}"; then
    violations+=("path:${path}")
  fi
done < <(git diff --cached --name-only)

while IFS= read -r line; do
  case "${line}" in
    '+++'* | '---'* | '@@'* | 'diff --git'* | 'index '* | 'new file mode '* | 'deleted file mode '*)
      continue
      ;;
    +*)
      added=${line:1}
      if is_denied_added_line "${added}"; then
        violations+=("content:${added:0:160}")
      fi
      ;;
  esac
done < <(git diff --cached --unified=0 --no-ext-diff --diff-filter=ACMRT -- . ':(exclude)scripts/check-staged-denylist.sh')

if ((${#violations[@]} > 0)); then
  printf 'staged denylist violation:\n' >&2
  printf '  %s\n' "${violations[@]}" >&2
  exit 1
fi

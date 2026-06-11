#!/usr/bin/env bash
set -euo pipefail

PATH_DENY_REGEX='^(dist|target|config|data|db|logs|keys|key-pool-router)/|(^|/)AGENTS\.md$|(^|/)docker-compose\.override\.yml$|(^|/)\.env($|\.)|(^|/)scripts/one_ai_key_(keys|upload_key)\.py$|(^|/)tests/test_one_ai_key_upload_key\.py$|(\.sqlite3?|\.db|\.db-[^/]*|\.wal|\.shm|\.keys|\.key|\.pem|\.log|\.pid|\.tmp)$|(^|/)[^/]*(key|keys|secret|secrets|token|tokens)[^/]*\.txt$'
INTERNAL_TRACE_REGEX='(For agentic workers|[Aa]gentic workers?|[Ss]ubagents?|[Ss]uperpowers:|plan-status|implementation workers?|main controller|子代理|主控|多轮[[:space:]]*质询|相互[[:space:]]*质询|内部审议|内部审查)'
SECRET_MATERIAL_REGEX='(sk-[A-Za-z0-9_-]{20,}|github_pat_[A-Za-z0-9_]{20,}|ghp_[A-Za-z0-9_]{20,}|-----BEGIN (RSA |OPENSSH |EC |DSA )?PRIVATE KEY-----)'
PROMO_INJECTION_REGEX='(邀请码|拉群|备用网址|欢迎加入|购买套餐|低价[[:space:]]*API|公益.*换[[:space:]]*key)'
CONTENT_DENY_REGEX="(${INTERNAL_TRACE_REGEX}|${SECRET_MATERIAL_REGEX}|${PROMO_INJECTION_REGEX})"
PUBLIC_DOC_ADDED_DENY_REGEX="(${CONTENT_DENY_REGEX}|[Ss]top[ -][Cc]ard|checkpoint evidence|artifact_sha|deployment_pin_evidence|operator_contract_evidence|raw command log|deployment transcript)"
PUBLIC_PLAN_DENY_REGEX="(${CONTENT_DENY_REGEX}|[Ss]top[ -][Cc]ard|Task [0-9]+ checkpoint|checkpoint evidence|Implementation Progress|Propagation audit|Cold scan result|Current status|artifact_sha|published_asset_verification|operator_contract_evidence|test_evidence|non_empty_filtered_test_evidence|local_ci_result|release_artifact_result|release_smoke_result|deployment_boundary_result|deployment_pin_evidence|production_smoke_result:[[:space:]]*\`pass|https://ai\.|hhhl|rtoc-gateway|dc\.hhhl|chat/room|Telegram|Discord|加入|购买|套餐|站长|充值|推广|广告|备用网址)"

is_denied_path() {
  local path=$1
  [[ "${path}" =~ ${PATH_DENY_REGEX} ]]
}

is_denied_added_line() {
  local line=$1
  [[ "${line}" =~ ${CONTENT_DENY_REGEX} ]]
}

is_denied_public_plan_line() {
  local line=$1
  [[ "${line}" =~ ${PUBLIC_PLAN_DENY_REGEX} ]]
}

is_public_doc_path() {
  local path=$1
  [[ "${path}" == "README.md" || "${path}" =~ ^docs/.*\.md$ ]]
}

is_denied_public_doc_added_line() {
  local line=$1
  [[ "${line}" =~ ${PUBLIC_DOC_ADDED_DENY_REGEX} ]]
}

check_public_plans() {
  local failed=0
  local path
  while IFS= read -r path; do
    [[ -f "${path}" ]] || continue
    local line_number=0
    local line
    while IFS= read -r line || [[ -n "${line}" ]]; do
      line_number=$((line_number + 1))
      if is_denied_public_plan_line "${line}"; then
        printf 'public plan hygiene violation: %s:%s: %s\n' \
          "${path}" "${line_number}" "${line:0:160}" >&2
        failed=1
      fi
    done < "${path}"
  done < <(git ls-files -- 'docs/plans/*.md')

  return "${failed}"
}

run_self_test() {
  local failed=0

  is_denied_path "target/debug/app" || failed=1
  is_denied_path "AGENTS.md" || failed=1
  is_denied_path "keys/local.keys" || failed=1
  is_denied_path "scripts/one_ai_key_upload_key.py" || failed=1
  is_denied_path "tests/test_one_ai_key_upload_key.py" || failed=1
  is_denied_path "docs/architecture.md" && failed=1
  is_denied_path "docs/plans/product-improvement-roadmap.md" && failed=1

  is_denied_added_line "For agentic workers: use superpowers:executing-plans" || failed=1
  is_denied_added_line "A subagent may own this slice" || failed=1
  is_denied_added_line "token = sk-abcdefghijklmnopqrstuvwxyz123456" || failed=1
  is_denied_added_line "欢迎加入测试群" || failed=1
  is_denied_added_line "This roadmap defines a public operator contract." && failed=1
  is_denied_public_doc_added_line "Stop Card: internal checkpoint" || failed=1
  is_denied_public_doc_added_line "artifact_sha: abc123" || failed=1
  is_denied_public_doc_added_line "This release verification record is public." && failed=1
  is_denied_public_plan_line "Task 7 checkpoint evidence:" || failed=1
  is_denied_public_plan_line "deployment_boundary_result: pass" || failed=1
  is_denied_public_plan_line "https://example.invalid/chat/room/test" || failed=1
  is_denied_public_plan_line "This roadmap defines a public operator contract." && failed=1
  is_denied_public_plan_line "Production smoke remains operator-run and redacted." && failed=1

  if ((failed)); then
    printf 'staged denylist self-test failed\n' >&2
    return 1
  fi
}

case "${1:-}" in
  --self-test)
    run_self_test
    exit 0
    ;;
  --check-public-plans)
    check_public_plans
    exit $?
    ;;
esac

violations=()
while IFS= read -r path; do
  if is_denied_path "${path}"; then
    violations+=("path:${path}")
  fi
done < <(git diff --cached --name-only)

current_diff_path=""
while IFS= read -r line; do
  case "${line}" in
    '+++ b/'*)
      current_diff_path="${line#+++ b/}"
      continue
      ;;
    '+++ /dev/null')
      current_diff_path=""
      continue
      ;;
    '+++'* | '---'* | '@@'* | 'diff --git'* | 'index '* | 'new file mode '* | 'deleted file mode '*)
      continue
      ;;
    +*)
      added=${line:1}
      if is_denied_added_line "${added}"; then
        violations+=("content:${added:0:160}")
      fi
      if is_public_doc_path "${current_diff_path}" && is_denied_public_doc_added_line "${added}"; then
        violations+=("public-doc:${current_diff_path}:${added:0:160}")
      fi
      ;;
  esac
done < <(git diff --cached --unified=0 --no-ext-diff --diff-filter=ACMRT -- . ':(exclude)scripts/check-staged-denylist.sh')

if ! check_public_plans; then
  violations+=("public-plans:tracked docs/plans hygiene check failed")
fi

if ((${#violations[@]} > 0)); then
  printf 'staged denylist violation:\n' >&2
  printf '  %s\n' "${violations[@]}" >&2
  exit 1
fi

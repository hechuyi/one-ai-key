#!/usr/bin/env bash
set -euo pipefail

ALLOW_PRODUCTION=false

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --allow-production)
      ALLOW_PRODUCTION=true
      shift
      ;;
    *)
      printf '{"status":"failed","reason_code":"unsupported_argument","message":"production smoke accepts only --allow-production"}\n'
      exit 2
      ;;
  esac
done

if [[ "${ALLOW_PRODUCTION}" != "true" ]]; then
  printf '{"status":"refused","reason_code":"production_smoke_requires_explicit_allow","message":"refusing production smoke without --allow-production"}\n'
  exit 2
fi

json_escape() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/\\n}"
  value="${value//$'\r'/\\r}"
  value="${value//$'\t'/\\t}"
  printf '%s' "${value}"
}

fail_json() {
  local reason_code="$1"
  local message="$2"
  printf '{"status":"failed","reason_code":"%s","message":"%s"}\n' \
    "$(json_escape "${reason_code}")" \
    "$(json_escape "${message}")"
  exit 2
}

resolve_existing_dir() {
  local path="$1"
  local reason_code="$2"
  local message="$3"
  local resolved
  if ! resolved=$(cd "${path}" 2>/dev/null && pwd -P); then
    fail_json "${reason_code}" "${message}"
  fi
  printf '%s' "${resolved}"
}

require_output_dir_writable() {
  local path="$1"
  local probe="${path}/.one-ai-key-production-smoke-write-test.$$"
  if ! ( : > "${probe}" ) 2>/dev/null; then
    fail_json "output_dir_not_writable" "ONE_AI_KEY_OUTPUT_DIR must be writable"
  fi
  rm -f "${probe}" 2>/dev/null || true
}

curl_config_escape() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/}"
  value="${value//$'\r'/}"
  printf '%s' "${value}"
}

safe_reason_code() {
  local value="${1:-}"
  local fallback="$2"
  if [[ "${value}" =~ ^[A-Za-z0-9_.:-]{1,80}$ ]]; then
    printf '%s' "${value}"
  else
    printf '%s' "${fallback}"
  fi
}

safe_optional_identity() {
  local name="$1"
  local value="${!name:-}"
  if [[ -z "${value}" ]]; then
    printf ''
    return
  fi
  local lower
  lower=$(printf '%s' "${value}" | tr '[:upper:]' '[:lower:]')
  if [[ ! "${value}" =~ ^[A-Za-z0-9_.:@+-]{1,128}$ ]] \
    || [[ "${lower}" == *"sk-"* ]] \
    || [[ "${lower}" == *"://"* ]] \
    || [[ "${lower}" == *"http"* ]] \
    || [[ "${lower}" == *"www."* ]] \
    || [[ "${lower}" == *"token"* ]] \
    || [[ "${lower}" == *"secret"* ]]; then
    fail_json "invalid_identity_value" "${name} must be a short non-secret release/checksum identity"
  fi
  printf '%s' "${value}"
}

require_env_value() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    fail_json "missing_required_env" "${name} is required"
  fi
}

require_env_name() {
  local name="$1"
  local value="$2"
  if [[ ! "${value}" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
    fail_json "invalid_token_env_name" "${name} must name an environment variable"
  fi
  if [[ -z "${!value:-}" ]]; then
    fail_json "missing_token_value" "${name} points to an empty environment variable"
  fi
}

require_env_value ONE_AI_KEY_PUBLIC_BASE_URL
require_env_value ONE_AI_KEY_MANAGEMENT_URL
require_env_value ONE_AI_KEY_CLIENT_TOKEN_ENV
require_env_value ONE_AI_KEY_MANAGEMENT_TOKEN_ENV
require_env_value ONE_AI_KEY_PUBLIC_MODEL

CLIENT_TOKEN_ENV_NAME="${ONE_AI_KEY_CLIENT_TOKEN_ENV}"
MANAGEMENT_TOKEN_ENV_NAME="${ONE_AI_KEY_MANAGEMENT_TOKEN_ENV}"
require_env_name ONE_AI_KEY_CLIENT_TOKEN_ENV "${CLIENT_TOKEN_ENV_NAME}"
require_env_name ONE_AI_KEY_MANAGEMENT_TOKEN_ENV "${MANAGEMENT_TOKEN_ENV_NAME}"

CLIENT_TOKEN="${!CLIENT_TOKEN_ENV_NAME}"
MANAGEMENT_TOKEN="${!MANAGEMENT_TOKEN_ENV_NAME}"
PUBLIC_MODEL="${ONE_AI_KEY_PUBLIC_MODEL}"
PUBLIC_BASE_URL="${ONE_AI_KEY_PUBLIC_BASE_URL%/}"
MANAGEMENT_URL="${ONE_AI_KEY_MANAGEMENT_URL%/}"
RELEASE_IDENTITY=$(safe_optional_identity ONE_AI_KEY_RELEASE_IDENTITY)
CHECKSUM_IDENTITY=$(safe_optional_identity ONE_AI_KEY_CHECKSUM_IDENTITY)

case "${PUBLIC_BASE_URL}" in
  */v1) ;;
  *) fail_json "invalid_public_base_url" "ONE_AI_KEY_PUBLIC_BASE_URL must include /v1" ;;
esac

case "${MANAGEMENT_URL}" in
  */v1) fail_json "invalid_management_url" "ONE_AI_KEY_MANAGEMENT_URL must be the management origin, not the client /v1 base URL" ;;
esac

SCRIPT_DIR=$(resolve_existing_dir "$(dirname "${BASH_SOURCE[0]}")" "script_dir_unavailable" "script directory is unavailable")
REPO_ROOT=$(resolve_existing_dir "${SCRIPT_DIR}/.." "repo_root_unavailable" "repository root is unavailable")

if [[ -n "${ONE_AI_KEY_OUTPUT_DIR:-}" ]]; then
  OUTPUT_DIR_INPUT="${ONE_AI_KEY_OUTPUT_DIR}"
  if [[ -d "${OUTPUT_DIR_INPUT}" ]]; then
    OUTPUT_DIR=$(resolve_existing_dir "${OUTPUT_DIR_INPUT}" "output_dir_unavailable" "ONE_AI_KEY_OUTPUT_DIR is unavailable")
  else
    OUTPUT_PARENT=$(dirname "${OUTPUT_DIR_INPUT}")
    OUTPUT_BASENAME=$(basename "${OUTPUT_DIR_INPUT}")
    if [[ ! -d "${OUTPUT_PARENT}" ]]; then
      fail_json "output_parent_missing" "ONE_AI_KEY_OUTPUT_DIR parent must exist"
    fi
    OUTPUT_PARENT_ABS=$(resolve_existing_dir "${OUTPUT_PARENT}" "output_parent_unavailable" "ONE_AI_KEY_OUTPUT_DIR parent is unavailable")
    OUTPUT_DIR="${OUTPUT_PARENT_ABS}/${OUTPUT_BASENAME}"
  fi
else
  TMP_PARENT=$(resolve_existing_dir "${TMPDIR:-/tmp}" "tmpdir_unavailable" "TMPDIR is unavailable")
  case "${TMP_PARENT}/" in
    "${REPO_ROOT}/"*) fail_json "tmpdir_inside_repository" "TMPDIR must not be inside the repository" ;;
  esac
  if ! OUTPUT_DIR=$(mktemp -d "${TMP_PARENT}/one-ai-key-production-smoke.XXXXXX" 2>/dev/null); then
    fail_json "output_dir_create_failed" "failed to create production smoke output directory"
  fi
fi

case "${OUTPUT_DIR}/" in
  "${REPO_ROOT}/"*) fail_json "output_dir_inside_repository" "ONE_AI_KEY_OUTPUT_DIR must not be inside the repository" ;;
esac

if ! mkdir -p "${OUTPUT_DIR}" 2>/dev/null; then
  fail_json "output_dir_create_failed" "failed to create production smoke output directory"
fi
require_output_dir_writable "${OUTPUT_DIR}"

for required_command in curl jq; do
  if ! command -v "${required_command}" >/dev/null 2>&1; then
    printf '{"status":"failed","reason_code":"missing_required_command","command":"%s"}\n' "${required_command}"
    exit 2
  fi
done

CURL_CONNECT_TIMEOUT="${ONE_AI_KEY_SMOKE_CONNECT_TIMEOUT_SECONDS:-10}"
CURL_MAX_TIME="${ONE_AI_KEY_SMOKE_MAX_TIME_SECONDS:-30}"
PUBLIC_ORIGIN="${PUBLIC_BASE_URL%/v1}"

CHECK_FILES=()
FAILED=0
CURL_BODY=""
CURL_STATUS=""

status_number() {
  local status="$1"
  if [[ "${status}" =~ ^[0-9]+$ ]]; then
    printf '%d' "$((10#${status}))"
  else
    printf '0'
  fi
}

write_curl_config() {
  local method="$1"
  local url="$2"
  local token="${3:-}"
  printf 'silent\n'
  printf 'write-out = "\\n%%{http_code}"\n'
  printf 'request = "%s"\n' "$(curl_config_escape "${method}")"
  printf 'connect-timeout = "%s"\n' "$(curl_config_escape "${CURL_CONNECT_TIMEOUT}")"
  printf 'max-time = "%s"\n' "$(curl_config_escape "${CURL_MAX_TIME}")"
  if [[ -n "${token}" ]]; then
    printf 'header = "Authorization: Bearer %s"\n' "$(curl_config_escape "${token}")"
  fi
  printf 'url = "%s"\n' "$(curl_config_escape "${url}")"
}

curl_capture() {
  local method="$1"
  local url="$2"
  local token="${3:-}"
  local request_body="${4:-}"

  local response
  if [[ -n "${request_body}" ]]; then
    if ! response=$(printf '%s' "${request_body}" | curl --config <(write_curl_config "${method}" "${url}" "${token}") --header "Content-Type: application/json" --data-binary @- 2>/dev/null); then
      CURL_STATUS="000"
      CURL_BODY=""
      return
    fi
  else
    if ! response=$(curl --config <(write_curl_config "${method}" "${url}" "${token}") 2>/dev/null); then
      CURL_STATUS="000"
      CURL_BODY=""
      return
    fi
  fi

  local separator=$'\n'
  CURL_STATUS="${response##*${separator}}"
  CURL_BODY="${response%${separator}*}"
  if [[ "${CURL_STATUS}" == "${response}" ]]; then
    CURL_STATUS="000"
    CURL_BODY=""
  fi
}

json_field() {
  local body="$1"
  local filter="$2"
  jq -r "${filter} // empty" <<<"${body}" 2>/dev/null || true
}

write_check() {
  local name="$1"
  local ok="$2"
  local http_status="$3"
  local reason_code="$4"
  reason_code=$(safe_reason_code "${reason_code}" "${name}_failed")
  local status_text="passed"
  if [[ "${ok}" != "true" ]]; then
    status_text="failed"
    FAILED=1
  fi
  local output="${OUTPUT_DIR}/${name}.json"
  if ! ( jq -n \
    --arg check "${name}" \
    --arg status "${status_text}" \
    --arg reason_code "${reason_code}" \
    --argjson ok "${ok}" \
    --argjson http_status "${http_status}" \
    '{
      check: $check,
      status: $status,
      ok: $ok,
      http_status: $http_status,
      reason_code: $reason_code
    }' > "${output}" ) 2>/dev/null; then
    fail_json "output_file_write_failed" "failed to write production smoke check artifact"
  fi
  CHECK_FILES+=("${output}")
}

curl_capture GET "${PUBLIC_ORIGIN}/health"
PUBLIC_LIVENESS_STATUS="${CURL_STATUS}"
PUBLIC_LIVENESS_HTTP=$(status_number "${PUBLIC_LIVENESS_STATUS}")
if [[ "${PUBLIC_LIVENESS_STATUS}" == "200" ]]; then
  write_check public_liveness true "${PUBLIC_LIVENESS_HTTP}" "public_liveness_ok"
else
  write_check public_liveness false "${PUBLIC_LIVENESS_HTTP}" "public_liveness_failed"
fi

curl_capture GET "${MANAGEMENT_URL}/management/health/serving" "${MANAGEMENT_TOKEN}"
SERVING_STATUS="${CURL_STATUS}"
SERVING_BODY="${CURL_BODY}"
SERVING_HTTP=$(status_number "${SERVING_STATUS}")
SERVING_STATE=$(json_field "${SERVING_BODY}" '.status')
SERVING_FLAG=$(json_field "${SERVING_BODY}" '.serving')
if [[ "${SERVING_STATUS}" == "200" && "${SERVING_FLAG}" == "true" ]]; then
  write_check management_serving true "${SERVING_HTTP}" "management_serving_ok"
else
  write_check management_serving false "${SERVING_HTTP}" "${SERVING_STATE:-management_serving_failed}"
fi

curl_capture GET "${MANAGEMENT_URL}/management/health/resilience" "${MANAGEMENT_TOKEN}"
RESILIENCE_STATUS="${CURL_STATUS}"
RESILIENCE_BODY="${CURL_BODY}"
RESILIENCE_HTTP=$(status_number "${RESILIENCE_STATUS}")
RESILIENCE_STATE=$(json_field "${RESILIENCE_BODY}" '.status')
if [[ "${RESILIENCE_STATUS}" == "200" && "${RESILIENCE_STATE}" != "blocked" ]]; then
  write_check management_resilience true "${RESILIENCE_HTTP}" "${RESILIENCE_STATE:-management_resilience_ok}"
else
  write_check management_resilience false "${RESILIENCE_HTTP}" "${RESILIENCE_STATE:-management_resilience_failed}"
fi

curl_capture GET "${PUBLIC_BASE_URL}/models" "${CLIENT_TOKEN}"
MODELS_STATUS="${CURL_STATUS}"
MODELS_BODY="${CURL_BODY}"
MODELS_HTTP=$(status_number "${MODELS_STATUS}")
MODEL_VISIBLE=$(jq -e --arg model "${PUBLIC_MODEL}" '.data | map(.id) | index($model) != null' <<<"${MODELS_BODY}" >/dev/null 2>&1 && printf true || printf false)
if [[ "${MODELS_STATUS}" == "200" && "${MODEL_VISIBLE}" == "true" ]]; then
  write_check client_models true "${MODELS_HTTP}" "client_model_visible"
else
  write_check client_models false "${MODELS_HTTP}" "client_model_not_visible"
fi

COMPLETION_REQUEST=$(jq -cn --arg model "${PUBLIC_MODEL}" \
  '{model:$model,messages:[{role:"user",content:"Return the word ok."}],stream:false}')

curl_capture POST "${PUBLIC_BASE_URL}/chat/completions" "${CLIENT_TOKEN}" "${COMPLETION_REQUEST}"
COMPLETION_STATUS="${CURL_STATUS}"
COMPLETION_BODY="${CURL_BODY}"
COMPLETION_HTTP=$(status_number "${COMPLETION_STATUS}")
COMPLETION_HAS_MESSAGE=$(jq -e '.choices[0].message.content | type == "string"' <<<"${COMPLETION_BODY}" >/dev/null 2>&1 && printf true || printf false)
if [[ "${COMPLETION_STATUS}" == "200" && "${COMPLETION_HAS_MESSAGE}" == "true" ]]; then
  write_check client_completion true "${COMPLETION_HTTP}" "client_completion_ok"
else
  COMPLETION_REASON=$(json_field "${COMPLETION_BODY}" '.error.code // .code')
  write_check client_completion false "${COMPLETION_HTTP}" "${COMPLETION_REASON:-client_completion_failed}"
fi

OVERALL_STATUS="ok"
OVERALL_REASON="production_smoke_ok"
if [[ "${FAILED}" -ne 0 ]]; then
  OVERALL_STATUS="failed"
  OVERALL_REASON="production_smoke_failed"
fi

SUMMARY_PATH="${OUTPUT_DIR}/summary.json"
if ! ( jq -s \
  --arg status "${OVERALL_STATUS}" \
  --arg reason_code "${OVERALL_REASON}" \
  --arg model "${PUBLIC_MODEL}" \
  --arg release_identity "${RELEASE_IDENTITY}" \
  --arg checksum_identity "${CHECKSUM_IDENTITY}" \
  --arg client_token_env "${CLIENT_TOKEN_ENV_NAME}" \
  --arg management_token_env "${MANAGEMENT_TOKEN_ENV_NAME}" \
  '{
    status: $status,
    reason_code: $reason_code,
    model: $model,
    release_identity: (if $release_identity == "" then null else $release_identity end),
    checksum_identity: (if $checksum_identity == "" then null else $checksum_identity end),
    token_sources: {
      client_token_env: $client_token_env,
      management_token_env: $management_token_env
    },
    checks: .,
    artifacts: {
      summary: "summary.json",
      redaction: [
        "raw request bodies not recorded",
        "raw response bodies not recorded",
        "token values not recorded",
        "complete URLs not recorded"
      ]
    }
  }' "${CHECK_FILES[@]}" > "${SUMMARY_PATH}" ) 2>/dev/null; then
  fail_json "summary_write_failed" "failed to write production smoke summary"
fi

cat "${SUMMARY_PATH}"
printf '\n'

if [[ "${FAILED}" -ne 0 ]]; then
  exit 1
fi

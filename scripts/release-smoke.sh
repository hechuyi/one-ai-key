#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)
NIX_STORE_VOLUME=${ONE_AI_KEY_NIX_STORE_VOLUME:-one-ai-key-nix-amd64}

if [[ "${1:-}" != "--inside-container" ]]; then
  if [[ "$(uname -s)" != "Linux" || "$(uname -m)" != "x86_64" ]]; then
    if ! command -v docker >/dev/null 2>&1; then
      printf 'error: docker is required to run release smoke outside x86_64 Linux\n' >&2
      exit 1
    fi
    exec docker run --rm \
      --platform linux/amd64 \
      -v "${REPO_ROOT}:/work" \
      -v "${NIX_STORE_VOLUME}:/nix" \
      -w /work \
      nixos/nix:latest \
      nix --extra-experimental-features "nix-command flakes" shell \
        nixpkgs#bash \
        nixpkgs#cargo \
        nixpkgs#curl \
        nixpkgs#jq \
        nixpkgs#python3 \
        nixpkgs#gnutar \
        nixpkgs#gzip \
        nixpkgs#coreutils \
        nixpkgs#perl \
        --command bash -lc '/work/scripts/release-smoke.sh --inside-container'
  fi
fi

for required in bash cargo curl jq python3 tar gzip shasum; do
  if ! command -v "${required}" >/dev/null 2>&1; then
    printf 'error: release smoke requires %s\n' "${required}" >&2
    exit 1
  fi
done

cd "${REPO_ROOT}"
unset KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE
unset KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE

PACKAGE_ID=$(cargo pkgid --locked)
PACKAGE_SPEC=${PACKAGE_ID##*#}
if [[ "${PACKAGE_SPEC}" == *@* ]]; then
  VERSION=${PACKAGE_SPEC##*@}
else
  VERSION=${PACKAGE_SPEC}
fi
PACKAGE_METADATA=$(cargo metadata --locked --no-deps --format-version 1)
PACKAGE_NAME=$(jq -r '.workspace_members[0] as $root | .packages[] | select(.id == $root) | .name' <<<"${PACKAGE_METADATA}")
METADATA_VERSION=$(jq -r '.workspace_members[0] as $root | .packages[] | select(.id == $root) | .version' <<<"${PACKAGE_METADATA}")
if [[ -z "${PACKAGE_NAME}" || "${PACKAGE_NAME}" == "null" || "${METADATA_VERSION}" != "${VERSION}" ]]; then
  printf 'error: could not derive release package metadata from cargo\n' >&2
  exit 1
fi
TARGET=${TARGET:-x86_64-unknown-linux-gnu}
ARCHIVE_NAME="${PACKAGE_NAME}-${VERSION}-${TARGET}.tar.gz"
ARCHIVE_PATH="${REPO_ROOT}/dist/${ARCHIVE_NAME}"
CHECKSUM_PATH="${ARCHIVE_PATH}.sha256"

if [[ ! -f "${ARCHIVE_PATH}" || ! -f "${CHECKSUM_PATH}" ]]; then
  printf 'error: expected release artifacts are missing under dist/. Run scripts/build-release-x86_64-linux-docker.sh first.\n' >&2
  exit 1
fi

(cd "${REPO_ROOT}/dist" && shasum -a 256 -c "${ARCHIVE_NAME}.sha256")

WORK_DIR=$(mktemp -d)
MOCK_PID=
SERVICE_PID=
cleanup() {
  if [[ -n "${SERVICE_PID}" ]]; then
    kill "${SERVICE_PID}" >/dev/null 2>&1 || true
    wait "${SERVICE_PID}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${MOCK_PID}" ]]; then
    kill "${MOCK_PID}" >/dev/null 2>&1 || true
    wait "${MOCK_PID}" >/dev/null 2>&1 || true
  fi
  rm -rf "${WORK_DIR}"
}
trap cleanup EXIT

tar -xzf "${ARCHIVE_PATH}" -C "${WORK_DIR}"
BIN="${WORK_DIR}/${PACKAGE_NAME}"
if [[ ! -x "${BIN}" ]]; then
  printf 'error: extracted release binary is not executable: %s\n' "${BIN}" >&2
  exit 1
fi

cd "${WORK_DIR}"

"${BIN}" --help >/dev/null
"${BIN}" init local --out config/local.yaml --keys data/relay.keys --dry-run --output json \
  | jq -e '.status == "dry_run" and .reason_code == "init_local_dry_run" and (.next_action.safe_argv | length > 0)' >/dev/null
"${BIN}" init local --out config/local.yaml --keys data/relay.keys --yes --output json \
  | jq -e '.status == "created" and .reason_code == "init_local_created" and (.next_action.safe_argv | length == 0)' >/dev/null

CLIENT_TOKEN="release-smoke-client-token"
MANAGEMENT_TOKEN="release-smoke-management-token"
UPSTREAM_TOKEN="release-smoke-upstream-token"
INVALID_CLIENT_TOKEN="release-smoke-invalid-client-token"
SERVICE_PORT=$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)
MOCK_PORT=$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)
MOCK_UPSTREAM_URL="http://127.0.0.1:${MOCK_PORT}/v1"
RAW_BODY_TEXT="raw body text"

cat > mock_upstream.py <<'PY'
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

class Handler(BaseHTTPRequestHandler):
    def _send_json(self, payload, status=200):
        raw = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        if self.path == "/v1/models":
            self._send_json({"object": "list", "data": [{"id": "provider/gpt-example", "object": "model"}]})
            return
        if self.path == "/v1/models/provider/gpt-example":
            self._send_json({"id": "provider/gpt-example", "object": "model"})
            return
        self._send_json({"error": {"code": "not_found"}}, status=404)

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length) if length else b"{}"
        try:
            request = json.loads(body.decode("utf-8"))
        except Exception:
            request = {}
        if self.path == "/v1/chat/completions":
            self._send_json({
                "id": "release-smoke-response",
                "object": "chat.completion",
                "model": request.get("model", "provider/gpt-example"),
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "release smoke ok"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 1, "completion_tokens": 3, "total_tokens": 4}
            })
            return
        self._send_json({"error": {"code": "not_found"}}, status=404)

    def log_message(self, _format, *_args):
        return

ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
PY

python3 mock_upstream.py "${MOCK_PORT}" >mock-upstream.log 2>&1 &
MOCK_PID=$!

python3 - <<'PY' "${SERVICE_PORT}" "${MOCK_UPSTREAM_URL}" "${CLIENT_TOKEN}" "${MANAGEMENT_TOKEN}" "config/local.yaml"
import pathlib
import sys

service_port, mock_upstream_url, client_token, management_token, config_path = sys.argv[1:]
path = pathlib.Path(config_path)
text = path.read_text()
text = text.replace("listen: 127.0.0.1:4101", f"listen: 127.0.0.1:{service_port}")
text = text.replace("<client-token-placeholder>", client_token)
text = text.replace("<management-token-placeholder>", management_token)
text = text.replace("https://relay.example/v1", mock_upstream_url)
path.write_text(text)
PY
printf '%s\n' "${UPSTREAM_TOKEN}" > data/relay.keys

"${BIN}" --config config/local.yaml check-config --output json \
  | tee check-config.json \
  | jq -e '.status == "ok" and .reason_code == "ok" and (.model_visibility_preview[] | select(.client_token_ref == "local-client") | .visible_models == ["gpt-example"])' >/dev/null

"${BIN}" --config config/local.yaml serve >service.log 2>&1 &
SERVICE_PID=$!

for _ in $(seq 1 100); do
  if curl -fsS "http://127.0.0.1:${SERVICE_PORT}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
curl -fsS "http://127.0.0.1:${SERVICE_PORT}/health" >/dev/null
curl -fsS "http://127.0.0.1:${SERVICE_PORT}/ready" \
  | jq -e '.status == "ready" and (.serving_channels > 0) and (.credentials.available > 0)' >/dev/null

MODELS_JSON=$(curl -fsS "http://127.0.0.1:${SERVICE_PORT}/v1/models" \
  -H "Authorization: Bearer ${CLIENT_TOKEN}")
printf '%s\n' "${MODELS_JSON}" \
  | jq -e '.data | map(.id) == ["gpt-example"]' >/dev/null
printf '%s\n' "${MODELS_JSON}" \
  | jq -e '["relay", "provider", "credential", "capabilit"] as $forbidden | tostring as $body | [$forbidden[] | select(. as $term | $body | contains($term))] | length == 0' >/dev/null

curl -fsS "http://127.0.0.1:${SERVICE_PORT}/v1/chat/completions" \
  -H "Authorization: Bearer ${CLIENT_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-example","messages":[{"role":"user","content":"ping"}]}' \
  | jq -e '.choices[0].message.content == "release smoke ok"' >/dev/null

INVALID_CLIENT_BODY="${WORK_DIR}/invalid-client.json"
INVALID_CLIENT_STATUS=$(curl -sS -o "${INVALID_CLIENT_BODY}" -w "%{http_code}" \
  "http://127.0.0.1:${SERVICE_PORT}/v1/models" \
  -H "Authorization: Bearer ${INVALID_CLIENT_TOKEN}")
if [[ "${INVALID_CLIENT_STATUS}" != "401" ]]; then
  printf 'error: invalid client token returned HTTP %s instead of 401\n' "${INVALID_CLIENT_STATUS}" >&2
  exit 1
fi
jq -e --arg invalid_client_token "${INVALID_CLIENT_TOKEN}" '
  .error.message == "invalid router api key"
  and (.error | tostring | contains($invalid_client_token) | not)
' "${INVALID_CLIENT_BODY}" >/dev/null
for forbidden_token in \
  "${INVALID_CLIENT_TOKEN}" \
  "${CLIENT_TOKEN}" \
  "${MANAGEMENT_TOKEN}" \
  "${UPSTREAM_TOKEN}"
do
  if grep -Fq "${forbidden_token}" "${INVALID_CLIENT_BODY}"; then
    printf 'error: invalid client token response leaked token material\n' >&2
    exit 1
  fi
done

CHECK_MODELS=$(jq -c '.model_visibility_preview[] | select(.client_token_ref == "local-client") | .visible_models' check-config.json)
CLIENT_MODELS=$(printf '%s\n' "${MODELS_JSON}" | jq -c '.data | map(.id)')
if [[ "${CHECK_MODELS}" != "${CLIENT_MODELS}" ]]; then
  printf 'error: check-config visibility does not match authenticated /v1/models\n' >&2
  exit 1
fi

export ONE_AI_KEY_MANAGEMENT_TOKEN="${MANAGEMENT_TOKEN}"
MANAGEMENT_URL="http://127.0.0.1:${SERVICE_PORT}"
COMMON=(--management-url "${MANAGEMENT_URL}" --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN)
MANAGEMENT_REPORTS=()
EXPECTED_MANAGEMENT_REPORTS=(
  "doctor.json"
  "client-tokens-list.json"
  "models-list.json"
  "models-explain.json"
  "route-explain.json"
  "keys-stats.json"
  "failures-tail.json"
  "reload-status.json"
  "reload-diff.json"
  "reload-apply-dry-run.json"
  "negative-management-url.txt"
)

capture_management_report() {
  local output_path=$1
  shift
  "${BIN}" "${COMMON[@]}" "$@" > "${output_path}"
  MANAGEMENT_REPORTS+=("${output_path}")
}

assert_bounded_evidence() {
  local report_path=$1
  local max_items=8
  jq -e --argjson max_items "${max_items}" '
    (.evidence | type == "object")
    and (.evidence.candidate_reason_codes | type == "array" and length <= $max_items)
    and (.evidence.candidate_limit | type == "number" and .evidence.candidate_limit >= 0)
    and (.evidence.route_target_count | type == "number" and .evidence.route_target_count >= 0)
    and (.evidence.endpoint_family_target_count | type == "number" and .evidence.endpoint_family_target_count >= 0)
    and (.evidence.unsupported_target_count | type == "number" and .evidence.unsupported_target_count >= 0)
    and (.evidence.unknown_or_missing_target_count | type == "number" and .evidence.unknown_or_missing_target_count >= 0)
    and (.evidence.preview_candidate_count | type == "number" and .evidence.preview_candidate_count >= 0)
  ' "${report_path}" >/dev/null
}

assert_expected_management_reports_captured() {
  if [[ "${#MANAGEMENT_REPORTS[@]}" -ne "${#EXPECTED_MANAGEMENT_REPORTS[@]}" ]]; then
    printf 'error: management report list did not match expected captured reports\n' >&2
    exit 1
  fi
  local index
  for index in "${!EXPECTED_MANAGEMENT_REPORTS[@]}"; do
    if [[ "${MANAGEMENT_REPORTS[$index]}" != "${EXPECTED_MANAGEMENT_REPORTS[$index]}" ]]; then
      printf 'error: management report list did not match expected captured reports\n' >&2
      exit 1
    fi
  done
}

assert_no_management_report_leaks() {
  if [[ "$#" -gt 0 ]]; then
    :
  else
    printf 'error: no management reports were provided for leak scanning\n' >&2
    exit 1
  fi
  local raw_management_url="${MANAGEMENT_URL}/v1"
  local forbidden_literals=(
    "${CLIENT_TOKEN}"
    "${MANAGEMENT_TOKEN}"
    "${UPSTREAM_TOKEN}"
    "${INVALID_CLIENT_TOKEN}"
    "${WORK_DIR}"
    "data/relay.keys"
    "${MOCK_UPSTREAM_URL}"
    "${MANAGEMENT_URL}"
    "${raw_management_url}"
    "${RAW_BODY_TEXT}"
  )
  local report
  local forbidden
  for report in "$@"; do
    if [[ -s "${report}" ]]; then
      :
    else
      printf 'error: management report is missing or empty: %s\n' "${report}" >&2
      exit 1
    fi
    for forbidden in "${forbidden_literals[@]}"; do
      if grep -Fq "${forbidden}" "${report}"; then
        printf 'error: management report %s leaked forbidden local value\n' "${report}" >&2
        exit 1
      fi
    done
    if grep -Eq "https?://[^[:space:]\"']*(token|key|secret|bearer|s[k]-)[^[:space:]\"']*" "${report}"; then
      printf 'error: management report %s leaked token-looking URL component\n' "${report}" >&2
      exit 1
    fi
  done
}

capture_management_report doctor.json doctor --output json
jq -e '.status and .reason_code and .side_effect_class and (.next_action.safe_argv | type == "array")' doctor.json >/dev/null
capture_management_report client-tokens-list.json client-tokens list --output json
jq -e '.status == "ok" and .reason_code == "client_tokens_available" and .side_effect_class and (.next_action.safe_argv | type == "array")' client-tokens-list.json >/dev/null
capture_management_report models-list.json models list --client-token-ref local-client --output json
jq -e '.status == "ok" and .reason_code == "model_routes_available" and .side_effect_class and (.next_action.safe_argv | type == "array")' models-list.json >/dev/null
capture_management_report models-explain.json models explain --model gpt-example --client-token-ref local-client --endpoint-family chat_completions --output json
jq -e '
  .status == "ok"
  and .can_use == true
  and .reason_code == "available"
  and .blocking_domain == "none"
  and .endpoint_family == "chat_completions"
  and .model == "gpt-example"
  and .client_token_ref == "local-client"
  and (.next_action.template_id | type == "string")
  and .next_action.side_effect_class == "runtime_readonly"
  and .next_action.requires_confirmation == false
  and (.next_action.safe_argv | type == "array")
  and .side_effect_class == "runtime_readonly"
  and (.data.availability.next_step.template_id | type == "string")
  and .data.availability.next_step.side_effect_class == "runtime_readonly"
  and .data.availability.next_step.requires_confirmation == false
  and (.data.availability.next_step.safe_argv | type == "array")
' models-explain.json >/dev/null
assert_bounded_evidence models-explain.json
MODEL_EXPLAIN_MODEL=$(jq -r '.model' models-explain.json)
MODEL_EXPLAIN_CLIENT_TOKEN_REF=$(jq -r '.client_token_ref' models-explain.json)
MODEL_EXPLAIN_REASON_CODE=$(jq -r '.reason_code' models-explain.json)
capture_management_report route-explain.json route explain gpt-example --client-token-ref local-client --output json
jq -e --arg model "${MODEL_EXPLAIN_MODEL}" --arg client_token_ref "${MODEL_EXPLAIN_CLIENT_TOKEN_REF}" --arg reason_code "${MODEL_EXPLAIN_REASON_CODE}" '
  .status == "available" and .reason_code == "available"
  and .reason_code == $reason_code
  and .admission_summary.status == "available"
  and .admission_summary.reason_code == "available"
  and .admission_summary.reason_code == .reason_code
  and .model == $model
  and .scope.client_token_ref == $client_token_ref
  and (.selected_target.channel_id | type == "string")
  and (.admission_summary.selected_target.channel_id == .selected_target.channel_id)
  and (.candidates | type == "array" and length > 0)
  and .side_effect_class == "runtime_readonly"
  and (.next_action.safe_argv | type == "array")
' route-explain.json >/dev/null
capture_management_report keys-stats.json keys stats --credential-set relay_credentials --output json
jq -e '.status and .reason_code and .side_effect_class and (.next_action.safe_argv | type == "array")' keys-stats.json >/dev/null
capture_management_report failures-tail.json failures tail --last 20 --output json
jq -e '
  .status
  and .reason_code
  and .side_effect_class
  and (.next_action.safe_argv | type == "array")
  and .availability_source == "bounded_evidence"
  and .current_availability == false
  and .window.kind == "bounded_recent_events"
  and (.window.limit | type == "number")
  and (.window.returned | type == "number")
  and (.window.truncated | type == "boolean")
  and .data.failure_count == 0
  and (.data.failures | length == 0)
' failures-tail.json >/dev/null
capture_management_report reload-status.json reload status --output json
jq -e '.status and .reason_code and .side_effect_class and (.next_action.safe_argv | type == "array")' reload-status.json >/dev/null
capture_management_report reload-diff.json reload diff --output json
jq -e '.status and .reason_code and .side_effect_class and (.next_action.safe_argv | type == "array")' reload-diff.json >/dev/null
capture_management_report reload-apply-dry-run.json reload apply --dry-run --output json
jq -e '.status == "planned" and .reason_code == "reload_apply_dry_run" and .side_effect_class and (.next_action.safe_argv | type == "array")' reload-apply-dry-run.json >/dev/null

set +e
NEGATIVE_OUTPUT=$("${BIN}" --management-url "${MANAGEMENT_URL}/v1" --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN models list --output json 2>&1)
NEGATIVE_STATUS=$?
set -e
if [[ "${NEGATIVE_STATUS}" -eq 0 || "${NEGATIVE_OUTPUT}" != *"client_base_url_used_for_management"* ]]; then
  printf 'error: client /v1 base URL was not rejected for management commands\n' >&2
  exit 1
fi
printf '%s\n' "${NEGATIVE_OUTPUT}" > negative-management-url.txt
MANAGEMENT_REPORTS+=("negative-management-url.txt")

assert_expected_management_reports_captured
assert_no_management_report_leaks "${MANAGEMENT_REPORTS[@]}"

printf 'release smoke passed for %s %s\n' "${PACKAGE_NAME}" "${VERSION}"

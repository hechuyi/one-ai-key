#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "${SCRIPT_DIR}/.." && pwd)
NIX_STORE_VOLUME=${ONE_AI_KEY_NIX_STORE_VOLUME:-one-ai-key-nix-amd64}
NIX_CACHE_VOLUME=${ONE_AI_KEY_NIX_CACHE_VOLUME:-one-ai-key-nix-cache-amd64}
CARGO_TARGET_VOLUME=${ONE_AI_KEY_CARGO_TARGET_VOLUME:-one-ai-key-cargo-target-amd64}
CARGO_HOME_VOLUME=${ONE_AI_KEY_CARGO_HOME_VOLUME:-one-ai-key-cargo-home-amd64}

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
      -v "${NIX_CACHE_VOLUME}:/root/.cache/nix" \
      -v "${CARGO_TARGET_VOLUME}:/cargo-target" \
      -v "${CARGO_HOME_VOLUME}:/cargo-home" \
      -e CARGO_TARGET_DIR=/cargo-target \
      -e CARGO_HOME=/cargo-home \
      -w /work \
      nixos/nix:latest \
      nix --extra-experimental-features "nix-command flakes" shell \
        nixpkgs#bash \
        nixpkgs#cargo \
        nixpkgs#curl \
        nixpkgs#git \
        nixpkgs#jq \
        nixpkgs#python3 \
        nixpkgs#gnutar \
        nixpkgs#gzip \
        nixpkgs#coreutils \
        nixpkgs#perl \
        --command bash -lc '/work/scripts/release-smoke.sh --inside-container'
  fi
fi

for required in bash cargo curl git jq python3 tar gzip shasum; do
  if ! command -v "${required}" >/dev/null 2>&1; then
    printf 'error: release smoke requires %s\n' "${required}" >&2
    exit 1
  fi
done

cd "${REPO_ROOT}"
unset KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE
unset KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE

sha256_stdin() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum
  else
    shasum -a 256
  fi
}

sha256_files_from_stdin() {
  if command -v sha256sum >/dev/null 2>&1; then
    xargs -0 sha256sum
  else
    xargs -0 shasum -a 256
  fi
}

current_source_tree_hash() {
  git ls-files -z \
    | LC_ALL=C sort -z \
    | sha256_files_from_stdin \
    | sha256_stdin \
    | cut -d ' ' -f 1
}

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
BUILD_INFO_PATH="${ARCHIVE_PATH}.build.json"

if [[ ! -f "${ARCHIVE_PATH}" || ! -f "${CHECKSUM_PATH}" || ! -f "${BUILD_INFO_PATH}" ]]; then
  printf 'error: expected release artifacts are missing under dist/. Run scripts/build-release-x86_64-linux-docker.sh first.\n' >&2
  exit 1
fi

(cd "${REPO_ROOT}/dist" && shasum -a 256 -c "${ARCHIVE_NAME}.sha256")
CURRENT_SOURCE_TREE_HASH=$(current_source_tree_hash)
if ! jq -e \
  --arg package_name "${PACKAGE_NAME}" \
  --arg version "${VERSION}" \
  --arg target "${TARGET}" \
  --arg archive_name "${ARCHIVE_NAME}" \
  --arg source_tree_hash "${CURRENT_SOURCE_TREE_HASH}" \
  '
    .schema_version == 1
    and .package_name == $package_name
    and .version == $version
    and .target == $target
    and .archive_name == $archive_name
    and .source_tree_hash == $source_tree_hash
    and ((.archive_sha256 | type) == "string")
    and (.archive_sha256 | length) == 64
  ' "${BUILD_INFO_PATH}" >/dev/null; then
  printf 'error: release smoke source tree fingerprint does not match the built artifact. Rebuild dist/ with scripts/build-release-x86_64-linux-docker.sh.\n' >&2
  exit 1
fi

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
REPLACEMENT_TOKEN="release-smoke-replacement-token"
INVALID_CLIENT_TOKEN="release-smoke-invalid-client-token"
NEW_CLIENT_TOKEN="release-smoke-new-client-token"
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
MOCK_UPSTREAM_EVENTS="${WORK_DIR}/mock-upstream-events.jsonl"
RAW_BODY_TEXT="raw body text"
UPSTREAM_503_MARKER="release smoke upstream 503"
LOCAL_ADMISSION_MARKER="release smoke local admission 503"

cat > mock_upstream.py <<'PY'
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

UPSTREAM_503_MARKER = "release smoke upstream 503"

class Handler(BaseHTTPRequestHandler):
    def _log_event(self, method):
        with open(sys.argv[2], "a", encoding="utf-8") as handle:
            handle.write(json.dumps({"method": method, "path": self.path}) + "\n")

    def _send_json(self, payload, status=200, headers=None):
        raw = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        for name, value in (headers or {}).items():
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        self._log_event("GET")
        if self.path == "/v1/models":
            self._send_json({"object": "list", "data": [{"id": "provider/gpt-example", "object": "model"}]})
            return
        if self.path in ("/v1/models/provider/gpt-example", "/v1/models/provider%2Fgpt-example"):
            self._send_json({"id": "provider/gpt-example", "object": "model"})
            return
        if self.path in ("/v1/models/provider/invalid-probe", "/v1/models/provider%2Finvalid-probe"):
            self._send_json({"error": {"code": "invalid_api_key"}}, status=401)
            return
        self._send_json({"error": {"code": "not_found"}}, status=404)

    def do_POST(self):
        self._log_event("POST")
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length) if length else b"{}"
        try:
            request = json.loads(body.decode("utf-8"))
        except Exception:
            request = {}
        if self.path == "/v1/chat/completions":
            messages = request.get("messages", [])
            if any(isinstance(message, dict) and message.get("content") == UPSTREAM_503_MARKER for message in messages):
                self._send_json({"error": {"code": "provider_unavailable", "message": UPSTREAM_503_MARKER}}, status=503, headers={"Retry-After": "30"})
                return
            self._send_json({
                "id": "release-smoke-response",
                "object": "chat.completion",
                "model": request.get("model", "provider/gpt-example"),
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "release smoke published model ok" if request.get("model") == "release-smoke-upstream-model" else "release smoke ok"},
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

: > "${MOCK_UPSTREAM_EVENTS}"
python3 mock_upstream.py "${MOCK_PORT}" "${MOCK_UPSTREAM_EVENTS}" >mock-upstream.log 2>&1 &
MOCK_PID=$!

mock_upstream_request_count() {
  local method=$1
  local request_path=$2
  python3 - <<'PY' "${MOCK_UPSTREAM_EVENTS}" "${method}" "${request_path}"
import json
import pathlib
import sys
from urllib.parse import urlsplit

path = pathlib.Path(sys.argv[1])
method = sys.argv[2]
request_path = sys.argv[3]
count = 0
if path.exists():
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        try:
            event = json.loads(line)
        except json.JSONDecodeError as error:
            raise SystemExit(f"failed to parse mock upstream event JSONL line {line_number}: {error}") from error
        if not isinstance(event, dict) or not isinstance(event.get("method"), str) or not isinstance(event.get("path"), str):
            raise SystemExit(f"mock upstream event missing string method/path on JSONL line {line_number}")
        if event["method"] == method and urlsplit(event["path"]).path == request_path:
            count += 1
print(count)
PY
}

mock_upstream_model_catalog_requests() {
  mock_upstream_request_count GET /v1/models
}

mock_upstream_chat_completion_posts() {
  mock_upstream_request_count POST /v1/chat/completions
}

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

export KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE="${WORK_DIR}/registry-store.sqlite"
export KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE="${WORK_DIR}/credential-store.sqlite"
UPSTREAM_MODELS_BEFORE_SERVICE=$(mock_upstream_model_catalog_requests)
if [[ "${UPSTREAM_MODELS_BEFORE_SERVICE}" != "0" ]]; then
  printf 'error: upstream /v1/models was called before service startup\n' >&2
  exit 1
fi

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
UPSTREAM_MODELS_AFTER_AUTHENTICATED_MODELS=$(mock_upstream_model_catalog_requests)
if [[ "${UPSTREAM_MODELS_AFTER_AUTHENTICATED_MODELS}" != "0" ]]; then
  printf 'error: authenticated /v1/models called upstream /v1/models\n' >&2
  exit 1
fi

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
export ONE_AI_KEY_NEW_CLIENT_TOKEN="${NEW_CLIENT_TOKEN}"
MANAGEMENT_URL="http://127.0.0.1:${SERVICE_PORT}"
COMMON=(--management-url "${MANAGEMENT_URL}" --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN)
MANAGEMENT_REPORTS=()
EXPECTED_MANAGEMENT_REPORTS=(
  "doctor.json"
  "client-tokens-list.json"
  "client-tokens-create-dry-run.json"
  "client-tokens-create-apply.json"
  "client-tokens-scope-update-dry-run.json"
  "client-tokens-scope-update-apply.json"
  "client-tokens-disable-dry-run.json"
  "client-tokens-disable-apply.json"
  "client-tokens-enable-dry-run.json"
  "client-tokens-enable-apply.json"
  "models-list.json"
  "models-explain.json"
  "route-explain.json"
  "keys-stats.json"
  "keys-replacement-plan.json"
  "keys-import-dry-run.json"
  "keys-import-apply.json"
  "keys-stats-after-import.json"
  "keys-probe-dry-run.json"
  "keys-probe.json"
  "keys-probe-apply-plan.json"
  "keys-probe-apply-dry-run.json"
  "keys-probe-invalid.json"
  "keys-probe-apply-invalid-plan.json"
  "keys-probe-apply-apply.json"
  "keys-disable-dry-run.json"
  "keys-disable-apply.json"
  "keys-restore-dry-run.json"
  "keys-restore-apply.json"
  "keys-stats-after-restore.json"
  "failures-tail.json"
  "response-filter-events.json"
  "reload-status.json"
  "reload-diff.json"
  "reload-apply-dry-run.json"
  "models-onboard-plan.json"
  "models-onboard-apply-dry-run.json"
  "models-onboard-apply.json"
  "reload-status-staged.json"
  "models-explain-staged.json"
  "reload-diff-staged.json"
  "reload-apply.json"
  "models-explain-published.json"
  "failures-tail-upstream-503.json"
  "failures-explain-upstream-503.json"
  "route-explain-provider-cooling-last-resort.json"
  "keys-disable-final-available.json"
  "failures-tail-local-admission-503.json"
  "failures-explain-local-admission-503.json"
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
    ((.evidence | type) == "object")
    and ((.evidence.candidate_reason_codes | type) == "array")
    and ((.evidence.candidate_reason_codes | length) <= $max_items)
    and ((.evidence.candidate_limit | type) == "number")
    and (.evidence.candidate_limit >= 0)
    and ((.evidence.route_target_count | type) == "number")
    and (.evidence.route_target_count >= 0)
    and ((.evidence.endpoint_family_target_count | type) == "number")
    and (.evidence.endpoint_family_target_count >= 0)
    and ((.evidence.unsupported_target_count | type) == "number")
    and (.evidence.unsupported_target_count >= 0)
    and ((.evidence.unknown_or_missing_target_count | type) == "number")
    and (.evidence.unknown_or_missing_target_count >= 0)
    and ((.evidence.preview_candidate_count | type) == "number")
    and (.evidence.preview_candidate_count >= 0)
  ' "${report_path}" >/dev/null
}

assert_management_report_envelope() {
  if [[ "$#" -gt 0 ]]; then
    :
  else
    printf 'error: no management reports were provided for envelope scanning\n' >&2
    exit 1
  fi
  local report
  for report in "$@"; do
    if [[ -s "${report}" ]]; then
      :
    else
      printf 'error: management report is missing or empty: %s\n' "${report}" >&2
      exit 1
    fi
    case "${report}" in
      *.json) ;;
      *) continue ;;
    esac
    if ! jq -e '
      (type == "object")
      and ((.status | type) == "string")
      and (.status | length > 0)
      and ((.reason | type) == "string")
      and ((.reason_code | type) == "string")
      and (.reason_code | length > 0)
      and ((.side_effect_class | type) == "string")
      and (.side_effect_class | length > 0)
      and ((.effect_vector | type) == "object")
      and ((.effect_vector.reads_local_files | type) == "boolean")
      and ((.effect_vector.reads_management_runtime | type) == "boolean")
      and ((.effect_vector.reads_management_store | type) == "boolean")
      and ((.effect_vector.writes_local_files | type) == "boolean")
      and ((.effect_vector.writes_management_store | type) == "boolean")
      and ((.effect_vector.calls_upstream | type) == "boolean")
      and ((.effect_vector.mutates_runtime | type) == "boolean")
      and ((.next_action | type) == "object")
      and ((.next_action.template_id | type) == "string")
      and (.next_action.template_id | length > 0)
      and ((.next_action.side_effect_class | type) == "string")
      and ((.next_action.requires_confirmation | type) == "boolean")
      and ((.next_action.safe_argv | type) == "array")
    ' "${report}" >/dev/null; then
      printf 'error: management report JSON envelope is invalid: %s\n' "${report}" >&2
      exit 1
    fi
  done
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

LEGACY_MANAGEMENT_REPORT_LABELS=(
  "unavailable_until_m4"
  "deferred_by_m1_m4"
  "keys_probe_unavailable_until_m3"
  "deferred_to_separate_plan"
  "deferred to a separate plan"
  "later reload/scope workflows when available"
  "until M4"
  "before M3"
  "M2.4b"
)

assert_no_legacy_management_report_labels() {
  if [[ "$#" -gt 0 ]]; then
    :
  else
    printf 'error: no management reports were provided for legacy label scanning\n' >&2
    exit 1
  fi
  local report
  local label
  for report in "$@"; do
    if [[ -s "${report}" ]]; then
      :
    else
      printf 'error: management report is missing or empty: %s\n' "${report}" >&2
      exit 1
    fi
    for label in "${LEGACY_MANAGEMENT_REPORT_LABELS[@]}"; do
      if grep -Fq "${label}" "${report}"; then
        printf 'error: management report %s leaked legacy internal phase label\n' "${report}" >&2
        exit 1
      fi
    done
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
    "${REPLACEMENT_TOKEN}"
    "${INVALID_CLIENT_TOKEN}"
    "${NEW_CLIENT_TOKEN}"
    "${WORK_DIR}"
    "data/relay.keys"
    "${MOCK_UPSTREAM_URL}"
    "${MANAGEMENT_URL}"
    "${raw_management_url}"
    "${RAW_BODY_TEXT}"
    "${UPSTREAM_503_MARKER}"
    "${LOCAL_ADMISSION_MARKER}"
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
capture_management_report client-tokens-create-dry-run.json client-tokens create --name release-smoke-client-extra --token-env ONE_AI_KEY_NEW_CLIENT_TOKEN --allowed-model gpt-example --unrestricted-channels --dry-run --output json
jq -e '
  .status == "dry_run"
  and .reason_code == "client_token_create_plan"
  and .side_effect_class == "offline_readonly"
  and .effect_vector.writes_management_store == false
  and .effect_vector.mutates_runtime == false
  and .raw_token_read == false
  and .mutating_create_sent == false
  and .allowed_model_groups == ["gpt-example"]
  and .unrestricted_channels == true
  and .next_action.requires_confirmation == true
' client-tokens-create-dry-run.json >/dev/null
capture_management_report client-tokens-create-apply.json client-tokens create --name release-smoke-client-extra --token-env ONE_AI_KEY_NEW_CLIENT_TOKEN --allowed-model gpt-example --unrestricted-channels --yes --output json
jq -e '
  .status == "ok"
  and .reason_code == "client_token_create_applied"
  and .side_effect_class == "management_write"
  and .effect_vector.writes_management_store == true
  and .effect_vector.mutates_runtime == true
  and .raw_token_read == true
  and .mutating_create_sent == true
  and (.token.id | type == "string")
  and .token.enabled == true
  and .token.scope_summary.allowed_model_group_count == 1
  and .token.scope_summary.unrestricted_channels == true
  and .next_action.requires_confirmation == false
' client-tokens-create-apply.json >/dev/null
NEW_CLIENT_TOKEN_ID=$(jq -r '.token.id' client-tokens-create-apply.json)
if [[ -z "${NEW_CLIENT_TOKEN_ID}" || "${NEW_CLIENT_TOKEN_ID}" == "null" ]]; then
  printf 'error: client token create smoke did not return a token id\n' >&2
  exit 1
fi
NEW_CLIENT_MODELS=$(curl -fsS "http://127.0.0.1:${SERVICE_PORT}/v1/models" \
  -H "Authorization: Bearer ${NEW_CLIENT_TOKEN}")
printf '%s\n' "${NEW_CLIENT_MODELS}" \
  | jq -e '.data | map(.id) == ["gpt-example"]' >/dev/null
capture_management_report client-tokens-scope-update-dry-run.json client-tokens scope-update "${NEW_CLIENT_TOKEN_ID}" --unrestricted-models --unrestricted-channels --dry-run --output json
jq -e --arg token_id "${NEW_CLIENT_TOKEN_ID}" '
  .status == "dry_run"
  and .reason_code == "client_token_scope_update_plan"
  and .side_effect_class == "offline_readonly"
  and .effect_vector.writes_management_store == false
  and .effect_vector.mutates_runtime == false
  and .token_id == $token_id
  and .allowed_model_groups == []
  and .allowed_channels == []
  and .unrestricted_model_groups == true
  and .unrestricted_channels == true
  and .mutating_scope_update_sent == false
  and .next_action.requires_confirmation == true
' client-tokens-scope-update-dry-run.json >/dev/null
capture_management_report client-tokens-scope-update-apply.json client-tokens scope-update "${NEW_CLIENT_TOKEN_ID}" --unrestricted-models --unrestricted-channels --yes --output json
jq -e --arg token_id "${NEW_CLIENT_TOKEN_ID}" '
  .status == "ok"
  and .reason_code == "client_token_scope_update_applied"
  and .side_effect_class == "management_write"
  and .effect_vector.writes_management_store == true
  and .effect_vector.mutates_runtime == true
  and .token_id == $token_id
  and .mutating_scope_update_sent == true
  and .token.enabled == true
  and .token.scope_summary.unrestricted_model_groups == true
  and .token.scope_summary.unrestricted_channels == true
  and .next_action.requires_confirmation == false
' client-tokens-scope-update-apply.json >/dev/null
NEW_CLIENT_MODELS_AFTER_SCOPE_UPDATE=$(curl -fsS "http://127.0.0.1:${SERVICE_PORT}/v1/models" \
  -H "Authorization: Bearer ${NEW_CLIENT_TOKEN}")
printf '%s\n' "${NEW_CLIENT_MODELS_AFTER_SCOPE_UPDATE}" \
  | jq -e '.data | map(.id) | index("gpt-example") != null' >/dev/null
capture_management_report client-tokens-disable-dry-run.json client-tokens disable "${NEW_CLIENT_TOKEN_ID}" --dry-run --output json
jq -e --arg token_id "${NEW_CLIENT_TOKEN_ID}" '
  .status == "dry_run"
  and .reason_code == "client_token_disable_plan"
  and .side_effect_class == "offline_readonly"
  and .effect_vector.writes_management_store == false
  and .effect_vector.mutates_runtime == false
  and .token_id == $token_id
  and .mutating_disable_sent == false
  and .next_action.requires_confirmation == true
' client-tokens-disable-dry-run.json >/dev/null
capture_management_report client-tokens-disable-apply.json client-tokens disable "${NEW_CLIENT_TOKEN_ID}" --yes --output json
jq -e --arg token_id "${NEW_CLIENT_TOKEN_ID}" '
  .status == "ok"
  and .reason_code == "client_token_disable_applied"
  and .side_effect_class == "management_write"
  and .effect_vector.writes_management_store == true
  and .effect_vector.mutates_runtime == true
  and .token_id == $token_id
  and .mutating_disable_sent == true
  and .token.enabled == false
  and .next_action.requires_confirmation == false
' client-tokens-disable-apply.json >/dev/null
NEW_CLIENT_DISABLED_STATUS=$(curl -sS -o new-client-disabled-models.json -w "%{http_code}" \
  "http://127.0.0.1:${SERVICE_PORT}/v1/models" \
  -H "Authorization: Bearer ${NEW_CLIENT_TOKEN}")
if [[ "${NEW_CLIENT_DISABLED_STATUS}" != "401" ]]; then
  printf 'error: new client token unexpectedly remained valid after disable\n' >&2
  exit 1
fi
jq -e '.error.message == "invalid router api key"' new-client-disabled-models.json >/dev/null
capture_management_report client-tokens-enable-dry-run.json client-tokens enable "${NEW_CLIENT_TOKEN_ID}" --dry-run --output json
jq -e --arg token_id "${NEW_CLIENT_TOKEN_ID}" '
  .status == "dry_run"
  and .reason_code == "client_token_enable_plan"
  and .side_effect_class == "offline_readonly"
  and .effect_vector.writes_management_store == false
  and .effect_vector.mutates_runtime == false
  and .token_id == $token_id
  and .mutating_enable_sent == false
  and .next_action.requires_confirmation == true
' client-tokens-enable-dry-run.json >/dev/null
capture_management_report client-tokens-enable-apply.json client-tokens enable "${NEW_CLIENT_TOKEN_ID}" --yes --output json
jq -e --arg token_id "${NEW_CLIENT_TOKEN_ID}" '
  .status == "ok"
  and .reason_code == "client_token_enable_applied"
  and .side_effect_class == "management_write"
  and .effect_vector.writes_management_store == true
  and .effect_vector.mutates_runtime == true
  and .token_id == $token_id
  and .mutating_enable_sent == true
  and .token.enabled == true
  and .next_action.requires_confirmation == false
' client-tokens-enable-apply.json >/dev/null
NEW_CLIENT_MODELS_AFTER_ENABLE=$(curl -fsS "http://127.0.0.1:${SERVICE_PORT}/v1/models" \
  -H "Authorization: Bearer ${NEW_CLIENT_TOKEN}") || {
  printf 'error: new client token did not work after enable\n' >&2
  exit 1
}
printf '%s\n' "${NEW_CLIENT_MODELS_AFTER_ENABLE}" \
  | jq -e '.data | map(.id) | index("gpt-example") != null' >/dev/null
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
capture_management_report route-explain.json route explain gpt-example --client-token-ref local-client --endpoint-family chat_completions --output json
jq -e --arg model "${MODEL_EXPLAIN_MODEL}" --arg client_token_ref "${MODEL_EXPLAIN_CLIENT_TOKEN_REF}" --arg reason_code "${MODEL_EXPLAIN_REASON_CODE}" '
  .status == "available" and .reason_code == "available"
  and .reason_code == $reason_code
  and .endpoint_family_status == "evaluated"
  and .endpoint_family == "chat_completions"
  and .can_use == true
  and .blocking_domain == "none"
  and .availability.status == "available"
  and .availability.reason_code == "available"
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
capture_management_report keys-replacement-plan.json keys replacement-plan --credential-set relay_credentials --model gpt-example --client-token-ref local-client --output json
jq -e '
  .status == "ok"
  and .reason_code == "keys_replacement_plan_projected"
  and .side_effect_class == "runtime_readonly"
  and .effect_vector.reads_management_runtime == true
  and .effect_vector.reads_management_store == true
  and .data.scan_mode == "management_projection_only"
  and .data.credential_set_id == "relay_credentials"
  and (.data.capacity_summary.available | type == "number")
  and (.data.replacement_need.status | type == "string")
  and .data.route_impact.status == "available"
  and .data.route_impact.model == "gpt-example"
  and (.data.route_impact.requested_credential_set.candidate_presence as $presence | ["selected_candidate", "candidate_not_selected", "not_candidate", "unknown"] | index($presence) != null)
  and (.data.safe_next_actions | type == "array" and length > 0)
  and (.next_action.safe_argv | type == "array")
' keys-replacement-plan.json >/dev/null
printf '%s\n' "${REPLACEMENT_TOKEN}" > replacement.keys
capture_management_report keys-import-dry-run.json keys import --credential-set relay_credentials --source replacement.keys --dry-run --output json
jq -e '
  .status == "dry_run"
  and .reason_code == "keys_import_local_preview"
  and .side_effect_class == "local_preview"
  and .effect_vector.reads_local_files == true
  and .effect_vector.writes_management_store == false
  and .effect_vector.mutates_runtime == false
  and .data.command == "keys import"
  and .data.non_empty_line_count == 1
  and .data.source_secrets_sent_to_management == false
  and (.next_action.safe_argv | type == "array")
  and .next_action.requires_confirmation == true
' keys-import-dry-run.json >/dev/null
capture_management_report keys-import-apply.json keys import --credential-set relay_credentials --source replacement.keys --yes --output json
jq -e '
  .status == "ok"
  and .reason_code == "keys_import_applied"
  and .side_effect_class == "management_write"
  and .effect_vector.reads_local_files == true
  and .effect_vector.writes_management_store == true
  and .effect_vector.mutates_runtime == true
  and .data.command == "keys import"
  and .data.source_secrets_sent_to_management == true
  and .data.management_response.imported_credentials == 1
  and .data.management_response.credential_statuses_returned >= 1
  and (.next_action.safe_argv | type == "array")
' keys-import-apply.json >/dev/null
capture_management_report keys-stats-after-import.json keys stats --credential-set relay_credentials --include-credential-refs --output json
jq -e '
  .status
  and .reason_code
  and .side_effect_class == "runtime_readonly"
  and (.credential_sets[0].credential_refs | index("cr:v1:pos:0") != null)
  and (.credential_sets[0].credential_refs | index("cr:v1:pos:1") != null)
' keys-stats-after-import.json >/dev/null
capture_management_report keys-probe-dry-run.json keys probe --credential-set relay_credentials --credential-ref cr:v1:pos:0 --model provider/gpt-example --dry-run --output json
jq -e '
  .status == "dry_run"
  and .reason_code == "keys_probe_plan"
  and .side_effect_class == "runtime_readonly"
  and .effect_vector.calls_upstream == false
  and .data.command == "keys probe"
  and .data.credential_ref == "cr:v1:pos:0"
  and .data.model == "provider/gpt-example"
  and .data.upstream_request_sent == false
  and (.next_action.safe_argv | type == "array")
' keys-probe-dry-run.json >/dev/null
capture_management_report keys-probe.json keys probe --credential-set relay_credentials --credential-ref cr:v1:pos:0 --model provider/gpt-example --yes --output json
jq -e '
  .status == "ok"
  and .reason_code == "keys_probe_recorded"
  and .side_effect_class == "upstream_touching"
  and .effect_vector.calls_upstream == true
  and .effect_vector.writes_management_store == true
  and .data.command == "keys probe"
  and .data.credential_ref == "cr:v1:pos:0"
  and .data.model == "provider/gpt-example"
  and .data.upstream_request_sent == true
  and .data.probe_evidence_persisted == true
  and .data.probe.outcome == "success"
  and .data.probe.upstream_status == 200
' keys-probe.json >/dev/null
capture_management_report keys-probe-apply-plan.json keys probe-apply plan --credential-set relay_credentials --credential-ref cr:v1:pos:0 --output json
jq -e '
  .status == "dry_run"
  and .reason_code == "keys_probe_apply_plan_projected"
  and .side_effect_class == "runtime_readonly"
  and .data.command == "keys probe-apply plan"
  and .data.credential_ref == "cr:v1:pos:0"
  and (.data.probe_result_ref | type == "string")
  and .data.probe.outcome == "success"
  and .data.mutating_apply_sent == false
' keys-probe-apply-plan.json >/dev/null
PROBE_RESULT_REF=$(jq -r '.data.probe_result_ref' keys-probe-apply-plan.json)
capture_management_report keys-probe-apply-dry-run.json keys probe-apply apply --credential-set relay_credentials --credential-ref cr:v1:pos:0 --probe-result-ref "${PROBE_RESULT_REF}" --dry-run --output json
jq -e --arg probe_result_ref "${PROBE_RESULT_REF}" '
  .status == "dry_run"
  and .reason_code == "keys_probe_apply_plan_projected"
  and .side_effect_class == "runtime_readonly"
  and .data.command == "keys probe-apply apply"
  and .data.credential_ref == "cr:v1:pos:0"
  and .data.probe_result_ref == $probe_result_ref
  and .data.probe_result_ref_matches == true
  and .data.mutating_apply_sent == false
  and .data.lifecycle_mutation_applied == false
' keys-probe-apply-dry-run.json >/dev/null
capture_management_report keys-probe-invalid.json keys probe --credential-set relay_credentials --credential-ref cr:v1:pos:0 --model provider/invalid-probe --yes --output json
jq -e '
  .status == "ok"
  and .reason_code == "keys_probe_recorded"
  and .side_effect_class == "upstream_touching"
  and .effect_vector.calls_upstream == true
  and .effect_vector.writes_management_store == true
  and .data.command == "keys probe"
  and .data.credential_ref == "cr:v1:pos:0"
  and .data.model == "provider/invalid-probe"
  and .data.probe.outcome == "invalid"
  and .data.probe.upstream_status == 401
' keys-probe-invalid.json >/dev/null
capture_management_report keys-probe-apply-invalid-plan.json keys probe-apply plan --credential-set relay_credentials --credential-ref cr:v1:pos:0 --output json
jq -e '
  .status == "dry_run"
  and .reason_code == "keys_probe_apply_plan_projected"
  and .side_effect_class == "runtime_readonly"
  and .data.command == "keys probe-apply plan"
  and .data.credential_ref == "cr:v1:pos:0"
  and (.data.probe_result_ref | type == "string")
  and .data.planned_action == "expire"
  and .data.probe.outcome == "invalid"
  and .data.mutating_apply_sent == false
' keys-probe-apply-invalid-plan.json >/dev/null
INVALID_PROBE_RESULT_REF=$(jq -r '.data.probe_result_ref' keys-probe-apply-invalid-plan.json)
capture_management_report keys-probe-apply-apply.json keys probe-apply apply --credential-set relay_credentials --credential-ref cr:v1:pos:0 --probe-result-ref "${INVALID_PROBE_RESULT_REF}" --yes --output json
jq -e --arg probe_result_ref "${INVALID_PROBE_RESULT_REF}" '
  .status == "ok"
  and .reason_code == "keys_probe_apply_applied"
  and .side_effect_class == "management_write"
  and .effect_vector.writes_management_store == true
  and .effect_vector.mutates_runtime == true
  and .data.command == "keys probe-apply apply"
  and .data.credential_ref == "cr:v1:pos:0"
  and .data.probe_result_ref == $probe_result_ref
  and .data.applied_action == "expire"
  and .data.probe.outcome == "invalid"
  and .data.mutating_apply_sent == true
  and .data.lifecycle_mutation_applied == true
  and .data.mutation.state_kind == "expired"
' keys-probe-apply-apply.json >/dev/null
capture_management_report keys-disable-dry-run.json keys disable --credential-set relay_credentials --credential-ref cr:v1:pos:1 --reason "operator verified unavailable credential" --dry-run --output json
jq -e '
  .status == "dry_run"
  and .reason_code == "keys_disable_plan"
  and .side_effect_class == "offline_readonly"
  and .data.command == "keys disable"
  and .data.credential_ref == "cr:v1:pos:1"
  and .data.mutating_disable_sent == false
  and .next_action.requires_confirmation == true
' keys-disable-dry-run.json >/dev/null
capture_management_report keys-disable-apply.json keys disable --credential-set relay_credentials --credential-ref cr:v1:pos:1 --reason "operator verified unavailable credential" --yes --output json
jq -e '
  .status == "ok"
  and .reason_code == "keys_disable_applied"
  and .side_effect_class == "management_write"
  and .effect_vector.writes_management_store == true
  and .effect_vector.mutates_runtime == true
  and .data.command == "keys disable"
  and .data.credential_ref == "cr:v1:pos:1"
  and .data.state_kind == "disabled"
  and .data.mutating_disable_sent == true
  and .next_action.requires_confirmation == false
' keys-disable-apply.json >/dev/null
capture_management_report keys-restore-dry-run.json keys restore --credential-set relay_credentials --credential-ref cr:v1:pos:0 --reason "operator verified recovered credential" --dry-run --output json
jq -e '
  .status == "dry_run"
  and .reason_code == "keys_restore_plan"
  and .side_effect_class == "offline_readonly"
  and .effect_vector.writes_management_store == false
  and .effect_vector.mutates_runtime == false
  and .effect_vector.calls_upstream == false
  and .data.command == "keys restore"
  and .data.credential_ref == "cr:v1:pos:0"
  and .data.mutating_restore_sent == false
  and (.next_action.safe_argv | type == "array")
  and .next_action.requires_confirmation == true
  and .next_action.side_effect_class == "management_write"
' keys-restore-dry-run.json >/dev/null
capture_management_report keys-restore-apply.json keys restore --credential-set relay_credentials --credential-ref cr:v1:pos:0 --reason "operator verified recovered credential" --yes --output json
jq -e '
  .status == "ok"
  and .reason_code == "keys_restore_applied"
  and .side_effect_class == "management_write"
  and .effect_vector.writes_management_store == true
  and .effect_vector.mutates_runtime == true
  and .data.command == "keys restore"
  and .data.credential_ref == "cr:v1:pos:0"
  and .data.state_kind == "available"
  and .data.mutating_restore_sent == true
  and .next_action.requires_confirmation == false
' keys-restore-apply.json >/dev/null
capture_management_report keys-stats-after-restore.json keys stats --credential-set relay_credentials --include-credential-refs --output json
jq -e '
  .status
  and .reason_code
  and .side_effect_class == "runtime_readonly"
  and (.credential_sets[0].credentials.available >= 1)
  and (.credential_sets[0].credentials.disabled >= 1)
  and (.credential_sets[0].credential_refs | index("cr:v1:pos:0") != null)
  and (.credential_sets[0].credential_refs | index("cr:v1:pos:1") != null)
' keys-stats-after-restore.json >/dev/null
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
capture_management_report response-filter-events.json response-filters events --last 20 --output json
jq -e '
  .status == "ok"
  and .reason_code == "no_response_filter_events_found"
  and .side_effect_class == "runtime_readonly"
  and .effect_vector.calls_upstream == false
  and .effect_vector.writes_management_store == false
  and .window.kind == "bounded_recent_response_filter_events"
  and (.window.limit | type == "number")
  and (.window.returned | type == "number")
  and (.window.truncated | type == "boolean")
  and .data.event_count == 0
  and (.data.events | length == 0)
' response-filter-events.json >/dev/null
capture_management_report reload-status.json reload status --output json
jq -e '.status and .reason_code and .side_effect_class and (.next_action.safe_argv | type == "array")' reload-status.json >/dev/null
capture_management_report reload-diff.json reload diff --output json
jq -e '.status and .reason_code and .side_effect_class and (.next_action.safe_argv | type == "array")' reload-diff.json >/dev/null
capture_management_report reload-apply-dry-run.json reload apply --dry-run --output json
jq -e '.status == "planned" and .reason_code == "reload_apply_dry_run" and .side_effect_class and (.next_action.safe_argv | type == "array")' reload-apply-dry-run.json >/dev/null

PUBLISHED_PUBLIC_MODEL="release-smoke-public-model"
PUBLISHED_UPSTREAM_MODEL="release-smoke-upstream-model"
PRE_ONBOARD_STAGED_REGISTRY_VERSION=$(jq -r '.staged_registry_version // .data.staged_registry_version // empty' reload-status.json)
if [[ -z "${PRE_ONBOARD_STAGED_REGISTRY_VERSION}" || "${PRE_ONBOARD_STAGED_REGISTRY_VERSION}" == "null" ]]; then
  printf 'error: reload status did not expose a staged registry version before model onboarding\n' >&2
  exit 1
fi
capture_management_report models-onboard-plan.json models onboard-plan --channel relay --public-model "${PUBLISHED_PUBLIC_MODEL}" --upstream-model "${PUBLISHED_UPSTREAM_MODEL}" --client-token-ref local-client --endpoint-family chat_completions --dry-run --output json
jq -e --arg published_public_model "${PUBLISHED_PUBLIC_MODEL}" --arg published_upstream_model "${PUBLISHED_UPSTREAM_MODEL}" '
  .status == "dry_run"
  and .reason_code == "models_onboard_plan_projected"
  and .public_model == $published_public_model
  and .upstream_model == $published_upstream_model
  and .planning_only_no_visibility_change == true
  and .client_visibility_changed == false
  and .live_discovery_called == false
  and .management_mutation_sent == false
  and .runtime_reload_required == false
  and .proposed_route.visibility_after_this_command == false
  and .side_effect_class == "runtime_readonly"
  and (.next_action.safe_argv | type == "array")
' models-onboard-plan.json >/dev/null
UPSTREAM_MODELS_AFTER_PLAN=$(mock_upstream_model_catalog_requests)
if [[ "${UPSTREAM_MODELS_AFTER_PLAN}" != "${UPSTREAM_MODELS_AFTER_AUTHENTICATED_MODELS}" ]]; then
  printf 'error: models onboard-plan dry-run called upstream /v1/models\n' >&2
  exit 1
fi

capture_management_report models-onboard-apply-dry-run.json models onboard-plan --channel relay --public-model "${PUBLISHED_PUBLIC_MODEL}" --upstream-model "${PUBLISHED_UPSTREAM_MODEL}" --apply --dry-run --output json
jq -e --arg published_public_model "${PUBLISHED_PUBLIC_MODEL}" --arg published_upstream_model "${PUBLISHED_UPSTREAM_MODEL}" '
  .status == "dry_run"
  and .reason_code == "models_onboard_apply_projected"
  and .public_model == $published_public_model
  and .upstream_model == $published_upstream_model
  and .planning_only_no_visibility_change == true
  and .client_visibility_changed == false
  and .live_discovery_called == false
  and .management_mutation_sent == false
  and .mutating_reload_sent == false
  and .staged_registry_version == null
  and .runtime_reload_required == null
  and .side_effect_class == "runtime_readonly"
' models-onboard-apply-dry-run.json >/dev/null
UPSTREAM_MODELS_AFTER_APPLY_DRY_RUN=$(mock_upstream_model_catalog_requests)
if [[ "${UPSTREAM_MODELS_AFTER_APPLY_DRY_RUN}" != "${UPSTREAM_MODELS_AFTER_AUTHENTICATED_MODELS}" ]]; then
  printf 'error: models onboard-plan apply dry-run called upstream /v1/models\n' >&2
  exit 1
fi

capture_management_report models-onboard-apply.json models onboard-plan --channel relay --public-model "${PUBLISHED_PUBLIC_MODEL}" --upstream-model "${PUBLISHED_UPSTREAM_MODEL}" --apply --expected-staged-registry-version "${PRE_ONBOARD_STAGED_REGISTRY_VERSION}" --yes --output json
jq -e --arg published_public_model "${PUBLISHED_PUBLIC_MODEL}" --arg published_upstream_model "${PUBLISHED_UPSTREAM_MODEL}" --argjson expected_version "${PRE_ONBOARD_STAGED_REGISTRY_VERSION}" '
  .status == "ok"
  and .reason_code == "models_onboard_apply_sent"
  and .public_model == $published_public_model
  and .upstream_model == $published_upstream_model
  and .expected_staged_registry_version == $expected_version
  and (.staged_registry_version | type == "number")
  and .staged_registry_version > $expected_version
  and .registry_version == .staged_registry_version
  and .runtime_reload_required == true
  and .applied_to_runtime == false
  and .client_visibility_changed == false
  and .management_mutation_sent == true
  and .mutating_reload_sent == false
  and .side_effect_class == "management_write"
' models-onboard-apply.json >/dev/null
STAGED_REGISTRY_VERSION=$(jq -r '.staged_registry_version' models-onboard-apply.json)

MODELS_BEFORE_RELOAD=$(curl -fsS "http://127.0.0.1:${SERVICE_PORT}/v1/models" \
  -H "Authorization: Bearer ${CLIENT_TOKEN}")
printf '%s\n' "${MODELS_BEFORE_RELOAD}" \
  | jq -e --arg published_public_model "${PUBLISHED_PUBLIC_MODEL}" '.data | map(.id) | index($published_public_model) == null' >/dev/null

capture_management_report reload-status-staged.json reload status --output json
jq -e --argjson staged_version "${STAGED_REGISTRY_VERSION}" '
  .status == "pending_reload"
  and .reason_code == "runtime_reload_required"
  and .staged_registry_version == $staged_version
  and .runtime_reload_required == true
  and .staged_vs_runtime.active_matches_staged == false
  and .staged_vs_runtime.reload_required_reason == "staged_registry_differs"
  and .mutating_reload_sent == false
  and .side_effect_class == "runtime_readonly"
' reload-status-staged.json >/dev/null

capture_management_report models-explain-staged.json models explain --model "${PUBLISHED_PUBLIC_MODEL}" --client-token-ref local-client --endpoint-family chat_completions --output json
jq -e --arg published_public_model "${PUBLISHED_PUBLIC_MODEL}" --argjson staged_version "${STAGED_REGISTRY_VERSION}" '
  .model == $published_public_model
  and .client_token_ref == "local-client"
  and .endpoint_family == "chat_completions"
  and .staged_registry_version == $staged_version
  and .runtime_reload_required == true
  and .side_effect_class == "runtime_readonly"
' models-explain-staged.json >/dev/null

capture_management_report reload-diff-staged.json reload diff --output json
jq -e --argjson staged_version "${STAGED_REGISTRY_VERSION}" '
  .status == "ok"
  and .reason_code == "reload_diff_available"
  and .staged_registry_version == $staged_version
  and .runtime_reload_required == true
  and .mutating_reload_sent == false
  and (.resource_changes | type == "array")
  and (.budget.truncated == false)
' reload-diff-staged.json >/dev/null

capture_management_report reload-apply.json reload apply --yes --expected-staged-registry-version "${STAGED_REGISTRY_VERSION}" --output json
jq -e --argjson staged_version "${STAGED_REGISTRY_VERSION}" '
  .status == "ok"
  and .reason_code == "runtime_reload_applied"
  and .expected_staged_registry_version == $staged_version
  and .mutating_reload_sent == true
  and .precondition_supported == true
  and .management_response.staged_registry_version == $staged_version
  and .management_response.runtime_reload_required == false
  and .side_effect_class == "management_write"
' reload-apply.json >/dev/null

capture_management_report models-explain-published.json models explain --model "${PUBLISHED_PUBLIC_MODEL}" --client-token-ref local-client --endpoint-family chat_completions --output json
jq -e --arg published_public_model "${PUBLISHED_PUBLIC_MODEL}" --argjson staged_version "${STAGED_REGISTRY_VERSION}" '
  .status == "ok"
  and .can_use == true
  and .reason_code == "available"
  and .blocking_domain == "none"
  and .model == $published_public_model
  and .client_token_ref == "local-client"
  and .endpoint_family == "chat_completions"
  and .staged_registry_version == $staged_version
  and .runtime_reload_required == false
  and .side_effect_class == "runtime_readonly"
' models-explain-published.json >/dev/null

MODELS_AFTER_RELOAD=$(curl -fsS "http://127.0.0.1:${SERVICE_PORT}/v1/models" \
  -H "Authorization: Bearer ${CLIENT_TOKEN}")
printf '%s\n' "${MODELS_AFTER_RELOAD}" \
  | jq -e --arg published_public_model "${PUBLISHED_PUBLIC_MODEL}" '.data | map(.id) | index($published_public_model) != null' >/dev/null

curl -fsS "http://127.0.0.1:${SERVICE_PORT}/v1/chat/completions" \
  -H "Authorization: Bearer ${CLIENT_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "{\"model\":\"${PUBLISHED_PUBLIC_MODEL}\",\"messages\":[{\"role\":\"user\",\"content\":\"ping\"}]}" \
  | jq -e '.choices[0].message.content == "release smoke published model ok"' >/dev/null

POSTS_BEFORE_UPSTREAM_503=$(mock_upstream_chat_completion_posts)
UPSTREAM_503_STATUS=$(curl -sS -o upstream-503.json -w "%{http_code}" \
  "http://127.0.0.1:${SERVICE_PORT}/v1/chat/completions" \
  -H "Authorization: Bearer ${CLIENT_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "$(jq -n --arg marker "${UPSTREAM_503_MARKER}" '{model:"gpt-example",messages:[{role:"user",content:$marker}]}')")
if [[ "${UPSTREAM_503_STATUS}" != "503" ]]; then
  printf 'error: upstream 503 smoke returned HTTP %s instead of 503\n' "${UPSTREAM_503_STATUS}" >&2
  exit 1
fi
POSTS_AFTER_UPSTREAM_503=$(mock_upstream_chat_completion_posts)
if (( POSTS_AFTER_UPSTREAM_503 != POSTS_BEFORE_UPSTREAM_503 + 1 )); then
  printf 'error: upstream 503 smoke did not reach mock upstream exactly once\n' >&2
  exit 1
fi
jq -e '.error.code == "provider_unavailable"' upstream-503.json >/dev/null
capture_management_report failures-tail-upstream-503.json failures tail --last 20 --endpoint-family chat_completions --output json
jq -e '
  .status == "degraded"
  and .availability_source == "bounded_evidence"
  and .current_availability == false
  and (.data.failures | any(
    .reason_code == "upstream_5xx"
    and .endpoint_family == "chat_completions"
    and .client_visible_status == "upstream_5xx"
    and .upstream_status == 503
    and .admission == null
  ))
' failures-tail-upstream-503.json >/dev/null
UPSTREAM_503_REQUEST_ID=$(jq -r '
  .data.failures
  | reverse
  | map(select(
    .reason_code == "upstream_5xx"
    and .endpoint_family == "chat_completions"
    and .client_visible_status == "upstream_5xx"
    and .upstream_status == 503
  ))
  | .[0].request_id // empty
' failures-tail-upstream-503.json)
if [[ -z "${UPSTREAM_503_REQUEST_ID}" ]]; then
  printf 'error: upstream 503 failure evidence did not include a request id\n' >&2
  exit 1
fi
capture_management_report failures-explain-upstream-503.json failures explain "${UPSTREAM_503_REQUEST_ID}" --endpoint-family chat_completions --output json
jq -e --arg request_id "${UPSTREAM_503_REQUEST_ID}" '
  .status == "degraded"
  and .reason_code == "upstream_5xx"
  and .scope.request_id == $request_id
  and .scope.endpoint_family == "chat_completions"
  and .data.explanation.endpoint_family == "chat_completions"
  and .data.explanation.client_visible_status == "upstream_5xx"
  and .data.explanation.upstream_status == 503
  and .data.explanation.admission == null
  and (.data.evidence | any(
    .request_id == $request_id
    and .reason_code == "upstream_5xx"
    and .endpoint_family == "chat_completions"
    and .client_visible_status == "upstream_5xx"
    and .upstream_status == 503
  ))
' failures-explain-upstream-503.json >/dev/null

capture_management_report route-explain-provider-cooling-last-resort.json route explain gpt-example --client-token-ref local-client --output json
jq -e '
  .status == "last_resort"
  and .reason_code == "provider_cooling_down_last_resort"
  and .admission_summary.status == "last_resort"
  and .admission_summary.reason_code == "provider_cooling_down_last_resort"
  and .admission_summary.last_resort_used == true
  and .admission_summary.last_resort_reason == "provider_cooling_down_last_resort"
  and .admission_summary.included_count == 1
  and .selected_target.channel_id == "relay"
' route-explain-provider-cooling-last-resort.json >/dev/null
POSTS_BEFORE_SOFT_LAST_RESORT=$(mock_upstream_chat_completion_posts)
SOFT_LAST_RESORT_STATUS=$(curl -sS -o provider-cooling-last-resort.json -w "%{http_code}" \
  "http://127.0.0.1:${SERVICE_PORT}/v1/chat/completions" \
  -H "Authorization: Bearer ${CLIENT_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-example","messages":[{"role":"user","content":"ping"}]}')
if [[ "${SOFT_LAST_RESORT_STATUS}" != "200" ]]; then
  printf 'error: provider-cooling last-resort smoke returned HTTP %s instead of 200\n' "${SOFT_LAST_RESORT_STATUS}" >&2
  exit 1
fi
POSTS_AFTER_SOFT_LAST_RESORT=$(mock_upstream_chat_completion_posts)
if (( POSTS_AFTER_SOFT_LAST_RESORT != POSTS_BEFORE_SOFT_LAST_RESORT + 1 )); then
  printf 'error: provider-cooling last-resort smoke did not reach mock upstream exactly once\n' >&2
  exit 1
fi
jq -e '.choices[0].message.content == "release smoke ok"' provider-cooling-last-resort.json >/dev/null

capture_management_report keys-disable-final-available.json keys disable --credential-set relay_credentials --credential-ref cr:v1:pos:0 --reason "release smoke final unavailable credential" --yes --output json
jq -e '
  .status == "ok"
  and .reason_code == "keys_disable_applied"
  and .side_effect_class == "management_write"
  and .data.command == "keys disable"
  and .data.credential_ref == "cr:v1:pos:0"
  and .data.state_kind == "disabled"
  and .data.mutating_disable_sent == true
' keys-disable-final-available.json >/dev/null

POSTS_BEFORE_LOCAL_ADMISSION=$(mock_upstream_chat_completion_posts)
LOCAL_ADMISSION_STATUS=$(curl -sS -o local-admission-503.json -w "%{http_code}" \
  "http://127.0.0.1:${SERVICE_PORT}/v1/chat/completions" \
  -H "Authorization: Bearer ${CLIENT_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "$(jq -n --arg marker "${LOCAL_ADMISSION_MARKER}" '{model:"gpt-example",messages:[{role:"user",content:$marker}]}')")
if [[ "${LOCAL_ADMISSION_STATUS}" != "503" ]]; then
  printf 'error: local admission smoke returned HTTP %s instead of 503\n' "${LOCAL_ADMISSION_STATUS}" >&2
  exit 1
fi
POSTS_AFTER_LOCAL_ADMISSION=$(mock_upstream_chat_completion_posts)
if [[ "${POSTS_AFTER_LOCAL_ADMISSION}" != "${POSTS_BEFORE_LOCAL_ADMISSION}" ]]; then
  printf 'error: local admission smoke unexpectedly reached mock upstream chat completions\n' >&2
  exit 1
fi
jq -e '.error.code == "no_route_candidate"' local-admission-503.json >/dev/null
capture_management_report failures-tail-local-admission-503.json failures tail --last 20 --endpoint-family chat_completions --output json
jq -e '
  .status == "degraded"
  and .availability_source == "bounded_evidence"
  and .current_availability == false
  and (.data.failures | any(
    .event_kind == "route_admission_denied"
    and .reason_code == "no_route_candidate"
    and .endpoint_family == "chat_completions"
    and .client_visible_status == "local_503"
    and .upstream_status == null
    and .admission.included_count == 0
  ))
' failures-tail-local-admission-503.json >/dev/null
LOCAL_ADMISSION_REQUEST_ID=$(jq -r '
  .data.failures
  | reverse
  | map(select(
    .event_kind == "route_admission_denied"
    and .reason_code == "no_route_candidate"
    and .endpoint_family == "chat_completions"
    and .client_visible_status == "local_503"
    and .upstream_status == null
  ))
  | .[0].request_id // empty
' failures-tail-local-admission-503.json)
if [[ -z "${LOCAL_ADMISSION_REQUEST_ID}" ]]; then
  printf 'error: local admission failure evidence did not include a request id\n' >&2
  exit 1
fi
capture_management_report failures-explain-local-admission-503.json failures explain "${LOCAL_ADMISSION_REQUEST_ID}" --endpoint-family chat_completions --output json
jq -e --arg request_id "${LOCAL_ADMISSION_REQUEST_ID}" '
  .status == "degraded"
  and .reason_code == "no_route_candidate"
  and .scope.request_id == $request_id
  and .scope.endpoint_family == "chat_completions"
  and .data.explanation.endpoint_family == "chat_completions"
  and .data.explanation.client_visible_status == "local_503"
  and .data.explanation.upstream_status == null
  and .data.explanation.admission.included_count == 0
  and (.data.evidence | any(
    .request_id == $request_id
    and .event_kind == "route_admission_denied"
    and .reason_code == "no_route_candidate"
    and .endpoint_family == "chat_completions"
    and .client_visible_status == "local_503"
    and .upstream_status == null
    and .admission.included_count == 0
  ))
' failures-explain-local-admission-503.json >/dev/null

UPSTREAM_MODELS_AFTER_WORKFLOW=$(mock_upstream_model_catalog_requests)
if [[ "${UPSTREAM_MODELS_AFTER_WORKFLOW}" != "${UPSTREAM_MODELS_AFTER_AUTHENTICATED_MODELS}" ]]; then
  printf 'error: operator workflows called upstream /v1/models\n' >&2
  exit 1
fi

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
assert_management_report_envelope "${MANAGEMENT_REPORTS[@]}"
assert_no_management_report_leaks "${MANAGEMENT_REPORTS[@]}"
assert_no_legacy_management_report_labels "${MANAGEMENT_REPORTS[@]}"

printf 'release smoke passed for %s %s\n' "${PACKAGE_NAME}" "${VERSION}"

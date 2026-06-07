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

python3 - <<'PY' "${SERVICE_PORT}" "${MOCK_PORT}" "${CLIENT_TOKEN}" "${MANAGEMENT_TOKEN}" "config/local.yaml"
import pathlib
import sys

service_port, mock_port, client_token, management_token, config_path = sys.argv[1:]
path = pathlib.Path(config_path)
text = path.read_text()
text = text.replace("listen: 127.0.0.1:4101", f"listen: 127.0.0.1:{service_port}")
text = text.replace("<client-token-placeholder>", client_token)
text = text.replace("<management-token-placeholder>", management_token)
text = text.replace("https://relay.example/v1", f"http://127.0.0.1:{mock_port}/v1")
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

CHECK_MODELS=$(jq -c '.model_visibility_preview[] | select(.client_token_ref == "local-client") | .visible_models' check-config.json)
CLIENT_MODELS=$(printf '%s\n' "${MODELS_JSON}" | jq -c '.data | map(.id)')
if [[ "${CHECK_MODELS}" != "${CLIENT_MODELS}" ]]; then
  printf 'error: check-config visibility does not match authenticated /v1/models\n' >&2
  exit 1
fi

export ONE_AI_KEY_MANAGEMENT_TOKEN="${MANAGEMENT_TOKEN}"
MANAGEMENT_URL="http://127.0.0.1:${SERVICE_PORT}"
COMMON=(--management-url "${MANAGEMENT_URL}" --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN)

"${BIN}" "${COMMON[@]}" doctor --output json \
  | jq -e '.status and .reason_code and .next_action' >/dev/null
"${BIN}" "${COMMON[@]}" client-tokens list --output json \
  | jq -e '.status == "ok" and .reason_code == "client_tokens_available" and .next_action' >/dev/null
"${BIN}" "${COMMON[@]}" models list --client-token-ref local-client --output json \
  | jq -e '.status == "ok" and .reason_code == "model_routes_available" and .next_action' >/dev/null
"${BIN}" "${COMMON[@]}" models explain --model gpt-example --client-token-ref local-client --output json \
  | jq -e '.status == "ok" and .reason_code and .next_action' >/dev/null
"${BIN}" "${COMMON[@]}" route explain gpt-example --client-token-ref local-client --output json \
  | jq -e '.status == "ok" and .reason_code and .next_action' >/dev/null
"${BIN}" "${COMMON[@]}" keys stats --credential-set relay_credentials --output json \
  | jq -e '.status and .reason_code and .next_action' >/dev/null
"${BIN}" "${COMMON[@]}" failures tail --last 20 --output json \
  | jq -e '.status and .reason_code and .next_action' >/dev/null
"${BIN}" "${COMMON[@]}" reload status --output json \
  | jq -e '.status and .reason_code and .next_action' >/dev/null
"${BIN}" "${COMMON[@]}" reload diff --output json \
  | jq -e '.status and .reason_code and .next_action' >/dev/null
"${BIN}" "${COMMON[@]}" reload apply --dry-run --output json \
  | jq -e '.status == "planned" and .reason_code == "reload_apply_dry_run" and .next_action' >/dev/null

set +e
NEGATIVE_OUTPUT=$("${BIN}" --management-url "${MANAGEMENT_URL}/v1" --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN models list --output json 2>&1)
NEGATIVE_STATUS=$?
set -e
if [[ "${NEGATIVE_STATUS}" -eq 0 || "${NEGATIVE_OUTPUT}" != *"client_base_url_used_for_management"* ]]; then
  printf 'error: client /v1 base URL was not rejected for management commands\n' >&2
  exit 1
fi

printf 'release smoke passed for %s %s\n' "${PACKAGE_NAME}" "${VERSION}"

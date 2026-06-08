# Operations

This document covers the runtime and deployment boundaries that operators should
keep stable for a small one-ai-key gateway. It intentionally avoids raw secrets,
request bodies, response bodies, and upstream-specific private data.

## Deployment Boundary

The gateway deployment host consumes a published GitHub Release tarball. On
NixOS, pin the release asset URL and its `sha256` in the host configuration, then
unpack and run the pinned binary. Do not build this repository on the deployment
host, do not run Cargo there, and do not use that host as an ad hoc Nix builder.

The supported build path remains local development host to release artifact:

```text
local Docker/Nix x86_64 build -> GitHub Release tarball + sha256 -> deployment host pin
```

A public client base URL has the shape `<public-gateway-base-url>/v1`, but the
URL is not a credential. Client tokens, management tokens, and upstream
credentials remain secret material and should never appear in Nix expressions,
Git history, tickets, copied terminal output, or documentation.

## Persistent State

Keep runtime state under a small number of explicit host paths. A typical
deployment uses `/opt/one-ai-key` as the service root:

```text
/opt/one-ai-key/config   runtime YAML and non-secret service configuration
/opt/one-ai-key/data     SQLite databases, mutable credential stores, and JSONL events
/opt/one-ai-key/secrets  client tokens, management tokens, upstream keys
/opt/one-ai-key/logs     process logs from the service manager or wrapper
```

`config` can contain local deployment shape, listener settings, route names, and
public model ids. It should not contain raw upstream keys unless the local host
policy deliberately treats the file as secret and keeps it out of Git.

`data` owns mutable runtime stores such as SQLite and writable credential state.
Database files are not release artifacts and must not be committed.

`secrets` owns token and key material. Files in this directory should be readable
only by the service user and the operator account that rotates them.

JSONL events and logs are operational evidence, not source material. They may
contain timing, route, status, and reason-code data, but operators should treat
them as sensitive because they can reveal deployment topology or operational
context. Some deployments keep JSONL events under `data`, for example
`/opt/one-ai-key/data/events.jsonl`, so treat configured event paths as mutable
runtime state even when they are not under a dedicated `logs` directory.

Do not commit SQLite databases, JSONL event streams, token files, upstream key
files, local YAML containing secrets, generated tarballs, or checksum sidecars.

## Runtime Workflow

Install from a release artifact, generate local config, check it offline, then
start the service:

```bash
shasum -a 256 -c one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz
one-ai-key init local --out config/local.yaml --keys data/relay.keys --dry-run
one-ai-key init local --out config/local.yaml --keys data/relay.keys --yes
one-ai-key check-config --config config/local.yaml
one-ai-key serve --config config/local.yaml
```

`check-config` is offline. It parses local YAML, expands `upstreams`, validates
local references, counts local credential lines, and reports redacted model
visibility. It does not open SQLite stores, probe upstreams, or call upstream
catalogs.

Configure clients with the OpenAI-compatible base URL and a client token:

```text
Base URL: <public-gateway-base-url>/v1
API Key: <client-token>
Model: <public-model-id>
```

Inspect the local client-visible catalog with the same client token:

```bash
curl <public-gateway-base-url>/v1/models \
  -H 'Authorization: Bearer <client-token>'
```

For local loopback checks, replace `<public-gateway-base-url>` with the service
origin, for example `http://127.0.0.1:4101`.

Operator commands use the management token and the management origin, not the
client `/v1` base URL:

```bash
export ONE_AI_KEY_MANAGEMENT_TOKEN=<management-token>

one-ai-key doctor --management-url <management-origin> \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN

one-ai-key models list --management-url <management-origin> \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --client-token-ref <client-token-ref>

one-ai-key models explain --management-url <management-origin> \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --model <public-model-id> \
  --client-token-ref <client-token-ref> \
  --endpoint-family chat_completions

one-ai-key route explain <public-model-id> --management-url <management-origin> \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --client-token-ref <client-token-ref>
```

The offline `check-config` visibility preview, authenticated `/v1/models`, and
the `models explain` / `route explain` views should agree for the same generated
config, public model id, and client-token reference.

## Operator Command Boundaries

All operator reports use a redacted envelope with status, stable reason code,
side-effect class, effect vector, scope/window data when applicable, and a safe
next action. Table output can be shorter than JSON, but it must preserve the
same decision-bearing fields.

Use `models explain --model <public-model-id> --client-token-ref <client-token
ref> --endpoint-family chat_completions` as the canonical first command for
client/model/endpoint availability. It answers whether that client-token ref can
use the model on the requested endpoint family and returns `can_use`,
`blocking_domain`, `endpoint_family`, bounded evidence, and a safe next_action before
any route-target detail is inspected.

Read-only commands do not write local files, call upstreams, or mutate
management state: `doctor`, `models list`, `models explain`, `route explain`,
`client-tokens list`, `keys list`, `keys stats`, `failures tail`, `failures
explain`, `reload status`, and `reload diff`.

Diagnosis recommendations stop at read-only commands. Diagnosis reports may
recommend read-only reload investigation only: `reload status` or `reload diff`.
`reload apply --dry-run` is an explicit operator reload-planning command, not a
diagnosis next action.

Dry-run commands preview the intended effect and must leave write/upstream bits
off: `init local --dry-run`, `keys import --credential-set <id> --source <path>
--dry-run`, `keys probe --credential-set <id> --credential-ref <ref> --model
<public-model> --dry-run`, `keys probe-apply apply --credential-set <id>
--credential-ref <ref> --dry-run`, `models onboard-plan --dry-run`, and `reload
apply --dry-run`.

Upstream-touching commands are explicit. `keys probe --yes` probes one
credential reference and may persist redacted probe evidence; it is not a
background health scan and it is not an automatic routing mutation.

Mutating commands require `--yes` or interactive confirmation:
`keys import --credential-set <id> --source <path> --yes`, `keys disable
--credential-set <id> --credential-ref <ref> --reason <reason> --yes`, `keys
probe-apply apply --credential-set <id> --credential-ref <ref>
--probe-result-ref <probe-ref> --yes`, and `reload apply
--expected-staged-registry-version <version> --yes`. Each report must disclose
whether it writes the management store, mutates active runtime state, or has no
automatic rollback.

## Diagnostic Red Lines

Diagnostics should use redacted structural evidence: HTTP status, stable reason
codes, public model ids, route names, credential lifecycle state, reload state,
release version, and checksum identity. Never paste raw client tokens,
management tokens, upstream keys, complete request bodies, complete response
bodies, provider payloads, absolute secret paths, or token-like URL components.

Do not preserve private deployment names, private host names, real gateway or
upstream domains, temporary key values, one-off replacement procedures,
conversation excerpts, workaround transcripts, or operator identities in project
docs, examples, release notes, test fixtures, or generated artifacts. Convert
recurring lessons into stable commands, reason codes, smoke checks, or checklist
items; otherwise remove them.

Do not copy advertising, promotion, invite, mirror-site, or unrelated marketing
text from logs, web pages, OCR, screenshots, or error output into project files
or operational records.

Before diagnosing application behavior, confirm the deployed binary is the
pinned release asset by checking the systemd `ExecStart` path or equivalent
package reference, tarball checksum, and host pin. If the host is running a
locally built binary, replace it with the pinned release artifact before
continuing.

### 502

Treat `502` as an upstream or relay failure boundary until typed evidence says
otherwise. Check the authenticated management health and alert endpoints for
redacted upstream status, credential lifecycle changes, and response-filter
events. If the failure followed a release change, compare the deployed version
and checksum with the GitHub Release asset and verify that the route target still
points at the intended upstream base URL.

For non-streaming Chat Completions and Responses requests, one-ai-key may hide
one selected-target pre-output 502, 503, 504, or transport failure when the body
is replayable, no client bytes have been sent, the request deadline can fit the
extra attempt, and the retry policy allows the chosen continuation. This is a
single conservative retry, not a background health check. Streaming requests,
Embeddings, named-pool forwarding, unknown endpoint families, and `/v1/models`
are outside this retry allowlist.

Do not debug a `502` by pasting raw upstream responses into docs or tickets.
Capture the status code, redacted provider/account/channel id, policy profile,
reason code, and whether any response bytes had already been sent to the client.

### 503

Treat `503` as local unavailability or exhausted routable capacity. Check
`/health`, `/ready`, `/management/health/serving`, `/management/health/resilience`,
and `/management/alerts` with the management bearer token. A process may be live
while the serving explanation reports no usable credential, suppressed route
targets, stale runtime state, or required operator input.

If `/ready` is healthy but client traffic receives `503`, inspect route preview
and credential state rather than restarting blindly. Restarting does not repair
expired credentials, empty key files, incorrect model routes, or stale staged
runtime state.

If the `503` came from a selected upstream target before response output, check
recent routing telemetry or `failures tail` for `retry_same_target`,
`retry_route_target`, `retry_credential`, or a stable denial reason such as
`failure_not_retryable`, `streaming_not_retryable`,
`effective_deadline_exhausted`, or `attempt_limit_reached`. A client-visible
`503` after those gates means the conservative retry was either ineligible or
already consumed.

### `no route candidate`

`no route candidate` means the compiled runtime could not find a usable route
candidate for the requested public model and client scope. Verify the public
model id, client-token scope, route target ordering, channel state, credential
set state, and any provider/account cooldown evidence. Use read-only explain
commands or management explain endpoints; do not expose the raw client token
used by the failing caller.

If a model was recently discovered or staged, confirm that sync apply and runtime
reload actually happened. Discovery alone does not make a model visible to
client traffic.

### `invalid router api key`

`invalid router api key` is a client-facing authentication failure. Confirm that
the caller is using a valid client token for this gateway, not a management token
and not an upstream provider key. Check token rotation history and service
configuration on the host, but never paste the token value into docs, tickets,
shared notes, shell history, or Git.

If the gateway sits behind a reverse proxy, verify that the `Authorization`
header reaches one-ai-key unchanged. Record only whether the header was present
and which redacted token name or stable client id was expected.

## Release Smoke

Before publishing a release, run the local artifact smoke from the repository
root after the Docker/Nix release build:

```bash
scripts/release-smoke.sh
```

That smoke uses the extracted release binary, generated placeholder tokens, and
a local mock upstream. It does not contact a deployment host.

After an operator intentionally pins a deployment host to a GitHub Release asset
and checksum, the operator may run a minimal deployment smoke against the public
base URL. The smoke should verify process liveness, authenticated management
health, `/v1/models`, and one non-sensitive completion request using a test
client token and a harmless prompt. Store only redacted status, reason codes,
route names, model ids, release version, and checksum in the smoke record.

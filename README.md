# one-ai-key

`one-ai-key` is a lightweight OpenAI-compatible key router for personal and
small-team deployments. Clients use one `/v1` base URL and one client token;
the router maps each public model id through explicit local model routes to an
upstream channel, credential set, and real upstream credential.

The operator surface is intentionally redacted. Management APIs and CLI reports
explain configuration, model visibility, route choice, key state, reload state,
and bounded recent failure evidence without printing raw client tokens,
management tokens, upstream keys, raw request bodies, raw response bodies,
absolute key-file paths, or token-like URL components.

The project is not a hosted multi-tenant platform, billing system, UI product,
live provider catalog, active health-check cluster, broad protocol converter, or
support bundle.

## What It Does

- Exposes OpenAI-compatible `/v1/*` routes for normal AI clients.
- Supports direct named-pool forwarding through `/pools/{pool}/...`.
- Maps public client model ids to explicit local route targets.
- Keeps upstream credentials out of client configuration.
- Supports multiple upstream auth shapes, including bearer and API-key headers.
- Imports replacement upstream credentials through the management API when a
  writable credential store is configured.
- Tracks credential lifecycle state such as available, cooling down, expired,
  quota exhausted, and disabled.
- Applies relay-specific error semantics through typed profiles and structured
  rules instead of free-form upstream text matching.
- Streams successful responses while keeping bounded pre-output guards for
  obvious upstream error envelopes and configured response-filter rejections.
- Serves `/v1/models` from compiled local runtime state. It does not call
  upstream `/v1/models` on the request path.
- Provides redacted operator commands for `doctor`, `models`, `route`, `keys`,
  `failures`, and `reload`.

## Fit

Use this project when you want:

- one client-facing API key and base URL for several official APIs or relays;
- local or private-network routing for personal or small-team usage;
- explicit model-to-channel routing instead of live upstream catalog fan-out;
- lightweight failover before client-visible output when retry gates allow it;
- redacted management APIs for credential import, lifecycle operations, model
  discovery, route preview, runtime reload, and health inspection.

Do not use it as:

- a public multi-tenant gateway;
- a billing, quota resale, or price synchronization platform;
- a full API gateway like Kong, Envoy, or HAProxy;
- a LiteLLM/New API replacement with teams, UI, budgets, or provider catalogs;
- a background probing service that continuously scans every upstream key;
- a system that must retry after streaming output has already begun.

## Install

Download a pinned Linux x86_64 release artifact and its checksum from:

```text
https://github.com/hechuyi/one-ai-key/releases
```

Then verify the checksum, unpack the tarball, and run the extracted
`one-ai-key` binary with a local configuration file:

```bash
shasum -a 256 -c one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz
./one-ai-key serve --config config/local.yaml
```

For development, run from source:

```bash
cargo run -- --config config/local.yaml
```

## Quick Start

Generate a local starter config and key file:

```bash
one-ai-key init local --out config/local.yaml --keys data/relay.keys --dry-run
one-ai-key init local --out config/local.yaml --keys data/relay.keys --yes
```

Put at least one upstream API key in `data/relay.keys`, one key per line. The
generated config contains placeholder client and management tokens; replace
them with your local values before serving traffic.

Check the config offline before starting the service:

```bash
one-ai-key check-config --config config/local.yaml
one-ai-key check-config --config config/local.yaml --output json
```

`check-config` parses YAML, expands `upstreams`, validates local references,
counts local credential lines, and reports a redacted model visibility preview.
It does not open SQLite stores, start the HTTP listener, probe upstreams, or call
upstream `/v1/models`.

Example `config/local.yaml`:

```yaml
listen: 127.0.0.1:4101

client_tokens:
  - name: local-client
    token: <client-token>

management:
  admin_token: <admin-token>

default_pool: relay
default_routing_profile: default-routing

routing_profiles:
  default-routing:
    key_selection: sticky_until_failure
    default_credential_cooldown_seconds: 20
    same_request_credential_retry:
      enabled: false
      max_retries: 0
    route_target_retry:
      enabled: true

upstreams:
  relay:
    template: openai_compatible_bearer
    api_base: <upstream-openai-compatible-base-url>/v1
    credential_set: relay_credentials
    keys_file: /data/relay-keys.txt
    models:
      - public_model: gpt-example
        upstream_model: provider/gpt-example
```

Start the service and inspect the client-visible model catalog:

```bash
one-ai-key serve --config config/local.yaml
curl http://127.0.0.1:4101/v1/models \
  -H 'Authorization: Bearer <client-token>'
```

Send a model-bearing request through the OpenAI-compatible client base URL:

```bash
curl http://127.0.0.1:4101/v1/chat/completions \
  -H 'Authorization: Bearer <client-token>' \
  -H 'Content-Type: application/json' \
  -d '{"model":"gpt-example","messages":[{"role":"user","content":"ok"}]}'
```

Client configuration:

```text
Base URL: http://127.0.0.1:4101/v1
API Key: <client-token>
Model: gpt-example
```

The client token is not the management token. Normal clients use the
OpenAI-compatible `/v1` base URL and the client token. Operator commands use a
management origin or `/management` base URL plus a management token reference:

```bash
export ONE_AI_KEY_MANAGEMENT_TOKEN=<management-token>

one-ai-key doctor \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN

one-ai-key models list \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --client-token-ref local-client

one-ai-key models explain \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --model gpt-example \
  --client-token-ref local-client

one-ai-key route explain gpt-example \
  --management-url http://127.0.0.1:4101 \
  --management-token-env ONE_AI_KEY_MANAGEMENT_TOKEN \
  --client-token-ref local-client
```

The management CLI rejects a client `/v1` base URL used as
`--management-url`; pass the service origin or the management base instead.

## Configuration Model

The `upstreams` shortcut is a convenience layer. At startup it expands into the
normal registry resources: provider, account, credential set, channel, policy
profile, routing profile, and model route references. The request path sees only
the resolved in-memory runtime state.

Important resource types:

- `client_tokens`: client-facing API keys and model/channel scope.
- `credential_sets`: upstream credential sources.
- `upstreams`: concise upstream/channel declarations.
- `model_routes`: explicit public model ids and ordered route targets.
- `policy_profiles`: upstream error classification and relay semantics.
- `routing_profiles`: key selection, same-request credential retry, and
  route-target retry behavior.
- `response_filter`: optional successful-response redaction or rejection rules.

Local mutable files should stay out of Git. Keep runtime config, key files,
SQLite databases, JSONL logs, and generated release artifacts under ignored
paths such as `config/`, `data/`, `db/`, `logs/`, and `dist/`.

For field-level configuration notes, see
[docs/configuration.md](docs/configuration.md).

## Routing And Failure Handling

Default routing behavior is conservative:

- a selected credential stays sticky until typed failure evidence changes its
  lifecycle state;
- same-request credential retry is opt-in and only applies before response bytes
  are sent to the client;
- route-target retry can move to another frozen route candidate when enabled and
  the request is replayable;
- provider/account `Retry-After` failure domains are soft route suppression:
  normal and degraded candidates win first, but a provider-cooling target may
  still serve as the last available candidate instead of producing an immediate
  local `no_route_candidate`;
- streaming, non-replayable, and partial-output paths do not transparently
  fallback;
- duplicate-charge risk is recorded as telemetry when retrying after an upstream
  transaction whose charge status cannot be proven.

Relay behavior is configured through structured evidence. The built-in
`relay_profile` values are `official_openai`, `generic_relay`, and
`untrusted_relay`. `balance_scope: credential` treats structured quota evidence
as selected-credential lifecycle state. `balance_scope: channel` treats
structured relay balance evidence as transient selected-channel suppression.

For the complete runtime contract, see
[docs/technical-design.md](docs/technical-design.md).

## Operations

Management endpoints require the management bearer token and are redaction
boundaries. They expose stable ids, counts, states, reason codes, health,
runtime reload, discovery, route preview, and lifecycle operations. They must
not expose raw upstream keys, raw client tokens, raw request bodies, raw response
bodies, absolute key-file paths, or token-like URL components.

Model discovery is management-only and does not change client traffic by itself.
Use sync planning/apply and runtime reload only when discovered models should be
staged as explicit public routes.

Use `one-ai-key doctor` as the first read-only runtime summary. By default it
reads runtime management projections without writing local files, calling
upstreams, or mutating management state. Add `--include-alerts`,
`--include-events`, or `--include-routes` only when the bounded extra projection
is needed.

`one-ai-key models list --client-token-ref <ref>` shows public models visible to
one configured client-token reference from compiled runtime state. It should
agree with authenticated `GET /v1/models` for the same client token.

`one-ai-key models explain --model <public-model>` and
`one-ai-key route explain <public-model>` are read-only management CLI views over
the compiled routing preview. They show selected route candidates, client-token
scope, reload status, and static endpoint capability metadata when the active
runtime exposes it. Capability output is diagnostic metadata only; it does not
probe upstreams, change routing, or imply protocol conversion between endpoint
families.

`one-ai-key keys list` and `one-ai-key keys stats` are read-only credential-set
views. `keys stats --credential-set <id> --include-credential-refs` may show
non-secret `credential_ref` values for individual workflows when the runtime has
a durable non-secret reference. `keys import --credential-set <id> --source
<path> --dry-run` previews a replacement import from a local file. `keys import
--credential-set <id> --source <path> --yes` writes only after explicit
confirmation and reports whether management store or active runtime state
changed.

`one-ai-key keys probe --credential-set <id> --credential-ref <ref> --model
<public-model> --dry-run` previews a single-credential probe. A confirmed
`--yes` probe is upstream-touching and may persist redacted probe evidence. Use
`keys probe-apply plan --credential-set <id> --credential-ref <ref>` and `keys
probe-apply apply --credential-set <id> --credential-ref <ref> --dry-run` before
a confirmed `keys probe-apply apply --credential-set <id> --credential-ref <ref>
--probe-result-ref <probe-ref> --yes`.

`one-ai-key failures tail --last <n>` and `one-ai-key failures explain
<request-id>` read only bounded recent failure evidence from management
projections. They are not historical storage, routing input, or a place to
quote upstream payloads.

`one-ai-key reload status` is a read-only operator CLI view over
`/management/runtime` and `/management/explain/runtime`. It reports the active
registry generation, active and staged registry versions, whether a runtime
reload is pending, and the last reload stable reason code when present. It does
not call reload, inspect local YAML, or make staged models visible.

`one-ai-key reload diff` is also read-only. It wraps
`/management/runtime/reload-diff` and returns a bounded, redacted typed diff when
the registry store has a staged projection to compare against the active
runtime. When no staged projection exists, it reports an unavailable state rather
than inventing a local diff.

`one-ai-key reload apply --dry-run` reads runtime status and reload diff data and
prints the expected staged registry version that a later mutation must use. A
confirmed runtime reload is explicit:

```bash
one-ai-key reload apply --yes --expected-staged-registry-version <version>
```

The CLI checks the current staged version before sending the mutation, and the
management API enforces the same precondition at
`POST /management/runtime/reload?expected_staged_registry_version=<version>`.
Missing or stale preconditions fail before audit events, reload timestamps, or
runtime state are changed.

Core health endpoints:

- `GET /health`: process liveness.
- `GET /ready`: serving readiness compatibility endpoint.
- `GET /management/health/serving`: authenticated serving explanation.
- `GET /management/health/resilience`: authenticated resilience summary.
- `GET /management/alerts`: operator-input and degraded-state alerts.

`/ready` can remain ready while `/management/alerts` reports that a credential
set needs replacement credentials. That means the process can still serve some
traffic, but operator action is needed to restore spare capacity.

For the management endpoint matrix and runtime-state semantics, see
[docs/architecture.md](docs/architecture.md). For response-filter operations,
see [docs/response-filter.md](docs/response-filter.md).

## Local Verification

Run the full local check before committing behavior changes:

```bash
scripts/local-ci.sh
```

The script runs formatting, locked dependency checking, clippy with warnings as
errors, and the full test suite.

## Release Build

The only supported Linux x86_64 release path is the local Docker/Nix wrapper:

```bash
scripts/build-release-x86_64-linux-docker.sh
```

The build writes:

```text
dist/one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz
dist/one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz.sha256
```

Publish those files to a GitHub release. Deployment hosts should consume the
release artifact instead of compiling locally. Details are in
[docs/release-build.md](docs/release-build.md).

## Documentation

- [Technical design](docs/technical-design.md): concepts, request lifecycle,
  routing, credential lifecycle, management state, and extension rules.
- [Configuration](docs/configuration.md): YAML resource model, upstream
  shortcuts, profiles, stores, limits, and local secret handling.
- [Architecture notes](docs/architecture.md): deeper module-level design and
  management API boundaries.
- [Response filter](docs/response-filter.md): successful-response filtering,
  pre-commit rejection actions, event boundaries, and alert semantics.
- [Operations](docs/operations.md): deployment boundaries, persistent state,
  safe diagnostics, and release smoke checks.
- [Performance budget](docs/performance-budget.md): request-path and memory
  constraints.
- [Release build](docs/release-build.md): local x86_64 Docker/Nix release
  contract.

## Project Metadata

- Source: https://github.com/hechuyi/one-ai-key
- Releases: https://github.com/hechuyi/one-ai-key/releases
- Issues: https://github.com/hechuyi/one-ai-key/issues
- License: not declared in this repository.

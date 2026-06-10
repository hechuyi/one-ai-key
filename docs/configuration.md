# one-ai-key Configuration

This document is a field-level entry point for the YAML configuration model.
The authoritative schema lives in `src/config.rs`; this page explains the
operator-facing resource model and the boundaries that matter for deployment.

## Configuration Loading

The service reads one YAML file at startup. The path comes from `--config` or
`KEY_POOL_ROUTER_CONFIG`; if neither is set, the default is
`config/router.yaml`.

Before semantic validation, the loader expands the top-level `upstreams`
shortcut into normal registry resources. The request path never reads the YAML
file, expands shortcuts, or queries persistent stores.

For local first-run setup, use the offline operator commands:

```bash
one-ai-key init local --out config/local.yaml --keys data/relay.keys --dry-run
one-ai-key init local --out config/local.yaml --keys data/relay.keys --yes
one-ai-key check-config --config config/local.yaml
```

`init local` writes placeholders only. Replace the generated client token,
management token, and add upstream keys to the generated key file before
expecting a serving smoke test to work.

`check-config` is an offline preflight. It parses the YAML, expands
`upstreams`, validates local references, reads local key files through a
diagnostic placeholder loader, and reports `model_visibility_preview` for
configured client-token references. It can also warn about obvious static
endpoint capability mismatches, such as an embedding-looking public route whose
enabled targets all declare `embeddings: unsupported`. It does not open SQLite
stores, perform migrations, start a listener, probe upstreams, reload runtime,
or call upstream model catalogs.

## Top-Level Process Fields

| Field | Purpose |
| --- | --- |
| `listen` | Socket address for the HTTP server. Defaults to the project default when omitted. |
| `client_tokens` | Bootstrap client-facing tokens and optional model/channel scope. |
| `management` | Management authentication, optional IP allowlist, role principals, and event window settings. |
| `timeouts` | Connect, non-streaming total, and streaming idle timeout overrides. |
| `routing` | Global route-candidate, model-discovery, and telemetry buffer limits. |
| `max_request_body_bytes` | Maximum client request body size accepted by the proxy. |
| `max_model_catalog_body_bytes` | Maximum management model-discovery response body. |
| `max_error_body_bytes` | Maximum upstream error body retained for classification. |
| `response_filter` | Optional successful-response filtering policy. |

These fields are process/runtime settings. They are not route targets by
themselves.

## Registry Resources

The normalized registry contains these resource families:

| Resource | Purpose |
| --- | --- |
| `providers` | Provider kind and provider-level enablement. |
| `accounts` | Provider account endpoint, upstream API base, auth header shape, and account enablement. |
| `credential_sets` | File-backed bootstrap location for upstream credentials. |
| `pools` | Runtime channels. A pool binds account/provider settings, credential set, error policy, routing policy, and optional static endpoint capability metadata. |
| `policy_profiles` | Shared relay/error classification behavior. |
| `routing_profiles` | Shared credential selection and retry behavior. |
| `model_routes` | Client-visible public model ids and ordered route targets. |
| `model_groups` | Optional model groups used for client-token scope. |

`default_pool` is used when a request path can be served without explicit model
routing. `default_pool` is not a default model and never injects a missing
`model` field. Clients must send explicit public model ids for
OpenAI-compatible requests that need model routing; those ids are checked
against compiled `model_routes` and client-token scope. `default_routing_profile`
provides the routing profile for channels that do not specify one.

## Public Model Catalog

Client-facing `/v1/models` is a compiled local public catalog. It is derived
from active runtime registry state, explicit `model_routes`, and the bearer
client token's model/channel scope. It is not a fan-out request to upstream
providers, not an aggregation of upstream live `/v1/models`, and not a source of
provider pricing, context-window, tool-support, or feature metadata.

Model publication therefore happens by staging an explicit public route and then
reloading runtime state. `models onboard-plan --dry-run` is read-only route
planning. `models onboard-plan --apply --dry-run` previews the staged-registry
write. Confirmed `models onboard-plan --apply
--expected-staged-registry-version <version> --yes` writes only a staged
`model_routes` entry for an existing channel; it does not change active runtime
until a separate `reload apply --expected-staged-registry-version <version>
--yes` succeeds.

The onboard apply step does not edit `client_tokens`, create model groups,
create providers, create accounts, create channels, create credential sets,
import credentials, probe upstreams, discover upstream models, or call upstream
`/v1/models`. If a client-token scope excludes the new route, diagnostics report
the mismatch and leave the scope unchanged.

## Upstream Shortcuts

`upstreams` is a concise authoring layer for common single-channel setups. It is
expanded before registry validation.

```yaml
upstreams:
  relay:
    template: openai_compatible_bearer
    api_base: https://relay.example/v1
    credential_set: relay_credentials
    keys_file: /data/relay-keys.txt
    models:
      - public_model: gpt-example
        upstream_model: provider/gpt-example
```

The shortcut creates or wires the corresponding provider/account/credential
set/channel/model-route resources. After expansion, runtime behavior is the same
as if those resources had been written explicitly.

Common templates include OpenAI-compatible bearer and API-key-header shapes.
Managed-site templates can provide defaults for known relays, but local
configuration should still make model exposure and credential sources explicit.

## Endpoint Capabilities

`pools.<id>.endpoint_capabilities` is static operator metadata for endpoint
families. It is resolved at startup or runtime reload and is intended for
diagnostics and management projections, not request-time enforcement.

```yaml
pools:
  relay:
    provider_kind: openai_compatible
    api_base: https://relay.example/v1
    credential_set: relay_credentials
    endpoint_capabilities:
      chat_completions: supported
      responses: unknown
      embeddings: unknown
      models: local_projection
      diagnostic_labels:
        - relay
```

Supported values for `chat_completions`, `responses`, and `embeddings` are
`supported`, `unsupported`, and `unknown`. Supported values for `models` are
`local_projection`, `unsupported`, and `unknown`; `local_projection` means the
router serves client-facing `/v1/models` from its compiled local public catalog.

OpenAI-compatible upstream templates populate conservative defaults:
`chat_completions: supported`, `responses: unknown`, `embeddings: unknown`, and
`models: local_projection`. The metadata does not list provider models, context
lengths, pricing, tool support, or provider-specific model features. It does not
call upstream `/v1/models`, probe credentials, reject client requests, or bridge
Responses API requests to Chat Completions.
`responses: supported` is only an operator-declared endpoint-family diagnostic.
It does not enable Responses defaulting, Responses-to-Chat conversion, missing
model injection, or any request-path protocol adapter.

Management projections and CLI explanation commands may display these fields.
`models explain` reads the compiled routing preview plus the management
model-availability projection when an endpoint family is requested. `route
explain` reads the compiled routing preview and, when `--endpoint-family` is
provided, attaches that same model-availability projection to the route report.
Those commands report candidate endpoint capabilities as structured JSON or a
table summary. They do not query `/management/channels/{id}` per candidate, call
upstream endpoints, mutate runtime state, or treat `unknown` as `unsupported`.

`check-config` treats capability diagnostics as warnings only. The current
offline check intentionally warns only for high-confidence mismatches: model
names that strongly look like embeddings routed only through enabled targets
whose resolved `embeddings` capability is explicitly `unsupported`. `unknown`
does not warn, because it means the router has no static evidence either way.

## Client Tokens

`client_tokens` are the only credentials clients should see. They can restrict:

- visible public models through `allowed_model_groups`;
- usable channels through `allowed_channels`;
- serving eligibility through `enabled`.

They are not upstream provider keys. Management API responses must not expose
their raw token values.

When SQLite-backed client-token storage is populated, that store is
authoritative for mutable client-token scope. Editing YAML does not silently
rewrite stored scope.

`models explain` is a diagnostic projection over one public model id,
client-token reference, and endpoint family. Authenticated `/v1/models` is the
actual catalog a client sees with its bearer token. A scope mismatch is an
explanation result, not an automatic repair path; operators must intentionally
change token scope or model groups through the supported configuration/store
workflow before expecting the route to appear for that client.

## Credential Sources

File-backed credential sets use:

```yaml
credential_sets:
  relay_credentials:
    keys_file: /data/relay-keys.txt
```

The file is a bootstrap source for upstream secrets. Raw keys should live under
ignored local paths such as `data/` and must not be committed.

When `KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE` is configured, the writable
credential store can import replacement credentials through management APIs.
Request forwarding still selects from in-memory pool state and does not query
SQLite on the hot path.

Raw upstream keys are not accepted as CLI positional arguments. Valid key
sources are configured `keys_file` entries, environment or startup secret
sources supported by the deployment, local source files controlled by the
operator, the writable credential store, and `one-ai-key keys import --source
<path>`. The `--source` value is a path to a local file, not the key value
itself. `keys import --dry-run` reads the file locally for redacted counts and
duplicate hints; confirmed `keys import --yes` sends the source contents to the
management credential store.

## Relay Error Policy

Relay behavior is configured with structured evidence, not free-form upstream
message parsing.

```yaml
policy_profiles:
  relay-policy:
    error_rules:
      relay_profile: generic_relay
      balance_scope: channel
```

Supported relay profiles:

- `official_openai`: standard OpenAI-compatible status semantics.
- `generic_relay`: conservative handling for relays with mixed status-code use.
- `untrusted_relay`: stricter relay assumptions for less predictable upstreams.

`balance_scope: credential` applies structured quota evidence to the selected
credential. `balance_scope: channel` applies structured relay balance evidence
as transient selected-channel suppression. Account/provider/credential-set
balance scopes are outside the current design.

## Routing Policy

Routing profiles define selection and retry boundaries:

```yaml
routing_profiles:
  default-routing:
    key_selection: sticky_until_failure
    default_credential_cooldown_seconds: 20
    same_request_credential_retry:
      enabled: false
      max_retries: 0
    route_target_retry:
      enabled: true
```

Same-request retry is gated by replayability, streaming status, output commit
state, endpoint-family allowlist, named-pool exclusion, attempt limits, policy,
frozen candidates, and deadline budget. The initial pre-output stability
allowlist covers only non-streaming Chat Completions and Responses requests.
Streaming, partial-output, Embeddings, named-pool forwarding, unknown endpoints,
and `/v1/models` do not transparently fallback.

## Management

Management configuration defines the admin token, optional IP allowlist,
principals, roles, and event retention:

```yaml
management:
  admin_token: <admin-token>
  ip_allowlist:
    - 127.0.0.1
  principals:
    - name: readonly
      token: <readonly-token>
      role: readonly
```

Roles are `readonly`, `operator`, and `admin`. Management endpoints are
redaction boundaries. If management is enabled on a non-loopback listen address,
configuration must provide a non-empty `management.ip_allowlist`; otherwise
startup fails.

## Writable Stores

Optional local SQLite stores are selected by environment variables:

| Environment variable | Purpose |
| --- | --- |
| `KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE` | Persists mutable registry resources and staged runtime changes. |
| `KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE` | Persists upstream credential lifecycle and imports. |

Writable stores are control-plane state. Client request forwarding continues to
use compiled runtime snapshots and in-memory credential pools. The credential
store is the durable control plane for replacement imports and lifecycle
commands; the request hot path does not read YAML, SQLite, or other credential
stores.

## Local File Hygiene

Keep these out of Git:

- runtime YAML containing real tokens;
- upstream key files;
- SQLite databases and WAL files;
- JSONL logs and event exports;
- release artifacts under `dist/`;
- locally rendered documentation exports.

The repository `.gitignore` and `.dockerignore` cover the normal local paths.

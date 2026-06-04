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
| `pools` | Runtime channels. A pool binds account/provider settings, credential set, error policy, and routing policy. |
| `policy_profiles` | Shared relay/error classification behavior. |
| `routing_profiles` | Shared credential selection and retry behavior. |
| `model_routes` | Client-visible public model ids and ordered route targets. |
| `model_groups` | Optional model groups used for client-token scope. |

`default_pool` is used when a request path can be served without explicit model
routing. `default_routing_profile` provides the routing profile for channels
that do not specify one.

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
state, attempt limits, policy, frozen candidates, and deadline budget. Streaming
or partial-output responses do not transparently fallback.

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
use compiled runtime snapshots and in-memory credential pools.

## Local File Hygiene

Keep these out of Git:

- runtime YAML containing real tokens;
- upstream key files;
- SQLite databases and WAL files;
- JSONL logs and event exports;
- release artifacts under `dist/`;
- locally rendered documentation exports.

The repository `.gitignore` and `.dockerignore` cover the normal local paths.

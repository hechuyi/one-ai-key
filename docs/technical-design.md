# one-ai-key Technical Design

This document describes the stable technical contract for `one-ai-key`. It is
written for maintainers and operators who need to understand how requests move
through the router, where state can change, and which boundaries should not be
crossed when extending the project.

## Design Goals

`one-ai-key` is a lightweight AI API key router. The central design goal is to
keep client configuration simple while keeping routing and lifecycle decisions
explicit, bounded, and observable.

The service optimizes for:

- personal and small-team deployments;
- one OpenAI-compatible client endpoint;
- explicit model routes instead of live upstream catalog lookup;
- runtime request handling from compiled in-memory state;
- redacted management operations;
- predictable retry and fallback behavior before client-visible output.

The service does not optimize for:

- public multi-tenant operation;
- billing, quotas, teams, or reseller accounting;
- active background health-check clusters;
- live upstream model aggregation on `/v1/models`;
- request-path storage joins;
- mid-stream continuation after bytes have been sent to a client.

## Architecture Overview

The service is split into four boundaries:

| Boundary | Responsibility | Request-path rule |
| --- | --- | --- |
| Data plane | Authenticate clients, plan routes, select credentials, forward requests, stream responses, and apply bounded pre-output guards. | Must use compiled runtime state only. |
| Control plane | Resolve configuration, expand upstream shortcuts, manage registry resources, and rebuild runtime snapshots. | May read/write registry stores, but only outside forwarding. |
| Management plane | Expose redacted operator APIs for lifecycle changes, discovery, preview, reload, health, and events. | Must not expose raw secrets or raw request/response material. |
| Persistence layer | Store mutable registry, credential, client-token, and event state when configured. | Must not become a synchronous dependency of the forwarding hot path. |

This split is the main performance and safety invariant. New features should
first decide which boundary owns the state transition, then expose only the
minimal projection needed by the other boundaries.

## Core Concepts

**Client token**

A client-facing API key accepted by the router. It can restrict visible public
models and channels. It is not an upstream provider credential.

**Public model**

The model id a client sends in an OpenAI-compatible request. A public model maps
to one or more route targets through `model_routes`.

**Route target**

One candidate for serving a public model. It names a channel and can optionally
rewrite the public model id to an upstream model id.

**Channel**

A resolved upstream endpoint plus provider/account metadata, credential set,
error policy, and routing policy. Channel health is runtime state and is
separate from credential lifecycle.

**Credential set**

A named collection of upstream credentials. Multiple channels can intentionally
share a credential set.

**Credential**

One upstream secret plus runtime lifecycle state. The raw secret must not leave
the credential store or runtime forwarding boundary.

**Policy profile**

Structured upstream error semantics. Policy profiles classify status/code/limit
evidence into failure kinds, scopes, retryability, and cooldowns.

**Routing profile**

Retry and selection behavior: key selection strategy, default cooldown,
same-request credential retry, and route-target retry.

**Registry document**

The startup/control-plane configuration document. YAML `upstreams` shortcuts are
expanded into ordinary registry resources before validation.

**Runtime snapshot**

The compiled in-memory state used by request forwarding. The proxy hot path must
use this state instead of parsing YAML, reading SQLite, or probing upstreams.

## Configuration Pipeline

Startup follows a fixed pipeline:

1. Read YAML from the configured file.
2. Expand `upstreams` shortcuts into provider/account/credential-set/channel
   resources.
3. Reject unknown top-level fields and malformed resource references.
4. Resolve templates, policy profiles, routing profiles, model routes, and
   credential sources.
5. Build `AppState` with compiled runtime structures.

Management writes can stage registry changes in writable stores. Staged changes
do not affect client traffic until runtime reload or process restart applies the
new compiled runtime.

Static endpoint capabilities are resolved with pool/channel configuration.
Provider-kind defaults and `upstreams` template defaults may populate
`endpoint_capabilities`, and explicit pool configuration may override those
fields. The capability model is deliberately endpoint-family metadata:
chat completions, Responses, embeddings, local models projection, and diagnostic
labels. It is not a provider/model catalog and must not contain model ids,
pricing, context windows, tool support, or other model-specific facts.
Static endpoint capabilities must not authorize Responses-to-Chat conversion,
missing-model defaulting, or endpoint-family fallback. Responses defaulting and
Responses-to-Chat adapters are parked follow-up work, not part of this M1-M4
implementation plan.

The request path must not:

- parse YAML;
- query registry storage;
- query credential storage;
- inspect profile provenance;
- discover upstream models;
- evaluate static endpoint capabilities;
- read default-model configuration to fill a missing `model` field;
- use static endpoint capabilities to bridge protocol families;
- rewrite Responses requests into Chat Completions requests;
- scan all credentials in a set;
- infer state from free-form upstream text.

## Request Lifecycle

OpenAI-compatible requests enter through `/v1/*`.

1. The proxy authenticates the client token.
2. The provider adapter extracts the requested public model when the endpoint
   needs model routing. The model id must come from the client request; the
   proxy does not inject a configured default model when that field is absent.
3. Route planning reads the compiled model route and client token scope.
4. The route planner builds a bounded frozen candidate set.
5. The selected channel's pool chooses an available credential.
6. The provider adapter rewrites upstream auth headers and, when configured,
   rewrites the model id for that route target. It does not convert Responses
   requests to Chat Completions or use endpoint capability metadata to choose a
   different protocol.
7. The proxy sends the upstream request.
8. Before sending bytes to the client, the proxy may classify transport
   failures, non-2xx upstream responses, guarded 2xx error envelopes, or
   configured pre-commit response-filter rejections.
9. Routing consumes the classified failure and returns one directive:
   `retry_credential`, `retry_route_target`, `retry_same_target`, or
   `return_error`.
10. Once a response body has been committed to the client, retry and fallback
    stop for that request.

Any future default-model exception requires a separate protocol/request-path
plan proving disabled-by-default behavior, explicit public route ids,
client-token scope enforcement, no YAML/store/upstream reads on the hot path, no
Responses authorization by static capability metadata, and no successful-response
buffering beyond the existing bounded guards.

Named-pool requests through `/pools/{pool}/...` select the named channel
directly instead of resolving a public model route. Provider adapters still
define the body semantics. OpenAI-compatible named-pool requests can extract
model context, enforce public-route scope rules, and use replayable forwarding
when the endpoint permits it. Generic named-pool bodies are treated
conservatively as streaming pass-through traffic.

## Model Catalog Semantics

Client-facing `/v1/models` is a local projection over the compiled runtime
registry. It returns public model ids visible to the authenticated client token.

`/v1/models` does not:

- call upstream providers;
- select credentials;
- buffer upstream catalog bodies;
- apply lifecycle transitions;
- expose internal upstream model ids for multi-target routes.

Model discovery exists only under management endpoints. Discovery can help an
operator stage model routes, but client traffic continues to depend on explicit
compiled `model_routes`.

## Credential Lifecycle

Credentials are stateful resources, not anonymous strings. Runtime selection can
observe these states:

- `Available`: eligible for request selection.
- `CoolingDown`: temporarily excluded until a monotonic deadline.
- `Expired`: durably excluded until restored.
- `QuotaExhausted`: durably excluded until restored or recharged.
- `Disabled`: manually excluded until explicitly enabled.

Automatic proxy-side failure handling can update in-memory credential state and
enqueue durable lifecycle persistence when a writable credential store is
configured. That enqueue is asynchronous and bounded; the proxy must not block
on SQLite writes in the request path.

Manual management commands use the same lifecycle vocabulary and redaction
rules. Writable-store mode persists durable lifecycle snapshots before applying
management mutations to runtime state.

## Routing And Retry

Retry decisions are centralized in `routing.rs`. The proxy observes evidence;
routing decides what state mutation and retry directive, if any, should occur.

The hard gates are:

- request body must be replayable;
- request must not be streaming;
- no response bytes may have been sent to the client;
- attempt limits must not be exhausted;
- policy must allow the retry directive;
- a frozen retry candidate must exist;
- the effective request deadline must fit another attempt.

Same-request credential retry is opt-in. It can try a different available
credential in the same credential set when the selected credential produced
typed credential-scoped failure evidence and the hard gates pass.

Route-target retry is separate. It can move to another frozen route target when
the failure scope and routing profile allow it.

Same-target retry is a narrow stability guard. It allows one additional
pre-output attempt for selected channel/provider failures when no fallback
target remains, no cooldown evidence was supplied, and the normal hard gates
pass.

Duplicate-charge risk is telemetry, not billing truth. The current runtime
emits `none` when the gateway has no evidence of a completed upstream
transaction or when no retry is attempted, and `unknown` when a retried attempt
may have reached provider accounting. There is no current `known_no_charge`
state.

## Channel Health And Failure Domains

Channel health is separate from credential lifecycle.

Channel-scoped failures can mark the selected channel as cooling down or
degraded. Cooldown excludes that channel from planning until its deadline
expires. Degraded state is a last-resort signal: healthy candidates are preferred
when available, but a degraded single target can still serve traffic if no
better route exists.

Provider/account failure domains are runtime-only suppression state layered
above channel health. They are intentionally in memory and do not become
database-backed circuit breakers.

Manual or configured disablement remains authoritative. Automatic cooldown
expiry or success recovery must not re-enable disabled providers, accounts,
channels, or credentials.

## Relay Error Semantics

Relay behavior must be represented with typed structured evidence:

- HTTP status;
- upstream error code;
- limit type;
- retry-after or structured cooldown evidence;
- explicit policy profile rules.

The built-in relay profiles are:

- `official_openai`: bare `401`/`403` can be credential auth failure.
- `generic_relay`: bare `401`/`403` are request-only unless structured
  invalid-key evidence is present.
- `untrusted_relay`: conservative relay semantics for less predictable
  upstreams.

`balance_scope: credential` means structured quota evidence applies to the
selected credential. `balance_scope: channel` means structured relay balance
evidence suppresses only the selected channel. Account/provider/credential-set
balance scopes are intentionally outside the current design.

Free-form upstream message matching is not part of hot-path lifecycle
classification.

## Success Guard And Response Filtering

Successful upstream responses are streamed. The router does not fully buffer
normal successful output.

For body-bearing 2xx JSON/SSE responses, the success guard can peek a bounded
prefix before success accounting. It detects obvious structured upstream error
envelopes before any client output and then reuses the normal failure and retry
path. Pass-through outcomes replay the peeked prefix exactly once.

Response filtering is a separate stream-boundary feature. Rules can redact or
reject matched successful output. Filter events store bounded metadata only:
request id, channel id, public model, rule id, action, content kind, reason
code, outcome, and whether the body had already been committed.

Only explicit pre-commit actions can create lifecycle evidence:

- `reject_and_expire_credential`
- `reject_and_cooldown_channel`

Events and alerts themselves are observability projections. They do not replay
into route planning, credential lifecycle, or channel health.

## Management Plane

Management endpoints are operator tools and redaction boundaries.

They expose:

- channel and pool status;
- credential-set operations state;
- redacted credential lists;
- credential import batches;
- client-token management;
- manual lifecycle commands;
- model discovery and sync planning;
- routing preview;
- runtime reload and runtime explanation;
- alerts, events, routing telemetry, serving health, and resilience health.

Management APIs must not expose:

- raw upstream keys;
- raw client or management tokens;
- token hashes;
- raw request bodies;
- raw response bodies;
- response-filter matched text;
- absolute key-file paths;
- URL userinfo or token-like query parameters.

All management routes are guarded by the management IP allowlist, bearer-token
authentication, and per-route role checks. When management is enabled on a
non-loopback listen address, configuration must provide a non-empty
`management.ip_allowlist`; otherwise startup fails.

## Persistence Boundaries

The project can use file-backed credentials for bootstrap and SQLite-backed
stores for mutable registry, credential, and client-token state.

Important rules:

- request forwarding reads compiled runtime state;
- credential-store writes happen behind async management or bounded persistence
  boundaries;
- when SQLite lifecycle snapshots are authoritative, JSONL management events are
  audit/display data for credential lifecycle;
- without snapshot authority, persisted management events can replay credential
  lifecycle and channel enable/disable state during startup;
- SQLite-backed client-token storage is authoritative when populated;
- editing YAML does not silently rewrite stored virtual key scope;
- client-token management mutations update the running token registry
  immediately after persistence instead of waiting for full runtime reload.

## Release And Deployment

The supported Linux x86_64 release artifact is built locally through:

```bash
scripts/build-release-x86_64-linux-docker.sh
```

Deployment hosts should consume GitHub release assets instead of compiling the
project locally.

Release assets are:

```text
one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz
one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz.sha256
```

The tarball metadata is normalized for deterministic archive output in the local
release path, and the checksum sidecar records only the archive basename.

## Extension Rules

When adding provider or relay support:

1. Prefer a provider template or management import adapter.
2. Keep protocol rewriting in provider adapters.
3. Keep lifecycle transitions in routing and pool state.
4. Add typed policy/profile fields instead of free-form message parsing.
5. Add management projections before exposing new operational state.
6. Preserve request-path constraints: no storage joins, live discovery, full
   buffering of successful responses, or all-credential scans.

When adding retry behavior:

1. Prove the request is replayable.
2. Prove no client output has begun.
3. Record a stable denial reason when retry is blocked.
4. Bound attempts and candidate count.
5. Emit telemetry without raw request or response material.
6. Treat duplicate-charge status conservatively.

## Verification Checklist

Before merging behavior changes:

```bash
scripts/local-ci.sh
```

Before publishing a release:

```bash
git status -sb --untracked-files=all
scripts/local-ci.sh
scripts/build-release-x86_64-linux-docker.sh
cd dist && sha256sum -c one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz.sha256
```

Also scan staged changes for raw keys, client tokens, management tokens,
databases, logs, generated release artifacts, and untrusted promotional text.

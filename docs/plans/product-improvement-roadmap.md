# Product Improvement Roadmap

This document is a product roadmap, not a task queue. It keeps one-ai-key focused
as a lightweight personal/small-team AI key router: clients keep one
OpenAI-compatible base URL and one client token, while routing, upstream keys,
failure handling, and maintenance stay behind the router.

The previous broad P0-P4 plan has been deliberately shrunk. The useful core is:

- explain whether a client can use a public model on a specific endpoint family;
- maintain keys through explicit management workflows;
- keep bounded failure evidence for troubleshooting and retry audit;
- absorb only small, pre-output upstream jitter through a conservative retry
  preset.

Everything else is parked unless a new plan explicitly reopens it.

## Product Boundary

one-ai-key should remain:

- lightweight and backend-first;
- explicit about local public model routes;
- client-invisible for upstream keys and relay quirks;
- predictable on small deployments;
- conservative about retry and fallback;
- redacted at every management boundary.

It must not become:

- a hosted multi-tenant platform;
- a billing, usage accounting, or resale system;
- a full UI product;
- an active health-check cluster;
- a live upstream model catalog aggregator;
- a broad OpenAI protocol conversion gateway;
- a persistent adaptive routing engine.

## Hard Invariants

The proxy request path must not:

- read YAML, SQLite, registry storage, credential storage, or client-token
  storage;
- call upstream `/v1/models` or any live catalog endpoint;
- scan all credentials in a set;
- compute route decisions from historical ledger data;
- complete protocol conversion based on endpoint capability metadata;
- retry after any client-visible output;
- transparently retry or fallback any streaming request, including pre-header
  failures;
- fallback after partial output;
- fully buffer successful responses;
- record raw request bodies, response bodies, upstream keys, client tokens,
  token hashes, complete URLs with token-like components, or untrusted upstream
  text in telemetry, logs, tests, docs, or reports.

Data-plane additions may only emit bounded facts: fixed-cost counters, bounded
attempt summaries, and non-blocking redacted event records. Failure evidence
does not absorb upstream jitter by itself; it only explains whether M3 retry was
eligible, attempted, denied, or risky. Overflow drops the oldest or current
event according to the concrete buffer/queue contract and increments a
dropped-event counter; it must not block request sending, credential mutation,
or response streaming.

Client-facing `/v1/models` remains a family-blind local public model projection.
It reports visible public model ids for the authenticated client token. It does
not assert Responses support, tools, vision, context window, pricing, adapter
status, endpoint-family support, upstream ids, or provider capability.

Endpoint-family availability belongs to management explain/status only.

## Roadmap

### M1: Self-Explanation Contract

**Goal:** make the router answer the operator's core question:

```text
client_token_ref + endpoint_family + public_model -> status / reason_code / next_action
```

This is not live health checking. It explains the current compiled runtime.

Keep:

- endpoint family as a compiled route visibility dimension;
- management-only availability explain;
- family-blind `/v1/models`;
- stable reason codes;
- redacted, bounded next actions;
- minimal config diagnostics for schema version and deprecated fields/templates.

Do not add:

- live upstream probes;
- upstream catalog fan-out;
- endpoint fallback;
- automatic protocol conversion;
- a config migration framework;
- full release-lifecycle migration matrices;
- capability metadata in `/v1/models`.

Acceptance gates:

- explain can distinguish token missing/disabled, model missing, endpoint
  family mismatch, unsupported endpoint family, no route, no usable key, and
  deprecated config;
- all outputs are redacted and machine-testable;
- `/v1/models` remains a simple local public-model list;
- deprecated config diagnostics report risk but do not rewrite config or change
  route semantics.

### M2: Bounded Maintenance And Failure Evidence

**Goal:** make daily maintenance less ad hoc without creating a control-plane
platform.

Key maintenance is a thin management API wrapper. It must not directly edit
active config files, key files, SQLite stores, or runtime memory.

Keep:

- `keys list` / `keys stats`;
- `keys import` or `keys append` with dry-run and explicit confirmation;
- a simple disable/retire action for bad keys when the existing management API
  supports it;
- manual `keys probe` only as an explicit upstream-touching operator action;
- bounded recent failure/attempt evidence for troubleshooting and retry audit;
- a small status summary for recent failures and dropped telemetry.

Park:

- `probe-apply`;
- selector promotion workflows;
- broad restore/promote lifecycle command sets;
- usage ledger, billing-style top views, audit ledger, and cost tracking;
- persistent failure ledger as a required prerequisite.

Failure evidence is not a routing input and must not be used to absorb
upstream jitter. The existing routing telemetry window is an in-memory ring
buffer controlled by `routing.telemetry_buffer_capacity`; the default is 1024
events. When full, the ring drops the oldest event. This roadmap does not make
persistent failure storage a prerequisite.

Failure evidence should be minimal: request id, time bucket, endpoint family,
public model, channel/credential reference when safe, failure kind, retry
decision, commit state, streaming state, duplicate-charge risk, and dropped
event count.

Acceptance gates:

- key CLI mutations go through management API auth, redaction, and existing
  mutation boundaries;
- no command prints raw keys, raw tokens, token hashes, absolute key paths, raw
  request/response bodies, or full upstream URLs with token-like components;
- failure evidence is bounded and redacted, with the default routing telemetry
  window remaining 1024 in-memory events unless explicitly configured;
- status output is one-screen operational context, not an analytics product.

### M3: Conservative Pre-Output Stability

**Goal:** reduce client-visible failures from small upstream jitter without
making retry behavior opaque.

This is the only stability behavior in this roadmap. It is a preset over
existing routing/policy mechanics, not a new stability engine.

Allowed conservative retry:

- at most one extra attempt;
- total wall-clock retry budget <= 1500 ms;
- budget includes backoff, candidate selection, and the extra attempt until
  upstream headers or terminal failure;
- retry may start only if the remaining effective client/request deadline can
  contain that budget;
- request body must be replayable;
- endpoint family must be known and retry-eligible;
- failure must be explicit pre-output transient evidence such as connect
  failure, TLS/connect timeout, header timeout, connection reset before
  headers, or 502/503/504 before body;
- duplicate-charge risk must be recorded when it cannot be proven absent.

Default prohibitions:

- no streaming retry or fallback;
- no partial-output fallback;
- no retry after client-visible output;
- no default cross-provider fallback;
- no retry for unknown failures;
- no retry for 401/403/404, model missing, permission errors, quota exhausted,
  context length, schema errors, or request parameter errors;
- 429 is not retryable by default;
- no active health scanning, sliding windows, adaptive routing, persistent
  breakers, or ledger-driven route decisions.

Acceptance gates:

- every retry decision has a recorded eligibility reason;
- every denial has a stable reason code;
- retry evidence is visible in management-only failure/explain output;
- the hot path still satisfies the hard invariants above.

## Parked Items

The following are outside this roadmap and must not be implemented by extending
this document:

- Responses-to-Chat adapter;
- future Responses route opt-in schema;
- adapter-backed Responses examples;
- endpoint capability triggered protocol conversion;
- automatic endpoint fallback;
- model alias/profile onboarding planner;
- automatic model exposure from upstream catalog;
- persistent model trust states;
- full config migration framework;
- status/top dashboards;
- billing, usage accounting, cost tracking, or per-client analytics;
- active background health probing;
- provider-wide adaptive breakers.

If any parked item becomes necessary, stop this roadmap and write a separate
plan with its own hot-path proof, user contract, and release gate.

## Completion Criteria

This roadmap is complete when M1-M3 are implemented, documented, tested, and
committed. Parked items are not part of completion.

Before closing the roadmap:

- filtered tests must prove they matched non-zero intended tests;
- `cargo test --locked` or a justified narrower release gate must pass;
- `git diff --check` must pass;
- staged files must exclude `dist/`, `target/`, `key-pool-router/`, `config/`,
  `data/`, SQLite, logs, keys, raw fixtures, and `AGENTS.md`;
- docs must describe stable product behavior, not support transcripts or future
  feature promises.

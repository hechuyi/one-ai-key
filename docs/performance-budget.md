# Performance Budget

Target deployment class: 1 vCPU, 1 GiB RAM.

This service is a personal AI account and API-key router, not a heavy control plane. Design decisions must keep the request path small and predictable.

## Runtime Budget

- The proxy request path must not perform disk I/O.
- The proxy request path must not allocate work proportional to all credentials. Credential selection in the active channel uses a selector index plus deadline promotion rather than scanning the full credential set per request.
- Successful upstream responses should stream; they must not be fully buffered.
- Body-bearing 2xx success guarding is bounded to the pre-output window only: at most 8192 peeked bytes, the first complete data-bearing SSE event, and 200 ms per attempt. Guard pass-through must replay the peeked prefix exactly once and then return to streaming.
- Response filtering is streaming and bounded. SSE filtering may hold bytes only until an event boundary; non-event pending bytes are capped at 8192 bytes with a 1024-byte overlap window.
- Guard and filter budgets compose sequentially, not cumulatively into full buffering. A guarded pass-through may hold at most the guard prefix before output, then the response filter may hold only its current SSE event or bounded non-event pending window. The combined contract is therefore capped pre-output inspection plus bounded streaming mutation; it must not retain the whole successful response, perform request-path disk I/O, or add an unbounded latency term after the 200 ms guard deadline.
- Non-streaming request bodies are bounded by `max_request_body_bytes`; the default is 2 MiB for 1c1G deployments.
- Generic named-pool request bodies are streamed through a bounded stream and must not be fully buffered on the proxy hot path.
- Upstream model catalog response bodies are bounded separately from request bodies by `max_model_catalog_body_bytes`; the default is 512 KiB.
- Client-facing `/v1/models` does not fan out to upstream providers. Management-only model discovery and sync planning keep explicit upstream catalog probes bounded by `routing.max_model_catalog_channels`; the default is 16 channels.
- Client-token model scope expansion is compiled into `AppState.runtime_catalogs`. Request forwarding, `/v1/models`, named-pool replayable authorization, and routing preview may test public-model membership against that in-memory map, but must not join against registry storage, client-token storage, or upstream catalog responses.
- Upstream error bodies are bounded by `max_error_body_bytes`.
- One shared `reqwest::Client` is used per `AppState`; do not create a client per request.
- Routing telemetry is an in-memory ring buffer bounded by `routing.telemetry_buffer_capacity`; the default is 1024 events.
- Automatic credential lifecycle persistence from proxy-side failure handling uses a bounded non-blocking queue. Enqueue failure must be observable through routing telemetry, but must not fall back to synchronous credential-store I/O on the request path.
- Provider/account failure-domain suppression is in-memory runtime state only. Request planning may consult the compiled account/provider breaker maps, but must not persist breaker transitions or query registry storage on the request path. 429/rate-limit evidence must remain credential scoped and must not open provider-wide suppression.
- Management operations may do disk I/O, but blocking filesystem work must run outside Tokio worker threads.
- Management lifecycle compensation after failed event recording must use the same async credential-store boundary as normal management persistence; it must not call blocking SQLite-backed persistence directly from an async management method.
- Management event append and durable lifecycle persistence must not hold proxy-path credential mutation or send gates; slow management persistence must not block automatic upstream-failure state transitions or new request sends.

## State Budget

- Credential runtime state is in memory and scales linearly with credential count.
- For the current local workload, hundreds to low thousands of credentials are acceptable.
- Credential IDs and fingerprints must not store raw secrets.
- Management event logs are append-only JSONL during the bootstrap phase, but only a bounded recent window is kept in memory for management listing. `management.event_window_capacity` controls that in-memory window and defaults to 1024 events. Startup replay reads the JSONL source directly. SQLite can replace this behind the same event-store boundary when channel/account management grows.

## Routing Budget

- Default strategy is sticky-until-failure.
- Do not round-robin every request by default; that increases upstream cooldown risk and unnecessary state churn.
- Route candidates are frozen once per request and carried through forwarding as one plan object. Do not recompute candidates after an upstream failure inside the same request.
- Explicit upstream cooldown evidence such as `Retry-After` must override local default cooldown.
- Same-request retry remains opt-in, not default.
- Same-target transient retry is capped at one pre-output retry and uses the same replayability, streaming, partial-output, and effective-deadline gates as other retry directives. It must not retry guarded 2xx bodies, responses with cooldown evidence, or any response after client output has begun.

## Review Gates

Before adding a dependency or background subsystem, check:

- Does it add a resident process, cache, or thread pool?
- Does it increase idle RSS substantially?
- Does it put CPU or I/O work on the proxy request path?
- Can the same boundary be implemented with existing Rust standard library or current dependencies?
- Can it run acceptably on 1c1G with 500 credentials and a single active user?

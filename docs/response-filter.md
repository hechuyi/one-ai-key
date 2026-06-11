# Response Filter

`response_filter` is a gateway-side safety boundary for upstream response
content. It is intended for relay contamination cases where an upstream or
intermediate service injects unwanted text into otherwise valid model output or
into a structured upstream error body. The filter is configured in YAML,
compiled at startup or runtime reload, and applied before matching response
bytes are returned to the client whenever the response path is still bounded.

The feature is deliberately separate from provider adapters and routing policy.
Provider adapters still own protocol and auth rewriting; routing still owns
retry and lifecycle state. Response filtering only inspects bytes already being
returned through the proxy boundary and decides whether to pass them through,
redact matching content, or replace blocked content with a local error payload.

## Configuration

Top-level YAML configuration:

```yaml
response_filter:
  enabled: true
  replacement: "[filtered]"
  rules:
    - id: unsafe-marker
      kind: literal
      value: unsafe-marker
      action: redact
      case_sensitive: false
    - id: structured-shape
      kind: required_regex
      pattern: '^\s*\{'
      action: reject
```

Fields:

- `enabled`: defaults to `false`. A disabled filter is equivalent to the previous direct streaming behavior.
- `replacement`: optional replacement text for redaction. Empty or omitted values use `[filtered]`.
- `rules`: ordered rules. Disabled rules are ignored during resolution. Enabled rule ids must be unique.

Rule fields:

- `id`: stable rule id, required and non-empty.
- `enabled`: defaults to `true`.
- `kind`: one of `literal`, `regex`, `required_literal`, `required_regex`.
- `value`: required for literal rule kinds.
- `pattern`: required for regex rule kinds.
- `case_sensitive`: applies to literal rule kinds and defaults to `false`.
- `action`: `redact` by default, `reject` for stricter fail-closed behavior, or
  one of the explicit lifecycle actions `reject_and_expire_credential` and
  `reject_and_cooldown_channel`.

Rule semantics:

- `literal`: blacklists a literal marker. Matching text is redacted or rejected.
- `regex`: blacklists a regular expression. Matching text is redacted or rejected.
- `required_literal`: allowlist-style rule. If the literal is absent, the rule action is applied.
- `required_regex`: allowlist-style rule. If the regex does not match, the rule action is applied.

Literal matching removes common zero-width format characters before matching, so simple visual obfuscation does not bypass a literal rule. Redaction replaces the matched original span, including ignored format characters. Regex rules run on the original text and should include their own normalization tolerance if needed.

## Runtime Behavior

The filter is stored in `AppState` as a compiled policy. `POST
/management/runtime/reload` replaces the compiled policy along with the rest of
the resolved runtime configuration. Proxy forwarding reads an immutable
snapshot of the current filter when an upstream response reaches a filterable
boundary.

Successful responses use streaming filtering. Upstream error responses first go
through the existing bounded error-body classifier path. If the request is
terminal after retry/fallback decisions and the error body was fully retained
under `max_error_body_bytes`, the same filter policy is applied before that
error body can be forwarded to the client. This catches contamination inside
JSON error envelopes without adding live upstream probes or unbounded body
buffering.

When no effective rules are configured, successful responses use the same direct streaming path as before. When rules are configured, the response stream is filtered without buffering the complete response:

- SSE-style streams are accumulated until an event boundary (`\n\n`), then filtered as one unit. This catches matches split across chunks inside a single event.
- If no event boundary arrives, pending bytes are capped. The filter releases a prefix and keeps a small overlap window for later chunk-spanning matches.
- Non-UTF-8 chunks are passed through unchanged.

`redact` preserves the response status and headers and replaces matched content
in the body. `reject` emits a local JSON error payload in the response stream.
Because headers may already have been sent for a streaming success response,
`reject` is a content-level block rather than an HTTP status rewrite.

For body-bearing non-streaming JSON/SSE responses covered by the bounded 2xx
success guard, the peeked prefix is also checked before response body commit.
If a rejecting rule matches that prefix, the proxy records a redacted event with
`body_committed=false` and returns a local sanitized error. Required-rule misses
can trigger this pre-commit path only when the guard has a complete pass result;
cap, deadline, unsupported, compressed, and partial prefixes are not treated as
proof that the full response lacks a required marker. The plain `reject` action
stops there and only returns the local sanitized error. The explicit lifecycle
actions synthesize a high-confidence `response_filter_rejected` failure before
the response is committed, then apply the requested lifecycle mutation:

- `reject_and_expire_credential` marks the selected credential expired.
- `reject_and_cooldown_channel` cools down the selected channel.

For the current request, these pre-commit lifecycle actions return the current
response-filter error. They do not trigger transparent same-request retry,
clean-credential replay, or route-target fallback.

Streaming requests, non-replayable bodies, matches after any body bytes have
been committed, compressed/non-UTF-8 responses, and prefix misses remain bounded
content filtering only; they do not perform post-output transparent fallback.

For retained upstream error bodies, `literal` and `regex` rules can redact or
reject matched text before forwarding. `required_literal` and `required_regex`
rules are not evaluated as missing on error bodies, because an ordinary upstream
error is not expected to have the same shape as a successful model response.
Rejecting lifecycle actions can still synthesize `response_filter_rejected`
evidence before any client output, but they do not trigger transparent retry.

## Event Boundary

When response-filter event capture is enabled, the proxy writes only bounded metadata to the in-memory `response_filter_events` ring after a rule outcome is known. The event is not itself replayed into routing or lifecycle decisions. Only the explicit `reject_and_expire_credential` and `reject_and_cooldown_channel` actions can create lifecycle evidence, and only while the response is still at a bounded pre-commit boundary. `GET /management/response-filter-events` exposes:

`capacity`, `dropped_events`, and `events`. Each event contains `event_id`, `created_at_unix_seconds`, `request_id`, `channel_id`, `public_model`, `rule_id`, `action`, `content_kind`, `reason_code`, `outcome`, and `body_committed`.

The event stream never stores matched text, raw chunks, request bodies, response bodies, upstream keys, client tokens, credential ids, or absolute key paths. Ring capacity defaults to 1024 events, rejects public configuration outside 1 to 4096, and is updated by runtime reload. `dropped_events` includes capacity eviction, runtime-reload shrink, and lock-contended best-effort append drops.

`GET /management/alerts` derives a management-only `response_filter_contamination` alert when at least three filter events for the same `(channel_id, rule_id)` occur inside `response_filter.alert_window_seconds`, which defaults to 900 seconds. The alert includes channel id, rule id, redact/reject counts, reason codes, and the window size. Alerts decay when matching events age out of the window. Alerts summarize operator-visible contamination signals only; they are not routing telemetry and are not lifecycle evidence.

## Boundaries

Response filtering must not query credential storage, registry storage, upstream model catalogs, or management APIs on the request path. It must not log or persist matched untrusted text.

Response-filter events and alerts are observability only. They are not retry input, routing input, lifecycle evidence, channel-health evidence, routing telemetry, channel lifecycle input, failure-domain state, or credential-selection input. Lifecycle mutation is available only through the explicit pre-commit rejecting actions described above.

Use conservative rules. Literal and regex blacklists are suitable for known relay contamination markers. Required rules are useful for strict deployments that expect a narrow response shape, but they can reject or redact legitimate model output if configured too broadly.

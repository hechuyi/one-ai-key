# Response Filter

`response_filter` is a gateway-side safety boundary for successful upstream responses. It is intended for relay contamination cases where an upstream or intermediate service injects unwanted text into otherwise valid model output. The filter is configured in YAML, compiled at startup or runtime reload, and applied on the streaming response boundary before bytes are returned to the client.

The feature is deliberately separate from provider adapters and routing policy. Provider adapters still own protocol and auth rewriting; routing still owns retry and lifecycle state. Response filtering only inspects successful response bytes and decides whether to pass them through, redact matching content, or replace blocked content with a local error payload.

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
- `action`: `redact` by default, or `reject` for stricter fail-closed behavior.

Rule semantics:

- `literal`: blacklists a literal marker. Matching text is redacted or rejected.
- `regex`: blacklists a regular expression. Matching text is redacted or rejected.
- `required_literal`: allowlist-style rule. If the literal is absent, the rule action is applied.
- `required_regex`: allowlist-style rule. If the regex does not match, the rule action is applied.

Literal matching removes common zero-width format characters before matching, so simple visual obfuscation does not bypass a literal rule. Redaction replaces the matched original span, including ignored format characters. Regex rules run on the original text and should include their own normalization tolerance if needed.

## Runtime Behavior

The filter is stored in `AppState` as a compiled policy. `POST /management/runtime/reload` replaces the compiled policy along with the rest of the resolved runtime configuration. Proxy forwarding reads an immutable snapshot of the current filter when a successful upstream response begins.

Filtering is applied only to successful upstream responses. Error responses are still handled by the existing bounded error-body classifier path.

When no effective rules are configured, successful responses use the same direct streaming path as before. When rules are configured, the response stream is filtered without buffering the complete response:

- SSE-style streams are accumulated until an event boundary (`\n\n`), then filtered as one unit. This catches matches split across chunks inside a single event.
- If no event boundary arrives, pending bytes are capped. The filter releases a prefix and keeps a small overlap window for later chunk-spanning matches.
- Non-UTF-8 chunks are passed through unchanged.

`redact` preserves the response status and headers and replaces matched content in the body. `reject` emits a local JSON error payload in the response stream. Because headers may already have been sent for a streaming success response, `reject` is a content-level block rather than an HTTP status rewrite.

## Event Boundary

When response-filter event capture is enabled, the proxy writes only bounded metadata to the in-memory `response_filter_events` ring. `GET /management/response-filter-events` exposes:

`event_id`, `created_at_unix_seconds`, `request_id`, `channel_id`, `public_model`, `rule_id`, `action`, `content_kind`, `reason_code`, `outcome`, and `body_committed`.

The event stream never stores matched text, raw chunks, request bodies, response bodies, upstream keys, client tokens, credential ids, or absolute key paths. Ring capacity defaults to 1024 events and is updated by runtime reload.

`GET /management/alerts` derives a management-only `response_filter_contamination` alert when at least three `redact` or `reject` events for the same `(channel_id, rule_id)` occur inside `response_filter.alert_window_seconds`, which defaults to 900 seconds. The alert includes channel id, rule id, redact/reject counts, reason codes, and the window size. Alerts decay when matching events age out of the window.

## Boundaries

Response filtering must not query credential storage, registry storage, upstream model catalogs, or management APIs on the request path. It must not mutate credential lifecycle state, channel health, routing telemetry, or failure domains. It must not log or persist matched untrusted text.

Response-filter events and alerts are observability only. They are not retry input, lifecycle evidence, channel-health evidence, routing telemetry, failure-domain state, or credential-selection input.

Use conservative rules. Literal and regex blacklists are suitable for known relay contamination markers. Required rules are useful for strict deployments that expect a narrow response shape, but they can reject or redact legitimate model output if configured too broadly.

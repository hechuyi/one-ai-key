# one-ai-key

Small Rust HTTP router for managing multiple upstream API key pools.

It is not limited to OpenAI. It supports:

- OpenAI-compatible automatic pool selection via `/v1/*` and request `model`.
- Direct named pool routing via `/pools/{pool}/...` for other protocols.
- Different upstream auth headers such as `Authorization: Bearer` or `x-api-key`.
- Built-in upstream templates for common relay/provider shapes so API base,
  auth header, policy profile, credential set, channel, and model route wiring
  are not hand-copied for every site.
- Sticky-until-error key behavior: keep using the current key, cool it down on configured switchable errors, then move to the next available key.
- YAML-configurable upstream error adaptation rules for relay-specific cooldown semantics.
- Management endpoints for redacted channel/credential status, routing preview, model discovery, manual expiration, restore, disable, enable, channel health reset, and cooldown reset.
- SQLite-backed virtual key management for creating, disabling, enabling, and rescoping client-facing keys without restarting the router.
- SQLite-backed management import for adding credentials to an existing credential set without editing key files.
- Configured request/error body limits and streamed successful responses.

Architecture notes live in `docs/architecture.md`.

Local development:

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
HTTPS_PROXY=http://127.0.0.1:10808 \
HTTP_PROXY=http://127.0.0.1:10808 \
CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse \
cargo run -- --config config/local.yaml
```

## Local x86_64 Linux Release Build

The only supported local release gate for the Linux x86_64 artifact is to run
the Docker wrapper from the macOS host at the repository root:

```bash
scripts/build-release-x86_64-linux-docker.sh
```

The wrapper runs Docker with `--platform linux/amd64`, starts
`nixos/nix:latest`, mounts this repository at `/work`, mounts the named Nix
store volume `rtoc-monitor-nix-amd64:/nix`, and then runs
`scripts/build-release-x86_64-linux.sh` inside the Nix shell.

Do not build the release artifact on `rtoc-gateway` or any other remote server.
Do not use Debian or Ubuntu Rust images as the release gate. Do not upload or
commit `dist/`, runtime config, SQLite databases, keys, tokens, JSONL logs, or
`AGENTS.md`.

The build writes `dist/one-ai-key-<version>-<target>.tar.gz` and a matching
`dist/one-ai-key-<version>-<target>.tar.gz.sha256`. The sha256 sidecar must
record only the archive basename, not an absolute path or `dist/`-prefixed
path.

Troubleshooting: a slow first Nix download is expected while Docker populates
the `rtoc-monitor-nix-amd64` volume; it is not a reason to switch to a server
build. If the wrapper fails, first check that Docker is available, the named
volume is mounted, and the command is being run from the repository root.

Runtime configuration is intentionally local. Keep operator config, key files,
SQLite databases, and other mutable state in ignored paths such as
`config/local.yaml` and `data/`. Do not commit deploy or runtime secrets.

When SQLite-backed credential, registry, or client-token storage is configured,
the local SQLite stores are authoritative for their respective mutable state.
For example, once the `client_tokens` table has rows, it is authoritative for
virtual key scope; editing YAML `client_tokens.allowed_model_groups` will not
rewrite an existing stored virtual key. Use the management API to change model
or channel scope.

Upstreams can be declared through the `upstreams` shortcut in YAML. Startup
expands these templates into the normal provider, account, credential-set,
channel, policy-profile, and model-route resources before validation. The
request path only sees the expanded in-memory resources.

```yaml
upstreams:
  xiaomi_mimo_token_plan_cn:
    template: xiaomi_mimo_token_plan_cn
    credential_set: xiaomi_token_plan
    keys_file: /data/xiaomi-token-plan-key.txt
```

`models` may contain plain public model names or structured entries when a
relay needs an upstream model rewrite:

```yaml
upstreams:
  relay:
    template: openai_compatible_bearer
    api_base: https://relay.example/v1
    keys_file: /data/relay-key.txt
    models:
      - public_model: gpt-5.4-mini
        upstream_model: provider/gpt-5.4-mini
```

Example request:

```bash
curl http://localhost:4101/v1/chat/completions \
  -H 'Authorization: Bearer <client-token>' \
  -H 'Content-Type: application/json' \
  -d '{"model":"gpt-5.4-mini","messages":[{"role":"user","content":"ok"}]}'
```

For non-OpenAI-compatible upstreams, target a named pool:

```bash
curl http://localhost:4101/pools/anthropic/v1/messages ...
```

Client configuration:

```text
Base URL: http://localhost:4101/v1
API Key: <client-token>
Model: gpt-5.4-mini or mimo-v2.5-pro
```

Update a client virtual key's model scope without restarting:

```bash
curl -sS http://localhost:4101/management/client-tokens/{token_id} \
  -X PATCH \
  -H 'Authorization: Bearer <admin-token>' \
  -H 'Content-Type: application/json' \
  -d '{"allowed_model_groups":["gpt-5.4-mini","mimo-v2.5-pro"]}' | jq
```

`allowed_model_groups` and `allowed_channels` are partial-update fields. Omit a
field to keep its current value. Pass an empty array deliberately to make that
dimension unrestricted.

Discover models exposed by one OpenAI-compatible upstream channel without
changing routing or persistent config:

```bash
curl -sS http://localhost:4101/management/channels/ai2_hhhl/model-discovery \
  -X POST \
  -H 'Authorization: Bearer <admin-token>' | jq
```

The discovery response is redacted and read-only. Use it to decide which
`model_routes` and client-token scopes to manage; it does not auto-add routes.
If an upstream returns a 2xx response that is not an OpenAI-style model catalog,
the response keeps `models` empty and reports a structured
`catalog_unsupported_or_malformed` error instead of guessing model names from
free-form provider output.

Preview how discovered models would map into explicit model routes without
writing registry state:

```bash
curl -sS http://localhost:4101/management/model-discovery/sync-plan \
  -X POST \
  -H 'Authorization: Bearer <admin-token>' \
  -H 'Content-Type: application/json' \
  -d '{"channel_ids":["ai2_hhhl"]}' | jq
```

The sync plan returns `would_create_route`, `would_add_target`, or `unchanged`
actions for each discovered model/channel pair. It is intentionally read-only:
use `sync-apply` when you want discovery to stage route changes in the writable
registry store.

Apply discovered route changes to the staged registry without hot-reloading
runtime:

```bash
curl -sS http://localhost:4101/management/model-discovery/sync-apply \
  -X POST \
  -H 'Authorization: Bearer <admin-token>' \
  -H 'Content-Type: application/json' \
  -d '{"channel_ids":["ai2_hhhl"]}' | jq
```

`sync-apply` creates missing routes and appends missing channel targets in one
validated registry transaction. It does not update virtual-key scope and does
not affect client traffic until `POST /management/runtime/reload` or a process
restart applies the staged registry.

Credential rotation runbook:

If local configuration enables a writable SQLite credential store, replacement
keys can be added through the management API without editing the original key
files or restarting the router.

Check whether operator input is needed:

```bash
curl -sS http://localhost:4101/management/alerts \
  -H 'Authorization: Bearer <admin-token>' | jq
```

In a local configuration where a credential set has only one credential,
`/ready` can still report `ready` while `/management/alerts` reports a
`credential_set_transition_required` warning. That means the router is serving,
but the upstream has no spare validated key. Import at least one replacement
credential per affected set to clear the operator-input warning and make
failover useful.

Inspect one credential set's operational state:

```bash
curl -sS http://localhost:4101/management/credential-sets/ai2_test/operations \
  -H 'Authorization: Bearer <admin-token>' | jq
```

List redacted credentials in a credential set:

```bash
curl -sS 'http://localhost:4101/management/credential-sets/ai2_test/credentials?state=all' \
  -H 'Authorization: Bearer <admin-token>' | jq
```

Import a replacement AI2 relay key:

```bash
curl -sS http://localhost:4101/management/credential-sets/ai2_test/credentials/import \
  -H 'Authorization: Bearer <admin-token>' \
  -H 'Content-Type: application/json' \
  -d '{"keys":["<upstream-api-key>"],"batch_id":"ai2-rotation-001"}' | jq
```

Import a replacement Xiaomi Token Plan key:

```bash
curl -sS http://localhost:4101/management/credential-sets/xiaomi_token_plan/credentials/import \
  -H 'Authorization: Bearer <admin-token>' \
  -H 'Content-Type: application/json' \
  -d '{"keys":["<upstream-api-key>"],"batch_id":"xiaomi-rotation-001"}' | jq
```

Manually expire a known bad credential after copying its redacted `credential_id`
from the list endpoint:

```bash
curl -sS http://localhost:4101/management/credential-sets/ai2_test/credentials/{credential_id}/expire \
  -H 'Authorization: Bearer <admin-token>' \
  -H 'Content-Type: application/json' \
  -d '{"reason":"upstream key rotated"}' | jq
```

The import response includes an `operations` summary. If it reports
`accepting_requests: true`, existing valid credentials still serve traffic while
the pool is degraded. If it reports `accepting_requests: false`, the credential
set is exhausted and the client API should be treated as stopped until valid
credentials are imported or restored.

# Operations And Troubleshooting

This document covers the runtime and deployment boundaries that operators should
keep stable for a small one-ai-key gateway. It intentionally avoids raw secrets,
request bodies, response bodies, and upstream-specific private data.

## Deployment Boundary

The gateway deployment host consumes a published GitHub Release tarball. On
NixOS, pin the release asset URL and its `sha256` in the host configuration, then
unpack and run the pinned binary. Do not build this repository on the VPS, do
not run Cargo there, and do not use the deployment host as an ad hoc Nix builder.

The supported build path remains local development host to release artifact:

```text
local Docker/Nix x86_64 build -> GitHub Release tarball + sha256 -> deployment host pin
```

A public client base URL such as `https://gateway.example/v1` may point at the
gateway, but the public URL is not a secret. Client tokens, management tokens,
and upstream credentials remain secret material and should never appear in Nix
expressions, Git history, issue text, chat logs, or troubleshooting snippets.

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
them as sensitive because they can reveal deployment topology or incident
context. Some deployments keep JSONL events under `data`, for example
`/opt/one-ai-key/data/events.jsonl`, so treat configured event paths as mutable
runtime state even when they are not under a dedicated `logs` directory.

Do not commit SQLite databases, JSONL event streams, token files, upstream key
files, local YAML containing secrets, generated tarballs, or checksum sidecars.

## Safe Diagnostics

Troubleshooting should use redacted, structural evidence: HTTP status, stable
reason codes, public model ids, route names, credential lifecycle state, reload
state, release version, and checksum identity. Never paste raw client tokens,
management tokens, upstream keys, complete request bodies, complete response
bodies, or provider payloads. Also do not copy advertising, promotion, invite,
mirror-site, or unrelated marketing text from logs, web pages, OCR, screenshots,
or error output into project files or incident notes.

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
configuration on the host, but never paste the token value into chat, docs,
tickets, shell history, or Git.

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

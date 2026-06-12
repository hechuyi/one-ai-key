# Release Build Contract

This project has one supported Linux x86_64 release build path.

From the repository root on the development host, run:

```bash
scripts/build-release-x86_64-linux-docker.sh
```

For full local verification through the same Docker/Nix x86_64 environment,
run:

```bash
scripts/local-ci-docker.sh
```

This repository has no `Dockerfile`. The supported container path is the wrapper
script above: it starts Docker with `--platform linux/amd64`, uses the
`nixos/nix:latest` image, mounts this repository at `/work`, mounts the
configured persistent Nix store volume at `/nix`, and then runs
`scripts/build-release-x86_64-linux.sh` inside that x86_64 Linux Nix
environment. The default volume is `one-ai-key-nix-amd64`; set
`ONE_AI_KEY_NIX_STORE_VOLUME` when a development host needs a different local
volume name. It also mounts a Nix user cache volume at `/root/.cache/nix`, so
Nix tarball and eval caches survive repeated Docker/Nix runs. The wrapper
mounts a separate Cargo target volume at `/cargo-target` and exports
`CARGO_TARGET_DIR=/cargo-target`, so release builds do not create
repository-local `target/` directories. It also mounts a Cargo home cache volume
at `/cargo-home` and exports `CARGO_HOME=/cargo-home`, so Cargo registry and Git
dependency caches survive repeated Docker/Nix runs. The four fixed default
reusable Docker volumes are `one-ai-key-nix-amd64`,
`one-ai-key-nix-cache-amd64`, `one-ai-key-cargo-target-amd64`, and
`one-ai-key-cargo-home-amd64`; they are build caches, not release artifacts, and
not routine cleanup targets. Repository-local `target/` is still ordinary build
output and may be deleted when needed.

`scripts/local-ci-docker.sh` uses the same four persistent volumes and then runs
`scripts/local-ci.sh` inside the container. Use it when the host toolchain is not
the intended x86_64 Linux Nix environment or when a developer wants CI output
without creating repository-local build artifacts.

`scripts/local-ci.sh` is the source-tree verification gate. It rejects
repository-local `target/`, runs staged denylist self-tests and public-plan
hygiene checks, runs `git diff --check`, then runs `cargo fmt -- --check`,
`cargo check --locked`, `cargo clippy --locked -- -D warnings`, and
`cargo test --locked`.

`scripts/build-release-x86_64-linux.sh` is the container entrypoint, not the
host entrypoint. It rejects non-Linux hosts, non-`x86_64` machines, and
environments without Nix, Cargo, or rustc from the active Nix shell. The default
target is `x86_64-unknown-linux-gnu`.

The container entrypoint builds the locked Cargo package, stages only the release
binary, and writes a deterministic tar/gzip archive. Tar entries are sorted by
name, owner and group are fixed to `0`, mtimes use `SOURCE_DATE_EPOCH` (default
`0`), and gzip runs with `-n`. The `.sha256` sidecar is generated from inside
`dist/`, so it contains only the archive basename. The local build also writes a
`*.build.json` metadata sidecar with the tracked source-tree fingerprint used by
`scripts/release-smoke.sh` to reject stale `dist/` artifacts. This metadata file
is a local verification guard, not a GitHub Release asset.

Do not build release artifacts on deployment hosts, random Linux shells,
Debian/Ubuntu Rust images, or ad hoc remote builders. Deployment hosts consume
published release artifacts; they do not compile them.

Gateway and NixOS deployments must pin only the published GitHub Release
tarball URL and its `sha256`. The gateway host must not pin a branch, local
checkout, moving archive URL, Docker image, or source build. The release asset is
the deployment contract.

## Artifact Contract

The published artifact set for a release is exactly the Linux x86_64 tarball and
its `.sha256` sidecar:

```text
one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz
one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz.sha256
```

The local build writes the same files under `dist/`:

```text
dist/one-ai-key-<version>-<target>.tar.gz
dist/one-ai-key-<version>-<target>.tar.gz.sha256
dist/one-ai-key-<version>-<target>.tar.gz.build.json
```

The SHA256 sidecar must contain only the archive basename:

```text
<sha256>  one-ai-key-<version>-<target>.tar.gz
```

It must not contain an absolute path or a `dist/`-prefixed path.

The tarball contains the release binary as the runnable contract. It does not
contain deployment config, client tokens, management tokens, upstream keys,
SQLite state, JSONL event streams, local logs, Nix caches, Cargo target
directories, private scripts, local build metadata, or operator notes.

A consumer verifies the uploaded sidecar, unpacks the tarball, pins the exact
asset URL and hash in deployment configuration, and runs the binary with local
config and secret files supplied by that deployment. A consumer must not infer
release identity from a branch name, a moving archive URL, a local checkout, a
Docker image tag, or a locally compiled binary.

## Publish Contract

Before publishing a GitHub release, run:

```bash
scripts/local-ci.sh
scripts/build-release-x86_64-linux-docker.sh
scripts/release-smoke.sh
git status -sb --untracked-files=all
```

`scripts/release-smoke.sh` must first verify the archive checksum and local
`*.build.json` source-tree fingerprint, then run the extracted artifact in a
tempdir with generated placeholder tokens and a local mock upstream. It must
cover offline config generation/checking, authenticated `/v1/models`, one
model-bearing request, redacted operator reports for `doctor`, `models`,
`route`, `keys`, `failures`, and `reload`, and rejection of a client `/v1` URL
used as a management URL. The smoke exercises the canonical first diagnosis command,
`models explain --model <public-model-id> --client-token-ref <client-token ref>
--endpoint-family chat_completions`, and verifies that it answers whether the
client-token ref can use the model on that endpoint family with `can_use`,
`blocking_domain`, `endpoint_family`, bounded evidence, and a safe next_action. It
must also exercise the Stage 3 model publication workflow: `models onboard-plan
--dry-run`, `models onboard-plan --apply --dry-run`, confirmed `models
onboard-plan --apply --expected-staged-registry-version <version> --yes`,
`reload diff`, confirmed `reload apply --expected-staged-registry-version
<version> --yes`, `models explain`, authenticated `/v1/models`, and one local
mock completion through the newly published public model id.

Release smoke is local, redacted, and operator-run. It must prove that
`/v1/models` is served from the compiled local public catalog and that the
publication workflow does not call upstream live `/v1/models`. It must not use
`cargo run`, a source checkout binary, real upstream credentials, private
deployment URLs, raw tokens, or a deployment host.

## Release Checklist

1. Bump the package version in `Cargo.toml`.
2. Refresh `Cargo.lock` with the locked package version that will be released.
3. Run `scripts/local-ci.sh` from the repository root.
4. Run `scripts/build-release-x86_64-linux-docker.sh` from the repository root.
   This is the local Docker/Nix x86_64 build path; do not use a repository
   `Dockerfile`, because the repository does not provide one.
5. Verify that `dist/one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz`, its
   `.sha256` sidecar, and the local `.build.json` sidecar exist, and that the
   checksum sidecar contains only the archive basename.
6. Run `scripts/release-smoke.sh`; it must exercise the extracted artifact with
   local placeholder tokens and a local mock upstream, including the explicit
   model publication path from `models onboard-plan --dry-run` through confirmed
   staged registry apply, `reload diff`, confirmed runtime reload, authenticated
   `/v1/models`, and one local mock completion.
7. Run `scripts/check-staged-denylist.sh`, then create the release commit and
   tag after checking that runtime state and generated artifacts are not staged.
8. Upload only the tarball and `.sha256` sidecar as GitHub Release assets.
9. Download the uploaded tarball and `.sha256` sidecar into a tempdir and verify
   the checksum from the uploaded sidecar. Confirm tag, Cargo version, asset
   filename, checksum filename, and release notes version match.
10. Record the local release verification result and the published asset
    verification result. If a deployment host has not been intentionally updated
    by the operator, record `deployment_pin_smoke: not_run_by_design`.
11. When an operator separately updates a gateway or NixOS deployment, pin the
    GitHub Release tarball URL and exact `sha256`; do not build on the host.
12. Optional production smoke belongs to that operator-run deployment action,
    not to `scripts/local-ci.sh`: process liveness, authenticated management
    health, `/v1/models`, and one harmless client completion through the public
    base URL. Do not store private server URLs, raw tokens, upstream keys, or
    default remote targets in local CI or release scripts. When using the
    repository harness, provide `ONE_AI_KEY_PUBLIC_BASE_URL`,
    `ONE_AI_KEY_MANAGEMENT_URL`, `ONE_AI_KEY_CLIENT_TOKEN_ENV`,
    `ONE_AI_KEY_MANAGEMENT_TOKEN_ENV`, and `ONE_AI_KEY_PUBLIC_MODEL`; optionally
    provide short non-secret `ONE_AI_KEY_RELEASE_IDENTITY` and
    `ONE_AI_KEY_CHECKSUM_IDENTITY` values for traceability, then run
    `scripts/production-smoke.sh --allow-production`; the script writes only
    redacted JSON and is not a release or CI gate.
13. Deployment records must contain only redacted status, reason codes, route
    names, model ids, release version, asset URL, and checksum. Do not record
    raw tokens, upstream keys, request bodies, or response bodies.

The release commit or tag must not include `dist/`, runtime config, SQLite
databases, key files, token files, JSONL logs, private agent files,
`AGENTS.md`, `target/`, or local deployment state.

The staged-path denylist is executable evidence. Human inspection of
`git diff --cached --name-only` is only accepted after
`scripts/check-staged-denylist.sh` exits 0. The gate rejects repository-local
build output, runtime state, local config, database/log/key/token material,
private scripts, `key-pool-router/`, and `AGENTS.md`. It also scans newly
staged text for internal execution traces, long-form secret material, and
promotional/injection residue. Product roadmaps, RFCs, ADRs, release
verification records, architecture notes, and operator documentation may be committed when they are
written as durable user or contributor documentation. Private coordination
notes, one-off support records, deployment-local facts, raw traces, and
assistant/process transcripts stay local and must be reduced into a public
contract before they enter the repository.

Local `config/`, `data/`, `db/`, `logs/`, `dist/`, `target/`, and deployment
state directories are operational or build outputs. Keep them ignored and out of
Git staging; release publication is the uploaded artifact pair, not repository
storage of generated files.

The first run can be slow while Docker populates the persistent Nix store volume.
That is cache warm-up, not a reason to switch build paths. If a build path is in
question, inspect the actual wrapper configuration instead of relying on the
container, volume, or image name.

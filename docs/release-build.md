# Release Build Contract

This project has one supported Linux x86_64 release build path.

From the repository root on the development host, run:

```bash
scripts/build-release-x86_64-linux-docker.sh
```

This repository has no `Dockerfile`. The supported container path is the wrapper
script above: it starts Docker with `--platform linux/amd64`, uses the
`nixos/nix:latest` image, mounts this repository at `/work`, mounts the
configured persistent Nix store volume at `/nix`, and then runs
`scripts/build-release-x86_64-linux.sh` inside that x86_64 Linux Nix
environment. The default volume is `one-ai-key-nix-amd64`; set
`ONE_AI_KEY_NIX_STORE_VOLUME` when a development host needs a different local
volume name. The wrapper also mounts a separate Cargo target volume at
`/cargo-target` and exports `CARGO_TARGET_DIR=/cargo-target`, so release builds
do not create repository-local `target/` directories.

`scripts/build-release-x86_64-linux.sh` is the container entrypoint, not the
host entrypoint. It rejects non-Linux hosts, non-`x86_64` machines, and
environments without Nix, Cargo, or rustc from the active Nix shell. The default
target is `x86_64-unknown-linux-gnu`.

The container entrypoint builds the locked Cargo package, stages only the release
binary, and writes a deterministic tar/gzip archive. Tar entries are sorted by
name, owner and group are fixed to `0`, mtimes use `SOURCE_DATE_EPOCH` (default
`0`), and gzip runs with `-n`. The `.sha256` sidecar is generated from inside
`dist/`, so it contains only the archive basename.

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
```

The SHA256 sidecar must contain only the archive basename:

```text
<sha256>  one-ai-key-<version>-<target>.tar.gz
```

It must not contain an absolute path or a `dist/`-prefixed path.

The tarball contains the release binary as the runnable contract. It does not
contain deployment config, client tokens, management tokens, upstream keys,
SQLite state, JSONL event streams, local logs, Nix caches, Cargo target
directories, private scripts, or operator notes.

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

`scripts/release-smoke.sh` must run the extracted artifact in a tempdir with
generated placeholder tokens and a local mock upstream. It must cover offline
config generation/checking, authenticated `/v1/models`, one model-bearing
request, redacted operator reports for `doctor`, `models`, `route`, `keys`,
`failures`, and `reload`, and rejection of a client `/v1` URL used as a
management URL. The smoke exercises the canonical first diagnosis command,
`models explain --model <public-model-id> --client-token-ref <client-token ref>
--endpoint-family chat_completions`, and verifies that it answers whether the
client-token ref can use the model on that endpoint family with `can_use`,
`blocking_domain`, `endpoint_family`, bounded evidence, and a safe next_action. It
must not use `cargo run`, a source checkout binary, real upstream
credentials, or a deployment host.

## Release Checklist

1. Bump the package version in `Cargo.toml`.
2. Refresh `Cargo.lock` with the locked package version that will be released.
3. Run `scripts/local-ci.sh` from the repository root.
4. Run `scripts/build-release-x86_64-linux-docker.sh` from the repository root.
   This is the local Docker/Nix x86_64 build path; do not use a repository
   `Dockerfile`, because the repository does not provide one.
5. Verify that `dist/one-ai-key-<version>-x86_64-unknown-linux-gnu.tar.gz` and
   its `.sha256` sidecar exist, and that the sidecar contains only the archive
   basename.
6. Run `scripts/release-smoke.sh`; it must exercise the extracted artifact with
   local placeholder tokens and a local mock upstream. Release and deployment
   smoke remains local, redacted, and operator-run.
7. Run `scripts/check-staged-denylist.sh`, then create the release commit and
   tag after checking that runtime state and generated artifacts are not staged.
8. Upload the tarball and `.sha256` sidecar as GitHub Release assets.
9. Download the uploaded tarball and `.sha256` sidecar into a tempdir and verify
   the checksum from the uploaded sidecar. Confirm tag, Cargo version, asset
   filename, checksum filename, and release notes version match.
10. Record the local release-ready stop node and the published asset
    verification result. If a deployment host has not been intentionally updated
    by the operator, record `deployment_pin_smoke: not_run_by_design`.
11. When an operator separately updates a gateway or NixOS deployment, pin the
    GitHub Release tarball URL and exact `sha256`; do not build on the host.
12. Optional deployment smoke belongs to that operator-run deployment action:
    process liveness, authenticated management health, `/v1/models`, and one
    harmless client completion through the public base URL.
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
private scripts, `key-pool-router/`, and `AGENTS.md`.

Local `config/`, `data/`, `db/`, `logs/`, `dist/`, `target/`, and deployment
state directories are operational or build outputs. Keep them ignored and out of
Git staging; release publication is the uploaded artifact pair, not repository
storage of generated files.

The first run can be slow while Docker populates the persistent Nix store volume.
That is cache warm-up, not a reason to switch build paths. If a build path is in
question, inspect the actual wrapper configuration instead of relying on the
container, volume, or image name.

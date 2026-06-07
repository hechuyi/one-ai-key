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

The release artifact is written under `dist/`:

```text
dist/one-ai-key-<version>-<target>.tar.gz
dist/one-ai-key-<version>-<target>.tar.gz.sha256
```

The SHA256 sidecar must contain only the archive basename:

```text
<sha256>  one-ai-key-<version>-<target>.tar.gz
```

It must not contain an absolute path or a `dist/`-prefixed path.

Before publishing a GitHub release, run:

```bash
scripts/local-ci.sh
scripts/build-release-x86_64-linux-docker.sh
git status -sb --untracked-files=all
```

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
6. Create the release commit and tag after checking that runtime state and
   generated artifacts are not staged.
7. Upload the tarball and `.sha256` sidecar as GitHub Release assets.
8. On the deployment host, update the gateway or NixOS pin to the GitHub Release
   tarball URL and exact `sha256`; do not build on the host.
9. Run a release smoke against the deployed gateway: process liveness,
   authenticated management health, `/v1/models`, and one harmless client
   completion through the public base URL.
10. Record the release version, asset URL, checksum, deployment host pin, smoke
    status, and any redacted reason codes. Do not record raw tokens, upstream
    keys, request bodies, or response bodies.

The release commit or tag must not include `dist/`, runtime config, SQLite
databases, key files, token files, JSONL logs, private agent files,
`AGENTS.md`, `target/`, or local deployment state.

The first run can be slow while Docker populates the persistent Nix store volume.
That is cache warm-up, not a reason to switch build paths. If a build path is in
question, inspect the actual wrapper configuration instead of relying on the
container, volume, or image name.

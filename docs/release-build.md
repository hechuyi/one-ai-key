# Release Build Contract

This project has one supported Linux x86_64 release build path.

From the repository root on the development host, run:

```bash
scripts/build-release-x86_64-linux-docker.sh
```

That wrapper is the host entrypoint. It starts Docker with `--platform
linux/amd64`, uses the `nixos/nix:latest` image, mounts this repository at
`/work`, mounts the configured persistent Nix store volume at `/nix`, and then
runs `scripts/build-release-x86_64-linux.sh` inside that x86_64 Linux Nix
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

The release commit or tag must not include `dist/`, runtime config, SQLite
databases, key files, token files, JSONL logs, private agent files,
`AGENTS.md`, `target/`, or local deployment state.

The first run can be slow while Docker populates the persistent Nix store volume.
That is cache warm-up, not a reason to switch build paths. If a build path is in
question, inspect the actual wrapper configuration instead of relying on the
container, volume, or image name.

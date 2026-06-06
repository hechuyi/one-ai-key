use std::fs;
use std::path::Path;
use std::process::Command;

fn read_repo_file(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

fn parse_cargo_pkgid(package_id: &str) -> (&str, &str) {
    let package_fragment = package_id
        .trim()
        .rsplit_once('#')
        .map_or(package_id.trim(), |(_, package_fragment)| package_fragment);
    package_fragment
        .rsplit_once('@')
        .unwrap_or_else(|| panic!("cargo pkgid output must end with @version: {package_id}"))
}

#[cfg(unix)]
fn assert_executable(path: &str) {
    use std::os::unix::fs::PermissionsExt;

    let metadata =
        fs::metadata(path).unwrap_or_else(|error| panic!("failed to stat {path}: {error}"));
    assert!(
        metadata.permissions().mode() & 0o111 != 0,
        "{path} must be executable on Unix"
    );
}

#[test]
fn local_ci_script_exists_is_executable_and_runs_required_cargo_commands_in_order() {
    let path = "scripts/local-ci.sh";
    assert!(Path::new(path).is_file(), "{path} must exist");
    assert_executable(path);

    let script = read_repo_file(path);
    let required_commands = [
        "cargo fmt -- --check",
        "cargo check --locked",
        "cargo clippy --locked -- -D warnings",
        "cargo test --locked",
    ];

    let mut previous_position = 0;
    for command in required_commands {
        let relative_position = script[previous_position..]
            .find(command)
            .unwrap_or_else(|| panic!("{path} must contain `{command}`"));
        previous_position += relative_position + command.len();
    }
}

#[test]
fn release_script_is_local_x86_64_linux_nix_command_gnu_packaging_contract() {
    let path = "scripts/build-release-x86_64-linux.sh";
    assert!(Path::new(path).is_file(), "{path} must exist");
    assert_executable(path);

    let script = read_repo_file(path);
    assert!(
        script.contains("uname -s"),
        "{path} must inspect the local OS"
    );
    assert!(
        script.contains("Linux"),
        "{path} must require local Linux execution"
    );
    assert!(
        script.contains("uname -m"),
        "{path} must inspect the local architecture"
    );
    assert!(
        script.contains("x86_64"),
        "{path} must require x86_64 execution"
    );
    assert!(
        script.contains("nix --version"),
        "{path} must require the Nix command to be available in the container"
    );
    assert!(
        script.contains("cargo --version"),
        "{path} must require cargo to be available in the active toolchain"
    );
    assert!(
        script.contains("rustc --version"),
        "{path} must require rustc to be available in the active toolchain"
    );
    assert!(
        script.contains("gzip --version"),
        "{path} must require gzip for deterministic compression"
    );
    assert!(
        !script.contains("/etc/os-release") && !script.to_ascii_lowercase().contains("nixos"),
        "{path} must not require /etc/os-release to identify as NixOS"
    );
    assert!(
        script.contains("x86_64-unknown-linux-gnu"),
        "{path} must default to the x86_64-unknown-linux-gnu target"
    );
    assert!(
        !script.contains("x86_64-unknown-linux-musl"),
        "{path} must not default to the musl target because plain nixpkgs rustc does not include musl std"
    );
    assert!(
        script.contains("cargo build --release --locked --target"),
        "{path} must perform a locked release build for the selected target"
    );
    assert!(
        script.contains("dist"),
        "{path} must write release output under dist/"
    );
    assert!(
        script.contains(".tar.gz"),
        "{path} must produce a tar.gz archive"
    );
    assert!(
        script.contains(".sha256"),
        "{path} must produce a sha256 checksum file"
    );
    assert!(
        script.contains("sha256"),
        "{path} must generate a SHA-256 checksum"
    );
    assert!(
        script.contains("SOURCE_DATE_EPOCH"),
        "{path} must expose SOURCE_DATE_EPOCH for deterministic archive metadata"
    );
    for required in [
        "--sort=name",
        "--mtime=\"@${SOURCE_DATE_EPOCH}\"",
        "--owner=0",
        "--group=0",
        "--numeric-owner",
        "gzip -n -9",
    ] {
        assert!(
            script.contains(required),
            "{path} must include deterministic archive option `{required}`"
        );
    }
    assert!(
        !script.contains(r#"sha256sum "${ARCHIVE_PATH}" > "${ARCHIVE_PATH}.sha256""#),
        "{path} must not checksum ARCHIVE_PATH directly because that records an absolute path in the sidecar"
    );
    assert!(
        script.contains(
            r#"(cd "${DIST_DIR}" && sha256sum "${ARCHIVE_NAME}" > "${ARCHIVE_NAME}.sha256")"#
        ),
        "{path} must generate the checksum from inside dist/ so the sidecar records only the archive basename"
    );

    for forbidden in ["ssh", "scp", "rsync", "gateway"] {
        assert!(
            !script.to_ascii_lowercase().contains(forbidden),
            "{path} must not contain remote operation token `{forbidden}`"
        );
    }
}

#[test]
fn docker_release_wrapper_runs_local_amd64_nix_container_with_cached_nix_store() {
    let path = "scripts/build-release-x86_64-linux-docker.sh";
    assert!(Path::new(path).is_file(), "{path} must exist");
    assert_executable(path);

    let script = read_repo_file(path);
    assert!(
        script.contains("docker run") && script.contains("--rm"),
        "{path} must run an ephemeral local Docker container"
    );
    assert!(
        script.contains("--platform linux/amd64"),
        "{path} must force the x86_64 Linux Docker platform"
    );
    assert!(
        script.contains("nixos/nix:latest"),
        "{path} must use the official Nix Docker image"
    );
    assert!(
        script.contains(r#""${REPO_ROOT}:/work""#) || script.contains(r#""${REPO_ROOT}:/work:"#),
        "{path} must mount the repository at /work"
    );
    assert!(
        script.contains("NIX_STORE_VOLUME=${ONE_AI_KEY_NIX_STORE_VOLUME:-one-ai-key-nix-amd64}")
            && script.contains(r#""${NIX_STORE_VOLUME}:/nix""#),
        "{path} must mount the configurable named Nix cache volume at /nix"
    );
    assert!(
        script.contains(
            "CARGO_TARGET_VOLUME=${ONE_AI_KEY_CARGO_TARGET_VOLUME:-one-ai-key-cargo-target-amd64}"
        ) && script.contains(r#""${CARGO_TARGET_VOLUME}:/cargo-target""#)
            && script.contains("-e CARGO_TARGET_DIR=/cargo-target"),
        "{path} must keep release Cargo build output outside the repository"
    );
    assert!(
        script.contains("nix-command flakes"),
        "{path} must enable nix-command and flakes"
    );
    for package in ["cargo", "rustc", "gcc", "pkg-config", "openssl"] {
        assert!(
            script.contains(package),
            "{path} must make `{package}` available in the Nix shell"
        );
    }
    assert!(
        !script.contains("nixpkgs#musl"),
        "{path} must not imply nixpkgs rustc can directly build the musl target"
    );
    assert!(
        script.contains("/work/scripts/build-release-x86_64-linux.sh"),
        "{path} must call the in-container release script"
    );
    for forbidden in ["ssh", "scp", "rsync", "remote", "token", "secret"] {
        assert!(
            !script.to_ascii_lowercase().contains(forbidden),
            "{path} must not contain remote operation or credential token `{forbidden}`"
        );
    }
}

#[test]
fn release_script_uses_cargo_pkgid_instead_of_greedy_metadata_json_sed_for_package_metadata() {
    let path = "scripts/build-release-x86_64-linux.sh";
    let script = read_repo_file(path);

    assert!(
        script.contains("cargo pkgid --locked"),
        "{path} must derive package metadata from cargo pkgid, not arbitrary cargo metadata JSON"
    );
    for key in ["name", "version"] {
        let greedy_metadata_pattern = format!(r#"s/.*\"{key}\":"#);
        assert!(
            !script.contains(&greedy_metadata_pattern),
            "{path} must not use greedy sed extraction over cargo metadata JSON for `{key}`"
        );
    }
}

#[test]
fn cargo_pkgid_package_name_source_resolves_to_one_ai_key_without_running_release_script() {
    let output = Command::new("cargo")
        .args(["pkgid", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap_or_else(|error| panic!("failed to run cargo pkgid --locked: {error}"));
    assert!(
        output.status.success(),
        "cargo pkgid --locked failed with status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let package_id = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("cargo pkgid output must be UTF-8: {error}"));
    let (package_name, package_version) = parse_cargo_pkgid(&package_id);

    assert_eq!(package_name, "one-ai-key");
    assert_eq!(package_version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn gitignore_ignores_local_dist_release_output() {
    let gitignore = read_repo_file(".gitignore");
    assert!(
        gitignore.lines().any(|line| line.trim() == "/dist/"),
        ".gitignore must ignore /dist/ release output"
    );
}

#[test]
fn gitignore_and_dockerignore_cover_sensitive_runtime_and_release_patterns() {
    let files = [
        (".gitignore", read_repo_file(".gitignore")),
        (".dockerignore", read_repo_file(".dockerignore")),
    ];
    let required_patterns = [
        ".env*",
        "config/*.yaml",
        "config/*.yml",
        "data/",
        "*.keys",
        "*.sqlite*",
        "*.sqlite3*",
        "*.db*",
        "*.wal",
        "*.shm",
        "*.log*",
        "dist/",
        "snapshots/",
        "golden/",
        "tests/snapshots/",
        "tests/golden/",
    ];

    for (path, contents) in files {
        let normalized = contents
            .lines()
            .map(|line| line.trim().trim_start_matches('/'))
            .collect::<Vec<_>>();
        for pattern in required_patterns {
            assert!(
                normalized.iter().any(|line| *line == pattern),
                "{path} must exclude sensitive runtime/release pattern `{pattern}`"
            );
        }
    }
}

#[test]
fn release_smoke_script_exists_is_executable_and_uses_released_binary() {
    let path = "scripts/release-smoke.sh";
    assert!(Path::new(path).is_file(), "{path} must exist");
    assert_executable(path);

    let script = read_repo_file(path);
    for required in [
        "tar -xzf",
        r#"BIN="${WORK_DIR}/${PACKAGE_NAME}""#,
        r#""${BIN}" --help"#,
        "init local --out config/local.yaml --keys data/relay.keys --dry-run --output json",
        "init local --out config/local.yaml --keys data/relay.keys --yes --output json",
        "--config config/local.yaml check-config --output json",
    ] {
        assert!(
            script.contains(required),
            "{path} must run release-smoke step `{required}` against the extracted binary"
        );
    }
    assert!(
        !script.contains("cargo run"),
        "{path} must not smoke test through cargo run"
    );
}

#[test]
fn release_smoke_script_covers_local_mock_data_plane_and_operator_commands() {
    let path = "scripts/release-smoke.sh";
    let script = read_repo_file(path);

    for required in [
        "ThreadingHTTPServer",
        "/v1/models",
        "/v1/chat/completions",
        "release smoke ok",
        "model_visibility_preview",
        "CLIENT_MODELS",
        "[$forbidden[] | select(. as $term | $body | contains($term))]",
        "doctor --output json",
        "client-tokens list --output json",
        "models list --client-token-ref local-client --output json",
        "models explain --model gpt-example --client-token-ref local-client --output json",
        "route explain gpt-example --client-token-ref local-client --output json",
        "keys stats --credential-set relay_credentials --output json",
        "failures tail --last 20 --output json",
        "reload status --output json",
        "reload diff --output json",
        "reload apply --dry-run --output json",
        "client_base_url_used_for_management",
    ] {
        assert!(
            script.contains(required),
            "{path} must cover release gate token `{required}`"
        );
    }
}

#[test]
fn release_smoke_script_is_local_redacted_and_cleans_processes() {
    let path = "scripts/release-smoke.sh";
    let script = read_repo_file(path);

    for required in [
        "docker run --rm",
        "--platform linux/amd64",
        "nixos/nix:latest",
        "mktemp -d",
        "trap cleanup EXIT",
        "kill \"${SERVICE_PID}\"",
        "kill \"${MOCK_PID}\"",
        "unset KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE",
        "unset KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE",
    ] {
        assert!(
            script.contains(required),
            "{path} must include local smoke safeguard `{required}`"
        );
    }

    for forbidden in ["ssh", "scp", "rsync", "gateway", "sk-"] {
        assert!(
            !script.to_ascii_lowercase().contains(forbidden),
            "{path} must not contain remote operation or secret-like token `{forbidden}`"
        );
    }
}

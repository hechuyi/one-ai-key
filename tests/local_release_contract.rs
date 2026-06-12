use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn read_repo_file(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|error| panic!("system clock before UNIX_EPOCH: {error}"))
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "one-ai-key-{prefix}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&path)
        .unwrap_or_else(|error| panic!("failed to create temp dir {}: {error}", path.display()));
    path
}

fn write_temp_file(root: &Path, relative_path: &str, contents: &str) {
    let path = root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|error| {
            panic!("failed to create temp parent {}: {error}", parent.display())
        });
    }
    fs::write(&path, contents)
        .unwrap_or_else(|error| panic!("failed to write temp file {}: {error}", path.display()));
}

fn parse_cargo_pkgid_version(package_id: &str) -> &str {
    let package_fragment = package_id
        .trim()
        .rsplit_once('#')
        .map_or(package_id.trim(), |(_, package_fragment)| package_fragment);
    package_fragment
        .rsplit_once('@')
        .map_or(package_fragment, |(_, version)| version)
}

fn cargo_workspace_root_package(metadata: &str) -> (String, String) {
    let value: serde_json::Value = serde_json::from_str(metadata)
        .unwrap_or_else(|error| panic!("cargo metadata output must be JSON: {error}"));
    let root = value["workspace_members"]
        .as_array()
        .and_then(|members| members.first())
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("cargo metadata must report a workspace root member"));
    let package = value["packages"]
        .as_array()
        .and_then(|packages| {
            packages
                .iter()
                .find(|package| package["id"].as_str() == Some(root))
        })
        .unwrap_or_else(|| panic!("cargo metadata must include the workspace root package"));
    let name = package["name"]
        .as_str()
        .unwrap_or_else(|| panic!("workspace root package must have a name"))
        .to_string();
    let version = package["version"]
        .as_str()
        .unwrap_or_else(|| panic!("workspace root package must have a version"))
        .to_string();
    (name, version)
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
    for required in [
        "CARGO_TARGET_DIR_ABS",
        r#"export CARGO_TARGET_DIR="${CARGO_TARGET_DIR_ABS}""#,
        "ensure_no_repository_target_dir",
        "repository-local target/",
    ] {
        assert!(
            script.contains(required),
            "{path} must enforce repository-local target discipline token `{required}`"
        );
    }

    let required_commands = [
        "git diff --check",
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
fn local_ci_docker_wrapper_runs_same_amd64_nix_container_and_persistent_caches() {
    let path = "scripts/local-ci-docker.sh";
    assert!(Path::new(path).is_file(), "{path} must exist");
    assert_executable(path);

    let script = read_repo_file(path);
    for required in [
        "docker run --rm",
        "--platform linux/amd64",
        "nixos/nix:latest",
        r#""${REPO_ROOT}:/work""#,
        "NIX_STORE_VOLUME=${ONE_AI_KEY_NIX_STORE_VOLUME:-one-ai-key-nix-amd64}",
        r#""${NIX_STORE_VOLUME}:/nix""#,
        "NIX_CACHE_VOLUME=${ONE_AI_KEY_NIX_CACHE_VOLUME:-one-ai-key-nix-cache-amd64}",
        r#""${NIX_CACHE_VOLUME}:/root/.cache/nix""#,
        "CARGO_TARGET_VOLUME=${ONE_AI_KEY_CARGO_TARGET_VOLUME:-one-ai-key-cargo-target-amd64}",
        r#""${CARGO_TARGET_VOLUME}:/cargo-target""#,
        "CARGO_HOME_VOLUME=${ONE_AI_KEY_CARGO_HOME_VOLUME:-one-ai-key-cargo-home-amd64}",
        r#""${CARGO_HOME_VOLUME}:/cargo-home""#,
        "-e CARGO_TARGET_DIR=/cargo-target",
        "-e CARGO_HOME=/cargo-home",
        "/work/scripts/local-ci.sh",
    ] {
        assert!(
            script.contains(required),
            "{path} must include local Docker/Nix CI contract token `{required}`"
        );
    }

    for package in [
        "cargo",
        "rustc",
        "rustfmt",
        "clippy",
        "bash",
        "curl",
        "jq",
        "gcc",
        "pkg-config",
        "openssl",
    ] {
        assert!(
            script.contains(package),
            "{path} must make `{package}` available in the Nix shell"
        );
    }

    for forbidden in ["ssh", "scp", "rsync", "gateway", "token", "secret"] {
        assert!(
            !script.to_ascii_lowercase().contains(forbidden),
            "{path} must not contain remote operation or credential token `{forbidden}`"
        );
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
        script.contains("jq --version"),
        "{path} must require jq for structured cargo metadata parsing"
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
    for required in [
        "current_source_tree_hash",
        "BUILD_INFO_PATH",
        ".build.json",
        "source_tree_hash",
        "archive_sha256",
        "jq -n",
    ] {
        assert!(
            script.contains(required),
            "{path} must write local build metadata token `{required}` for stale dist detection"
        );
    }

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
            "NIX_CACHE_VOLUME=${ONE_AI_KEY_NIX_CACHE_VOLUME:-one-ai-key-nix-cache-amd64}"
        ) && script.contains(r#""${NIX_CACHE_VOLUME}:/root/.cache/nix""#),
        "{path} must persist the Nix user Git/eval cache in a configurable named Docker volume"
    );
    assert!(
        script.contains(
            "CARGO_TARGET_VOLUME=${ONE_AI_KEY_CARGO_TARGET_VOLUME:-one-ai-key-cargo-target-amd64}"
        ) && script.contains(r#""${CARGO_TARGET_VOLUME}:/cargo-target""#)
            && script.contains("-e CARGO_TARGET_DIR=/cargo-target"),
        "{path} must keep release Cargo build output outside the repository"
    );
    assert!(
        script.contains(
            "CARGO_HOME_VOLUME=${ONE_AI_KEY_CARGO_HOME_VOLUME:-one-ai-key-cargo-home-amd64}"
        ) && script.contains(r#""${CARGO_HOME_VOLUME}:/cargo-home""#)
            && script.contains("-e CARGO_HOME=/cargo-home"),
        "{path} must persist Cargo home cache in a configurable named Docker volume"
    );
    assert!(
        script.contains("nix-command flakes"),
        "{path} must enable nix-command and flakes"
    );
    for package in ["cargo", "rustc", "jq", "gcc", "pkg-config", "openssl"] {
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
fn release_scripts_use_structured_cargo_metadata_without_greedy_json_sed() {
    for path in [
        "scripts/build-release-x86_64-linux.sh",
        "scripts/release-smoke.sh",
    ] {
        let script = read_repo_file(path);

        assert!(
            script.contains("cargo pkgid --locked"),
            "{path} must keep cargo pkgid as the release version source"
        );
        assert!(
            script.contains("cargo metadata --locked --no-deps --format-version 1"),
            "{path} must derive the package name from structured cargo metadata"
        );
        assert!(
            script.contains(".workspace_members[0] as $root")
                && script.contains("select(.id == $root)"),
            "{path} must select the workspace root package, not an arbitrary package"
        );
        for key in ["name", "version"] {
            let greedy_metadata_pattern = format!(r#"s/.*\"{key}\":"#);
            assert!(
                !script.contains(&greedy_metadata_pattern),
                "{path} must not use greedy sed extraction over cargo metadata JSON for `{key}`"
            );
        }
    }
}

#[test]
fn cargo_release_identity_sources_resolve_to_one_ai_key_without_running_release_script() {
    let pkgid_output = Command::new("cargo")
        .args(["pkgid", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap_or_else(|error| panic!("failed to run cargo pkgid --locked: {error}"));
    assert!(
        pkgid_output.status.success(),
        "cargo pkgid --locked failed with status {:?}: {}",
        pkgid_output.status.code(),
        String::from_utf8_lossy(&pkgid_output.stderr)
    );

    let package_id = String::from_utf8(pkgid_output.stdout)
        .unwrap_or_else(|error| panic!("cargo pkgid output must be UTF-8: {error}"));
    let pkgid_version = parse_cargo_pkgid_version(&package_id);

    let metadata_output = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap_or_else(|error| panic!("failed to run cargo metadata --locked: {error}"));
    assert!(
        metadata_output.status.success(),
        "cargo metadata --locked failed with status {:?}: {}",
        metadata_output.status.code(),
        String::from_utf8_lossy(&metadata_output.stderr)
    );
    let metadata = String::from_utf8(metadata_output.stdout)
        .unwrap_or_else(|error| panic!("cargo metadata output must be UTF-8: {error}"));
    let (package_name, metadata_version) = cargo_workspace_root_package(&metadata);

    assert_eq!(package_name, "one-ai-key");
    assert_eq!(metadata_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(pkgid_version, metadata_version);
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
        "__pycache__/",
        ".pytest_cache/",
        ".ruff_cache/",
        ".mypy_cache/",
        ".coverage",
        "coverage/",
        "htmlcov/",
    ];

    for (path, contents) in files {
        let normalized = contents
            .lines()
            .map(|line| line.trim().trim_start_matches('/'))
            .collect::<Vec<_>>();
        for pattern in required_patterns {
            assert!(
                normalized.contains(&pattern),
                "{path} must exclude sensitive runtime/release pattern `{pattern}`"
            );
        }
    }
}

#[test]
fn local_ci_runs_repository_hygiene_self_tests_without_reading_git_index_state() {
    let path = "scripts/local-ci.sh";
    let script = read_repo_file(path);

    for required in [
        "scripts/check-staged-denylist.sh --self-test",
        "scripts/check-staged-denylist.sh --check-public-plans",
    ] {
        assert!(
            script.contains(required),
            "{path} must run repository hygiene check `{required}` before Cargo verification"
        );
    }
}

#[test]
fn staged_denylist_has_public_plan_hygiene_contract() {
    let path = "scripts/check-staged-denylist.sh";
    assert!(Path::new(path).is_file(), "{path} must exist");
    assert_executable(path);

    let script = read_repo_file(path);
    for required in [
        "--check-public-plans",
        "git ls-files -- 'docs/plans/*.md'",
        "PUBLIC_PLAN_DENY_REGEX",
        "Task [0-9]+ checkpoint",
        "deployment_boundary_result",
        "chat/room",
    ] {
        assert!(
            script.contains(required),
            "{path} must contain public plan hygiene contract token `{required}`"
        );
    }

    let output = Command::new(path)
        .arg("--self-test")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap_or_else(|error| panic!("failed to run {path} --self-test: {error}"));
    assert!(
        output.status.success(),
        "{path} --self-test must pass: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn staged_denylist_rejects_case_variant_and_nested_generated_paths() {
    let repo = unique_temp_dir("staged-denylist-fixtures");
    let script_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/check-staged-denylist.sh");

    let init = Command::new("git")
        .arg("init")
        .current_dir(&repo)
        .output()
        .unwrap_or_else(|error| panic!("failed to initialize temp git repo: {error}"));
    assert!(
        init.status.success(),
        "git init failed: stdout={} stderr={}",
        String::from_utf8_lossy(&init.stdout),
        String::from_utf8_lossy(&init.stderr)
    );

    for (path, contents) in [
        ("Target/debug/app.o", "build artifact\n"),
        ("nested/__pycache__/cache.pyc", "python cache\n"),
        ("runtime/API_KEYS.JSON", "{}\n"),
    ] {
        write_temp_file(&repo, path, contents);
    }

    let add = Command::new("git")
        .arg("add")
        .arg(".")
        .current_dir(&repo)
        .output()
        .unwrap_or_else(|error| panic!("failed to stage temp fixture files: {error}"));
    assert!(
        add.status.success(),
        "git add failed: stdout={} stderr={}",
        String::from_utf8_lossy(&add.stdout),
        String::from_utf8_lossy(&add.stderr)
    );

    let output = Command::new("bash")
        .arg(&script_path)
        .current_dir(&repo)
        .output()
        .unwrap_or_else(|error| panic!("failed to run staged denylist: {error}"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let _ = fs::remove_dir_all(&repo);

    assert!(
        !output.status.success(),
        "staged denylist must reject generated/cache/secret-like staged paths: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        stderr
    );
    for denied_path in [
        "path:Target/debug/app.o",
        "path:nested/__pycache__/cache.pyc",
        "path:runtime/API_KEYS.JSON",
    ] {
        assert!(
            stderr.contains(denied_path),
            "staged denylist stderr must include `{denied_path}`; stderr={stderr}"
        );
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
        "BUILD_INFO_PATH",
        ".build.json",
        "current_source_tree_hash",
        "CURRENT_SOURCE_TREE_HASH",
        "release smoke source tree fingerprint does not match the built artifact",
        r#".source_tree_hash == $source_tree_hash"#,
        r#""${BIN}" --help"#,
        r#""${BIN}" models onboard-plan --help"#,
        "FORBIDDEN_ONBOARD_HELP_FLAGS",
        "release binary exposes legacy models onboard-plan flag",
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
        "/ready",
        ".credentials.available > 0",
        "model_visibility_preview",
        "CLIENT_MODELS",
        "[$forbidden[] | select(. as $term | $body | contains($term))]",
        "INVALID_CLIENT_TOKEN",
        "INVALID_CLIENT_STATUS",
        "invalid router api key",
        "invalid client token response leaked token material",
        "UPSTREAM_MODELS_BEFORE_SERVICE=$(mock_upstream_model_catalog_requests)",
        "UPSTREAM_MODELS_AFTER_AUTHENTICATED_MODELS=$(mock_upstream_model_catalog_requests)",
        "authenticated /v1/models called upstream /v1/models",
        "side_effect_class",
        "models-explain.json",
        "models explain --model gpt-example --client-token-ref local-client --endpoint-family chat_completions --output json",
        r#".can_use == true"#,
        r#".blocking_domain == "none""#,
        r#".endpoint_family == "chat_completions""#,
        r#".model == "gpt-example""#,
        r#".client_token_ref == "local-client""#,
        r#".reason_code == "available""#,
        r#".next_action.template_id"#,
        r#".requires_confirmation == false"#,
        r#".next_action.safe_argv"#,
        "assert_bounded_evidence",
        r#".status == "available" and .reason_code == "available""#,
        r#".admission_summary.status == "available""#,
        r#".admission_summary.reason_code == "available""#,
        r#".admission_summary.reason_code == .reason_code"#,
        r#".model == $model"#,
        r#".scope.client_token_ref == $client_token_ref"#,
        "selected_target",
        "candidates",
        "doctor --output json",
        "client-tokens list --output json",
        "models list --client-token-ref local-client --output json",
        "route explain gpt-example --client-token-ref local-client --endpoint-family chat_completions --output json",
        r#".endpoint_family_status == "evaluated""#,
        r#".can_use == true"#,
        r#".availability.status == "available""#,
        "keys stats --credential-set relay_credentials --output json",
        "failures tail --last 20 --output json",
        "failures-tail.json",
        r#".availability_source == "bounded_evidence""#,
        r#".current_availability == false"#,
        r#".window.kind == "bounded_recent_events""#,
        r#".window.limit"#,
        r#".window.returned"#,
        r#".window.truncated"#,
        r#".data.failures | length == 0"#,
        "response-filter-samples.yaml",
        "response-filters check --samples response-filter-samples.yaml --output json",
        "response-filter-check.json",
        r#".reason_code == "response_filter_samples_passed""#,
        r#".side_effect_class == "offline_readonly""#,
        r#".data.sample_count == 2"#,
        r#".data.failed_count == 0"#,
        "release-smoke-redact",
        "release-smoke-reject",
        "response-filters events --last 20 --output json",
        "response-filter-events.json",
        r#".reason_code == "no_response_filter_events_found""#,
        r#".window.kind == "bounded_recent_response_filter_events""#,
        r#".data.event_count == 0"#,
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
fn release_smoke_expected_management_reports_cover_every_captured_report() {
    let path = "scripts/release-smoke.sh";
    let script = read_repo_file(path);
    let expected_reports = script
        .split("EXPECTED_MANAGEMENT_REPORTS=(")
        .nth(1)
        .and_then(|tail| tail.split("\n)").next())
        .unwrap_or_else(|| panic!("{path} must declare EXPECTED_MANAGEMENT_REPORTS"));
    let captured_reports = script
        .lines()
        .filter_map(|line| {
            line.trim_start()
                .strip_prefix("capture_management_report ")
                .and_then(|tail| tail.split_whitespace().next())
        })
        .collect::<Vec<_>>();

    assert!(
        !captured_reports.is_empty(),
        "{path} must capture management reports for release-smoke verification"
    );
    for report in captured_reports {
        assert!(
            expected_reports.contains(&format!(r#""{report}""#)),
            "{path} EXPECTED_MANAGEMENT_REPORTS must include captured report {report}"
        );
    }
}

#[test]
fn release_smoke_script_covers_client_token_lifecycle_workflow() {
    let path = "scripts/release-smoke.sh";
    let script = read_repo_file(path);

    for required in [
        "NEW_CLIENT_TOKEN",
        "ONE_AI_KEY_NEW_CLIENT_TOKEN",
        "client-tokens-create-dry-run.json",
        "client-tokens-create-apply.json",
        "client-tokens-scope-update-dry-run.json",
        "client-tokens-scope-update-apply.json",
        "client-tokens-disable-dry-run.json",
        "client-tokens-disable-apply.json",
        "client-tokens-enable-dry-run.json",
        "client-tokens-enable-apply.json",
        "client-tokens create --name release-smoke-client-extra --token-env ONE_AI_KEY_NEW_CLIENT_TOKEN --allowed-model gpt-example --unrestricted-channels --dry-run --output json",
        "client-tokens create --name release-smoke-client-extra --token-env ONE_AI_KEY_NEW_CLIENT_TOKEN --allowed-model gpt-example --unrestricted-channels --yes --output json",
        "NEW_CLIENT_TOKEN_ID",
        r#".raw_token_read == false"#,
        r#".raw_token_read == true"#,
        r#".mutating_create_sent == false"#,
        r#".mutating_create_sent == true"#,
        r#".reason_code == "client_token_create_plan""#,
        r#".reason_code == "client_token_create_applied""#,
        r#".token.scope_summary.allowed_model_group_count == 1"#,
        "client-tokens scope-update \"${NEW_CLIENT_TOKEN_ID}\" --unrestricted-models --unrestricted-channels --dry-run --output json",
        "client-tokens scope-update \"${NEW_CLIENT_TOKEN_ID}\" --unrestricted-models --unrestricted-channels --yes --output json",
        r#".reason_code == "client_token_scope_update_plan""#,
        r#".reason_code == "client_token_scope_update_applied""#,
        r#".unrestricted_model_groups == true"#,
        r#".unrestricted_channels == true"#,
        r#".mutating_scope_update_sent == false"#,
        r#".mutating_scope_update_sent == true"#,
        "client-tokens disable \"${NEW_CLIENT_TOKEN_ID}\" --dry-run --output json",
        "client-tokens disable \"${NEW_CLIENT_TOKEN_ID}\" --yes --output json",
        r#".reason_code == "client_token_disable_plan""#,
        r#".reason_code == "client_token_disable_applied""#,
        r#".mutating_disable_sent == false"#,
        r#".mutating_disable_sent == true"#,
        "client-tokens enable \"${NEW_CLIENT_TOKEN_ID}\" --dry-run --output json",
        "client-tokens enable \"${NEW_CLIENT_TOKEN_ID}\" --yes --output json",
        r#".reason_code == "client_token_enable_plan""#,
        r#".reason_code == "client_token_enable_applied""#,
        r#".mutating_enable_sent == false"#,
        r#".mutating_enable_sent == true"#,
        "new client token unexpectedly remained valid after disable",
        "new client token did not work after enable",
        "release-smoke-new-client-token",
    ] {
        assert!(
            script.contains(required),
            "{path} must cover client-token lifecycle release-smoke token `{required}`"
        );
    }
}

#[test]
fn release_smoke_script_covers_local_admission_and_upstream_503_failure_evidence() {
    let path = "scripts/release-smoke.sh";
    let script = read_repo_file(path);

    for required in [
        "upstream-503.json",
        "failures-tail-upstream-503.json",
        "failures-explain-upstream-503.json",
        "failures tail --last 20 --endpoint-family chat_completions --output json",
        "failures explain \"${UPSTREAM_503_REQUEST_ID}\" --endpoint-family chat_completions --output json",
        "route-explain-provider-cooling-last-resort.json",
        "provider-cooling-last-resort.json",
        "keys-disable-final-available.json",
        "local-admission-503.json",
        "failures-tail-local-admission-503.json",
        "failures-explain-local-admission-503.json",
        "failures explain \"${LOCAL_ADMISSION_REQUEST_ID}\" --endpoint-family chat_completions --output json",
        "mock_upstream_chat_completion_posts",
        "POSTS_BEFORE_UPSTREAM_503",
        "POSTS_AFTER_UPSTREAM_503",
        "upstream 503 smoke did not reach mock upstream exactly once",
        "POSTS_BEFORE_SOFT_LAST_RESORT",
        "POSTS_AFTER_SOFT_LAST_RESORT",
        "provider-cooling last-resort smoke did not reach mock upstream exactly once",
        r#".reason_code == "provider_cooling_down_last_resort""#,
        "POSTS_BEFORE_LOCAL_ADMISSION",
        "POSTS_AFTER_LOCAL_ADMISSION",
        "local admission smoke unexpectedly reached mock upstream chat completions",
        r#".reason_code == "upstream_5xx""#,
        r#".endpoint_family == "chat_completions""#,
        r#".scope.endpoint_family == "chat_completions""#,
        r#".data.explanation.endpoint_family == "chat_completions""#,
        r#".client_visible_status == "upstream_5xx""#,
        r#".upstream_status == 503"#,
        r#".event_kind == "route_admission_denied""#,
        r#".client_visible_status == "local_503""#,
        r#".upstream_status == null"#,
        r#".admission.included_count == 0"#,
        r#".reason_code == "no_route_candidate""#,
    ] {
        assert!(
            script.contains(required),
            "{path} must cover local/upstream 503 failure evidence token `{required}`"
        );
    }
}

#[test]
fn release_smoke_models_explain_diagnosis_is_canonical_endpoint_family() {
    let path = "scripts/release-smoke.sh";
    let script = read_repo_file(path);

    let canonical_models_explain_lines = script
        .lines()
        .filter(|line| {
            line.contains("capture_management_report")
                && line.contains("models")
                && line.contains("explain")
        })
        .collect::<Vec<_>>();

    assert!(
        !canonical_models_explain_lines.is_empty(),
        "{path} must smoke canonical models explain diagnosis"
    );
    for line in canonical_models_explain_lines {
        assert!(
            line.contains("--endpoint-family"),
            "{path} canonical models explain smoke must include --endpoint-family: {line}"
        );
    }
}

#[test]
fn release_smoke_route_explain_followup_carries_endpoint_family() {
    let path = "scripts/release-smoke.sh";
    let script = read_repo_file(path);

    let canonical_route_explain_lines = script
        .lines()
        .filter(|line| {
            line.contains("capture_management_report")
                && line.contains("route-explain.json")
                && line.contains("route")
                && line.contains("explain")
        })
        .collect::<Vec<_>>();

    assert!(
        !canonical_route_explain_lines.is_empty(),
        "{path} must smoke canonical route explain follow-up"
    );
    for line in canonical_route_explain_lines {
        assert!(
            line.contains("--endpoint-family"),
            "{path} canonical route explain follow-up must include --endpoint-family: {line}"
        );
    }
}

#[test]
fn diagnostic_next_action_static_contract_excludes_mutating_and_probe_commands() {
    let diagnostic_contract = read_repo_file("src/diagnostic_contract.rs")
        .split("#[cfg(test)]")
        .next()
        .unwrap_or_default()
        .to_string();
    let models_cli = read_repo_file("src/cli_commands/models.rs")
        .split("#[cfg(test)]")
        .next()
        .unwrap_or_default()
        .to_string();

    for (path, text) in [
        ("src/diagnostic_contract.rs", diagnostic_contract),
        ("src/cli_commands/models.rs", models_cli),
    ] {
        for forbidden in [
            r#""keys", "import""#,
            r#""keys", "probe""#,
            r#""reload", "apply""#,
            r#""curl""#,
            "template_id: \"keys_import\"",
            "template_id: \"keys_probe\"",
            "template_id: \"reload_apply\"",
            "template_id: \"curl\"",
        ] {
            assert!(
                !text.contains(forbidden),
                "{path} diagnostic next_action contract must not include forbidden command token `{forbidden}`"
            );
        }
    }
}

#[test]
fn release_smoke_script_covers_local_model_publication_workflow() {
    let path = "scripts/release-smoke.sh";
    let script = read_repo_file(path);

    for required in [
        r#"export KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE="${WORK_DIR}/registry-store.sqlite""#,
        "release-smoke-public-model",
        "release-smoke-upstream-model",
        "models-onboard-plan.json",
        "models-onboard-apply-dry-run.json",
        "models-onboard-apply.json",
        "reload-status-staged.json",
        "models-explain-staged.json",
        "reload-diff-staged.json",
        "reload-apply.json",
        "models-explain-published.json",
        "STAGED_REGISTRY_VERSION",
        "PRE_ONBOARD_STAGED_REGISTRY_VERSION",
        "models onboard-plan --channel",
        "--public-model \"${PUBLISHED_PUBLIC_MODEL}\"",
        "--upstream-model \"${PUBLISHED_UPSTREAM_MODEL}\"",
        "--client-token-ref local-client",
        "--endpoint-family chat_completions",
        "--apply --dry-run",
        "--apply --expected-staged-registry-version",
        r#".credential_set_ref == "relay_credentials""#,
        r#".staged_registry_version == $pre_onboard_staged_registry_version"#,
        "reload apply --yes --expected-staged-registry-version",
        r#".planning_only_no_visibility_change == true"#,
        r#".client_visibility_changed == false"#,
        r#".live_discovery_called == false"#,
        r#".management_mutation_sent == false"#,
        r#".runtime_reload_required == true"#,
        r#".applied_to_runtime == false"#,
        r#".staged_vs_runtime.active_matches_staged == false"#,
        r#".staged_vs_runtime.reload_required_reason == "staged_registry_differs""#,
        r#".reason_code == "reload_diff_available""#,
        r#".resource_changes | type == "array""#,
        r#".budget.truncated == false"#,
        r#".reason_code == "runtime_reload_applied""#,
        r#".can_use == true"#,
        r#"map(.id) | index($published_public_model) == null"#,
        r#"map(.id) | index($published_public_model) != null"#,
        "UPSTREAM_MODELS_AFTER_WORKFLOW",
        "UPSTREAM_MODELS_AFTER_AUTHENTICATED_MODELS",
        "release smoke published model ok",
    ] {
        assert!(
            script.contains(required),
            "{path} must cover local model publication smoke token `{required}`"
        );
    }
}

#[test]
fn release_smoke_script_checks_management_reports_are_redacted_and_bounded() {
    let path = "scripts/release-smoke.sh";
    let script = read_repo_file(path);

    for required in [
        "MANAGEMENT_REPORTS=(",
        "EXPECTED_MANAGEMENT_REPORTS=(",
        "assert_expected_management_reports_captured",
        "assert_management_report_envelope",
        "assert_no_management_report_leaks",
        "LEGACY_MANAGEMENT_REPORT_LABELS=(",
        "assert_no_legacy_management_report_labels",
        "unavailable_until_m4",
        "deferred_by_m1_m4",
        "keys_probe_unavailable_until_m3",
        "deferred_to_separate_plan",
        "deferred to a separate plan",
        "later reload/scope workflows when available",
        "until M4",
        "before M3",
        "M2.4b",
        "legacy internal phase label",
        "[[ \"$#\" -gt 0 ]]",
        "[[ -s \"${report}\" ]]",
        "management report list did not match expected captured reports",
        "management report is missing or empty",
        "doctor.json",
        "client-tokens-list.json",
        "models-list.json",
        "models-explain.json",
        "route-explain.json",
        "keys-stats.json",
        "keys-replacement-plan.json",
        "keys-import-dry-run.json",
        "keys-import-apply.json",
        "keys-stats-after-import.json",
        "keys-probe-dry-run.json",
        "keys-probe.json",
        "keys-probe-apply-plan.json",
        "keys-probe-apply-dry-run.json",
        "keys-probe-invalid.json",
        "keys-probe-apply-invalid-plan.json",
        "keys-probe-apply-apply.json",
        "keys-disable-dry-run.json",
        "keys-disable-apply.json",
        "keys-restore-dry-run.json",
        "keys-restore-apply.json",
        "keys-stats-after-restore.json",
        "failures-tail.json",
        "response-filter-check.json",
        "reload-status.json",
        "reload-diff.json",
        "reload-apply-dry-run.json",
        "models-onboard-plan.json",
        "models-onboard-apply-dry-run.json",
        "models-onboard-apply.json",
        "reload-status-staged.json",
        "models-explain-staged.json",
        "reload-diff-staged.json",
        "reload-apply.json",
        "models-explain-published.json",
        "failures-tail-upstream-503.json",
        "failures-explain-upstream-503.json",
        "route-explain-provider-cooling-last-resort.json",
        "keys-disable-final-available.json",
        "failures-tail-local-admission-503.json",
        "failures-explain-local-admission-503.json",
        "negative-management-url.txt",
        "release-smoke-client-token",
        "release-smoke-management-token",
        "release-smoke-upstream-token",
        "release-smoke-invalid-client-token",
        "UPSTREAM_503_MARKER",
        "LOCAL_ADMISSION_MARKER",
        "RESPONSE_FILTER_REDACT_MARKER",
        "RESPONSE_FILTER_REJECT_MARKER",
        "${UPSTREAM_503_MARKER}",
        "${LOCAL_ADMISSION_MARKER}",
        "${RESPONSE_FILTER_REDACT_MARKER}",
        "${RESPONSE_FILTER_REJECT_MARKER}",
        "${WORK_DIR}",
        "data/relay.keys",
        "127.0.0.1:${MOCK_PORT}",
        "raw_management_url",
        "raw body text",
        "grep -Eq",
        "token-looking URL component",
        "assert_bounded_evidence",
        "bounded_evidence",
        "max_items",
        "management report JSON envelope is invalid",
        "reads_local_files",
        "reads_management_runtime",
        "reads_management_store",
        "writes_local_files",
        "writes_management_store",
        "calls_upstream",
        "mutates_runtime",
        ".next_action.template_id",
        ".next_action.side_effect_class",
        ".next_action.requires_confirmation",
        ".next_action.safe_argv",
        ".data.replacement_workflow.operator_maintenance_priority",
        ".data.replacement_workflow.blocking_reason_code",
        ".data.replacement_workflow.next_action.template_id",
        ".data.replacement_workflow.next_action.safe_argv",
        "active_capacity_missing",
        "selected_capacity_low",
        "fallback_capacity_low",
        "inventory_only",
        "route_impact_unknown",
        "no_available_credentials",
        "credential_set_exhausted",
        "route_not_candidate",
        "route_preview_unavailable",
        r#"(.next_action.safe_argv | type) == "array""#,
        ".evidence.candidate_limit >= 0",
        ".evidence.endpoint_family_target_count >= 0",
        ".evidence.preview_candidate_count >= 0",
        "assert_management_report_envelope \"${MANAGEMENT_REPORTS[@]}\"",
        "assert_no_legacy_management_report_labels \"${MANAGEMENT_REPORTS[@]}\"",
        "from urllib.parse import urlsplit",
        "failed to parse mock upstream event JSONL",
        "mock upstream event missing string method/path",
        "urlsplit(event[\"path\"]).path",
    ] {
        assert!(
            script.contains(required),
            "{path} must contain shared redaction/bounded-output guard `{required}`"
        );
    }
    for forbidden in [
        "local truncated=false",
        r#"$truncated == false"#,
        "--argjson truncated",
        "except Exception:\n            continue",
    ] {
        assert!(
            !script.contains(forbidden),
            "{path} must not contain constant bounded evidence assertion `{forbidden}`"
        );
    }
    let ordinary_leak_scan = script
        .find("assert_no_management_report_leaks \"${MANAGEMENT_REPORTS[@]}\"")
        .expect("release smoke must run ordinary management report leak scan");
    let legacy_label_scan = script
        .find("assert_no_legacy_management_report_labels \"${MANAGEMENT_REPORTS[@]}\"")
        .expect("release smoke must run legacy management label scan");
    let envelope_scan = script
        .find("assert_management_report_envelope \"${MANAGEMENT_REPORTS[@]}\"")
        .expect("release smoke must run management report envelope scan");
    assert!(
        ordinary_leak_scan < legacy_label_scan,
        "{path} must scan management reports for ordinary leaks before legacy phase labels"
    );
    assert!(
        envelope_scan < ordinary_leak_scan,
        "{path} must validate management report envelopes before leak scans"
    );
}

#[test]
fn release_smoke_script_is_local_redacted_and_cleans_processes() {
    let path = "scripts/release-smoke.sh";
    let script = read_repo_file(path);

    for required in [
        "docker run --rm",
        "--platform linux/amd64",
        "nixos/nix:latest",
        "NIX_CACHE_VOLUME=${ONE_AI_KEY_NIX_CACHE_VOLUME:-one-ai-key-nix-cache-amd64}",
        r#""${NIX_CACHE_VOLUME}:/root/.cache/nix""#,
        "CARGO_HOME_VOLUME=${ONE_AI_KEY_CARGO_HOME_VOLUME:-one-ai-key-cargo-home-amd64}",
        r#""${CARGO_HOME_VOLUME}:/cargo-home""#,
        "-e CARGO_HOME=/cargo-home",
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

#[test]
fn production_smoke_script_is_guarded_parameterized_and_redacted() {
    let path = "scripts/production-smoke.sh";
    assert!(Path::new(path).is_file(), "{path} must exist");
    assert_executable(path);

    let script = read_repo_file(path);

    for required in [
        "--allow-production",
        "refusing production smoke without --allow-production",
        "ONE_AI_KEY_PUBLIC_BASE_URL",
        "ONE_AI_KEY_MANAGEMENT_URL",
        "ONE_AI_KEY_CLIENT_TOKEN_ENV",
        "ONE_AI_KEY_MANAGEMENT_TOKEN_ENV",
        "ONE_AI_KEY_PUBLIC_MODEL",
        "ONE_AI_KEY_RELEASE_IDENTITY",
        "ONE_AI_KEY_CHECKSUM_IDENTITY",
        "ONE_AI_KEY_OUTPUT_DIR",
        "mktemp -d",
        "curl",
        "jq",
        "--config",
        "curl_config_escape",
        "safe_reason_code",
        "/health",
        "/management/health/serving",
        "/management/health/resilience",
        "/models",
        "/chat/completions",
        "Authorization: Bearer",
        "client_token_env",
        "management_token_env",
        "public_liveness",
        "management_serving",
        "management_resilience",
        "client_models",
        "client_completion",
        "summary.json",
        "raw request bodies",
        "raw response bodies",
    ] {
        assert!(
            script.contains(required),
            "{path} must include guarded production smoke contract token `{required}`"
        );
    }

    let lower = script.to_ascii_lowercase();
    for forbidden in [
        "ssh",
        "scp",
        "rsync",
        "systemctl",
        "nixos-rebuild",
        "cargo ",
        "git ",
        "raw_dir",
        "completion-request.json",
        "public-liveness.body",
        r#"args+=(--header "authorization: bearer ${token}")"#,
        r#"--header "authorization: bearer ${token}""#,
        "default_public_base_url",
        "default_management_url",
        "default_model",
    ] {
        assert!(
            !lower.contains(forbidden),
            "{path} must not contain production mutation or private-default token `{forbidden}`"
        );
    }
}

#[test]
fn production_smoke_passes_against_local_mock_and_keeps_artifacts_redacted() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|error| panic!("failed to bind local production-smoke mock: {error}"));
    listener
        .set_nonblocking(true)
        .unwrap_or_else(|error| panic!("failed to set mock listener nonblocking: {error}"));
    let port = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("failed to read local mock address: {error}"))
        .port();
    let (done_tx, done_rx) = mpsc::channel();
    let client_token = "production-smoke-client-token-fixture";
    let management_token = "production-smoke-management-token-fixture";
    let public_model = "production-smoke-contract-model";
    let mock_thread = thread::spawn(move || {
        let mut accepted = 0usize;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while accepted < 5 {
            match listener.accept() {
                Ok((stream, _)) => {
                    accepted += 1;
                    handle_production_smoke_mock_connection(
                        stream,
                        client_token,
                        management_token,
                        public_model,
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                    break;
                }
                Err(error) => panic!("mock accept failed: {error}"),
            }
        }
        let _ = done_tx.send(accepted);
    });

    let output_dir = unique_temp_dir("production-smoke-positive");
    let public_base_url = format!("http://127.0.0.1:{port}/v1");
    let management_url = format!("http://127.0.0.1:{port}");
    let output = Command::new("scripts/production-smoke.sh")
        .arg("--allow-production")
        .env("ONE_AI_KEY_PUBLIC_BASE_URL", &public_base_url)
        .env("ONE_AI_KEY_MANAGEMENT_URL", &management_url)
        .env(
            "ONE_AI_KEY_CLIENT_TOKEN_ENV",
            "ONE_AI_KEY_TEST_CLIENT_TOKEN",
        )
        .env(
            "ONE_AI_KEY_MANAGEMENT_TOKEN_ENV",
            "ONE_AI_KEY_TEST_MANAGEMENT_TOKEN",
        )
        .env("ONE_AI_KEY_TEST_CLIENT_TOKEN", client_token)
        .env("ONE_AI_KEY_TEST_MANAGEMENT_TOKEN", management_token)
        .env("ONE_AI_KEY_PUBLIC_MODEL", public_model)
        .env("ONE_AI_KEY_RELEASE_IDENTITY", "one-ai-key-v0.3.0")
        .env("ONE_AI_KEY_CHECKSUM_IDENTITY", "sha256:0123456789abcdef")
        .env("ONE_AI_KEY_OUTPUT_DIR", &output_dir)
        .env("ONE_AI_KEY_SMOKE_CONNECT_TIMEOUT_SECONDS", "1")
        .env("ONE_AI_KEY_SMOKE_MAX_TIME_SECONDS", "2")
        .output()
        .unwrap_or_else(|error| {
            panic!("failed to run production smoke positive contract: {error}")
        });

    assert!(
        output.status.success(),
        "production smoke should pass against local mock, stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let accepted = done_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|error| panic!("mock server did not finish within bounded wait: {error}"));
    assert_eq!(
        accepted, 5,
        "mock server should observe exactly five checks"
    );
    mock_thread
        .join()
        .unwrap_or_else(|_| panic!("mock server thread panicked"));

    let summary_path = output_dir.join("summary.json");
    let summary: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&summary_path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", summary_path.display())),
    )
    .unwrap_or_else(|error| panic!("summary must be JSON: {error}"));
    assert_eq!(summary["status"], "ok");
    assert_eq!(summary["reason_code"], "production_smoke_ok");
    assert_eq!(summary["model"], public_model);
    assert_eq!(summary["release_identity"], "one-ai-key-v0.3.0");
    assert_eq!(summary["checksum_identity"], "sha256:0123456789abcdef");
    assert_eq!(
        summary["token_sources"]["client_token_env"],
        "ONE_AI_KEY_TEST_CLIENT_TOKEN"
    );
    assert_eq!(
        summary["token_sources"]["management_token_env"],
        "ONE_AI_KEY_TEST_MANAGEMENT_TOKEN"
    );
    let checks = summary["checks"]
        .as_array()
        .unwrap_or_else(|| panic!("summary checks must be an array"));
    assert_eq!(checks.len(), 5);
    let expected_checks = [
        ("public_liveness", "public_liveness_ok"),
        ("management_serving", "management_serving_ok"),
        ("management_resilience", "resilient"),
        ("client_models", "client_model_visible"),
        ("client_completion", "client_completion_ok"),
    ];
    for (check_name, reason_code) in expected_checks {
        let check = checks
            .iter()
            .find(|check| check["check"] == check_name)
            .unwrap_or_else(|| panic!("summary missing check {check_name}"));
        assert_eq!(check["ok"], true, "check {check_name} should pass");
        assert_eq!(check["http_status"], 200, "check {check_name} HTTP");
        assert_eq!(check["reason_code"], reason_code);
        let artifact = output_dir.join(format!("{check_name}.json"));
        assert!(artifact.is_file(), "{} must exist", artifact.display());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_redacted_production_smoke_text(
        "stdout",
        &stdout,
        &[
            client_token,
            management_token,
            &public_base_url,
            &management_url,
            "Return the word ok.",
            "mock-response-poison",
            "assistant-visible-ok",
        ],
    );
    assert_redacted_production_smoke_text(
        "stderr",
        &stderr,
        &[
            client_token,
            management_token,
            &public_base_url,
            &management_url,
            "Return the word ok.",
            "mock-response-poison",
            "assistant-visible-ok",
        ],
    );
    for entry in fs::read_dir(&output_dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", output_dir.display()))
    {
        let entry =
            entry.unwrap_or_else(|error| panic!("failed to read output dir entry: {error}"));
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let contents = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        assert_redacted_production_smoke_text(
            &path.display().to_string(),
            &contents,
            &[
                client_token,
                management_token,
                &public_base_url,
                &management_url,
                "Return the word ok.",
                "mock-response-poison",
                "assistant-visible-ok",
            ],
        );
    }
    let _ = fs::remove_dir_all(output_dir);
}

fn handle_production_smoke_mock_connection(
    mut stream: TcpStream,
    client_token: &str,
    management_token: &str,
    public_model: &str,
) {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .unwrap_or_else(|error| panic!("failed to clone mock stream: {error}")),
    );
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .unwrap_or_else(|error| panic!("failed to read mock request line: {error}"));
    let mut authorization = String::new();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .unwrap_or_else(|error| panic!("failed to read mock header: {error}"));
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("content-length:") {
            content_length = value
                .trim()
                .parse::<usize>()
                .unwrap_or_else(|error| panic!("invalid content-length in mock request: {error}"));
        }
        if lower.starts_with("authorization:") {
            authorization = trimmed.to_string();
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut body)
            .unwrap_or_else(|error| panic!("failed to read mock request body: {error}"));
    }
    let body = String::from_utf8_lossy(&body);
    let parts = request_line.split_whitespace().collect::<Vec<_>>();
    let method = parts.first().copied().unwrap_or_default();
    let path = parts.get(1).copied().unwrap_or_default();

    let authorized_client = authorization == format!("Authorization: Bearer {client_token}");
    let authorized_management =
        authorization == format!("Authorization: Bearer {management_token}");
    let (status, response) = match (method, path) {
        ("GET", "/health") => (
            200,
            r#"{"status":"ok","poison":"mock-response-poison-health"}"#.to_string(),
        ),
        ("GET", "/management/health/serving") if authorized_management => (
            200,
            r#"{"status":"serving","serving":true,"poison":"mock-response-poison-serving"}"#
                .to_string(),
        ),
        ("GET", "/management/health/resilience") if authorized_management => (
            200,
            r#"{"status":"resilient","poison":"mock-response-poison-resilience"}"#.to_string(),
        ),
        ("GET", "/v1/models") if authorized_client => (
            200,
            format!(
                r#"{{"object":"list","data":[{{"id":"{public_model}","object":"model"}}],"poison":"mock-response-poison-models"}}"#
            ),
        ),
        ("POST", "/v1/chat/completions")
            if authorized_client && body.contains(public_model) && body.contains("Return the word ok.") =>
        {
            (
                200,
                r#"{"choices":[{"message":{"role":"assistant","content":"assistant-visible-ok"}}],"poison":"mock-response-poison-completion"}"#.to_string(),
            )
        }
        _ => (
            401,
            r#"{"error":{"code":"unauthorized","message":"mock unauthorized"}}"#.to_string(),
        ),
    };
    write_mock_response(&mut stream, status, &response);
}

fn write_mock_response(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = if status == 200 { "OK" } else { "Unauthorized" };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap_or_else(|error| panic!("failed to write mock response: {error}"));
}

fn assert_redacted_production_smoke_text(label: &str, text: &str, forbidden: &[&str]) {
    for forbidden in forbidden {
        assert!(
            !text.contains(forbidden),
            "{label} leaked forbidden production-smoke text `{forbidden}`"
        );
    }
    assert!(
        !text.contains("/health")
            && !text.contains("/management/health/serving")
            && !text.contains("/management/health/resilience")
            && !text.contains("/v1/models")
            && !text.contains("/v1/chat/completions"),
        "{label} leaked complete production-smoke endpoint path"
    );
}

#[test]
fn production_smoke_refuses_repository_output_dir_before_creating_it() {
    let path = ".tmp-production-smoke-contract-output";
    let _ = fs::remove_dir_all(path);

    let output = Command::new("scripts/production-smoke.sh")
        .arg("--allow-production")
        .env("ONE_AI_KEY_PUBLIC_BASE_URL", "http://127.0.0.1:1/v1")
        .env("ONE_AI_KEY_MANAGEMENT_URL", "http://127.0.0.1:1")
        .env(
            "ONE_AI_KEY_CLIENT_TOKEN_ENV",
            "ONE_AI_KEY_TEST_CLIENT_TOKEN",
        )
        .env(
            "ONE_AI_KEY_MANAGEMENT_TOKEN_ENV",
            "ONE_AI_KEY_TEST_MANAGEMENT_TOKEN",
        )
        .env("ONE_AI_KEY_TEST_CLIENT_TOKEN", "client-token-placeholder")
        .env(
            "ONE_AI_KEY_TEST_MANAGEMENT_TOKEN",
            "management-token-placeholder",
        )
        .env("ONE_AI_KEY_PUBLIC_MODEL", "contract-test-model")
        .env("ONE_AI_KEY_OUTPUT_DIR", path)
        .output()
        .unwrap_or_else(|error| panic!("failed to run production smoke contract: {error}"));

    assert!(
        !output.status.success(),
        "production smoke must refuse repository-local output dirs"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("output_dir_inside_repository"),
        "production smoke must emit a redacted repository-output refusal"
    );
    assert!(
        output.stderr.is_empty(),
        "production smoke refusal must not print non-JSON stderr"
    );
    assert!(
        !Path::new(path).exists(),
        "production smoke must refuse before creating repository-local output dirs"
    );
}

#[test]
fn production_smoke_refuses_repository_tmpdir_before_creating_output() {
    let tmp_parent = ".tmp-production-smoke-contract-tmp";
    let _ = fs::remove_dir_all(tmp_parent);
    fs::create_dir_all(tmp_parent)
        .unwrap_or_else(|error| panic!("failed to create {tmp_parent}: {error}"));

    let output = Command::new("scripts/production-smoke.sh")
        .arg("--allow-production")
        .env("ONE_AI_KEY_PUBLIC_BASE_URL", "http://127.0.0.1:1/v1")
        .env("ONE_AI_KEY_MANAGEMENT_URL", "http://127.0.0.1:1")
        .env(
            "ONE_AI_KEY_CLIENT_TOKEN_ENV",
            "ONE_AI_KEY_TEST_CLIENT_TOKEN",
        )
        .env(
            "ONE_AI_KEY_MANAGEMENT_TOKEN_ENV",
            "ONE_AI_KEY_TEST_MANAGEMENT_TOKEN",
        )
        .env("ONE_AI_KEY_TEST_CLIENT_TOKEN", "client-token-placeholder")
        .env(
            "ONE_AI_KEY_TEST_MANAGEMENT_TOKEN",
            "management-token-placeholder",
        )
        .env("ONE_AI_KEY_PUBLIC_MODEL", "contract-test-model")
        .env("TMPDIR", tmp_parent)
        .env_remove("ONE_AI_KEY_OUTPUT_DIR")
        .output()
        .unwrap_or_else(|error| panic!("failed to run production smoke contract: {error}"));

    assert!(
        !output.status.success(),
        "production smoke must refuse repository-local TMPDIR"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("tmpdir_inside_repository"),
        "production smoke must emit a redacted repository-TMPDIR refusal"
    );
    assert!(
        output.stderr.is_empty(),
        "production smoke TMPDIR refusal must not print non-JSON stderr"
    );

    let child_count = fs::read_dir(tmp_parent)
        .unwrap_or_else(|error| panic!("failed to read {tmp_parent}: {error}"))
        .count();
    let _ = fs::remove_dir_all(tmp_parent);
    assert_eq!(
        child_count, 0,
        "production smoke must refuse repository-local TMPDIR before creating output"
    );
}

#[cfg(unix)]
#[test]
fn production_smoke_reports_json_when_check_artifact_write_fails() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|error| panic!("system time must be after epoch: {error}"))
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "one-ai-key-production-smoke-unwritable-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path)
        .unwrap_or_else(|error| panic!("failed to create {}: {error}", path.display()));
    let conflicting_artifact = path.join("public_liveness.json");
    fs::create_dir_all(&conflicting_artifact).unwrap_or_else(|error| {
        panic!(
            "failed to create conflicting artifact dir {}: {error}",
            conflicting_artifact.display()
        )
    });

    let output = Command::new("scripts/production-smoke.sh")
        .arg("--allow-production")
        .env("ONE_AI_KEY_PUBLIC_BASE_URL", "http://127.0.0.1:1/v1")
        .env("ONE_AI_KEY_MANAGEMENT_URL", "http://127.0.0.1:1")
        .env(
            "ONE_AI_KEY_CLIENT_TOKEN_ENV",
            "ONE_AI_KEY_TEST_CLIENT_TOKEN",
        )
        .env(
            "ONE_AI_KEY_MANAGEMENT_TOKEN_ENV",
            "ONE_AI_KEY_TEST_MANAGEMENT_TOKEN",
        )
        .env("ONE_AI_KEY_TEST_CLIENT_TOKEN", "client-token-placeholder")
        .env(
            "ONE_AI_KEY_TEST_MANAGEMENT_TOKEN",
            "management-token-placeholder",
        )
        .env("ONE_AI_KEY_PUBLIC_MODEL", "contract-test-model")
        .env("ONE_AI_KEY_OUTPUT_DIR", &path)
        .output()
        .unwrap_or_else(|error| panic!("failed to run production smoke contract: {error}"));

    let _ = fs::remove_dir_all(&path);

    assert!(
        !output.status.success(),
        "production smoke must fail when an artifact cannot be written"
    );
    assert!(
        output.stderr.is_empty(),
        "production smoke artifact write failures must not print non-JSON stderr"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("stdout must be JSON, got {stdout:?}: {error}"));
    assert_eq!(
        value["reason_code"], "output_file_write_failed",
        "production smoke must return a redacted structured refusal"
    );
}

#[test]
fn operator_confidence_local_release_ready_record_is_public_and_bounded() {
    let path = "docs/release-records/v0.2-local-release-ready.md";
    assert!(Path::new(path).is_file(), "{path} must exist");

    let record = read_repo_file(path);
    for required in [
        "Stop node: `v0.2_local_release_ready`",
        &format!("Cargo package version: `{}`", env!("CARGO_PKG_VERSION")),
        "M1-M4 operator confidence baseline",
        "local CI: `pass`",
        "release artifact build: `pass`",
        "artifact shape check: `pass`",
        "release smoke: `pass`",
        "staged denylist: `pass`",
        "public docs: `pass`",
        "GitHub release publication: `not_run_by_design`",
        "deployment pin smoke: `not_run_by_design`",
        "Production smoke remains operator-run",
        "No request-path retry, routing, endpoint conversion, model exposure, or credential lifecycle behavior changed for this stop node.",
    ] {
        assert!(
            record.contains(required),
            "{path} must contain bounded release-ready record token `{required}`"
        );
    }

    for forbidden in [
        "sk-",
        "github_pat_",
        "ghp_",
        "http://",
        "https://",
        "/Users/",
        "rtoc-gateway",
        "hhhl",
        "dc.",
        "chat/room",
        "Telegram",
        "Discord",
        "raw command log",
        "artifact_sha",
        "deployment transcript",
    ] {
        assert!(
            !record.contains(forbidden),
            "{path} must not contain private or over-specific release evidence `{forbidden}`"
        );
    }
}

#[test]
fn operator_confidence_published_release_record_is_public_and_bounded() {
    let path = "docs/release-records/v0.2-published-release-complete.md";
    assert!(Path::new(path).is_file(), "{path} must exist");

    let record = read_repo_file(path);
    for required in [
        "Stop node: `v0.2_published_release_complete`",
        &format!("Cargo package version: `{}`", env!("CARGO_PKG_VERSION")),
        "Git tag: `v0.2.1`",
        "Release URL: `https://github.com/hechuyi/one-ai-key/releases/tag/v0.2.1`",
        "`one-ai-key-0.2.1-x86_64-unknown-linux-gnu.tar.gz`",
        "`one-ai-key-0.2.1-x86_64-unknown-linux-gnu.tar.gz.sha256`",
        "`8411d6ccbaaeb8bf14f1283a059197fcbe9cd9c3364d91b3f1bf2dd980df5ab9`",
        "GitHub release publication: `pass`",
        "uploaded asset download: `pass`",
        "uploaded checksum verification: `pass`",
        "uploaded archive shape check: `pass`",
        "deployment pin smoke: `not_run_by_design`",
        "Deployment pin verification remains a separate operator-run stop node.",
    ] {
        assert!(
            record.contains(required),
            "{path} must contain bounded published-release record token `{required}`"
        );
    }

    for forbidden in [
        "sk-",
        "github_pat_",
        "ghp_",
        "/Users/",
        "rtoc-gateway",
        "hhhl",
        "dc.",
        "chat/room",
        "Telegram",
        "Discord",
        "raw command log",
        "deployment transcript",
    ] {
        assert!(
            !record.contains(forbidden),
            "{path} must not contain private or over-specific release evidence `{forbidden}`"
        );
    }
}

#[test]
fn operator_confidence_deployment_pin_record_is_public_and_bounded() {
    let path = "docs/release-records/v0.2-deployment-pin-verified.md";
    assert!(Path::new(path).is_file(), "{path} must exist");

    let record = read_repo_file(path);
    for required in [
        "Stop node: `v0.2_deployment_pin_verified`",
        &format!("Cargo package version: `{}`", env!("CARGO_PKG_VERSION")),
        "Git tag: `v0.2.1`",
        "`one-ai-key-0.2.1-x86_64-unknown-linux-gnu.tar.gz`",
        "`8411d6ccbaaeb8bf14f1283a059197fcbe9cd9c3364d91b3f1bf2dd980df5ab9`",
        "NixOS release pin consumed the GitHub release artifact by URL and hash.",
        "service unit used the pinned release package",
        "public `/v1/models` smoke: `pass`",
        "public `/v1/responses` smoke: `pass`",
        "operator key helper status command: `pass`",
        "No server-side source build was performed.",
    ] {
        assert!(
            record.contains(required),
            "{path} must contain bounded deployment-pin record token `{required}`"
        );
    }

    for forbidden in [
        "sk-",
        "github_pat_",
        "ghp_",
        "/Users/",
        "rtoc-gateway",
        "ai.rtoc",
        "hhhl",
        "dc.",
        "chat/room",
        "Telegram",
        "Discord",
        "raw command log",
        "deployment transcript",
    ] {
        assert!(
            !record.contains(forbidden),
            "{path} must not contain private or over-specific deployment evidence `{forbidden}`"
        );
    }
}

#[test]
fn release_build_docs_protect_persistent_docker_build_caches() {
    let path = "docs/release-build.md";
    let docs = read_repo_file(path);

    for required in [
        "one-ai-key-nix-amd64",
        "one-ai-key-nix-cache-amd64",
        "one-ai-key-cargo-target-amd64",
        "one-ai-key-cargo-home-amd64",
        "build caches, not release artifacts",
        "not routine cleanup targets",
        "Repository-local `target/`",
        "ordinary build",
        "output and may be deleted",
    ] {
        assert!(
            docs.contains(required),
            "{path} must document persistent build-cache discipline token `{required}`"
        );
    }
}

#[test]
fn docs_make_models_explain_endpoint_family_the_canonical_first_diagnosis() {
    let docs = [
        ("README.md", read_repo_file("README.md")),
        ("docs/operations.md", read_repo_file("docs/operations.md")),
        (
            "docs/release-build.md",
            read_repo_file("docs/release-build.md"),
        ),
    ];

    for (path, text) in docs {
        for required in [
            "models explain",
            "--endpoint-family chat_completions",
            "client-token ref",
            "endpoint family",
            "can_use",
            "blocking_domain",
            "bounded evidence",
            "safe next_action",
        ] {
            assert!(
                text.contains(required),
                "{path} must document canonical diagnosis field `{required}`"
            );
        }
    }

    let operations = read_repo_file("docs/operations.md");
    assert!(
        operations.contains("Diagnosis recommendations stop at read-only commands")
            || operations.contains("diagnosis recommendations stop at read-only commands"),
        "docs/operations.md must state that diagnosis recommendations stop at read-only commands"
    );
    let release_build = read_repo_file("docs/release-build.md");
    assert!(
        release_build.contains("local")
            && release_build.contains("redacted")
            && release_build.contains("operator-run"),
        "docs/release-build.md must state that release/deployment smoke remains local, redacted, and operator-run"
    );
}

use crate::{
    credential_repository::{
        CredentialRepository, CredentialSetId, CredentialSetSource, ImportedCredential, KeyImport,
        KeyImportReport,
    },
    credentials::CredentialSource,
    endpoint_capabilities::EndpointSupport,
    registry::{RegistryDocument, RegistryRepository, YamlRegistryRepository},
};
use serde::Serialize;
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct CheckConfigOptions {
    pub config_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckConfigReport {
    pub diagnostics_schema_version: u32,
    pub status: DiagnosticStatus,
    pub reason_code: String,
    pub deprecated_fields: Vec<String>,
    pub deprecated_templates: Vec<String>,
    pub resource_counts: BTreeMap<String, usize>,
    pub warnings: Vec<String>,
    pub config_path: String,
    pub model_visibility_preview: Vec<ModelVisibilityPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelVisibilityPreview {
    pub client_token_ref: String,
    pub visible_models: Vec<String>,
    pub reason_code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticStatus {
    Ok,
    Error,
}

impl CheckConfigReport {
    fn status_code(&self) -> &'static str {
        match self.status {
            DiagnosticStatus::Ok => "ok",
            DiagnosticStatus::Error => "error",
        }
    }

    fn reason(&self) -> &'static str {
        match self.reason_code.as_str() {
            "ok" => "Configuration is valid for local startup.",
            "local_io_error" => {
                "Configuration validation failed because a local file is unavailable."
            }
            "malformed_config" => "Configuration YAML is malformed.",
            _ => "Configuration validation failed.",
        }
    }

    fn effect(&self) -> crate::cli_effects::CommandEffect {
        crate::cli_effects::CommandEffect {
            side_effect_class: crate::cli_effects::SideEffectClass::OfflineReadonly,
            effect_vector: crate::cli_effects::EffectVector {
                reads_local_files: true,
                ..crate::cli_effects::EffectVector::default()
            },
        }
    }

    fn next_action(&self) -> serde_json::Value {
        if self.status == DiagnosticStatus::Ok {
            json!({
                "summary": "Configuration validation passed. Start the service with the same config when ready.",
                "template_id": "serve",
                "safe_argv": ["one-ai-key", "serve", "--config", "<config>"],
                "side_effect_class": "serve_process",
                "requires_confirmation": false,
            })
        } else {
            json!({
                "summary": "Fix the local configuration or referenced files, then rerun check-config.",
                "template_id": "check_config",
                "safe_argv": ["one-ai-key", "check-config", "--config", "<config>"],
                "side_effect_class": "offline_readonly",
                "requires_confirmation": false,
            })
        }
    }

    fn report_value(&self) -> serde_json::Value {
        crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
            status: self.status_code(),
            reason: self.reason(),
            reason_code: &self.reason_code,
            effect: self.effect(),
            scope: json!({
                "config_path": self.config_path,
                "mode": "offline_config_validation",
            }),
            window: serde_json::Value::Null,
            next_action: self.next_action(),
            data: json!({
                "config_path": self.config_path,
                "diagnostics_schema_version": self.diagnostics_schema_version,
                "deprecated_fields": self.deprecated_fields,
                "deprecated_templates": self.deprecated_templates,
                "resource_counts": self.resource_counts,
                "warnings": self.warnings,
                "model_visibility_preview": self.model_visibility_preview,
            }),
        })
    }

    pub fn exit_code(&self) -> i32 {
        match self.status {
            DiagnosticStatus::Ok => 0,
            DiagnosticStatus::Error if self.reason_code == "local_io_error" => 2,
            DiagnosticStatus::Error if self.reason_code == "malformed_config" => 2,
            DiagnosticStatus::Error => 1,
        }
    }

    pub fn render_table(&self) -> String {
        let report = self.report_value();
        let mut output = String::new();
        crate::cli_report::append_report_envelope_table_fields(&mut output, &report);
        output.push_str(&format!("Config: {}\n", self.config_path));
        output.push_str(&format!("Warnings: {}\n", self.warnings.len()));
        for (index, warning) in self.warnings.iter().take(5).enumerate() {
            output.push_str(&format!(
                "warning[{index}]: {}\n",
                crate::cli_report::escape_table_value(warning)
            ));
        }
        if self.warnings.len() > 5 {
            output.push_str(&format!(
                "warnings_omitted: {}\n",
                self.warnings.len().saturating_sub(5)
            ));
        }
        output.push_str(&format!(
            "Model visibility preview: {}\n",
            render_visibility_preview(&self.model_visibility_preview)
        ));
        output
    }

    pub fn render_json(&self) -> String {
        self.report_value().to_string()
    }
}

pub fn check_config(options: CheckConfigOptions) -> CheckConfigReport {
    let config_path = safe_display_path(&options.config_path);
    let deprecated = deprecated_config_diagnostics(&options.config_path);
    let document = match YamlRegistryRepository::new(options.config_path.clone()).load_registry() {
        Ok(document) => document,
        Err(error) => {
            let message = error.to_string();
            let reason_code = if message.contains("read registry config") {
                "local_io_error"
            } else if message.contains("parse registry config YAML") {
                "malformed_config"
            } else {
                "config_validation_failed"
            };
            return CheckConfigReport {
                diagnostics_schema_version: 1,
                status: DiagnosticStatus::Error,
                reason_code: reason_code.to_string(),
                deprecated_fields: deprecated.fields,
                deprecated_templates: deprecated.templates,
                resource_counts: BTreeMap::new(),
                warnings: merge_warnings(
                    deprecated.warnings,
                    vec![redacted_error_summary(&message)],
                ),
                config_path,
                model_visibility_preview: Vec::new(),
            };
        }
    };

    let model_visibility_preview = model_visibility_preview(&document);
    let repository = DiagnosticCredentialRepository;
    match document
        .clone()
        .resolve_with_credential_repository_and_store_path(&repository, None)
    {
        Ok(resolved) => {
            let warnings =
                merge_warnings(deprecated.warnings, endpoint_capability_warnings(&resolved));
            CheckConfigReport {
                diagnostics_schema_version: 1,
                status: DiagnosticStatus::Ok,
                reason_code: "ok".to_string(),
                deprecated_fields: deprecated.fields,
                deprecated_templates: deprecated.templates,
                resource_counts: resource_counts(&document),
                warnings,
                config_path,
                model_visibility_preview,
            }
        }
        Err(error) => {
            let message = error.to_string();
            let reason_code = if message.contains("diagnostic key file") {
                "local_io_error"
            } else {
                "config_validation_failed"
            };
            CheckConfigReport {
                diagnostics_schema_version: 1,
                status: DiagnosticStatus::Error,
                reason_code: reason_code.to_string(),
                deprecated_fields: deprecated.fields,
                deprecated_templates: deprecated.templates,
                resource_counts: resource_counts(&document),
                warnings: merge_warnings(
                    deprecated.warnings,
                    vec![redacted_error_summary(&message)],
                ),
                config_path,
                model_visibility_preview,
            }
        }
    }
}

fn model_visibility_preview(document: &RegistryDocument) -> Vec<ModelVisibilityPreview> {
    document
        .client_tokens
        .iter()
        .map(|client_token| {
            let visible_models = if client_token.enabled {
                visible_models_for_client(document, client_token)
            } else {
                Vec::new()
            };
            let reason_code = if !client_token.enabled {
                "client_token_disabled"
            } else if !visible_models.is_empty() {
                "models_visible"
            } else if document.model_routes.is_empty() {
                "model_route_missing"
            } else if !client_token.allowed_model_groups.is_empty() {
                "client_scope_empty"
            } else {
                "target_channel_disabled"
            };
            ModelVisibilityPreview {
                client_token_ref: client_token.name.clone(),
                visible_models,
                reason_code: reason_code.to_string(),
            }
        })
        .collect()
}

fn visible_models_for_client(
    document: &RegistryDocument,
    client_token: &crate::config::ClientTokenConfig,
) -> Vec<String> {
    let mut visible = BTreeSet::new();
    for (public_model, route) in &document.model_routes {
        if !client_model_allowed(document, &client_token.allowed_model_groups, public_model) {
            continue;
        }
        if route
            .targets
            .iter()
            .any(|target| route_target_visible(document, target, &client_token.allowed_channels))
        {
            visible.insert(public_model.clone());
        }
    }
    visible.into_iter().collect()
}

fn endpoint_capability_warnings(config: &crate::config::ResolvedConfig) -> Vec<String> {
    let mut warnings = Vec::new();
    for route in config.model_routes.values() {
        if !route_looks_like_embeddings(route) {
            continue;
        }

        let mut serving_channels = Vec::new();
        let mut all_serving_targets_reject_embeddings = true;
        for target in &route.targets {
            if !target.enabled {
                continue;
            }
            let Some(pool) = config.pools.get(&target.channel_id.0) else {
                continue;
            };
            if !(pool.configured_enabled
                && pool.provider_enabled
                && pool.account_configured_enabled
                && pool.account_enabled)
            {
                continue;
            }
            serving_channels.push(target.channel_id.0.clone());
            all_serving_targets_reject_embeddings &=
                pool.endpoint_capabilities.embeddings == EndpointSupport::Unsupported;
        }

        if serving_channels.is_empty() || !all_serving_targets_reject_embeddings {
            continue;
        }
        serving_channels.sort();
        serving_channels.dedup();
        warnings.push(format!(
            "endpoint_capability_mismatch: public model {} looks like an embeddings model, but all enabled route targets declare embeddings: unsupported (channels: {})",
            warning_atom(&route.public_model),
            serving_channels
                .iter()
                .map(|channel| warning_atom(channel))
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    warnings.sort();
    warnings
}

fn route_looks_like_embeddings(route: &crate::route_plan::ModelRoute) -> bool {
    model_name_looks_like_embeddings(&route.public_model)
        || route
            .targets
            .iter()
            .filter_map(|target| target.upstream_model.as_deref())
            .any(model_name_looks_like_embeddings)
}

fn model_name_looks_like_embeddings(model: &str) -> bool {
    let lowered = model.to_ascii_lowercase();
    if lowered.starts_with("text-embedding-")
        || lowered.starts_with("embedding-")
        || lowered.starts_with("embeddings-")
        || lowered.starts_with("embed-")
    {
        return true;
    }
    lowered
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| matches!(token, "embedding" | "embeddings" | "embed"))
}

fn warning_atom(value: &str) -> String {
    const MAX_LEN: usize = 96;
    let mut rendered = value
        .chars()
        .flat_map(|ch| ch.escape_default())
        .collect::<String>();
    if rendered.len() > MAX_LEN {
        rendered.truncate(MAX_LEN);
        rendered.push_str("...");
    }
    rendered
}

fn client_model_allowed(
    document: &RegistryDocument,
    allowed_model_groups: &[String],
    model: &str,
) -> bool {
    if allowed_model_groups.is_empty() {
        return true;
    }
    allowed_model_groups.iter().any(|allowed| {
        allowed == model
            || document
                .model_groups
                .get(allowed)
                .is_some_and(|group| group.models.iter().any(|member| member == model))
    })
}

fn route_target_visible(
    document: &RegistryDocument,
    target: &crate::config::ModelRouteTargetConfig,
    allowed_channels: &[String],
) -> bool {
    if !target.enabled {
        return false;
    }
    if !allowed_channels.is_empty()
        && !allowed_channels
            .iter()
            .any(|allowed| allowed == &target.channel)
    {
        return false;
    }
    document
        .pools
        .get(&target.channel)
        .is_some_and(|pool| pool.enabled)
}

fn render_visibility_preview(previews: &[ModelVisibilityPreview]) -> String {
    if previews.is_empty() {
        return "none".to_string();
    }
    previews
        .iter()
        .map(|preview| {
            let visible_models = if preview.visible_models.is_empty() {
                "<none>".to_string()
            } else {
                preview.visible_models.join(",")
            };
            format!(
                "{} [{}] {}",
                preview.client_token_ref, preview.reason_code, visible_models
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn resource_counts(document: &RegistryDocument) -> BTreeMap<String, usize> {
    BTreeMap::from([
        ("client_tokens".to_string(), document.client_tokens.len()),
        (
            "credential_sets".to_string(),
            document.credential_sets.len(),
        ),
        ("model_routes".to_string(), document.model_routes.len()),
        ("pools".to_string(), document.pools.len()),
        ("providers".to_string(), document.providers.len()),
    ])
}

#[derive(Default)]
struct DeprecatedConfigDiagnostics {
    fields: Vec<String>,
    templates: Vec<String>,
    warnings: Vec<String>,
}

fn deprecated_config_diagnostics(path: &Path) -> DeprecatedConfigDiagnostics {
    let Ok(raw) = fs::read_to_string(path) else {
        return DeprecatedConfigDiagnostics::default();
    };
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(&raw) else {
        return DeprecatedConfigDiagnostics::default();
    };
    deprecated_config_diagnostics_from_yaml(&value)
}

fn deprecated_config_diagnostics_from_yaml(
    value: &serde_yaml::Value,
) -> DeprecatedConfigDiagnostics {
    let mut fields = BTreeSet::new();
    if yaml_sequence_mappings_at(value, "client_tokens")
        .iter()
        .any(|client_token| yaml_mapping_contains_key(client_token, "allowed_model_groups"))
    {
        fields.insert("client_tokens.allowed_model_groups".to_string());
    }

    let mut warnings = Vec::new();
    if fields.contains("client_tokens.allowed_model_groups") {
        warnings.push("deprecated_config_field: client_tokens.allowed_model_groups is a legacy model-scope field name; check_config preserves current scope semantics".to_string());
    }

    DeprecatedConfigDiagnostics {
        fields: fields.into_iter().collect(),
        templates: Vec::new(),
        warnings,
    }
}

fn yaml_sequence_mappings_at<'a>(
    value: &'a serde_yaml::Value,
    key: &str,
) -> Vec<&'a serde_yaml::Mapping> {
    value
        .as_mapping()
        .and_then(|mapping| mapping.get(serde_yaml::Value::String(key.to_string())))
        .and_then(serde_yaml::Value::as_sequence)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_yaml::Value::as_mapping)
                .collect()
        })
        .unwrap_or_default()
}

fn yaml_mapping_contains_key(mapping: &serde_yaml::Mapping, key: &str) -> bool {
    mapping.contains_key(serde_yaml::Value::String(key.to_string()))
}

fn merge_warnings(mut left: Vec<String>, right: Vec<String>) -> Vec<String> {
    left.extend(right);
    left.sort();
    left.dedup();
    left
}

fn redacted_error_summary(message: &str) -> String {
    if message.contains("diagnostic key file") {
        "local credential file is missing or unreadable".to_string()
    } else if message.contains("unknown") {
        "configuration contains an unsupported field or value".to_string()
    } else {
        "configuration validation failed".to_string()
    }
}

fn safe_display_path(path: &Path) -> String {
    if path.is_relative() {
        return path.display().to_string();
    }
    path.file_name()
        .map(|name| format!("<absolute-path-redacted>/{}", name.to_string_lossy()))
        .unwrap_or_else(|| "<absolute-path-redacted>".to_string())
}

struct DiagnosticCredentialRepository;

impl CredentialRepository for DiagnosticCredentialRepository {
    fn load_credential_set(
        &self,
        _credential_set_id: &CredentialSetId,
        source: &CredentialSetSource,
    ) -> anyhow::Result<KeyImport> {
        let CredentialSetSource::File { path } = source;
        let raw = fs::read_to_string(path)
            .map_err(|error| anyhow::anyhow!("diagnostic key file read failed: {error}"))?;
        let mut credentials = Vec::new();
        let mut non_empty_count = 0usize;
        for (index, line) in raw.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            non_empty_count += 1;
            credentials.push(ImportedCredential {
                secret: format!("diagnostic-placeholder-key-{}", index + 1),
                source: CredentialSource {
                    source_path: None,
                    source_line: Some(index + 1),
                    batch_id: None,
                },
            });
        }
        Ok(KeyImport {
            credentials,
            report: KeyImportReport {
                source_path: path.clone(),
                physical_line_count: raw.lines().count(),
                non_empty_count,
                unique_count: non_empty_count,
                duplicate_occurrence_count: 0,
                ignored_empty_count: raw.lines().count().saturating_sub(non_empty_count),
                invalid_line_count: 0,
                claimed_count: None,
                claim_source: None,
                import_generation: 0,
                last_imported_at_unix_seconds: 0,
                duplicate_fingerprints: Vec::new(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvRestore {
        credential_store: Option<std::ffi::OsString>,
        registry_store: Option<std::ffi::OsString>,
    }

    impl EnvRestore {
        fn capture() -> Self {
            Self {
                credential_store: env::var_os("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE"),
                registry_store: env::var_os("KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE"),
            }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            if let Some(value) = &self.credential_store {
                env::set_var("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE", value);
            } else {
                env::remove_var("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE");
            }
            if let Some(value) = &self.registry_store {
                env::set_var("KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE", value);
            } else {
                env::remove_var("KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE");
            }
        }
    }

    fn unique_temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir().join(format!("one-ai-key-check-config-{suffix}"))
    }

    fn write_config(root: &Path, body: &str) -> PathBuf {
        fs::create_dir_all(root).unwrap();
        let path = root.join("config.yaml");
        fs::write(&path, body).unwrap();
        path
    }

    fn valid_config(keys_file: &Path) -> String {
        format!(
            r#"
listen: 127.0.0.1:4101
client_tokens:
  - name: local-client
    token: secret-client-token
management:
  admin_token: secret-management-token
default_pool: relay
default_routing_profile: default-routing
routing_profiles:
  default-routing:
    key_selection: sticky_until_failure
    default_credential_cooldown_seconds: 20
    same_request_credential_retry:
      enabled: false
      max_retries: 0
    route_target_retry:
      enabled: true
upstreams:
  relay:
    template: openai_compatible_bearer
    api_base: https://relay.example/v1
    credential_set: relay_credentials
    keys_file: {}
    models:
      - public_model: gpt-example
        upstream_model: provider/gpt-example
"#,
            serde_yaml::to_string(&keys_file.to_string_lossy().to_string())
                .unwrap()
                .trim()
        )
    }

    fn embedding_route_on_chat_only_pool_config(keys_file: &Path, api_base: &str) -> String {
        format!(
            r#"
listen: 127.0.0.1:4101
client_tokens:
  - name: local-client
    token: secret-client-token
management:
  admin_token: secret-management-token
default_pool: relay
default_routing_profile: default-routing
routing_profiles:
  default-routing:
    key_selection: sticky_until_failure
    default_credential_cooldown_seconds: 20
    same_request_credential_retry:
      enabled: false
      max_retries: 0
    route_target_retry:
      enabled: true
credential_sets:
  relay_credentials:
    keys_file: {}
pools:
  relay:
    provider_kind: openai_compatible
    api_base: {}
    credential_set: relay_credentials
    endpoint_capabilities:
      chat_completions: supported
      responses: unknown
      embeddings: unsupported
      models: local_projection
model_routes:
  text-embedding-3-small:
    strategy: priority
    targets:
      - channel: relay
        upstream_model: text-embedding-3-small
"#,
            serde_yaml::to_string(&keys_file.to_string_lossy().to_string())
                .unwrap()
                .trim(),
            serde_yaml::to_string(&api_base.to_string()).unwrap().trim(),
        )
    }

    #[test]
    fn check_config_valid_config_returns_success_without_secret_output() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();
        env::remove_var("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE");
        env::remove_var("KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE");
        let root = unique_temp_root();
        let keys = root.join("relay.keys");
        fs::create_dir_all(&root).unwrap();
        fs::write(&keys, "synthetic-upstream-key\n").unwrap();
        let config = write_config(&root, &valid_config(&keys));

        let report = check_config(CheckConfigOptions {
            config_path: config,
        });

        assert_eq!(report.status, DiagnosticStatus::Ok);
        assert_eq!(report.exit_code(), 0);
        assert_eq!(report.resource_counts["credential_sets"], 1);
        assert_eq!(report.resource_counts["model_routes"], 1);
        assert_eq!(report.deprecated_fields, Vec::<String>::new());
        assert_eq!(report.deprecated_templates, Vec::<String>::new());
        let rendered = format!("{}\n{}", report.render_table(), report.render_json());
        let json_report: serde_json::Value = serde_json::from_str(&report.render_json()).unwrap();
        assert_eq!(json_report["side_effect_class"], "offline_readonly");
        assert_eq!(json_report["effect_vector"]["reads_local_files"], true);
        assert_eq!(json_report["window"], serde_json::Value::Null);
        assert!(json_report["next_action"]["safe_argv"].is_array());
        assert!(rendered.contains("side_effect_class: offline_readonly"));
        assert!(rendered.contains("effect.reads_local_files: true"));
        assert!(rendered.contains("next_action.safe_argv[0]: one-ai-key"));
        assert!(!rendered.contains("secret-client-token"));
        assert!(!rendered.contains("secret-management-token"));
        assert!(!rendered.contains("synthetic-upstream-key"));
    }

    #[test]
    fn check_config_reports_legacy_client_model_scope_field_without_side_effects() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();
        let root = unique_temp_root();
        let keys = root.join("relay.keys");
        let credential_store = root.join("credential-store.sqlite");
        let registry_store = root.join("registry-store.sqlite");
        fs::create_dir_all(&root).unwrap();
        fs::write(&keys, "synthetic-upstream-key\n").unwrap();
        env::set_var("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE", &credential_store);
        env::set_var("KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE", &registry_store);
        let config_body = valid_config(&keys).replace(
            "    token: secret-client-token",
            "    token: secret-client-token\n    allowed_model_groups:\n      - gpt-example",
        );
        let config = write_config(&root, &config_body);

        let report = check_config(CheckConfigOptions {
            config_path: config,
        });

        assert_eq!(report.status, DiagnosticStatus::Ok);
        assert_eq!(
            report.deprecated_fields,
            vec!["client_tokens.allowed_model_groups".to_string()]
        );
        assert_eq!(report.deprecated_templates, Vec::<String>::new());
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning == "deprecated_config_field: client_tokens.allowed_model_groups is a legacy model-scope field name; check_config preserves current scope semantics"));
        let rendered = format!("{}\n{}", report.render_table(), report.render_json());
        assert!(rendered.contains("deprecated_config_field"));
        assert!(!rendered.contains(&keys.to_string_lossy().to_string()));
        assert!(!rendered.contains("secret-client-token"));
        assert!(!rendered.contains("secret-management-token"));
        assert!(!rendered.contains("synthetic-upstream-key"));
        assert!(!credential_store.exists());
        assert!(!registry_store.exists());
    }

    #[test]
    fn check_config_reports_model_visibility_preview() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();
        env::remove_var("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE");
        env::remove_var("KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE");
        let root = unique_temp_root();
        let keys = root.join("relay.keys");
        fs::create_dir_all(&root).unwrap();
        fs::write(&keys, "synthetic-upstream-key\n").unwrap();
        let config = write_config(&root, &valid_config(&keys));

        let report = check_config(CheckConfigOptions {
            config_path: config,
        });

        assert_eq!(report.status, DiagnosticStatus::Ok);
        assert_eq!(report.model_visibility_preview.len(), 1);
        assert_eq!(
            report.model_visibility_preview[0].client_token_ref,
            "local-client"
        );
        assert_eq!(
            report.model_visibility_preview[0].visible_models,
            vec!["gpt-example".to_string()]
        );
        assert_eq!(
            report.model_visibility_preview[0].reason_code,
            "models_visible"
        );
        let rendered = format!("{}\n{}", report.render_table(), report.render_json());
        assert!(rendered.contains("Model visibility preview"));
        assert!(rendered.contains("local-client"));
        assert!(rendered.contains("gpt-example"));
        assert!(!rendered.contains("secret-client-token"));
    }

    #[test]
    fn check_config_reports_disabled_client_token_without_visible_models() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();
        env::remove_var("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE");
        env::remove_var("KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE");
        let root = unique_temp_root();
        let keys = root.join("relay.keys");
        fs::create_dir_all(&root).unwrap();
        fs::write(&keys, "synthetic-upstream-key\n").unwrap();
        let config_body = valid_config(&keys).replace(
            "    token: secret-client-token",
            "    token: secret-client-token\n    enabled: false",
        );
        let config = write_config(&root, &config_body);

        let report = check_config(CheckConfigOptions {
            config_path: config,
        });

        assert_eq!(report.status, DiagnosticStatus::Ok);
        assert_eq!(report.model_visibility_preview.len(), 1);
        assert_eq!(
            report.model_visibility_preview[0].visible_models,
            Vec::<String>::new()
        );
        assert_eq!(
            report.model_visibility_preview[0].reason_code,
            "client_token_disabled"
        );
    }

    mod endpoint_capability_diagnostics_tests {
        use super::*;

        #[test]
        fn check_config_warns_endpoint_capability_mismatch() {
            let _guard = ENV_LOCK.lock().unwrap();
            let _restore = EnvRestore::capture();
            env::remove_var("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE");
            env::remove_var("KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE");
            let root = unique_temp_root();
            let keys = root.join("relay.keys");
            fs::create_dir_all(&root).unwrap();
            fs::write(&keys, "synthetic-upstream-key\n").unwrap();
            let config = write_config(
                &root,
                &embedding_route_on_chat_only_pool_config(&keys, "https://relay.example/v1"),
            );

            let report = check_config(CheckConfigOptions {
                config_path: config,
            });

            assert_eq!(report.status, DiagnosticStatus::Ok);
            assert_eq!(report.reason_code, "ok");
            assert_eq!(report.warnings.len(), 1);
            assert!(report.warnings[0].contains("endpoint_capability_mismatch"));
            assert!(report.warnings[0].contains("text-embedding-3-small"));
            assert!(report.warnings[0].contains("relay"));

            let json_report: serde_json::Value =
                serde_json::from_str(&report.render_json()).unwrap();
            assert_eq!(
                json_report["warnings"][0],
                serde_json::Value::String(report.warnings[0].clone())
            );
            assert_eq!(json_report["diagnostics_schema_version"], 1);
            assert_eq!(
                json_report["deprecated_fields"],
                serde_json::Value::Array(Vec::new())
            );
            assert_eq!(
                json_report["deprecated_templates"],
                serde_json::Value::Array(Vec::new())
            );
            let table = report.render_table();
            assert!(table.contains("Warnings: 1"));
            assert!(table.contains("warning[0]:"));
            assert!(table.contains("endpoint_capability_mismatch"));
            let rendered = format!("{table}\n{}", report.render_json());
            assert!(!rendered.contains("https://relay.example/v1"));
            assert!(!rendered.contains("secret-client-token"));
            assert!(!rendered.contains("secret-management-token"));
            assert!(!rendered.contains("synthetic-upstream-key"));
        }

        #[test]
        fn check_config_capability_warning_is_offline() {
            let _guard = ENV_LOCK.lock().unwrap();
            let _restore = EnvRestore::capture();
            let root = unique_temp_root();
            let keys = root.join("relay.keys");
            let credential_store = root.join("credential-store.sqlite");
            let registry_store = root.join("registry-store.sqlite");
            fs::create_dir_all(&root).unwrap();
            fs::write(&keys, "synthetic-upstream-key\n").unwrap();
            env::set_var("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE", &credential_store);
            env::set_var("KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE", &registry_store);
            let config = write_config(
                &root,
                &embedding_route_on_chat_only_pool_config(&keys, "https://relay.example/v1"),
            );

            let report = check_config(CheckConfigOptions {
                config_path: config,
            });
            let json_report: serde_json::Value =
                serde_json::from_str(&report.render_json()).unwrap();

            assert_eq!(report.status, DiagnosticStatus::Ok);
            assert_eq!(json_report["side_effect_class"], "offline_readonly");
            assert_eq!(json_report["effect_vector"]["reads_local_files"], true);
            assert_eq!(
                json_report["effect_vector"]["reads_management_runtime"],
                false
            );
            assert_eq!(
                json_report["effect_vector"]["reads_management_store"],
                false
            );
            assert_eq!(json_report["effect_vector"]["calls_upstream"], false);
            assert!(!credential_store.exists());
            assert!(!registry_store.exists());
        }

        #[test]
        fn check_config_capability_warning_does_not_probe_upstream() {
            let _guard = ENV_LOCK.lock().unwrap();
            let _restore = EnvRestore::capture();
            env::remove_var("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE");
            env::remove_var("KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE");
            let root = unique_temp_root();
            let keys = root.join("relay.keys");
            fs::create_dir_all(&root).unwrap();
            fs::write(&keys, "synthetic-upstream-key\n").unwrap();
            let config = write_config(
                &root,
                &embedding_route_on_chat_only_pool_config(&keys, "http://127.0.0.1:1/v1"),
            );

            let report = check_config(CheckConfigOptions {
                config_path: config,
            });

            assert_eq!(report.status, DiagnosticStatus::Ok);
            assert_eq!(report.reason_code, "ok");
            assert!(report.warnings[0].contains("endpoint_capability_mismatch"));
        }
    }

    #[test]
    fn check_config_unknown_field_is_validation_error() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();
        env::remove_var("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE");
        env::remove_var("KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE");
        let root = unique_temp_root();
        let config = write_config(
            &root,
            r#"
listen: 127.0.0.1:4101
unknown_field: true
pools: {}
"#,
        );

        let report = check_config(CheckConfigOptions {
            config_path: config,
        });

        assert_eq!(report.status, DiagnosticStatus::Error);
        assert_eq!(report.reason_code, "config_validation_failed");
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn check_config_missing_key_file_is_local_io_and_does_not_open_sqlite_env_paths() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _restore = EnvRestore::capture();
        let root = unique_temp_root();
        let config = write_config(&root, &valid_config(&root.join("missing.keys")));
        let credential_store = root.join("credential-store.sqlite");
        let registry_store = root.join("registry-store.sqlite");
        env::set_var("KEY_POOL_ROUTER_SQLITE_CREDENTIAL_STORE", &credential_store);
        env::set_var("KEY_POOL_ROUTER_SQLITE_REGISTRY_STORE", &registry_store);

        let report = check_config(CheckConfigOptions {
            config_path: config,
        });

        assert_eq!(report.status, DiagnosticStatus::Error);
        assert_eq!(report.reason_code, "local_io_error");
        assert_eq!(report.exit_code(), 2);
        assert!(!credential_store.exists());
        assert!(!registry_store.exists());
        assert!(!credential_store.with_extension("sqlite-wal").exists());
        assert!(!registry_store.with_extension("sqlite-wal").exists());
    }
}

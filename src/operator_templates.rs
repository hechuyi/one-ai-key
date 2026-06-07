use anyhow::Context;
use std::{
    fs,
    path::{Path, PathBuf},
};

const LOCAL_CONFIG_TEMPLATE_CLIENT_TOKEN: &str = "<client-token-placeholder>";
const LOCAL_CONFIG_TEMPLATE_MANAGEMENT_TOKEN: &str = "<management-token-placeholder>";
const LOCAL_TEMPLATE_KEY_FILE: &str =
    "# Add upstream API keys here, one per line. Do not commit this file.\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitLocalOptions {
    pub out: PathBuf,
    pub keys: PathBuf,
    pub mode: InitLocalMode,
    pub force: bool,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitLocalMode {
    DryRun,
    Write,
    NeedsConfirmation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitLocalReport {
    pub dry_run: bool,
    pub config_path: String,
    pub keys_path: String,
    pub resource_ids: Vec<&'static str>,
}

impl InitLocalReport {
    fn status_code(&self) -> &'static str {
        if self.dry_run {
            "dry_run"
        } else {
            "created"
        }
    }

    fn reason_code(&self) -> &'static str {
        if self.dry_run {
            "init_local_dry_run"
        } else {
            "init_local_created"
        }
    }

    fn effect(&self) -> crate::cli_effects::CommandEffect {
        if self.dry_run {
            crate::cli_effects::CommandEffect {
                side_effect_class: crate::cli_effects::SideEffectClass::OfflineReadonly,
                effect_vector: crate::cli_effects::EffectVector::default(),
            }
        } else {
            crate::cli_effects::CommandEffect {
                side_effect_class: crate::cli_effects::SideEffectClass::LocalWrite,
                effect_vector: crate::cli_effects::EffectVector {
                    writes_local_files: true,
                    ..crate::cli_effects::EffectVector::default()
                },
            }
        }
    }

    fn next_action(&self) -> serde_json::Value {
        if self.dry_run {
            serde_json::json!({
                "summary": "Review the local template plan, then rerun init local with explicit confirmation to write files.",
                "template_id": "init_local_confirmed",
                "safe_argv": ["one-ai-key", "init", "local", "--out", "<config>", "--keys", "<keys>", "--yes"],
                "side_effect_class": "local_write",
                "requires_confirmation": true,
            })
        } else {
            serde_json::json!({
                "summary": "Local template files were created.",
                "template_id": "no_action_required",
                "safe_argv": [],
                "side_effect_class": "offline_readonly",
                "requires_confirmation": false,
            })
        }
    }

    fn report_value(&self) -> serde_json::Value {
        crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
            status: self.status_code(),
            reason: if self.dry_run {
                "Local template write plan was rendered without writing files."
            } else {
                "Local template files were created."
            },
            reason_code: self.reason_code(),
            effect: self.effect(),
            scope: serde_json::json!({
                "config_path": self.config_path,
                "keys_path": self.keys_path,
            }),
            window: serde_json::Value::Null,
            next_action: self.next_action(),
            data: serde_json::json!({
                "dry_run": self.dry_run,
                "config_path": self.config_path,
                "keys_path": self.keys_path,
                "resource_ids": self.resource_ids,
            }),
        })
    }

    pub fn render_json(&self) -> String {
        serde_json::to_string_pretty(&self.report_value()).expect("init-local report should render")
    }

    pub fn render_table(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("Status: {}\n", self.status_code()));
        output.push_str(&format!("Reason code: {}\n", self.reason_code()));
        output.push_str(&format!("Config: {}\n", self.config_path));
        output.push_str(&format!("Keys: {}\n", self.keys_path));
        output.push_str(&format!("Resources: {}\n", self.resource_ids.join(", ")));
        output.push('\n');
        let report = self.report_value();
        crate::cli_report::append_report_envelope_table_fields(&mut output, &report);
        output
    }
}

pub fn init_local(options: InitLocalOptions) -> anyhow::Result<InitLocalReport> {
    if matches!(options.mode, InitLocalMode::NeedsConfirmation) {
        anyhow::bail!("missing required confirmation for local_write command");
    }
    let report = InitLocalReport {
        dry_run: matches!(options.mode, InitLocalMode::DryRun),
        config_path: safe_display_path(&options.out),
        keys_path: safe_display_path(&options.keys),
        resource_ids: vec![
            "local-client",
            "relay",
            "relay_credentials",
            "default-routing",
            "gpt-example",
        ],
    };
    if report.dry_run {
        return Ok(report);
    }

    ensure_can_write(&options.out, options.force)?;
    ensure_can_write(&options.keys, options.force)?;
    if let Some(parent) = options.out.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    if let Some(parent) = options.keys.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&options.out, local_config_yaml(&options.keys))
        .with_context(|| format!("write {}", options.out.display()))?;
    fs::write(&options.keys, LOCAL_TEMPLATE_KEY_FILE)
        .with_context(|| format!("write {}", options.keys.display()))?;
    Ok(report)
}

fn ensure_can_write(path: &Path, force: bool) -> anyhow::Result<()> {
    if path.exists() && !force {
        anyhow::bail!("refusing to overwrite {}", safe_display_path(path));
    }
    Ok(())
}

fn safe_display_path(path: &Path) -> String {
    if path.is_relative() {
        return path.display().to_string();
    }
    path.file_name()
        .map(|name| format!("<absolute-path-redacted>/{}", name.to_string_lossy()))
        .unwrap_or_else(|| "<absolute-path-redacted>".to_string())
}

fn yaml_string(value: &str) -> String {
    serde_yaml::to_string(value)
        .expect("serialize YAML scalar")
        .trim_end()
        .to_string()
}

fn local_config_yaml(keys_file: &Path) -> String {
    let keys_file = yaml_string(&keys_file.to_string_lossy());
    format!(
        r#"listen: 127.0.0.1:4101

client_tokens:
  - name: local-client
    token: {LOCAL_CONFIG_TEMPLATE_CLIENT_TOKEN}

management:
  admin_token: {LOCAL_CONFIG_TEMPLATE_MANAGEMENT_TOKEN}

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
    keys_file: {keys_file}
    models:
      - public_model: gpt-example
        upstream_model: provider/gpt-example
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::AppConfig, upstream_templates::expand_raw_yaml};
    use std::{
        fs,
        path::Path,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_relative_path(prefix: &str, filename: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        PathBuf::from("tmp")
            .join("one-ai-key-test-output")
            .join(format!("{prefix}-{suffix}"))
            .join(filename)
    }

    fn unique_temp_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("one-ai-key-init-local-{suffix}"))
    }

    fn write_options(root: &Path) -> InitLocalOptions {
        InitLocalOptions {
            out: root.join("config/local.yaml"),
            keys: root.join("data/relay.keys"),
            mode: InitLocalMode::Write,
            force: false,
            output: crate::cli_report::OutputFormat::Table,
        }
    }

    #[test]
    fn init_local_dry_run_reports_paths_and_resource_ids_without_writing() {
        let out = unique_relative_path("init-local-dry-run", "config/local.yaml");
        let keys = out
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("data/relay.keys");
        let report = init_local(InitLocalOptions {
            out: out.clone(),
            keys: keys.clone(),
            mode: InitLocalMode::DryRun,
            force: false,
            output: crate::cli_report::OutputFormat::Json,
        })
        .unwrap();

        assert!(!out.exists());
        assert!(!keys.exists());
        let json_report: serde_json::Value =
            serde_json::from_str(&report.render_json()).expect("init-local report json");
        assert_eq!(json_report["status"], "dry_run");
        assert_eq!(json_report["reason_code"], "init_local_dry_run");
        assert_eq!(json_report["side_effect_class"], "offline_readonly");
        assert_eq!(json_report["effect_vector"]["writes_local_files"], false);
        assert!(json_report["scope"]["config_path"]
            .as_str()
            .unwrap()
            .ends_with(&["config", "local.yaml"].join("/")));
        assert_eq!(json_report["next_action"]["safe_argv"][0], "one-ai-key");
        let rendered = report.render_table();
        assert!(rendered.contains("Status: dry_run"));
        assert!(rendered.contains("Reason code: init_local_dry_run"));
        assert!(rendered.contains("side_effect_class: offline_readonly"));
        assert!(rendered.contains("effect.writes_local_files: false"));
        assert!(rendered.contains("scope.config_path:"));
        assert!(rendered.contains("scope.keys_path:"));
        assert!(rendered.contains("next_action.safe_argv[0]: one-ai-key"));
        assert!(rendered.contains("config/local.yaml"));
        assert!(rendered.contains("data/relay.keys"));
        assert!(rendered.contains("relay_credentials"));
        assert!(rendered.contains("gpt-example"));
    }

    #[test]
    fn init_local_write_creates_placeholder_yaml_and_key_file() {
        let root = unique_temp_root();
        let options = write_options(&root);
        let report = init_local(options.clone()).unwrap();

        assert!(!report.dry_run);
        let yaml = fs::read_to_string(&options.out).unwrap();
        let keys = fs::read_to_string(&options.keys).unwrap();
        assert!(yaml.contains("<client-token-placeholder>"));
        assert!(yaml.contains("<management-token-placeholder>"));
        assert!(yaml.contains("openai_compatible_bearer"));
        assert!(yaml.contains("gpt-example"));
        assert!(keys.contains("Add upstream API keys"));
        for forbidden in ["sk-", "secret-config-value", "token hash"] {
            assert!(!yaml.contains(forbidden));
            assert!(!keys.contains(forbidden));
        }

        let expanded = expand_raw_yaml(&yaml).unwrap();
        let cfg: AppConfig = serde_yaml::from_str(&expanded).unwrap();
        assert_eq!(cfg.listen.to_string(), "127.0.0.1:4101");
        assert_eq!(cfg.client_tokens[0].name, "local-client");
        assert!(cfg.pools.contains_key("relay"));
        assert!(cfg.credential_sets.contains_key("relay_credentials"));
        assert!(cfg.model_routes.contains_key("gpt-example"));
    }

    #[test]
    fn init_local_write_refuses_existing_files_without_force() {
        let root = unique_temp_root();
        let options = write_options(&root);
        fs::create_dir_all(options.out.parent().unwrap()).unwrap();
        fs::create_dir_all(options.keys.parent().unwrap()).unwrap();
        fs::write(&options.out, "existing config").unwrap();
        fs::write(&options.keys, "existing key").unwrap();

        let error = init_local(options.clone()).unwrap_err().to_string();

        assert!(error.contains("refusing to overwrite"));
        assert_eq!(fs::read_to_string(&options.out).unwrap(), "existing config");
        assert_eq!(fs::read_to_string(&options.keys).unwrap(), "existing key");
    }

    #[test]
    fn init_local_write_allows_existing_files_with_force() {
        let root = unique_temp_root();
        let mut options = write_options(&root);
        options.force = true;
        fs::create_dir_all(options.out.parent().unwrap()).unwrap();
        fs::create_dir_all(options.keys.parent().unwrap()).unwrap();
        fs::write(&options.out, "existing config").unwrap();
        fs::write(&options.keys, "existing key").unwrap();

        init_local(options.clone()).unwrap();

        assert!(fs::read_to_string(&options.out)
            .unwrap()
            .contains("<client-token-placeholder>"));
        assert!(fs::read_to_string(&options.keys)
            .unwrap()
            .contains("Add upstream API keys"));
    }
}

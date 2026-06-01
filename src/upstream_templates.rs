use std::{collections::HashMap, path::PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

use crate::{
    config::{
        AccountConfig, CredentialSetConfig, ErrorAdaptationActionConfig,
        ErrorAdaptationMatcherConfig, ErrorAdaptationRuleConfig, ErrorRulesConfig,
        ModelRouteConfig, ModelRouteTargetConfig, PolicyProfileConfig, PoolConfig,
        ProbeResultActionConfig, ProviderConfig,
    },
    error::{FailureKind, FailureScope},
    provider::ProviderKind,
};

#[derive(Debug, Clone, Deserialize)]
struct UpstreamShortcutConfig {
    template: String,
    keys_file: PathBuf,
    #[serde(default)]
    api_base: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    credential_set: Option<String>,
    #[serde(default)]
    policy_profile: Option<String>,
    #[serde(default)]
    auth_header: Option<String>,
    #[serde(default)]
    auth_prefix: Option<String>,
    #[serde(default = "crate::config::default_enabled")]
    enabled: bool,
    #[serde(default)]
    models: Vec<UpstreamShortcutModelConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct UpstreamShortcutDocument {
    #[serde(default)]
    upstreams: HashMap<String, UpstreamShortcutConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum UpstreamShortcutModelConfig {
    Name(String),
    Route {
        public_model: String,
        #[serde(default)]
        upstream_model: Option<String>,
    },
}

struct UpstreamShortcutModelRoute {
    public_model: String,
    upstream_model: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct UpstreamCommonAdapter {
    provider_kind: ProviderKind,
    auth_header: &'static str,
    auth_prefix: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamTemplateKind {
    Common(UpstreamCommonAdapterKind),
    ManagedSite(ManagedUpstreamSiteKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamCommonAdapterKind {
    OpenAiCompatibleBearer,
    OpenAiCompatibleApiKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedUpstreamSiteKind {
    Ai2Hhhl,
    DeepseekOfficial,
    XiaomiMimoTokenPlanCn,
}

struct UpstreamSiteOverride {
    provider_id: Option<&'static str>,
    api_base: Option<&'static str>,
    policy_profile_id: Option<&'static str>,
    policy_profile: Option<PolicyProfileConfig>,
    default_models: &'static [&'static str],
}

struct UpstreamTemplateDefaults {
    adapter: UpstreamCommonAdapter,
    site: UpstreamSiteOverride,
}

struct UpstreamCommonAdapterTemplateEntry {
    name: &'static str,
    kind: UpstreamCommonAdapterKind,
}

struct ManagedUpstreamSiteTemplateEntry {
    name: &'static str,
    kind: ManagedUpstreamSiteKind,
    adapter_kind: UpstreamCommonAdapterKind,
}

const OPENAI_COMPATIBLE_BEARER: UpstreamCommonAdapter = UpstreamCommonAdapter {
    provider_kind: ProviderKind::OpenAiCompatible,
    auth_header: "authorization",
    auth_prefix: "Bearer ",
};

const OPENAI_COMPATIBLE_API_KEY: UpstreamCommonAdapter = UpstreamCommonAdapter {
    provider_kind: ProviderKind::OpenAiCompatible,
    auth_header: "api-key",
    auth_prefix: "",
};

const UPSTREAM_COMMON_ADAPTER_REGISTRY: &[UpstreamCommonAdapterTemplateEntry] = &[
    UpstreamCommonAdapterTemplateEntry {
        name: "openai_compatible_bearer",
        kind: UpstreamCommonAdapterKind::OpenAiCompatibleBearer,
    },
    UpstreamCommonAdapterTemplateEntry {
        name: "openai_compatible_api_key",
        kind: UpstreamCommonAdapterKind::OpenAiCompatibleApiKey,
    },
];

const MANAGED_UPSTREAM_SITE_REGISTRY: &[ManagedUpstreamSiteTemplateEntry] = &[
    ManagedUpstreamSiteTemplateEntry {
        name: "ai2_hhhl",
        kind: ManagedUpstreamSiteKind::Ai2Hhhl,
        adapter_kind: UpstreamCommonAdapterKind::OpenAiCompatibleBearer,
    },
    ManagedUpstreamSiteTemplateEntry {
        name: "deepseek_official",
        kind: ManagedUpstreamSiteKind::DeepseekOfficial,
        adapter_kind: UpstreamCommonAdapterKind::OpenAiCompatibleBearer,
    },
    ManagedUpstreamSiteTemplateEntry {
        name: "xiaomi_mimo_token_plan_cn",
        kind: ManagedUpstreamSiteKind::XiaomiMimoTokenPlanCn,
        adapter_kind: UpstreamCommonAdapterKind::OpenAiCompatibleApiKey,
    },
];

pub(crate) fn expand_raw_yaml(raw: &str) -> anyhow::Result<String> {
    let document: UpstreamShortcutDocument = serde_yaml::from_str(raw)?;
    if document.upstreams.is_empty() {
        return Ok(raw.to_string());
    }

    let mut value: Value = serde_yaml::from_str(raw)?;
    expand_in_value(&mut value, document.upstreams)?;
    serde_yaml::to_string(&value).map_err(Into::into)
}

fn expand_in_value(
    value: &mut Value,
    upstreams: HashMap<String, UpstreamShortcutConfig>,
) -> anyhow::Result<()> {
    let root = value
        .as_mapping_mut()
        .ok_or_else(|| anyhow::anyhow!("root config must be a YAML mapping"))?;
    root.remove(Value::String("upstreams".to_string()));

    let mut upstreams = upstreams.into_iter().collect::<Vec<_>>();
    upstreams.sort_by(|(left, _), (right, _)| left.cmp(right));

    for (raw_id, upstream) in upstreams {
        let channel_id = crate::config::normalize_resource_id("upstreams", raw_id)?;
        let template = upstream_template_defaults(&upstream.template).ok_or_else(|| {
            anyhow::anyhow!("unsupported upstream template {}", upstream.template)
        })?;
        let provider_id = upstream
            .provider
            .map(|id| crate::config::normalize_resource_id("upstreams.provider", id))
            .transpose()?
            .or_else(|| template.site.provider_id.map(ToOwned::to_owned))
            .unwrap_or_else(|| format!("{channel_id}_provider"));
        let account_id = upstream
            .account
            .map(|id| crate::config::normalize_resource_id("upstreams.account", id))
            .transpose()?
            .unwrap_or_else(|| format!("{channel_id}_account"));
        let credential_set_id = upstream
            .credential_set
            .map(|id| crate::config::normalize_resource_id("upstreams.credential_set", id))
            .transpose()?
            .unwrap_or_else(|| format!("{channel_id}_credentials"));
        let policy_profile_id = upstream
            .policy_profile
            .map(|id| crate::config::normalize_resource_id("upstreams.policy_profile", id))
            .transpose()?
            .or_else(|| template.site.policy_profile_id.map(ToOwned::to_owned));
        let api_base = upstream
            .api_base
            .or_else(|| template.site.api_base.map(ToOwned::to_owned))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "upstream {channel_id} template {} requires api_base",
                    upstream.template
                )
            })?;
        let auth_header = upstream
            .auth_header
            .unwrap_or_else(|| template.adapter.auth_header.to_string());
        let auth_prefix = upstream
            .auth_prefix
            .unwrap_or_else(|| template.adapter.auth_prefix.to_string());

        insert_if_absent(
            root,
            "providers",
            provider_id.clone(),
            ProviderConfig {
                provider_kind: template.adapter.provider_kind,
                enabled: true,
            },
        )?;
        insert_unique(
            root,
            "accounts",
            account_id.clone(),
            AccountConfig {
                provider: provider_id,
                api_base: api_base.clone(),
                auth_header: auth_header.clone(),
                auth_prefix: auth_prefix.clone(),
                enabled: upstream.enabled,
            },
        )?;
        insert_unique(
            root,
            "credential_sets",
            credential_set_id.clone(),
            CredentialSetConfig {
                keys_file: upstream.keys_file,
            },
        )?;
        if let (Some(policy_profile_id), Some(policy_profile)) =
            (policy_profile_id.as_ref(), template.site.policy_profile)
        {
            insert_if_absent(
                root,
                "policy_profiles",
                policy_profile_id.clone(),
                policy_profile,
            )?;
        }
        insert_unique(
            root,
            "pools",
            channel_id.clone(),
            PoolConfig {
                enabled: upstream.enabled,
                account: Some(account_id),
                policy_profile: policy_profile_id,
                routing_profile: None,
                provider_kind: template.adapter.provider_kind,
                api_base,
                credential_set: credential_set_id,
                auth_header,
                auth_prefix,
                error_rules: ErrorRulesConfig::default(),
            },
        )?;

        let model_routes: Vec<UpstreamShortcutModelRoute> = if upstream.models.is_empty() {
            template
                .site
                .default_models
                .iter()
                .map(|model| UpstreamShortcutModelRoute {
                    public_model: (*model).to_string(),
                    upstream_model: None,
                })
                .collect()
        } else {
            upstream
                .models
                .into_iter()
                .map(UpstreamShortcutModelConfig::into_route)
                .collect::<anyhow::Result<Vec<_>>>()?
        };
        for model_route in model_routes {
            anyhow::ensure!(
                !model_route.public_model.is_empty(),
                "upstream {channel_id} models must not contain empty names"
            );
            insert_model_route_target(
                root,
                model_route.public_model,
                ModelRouteTargetConfig {
                    channel: channel_id.clone(),
                    upstream_model: model_route.upstream_model,
                    priority: crate::config::default_route_target_priority(),
                    weight: crate::config::default_route_target_weight(),
                    enabled: true,
                },
            )?;
        }
    }
    Ok(())
}

impl UpstreamShortcutModelConfig {
    fn into_route(self) -> anyhow::Result<UpstreamShortcutModelRoute> {
        let (public_model, upstream_model) = match self {
            Self::Name(public_model) => (public_model, None),
            Self::Route {
                public_model,
                upstream_model,
            } => (public_model, upstream_model),
        };
        let public_model = public_model.trim().to_string();
        let upstream_model = upstream_model
            .map(|model| {
                let model = model.trim().to_string();
                anyhow::ensure!(!model.is_empty(), "upstream_model must not be empty");
                Ok(model)
            })
            .transpose()?;
        Ok(UpstreamShortcutModelRoute {
            public_model,
            upstream_model,
        })
    }
}

fn upstream_template_defaults(template: &str) -> Option<UpstreamTemplateDefaults> {
    UpstreamTemplateKind::parse(template).map(UpstreamTemplateKind::defaults)
}

impl UpstreamTemplateKind {
    fn parse(template: &str) -> Option<Self> {
        UpstreamCommonAdapterKind::parse(template)
            .map(Self::Common)
            .or_else(|| ManagedUpstreamSiteKind::parse(template).map(Self::ManagedSite))
    }

    fn defaults(self) -> UpstreamTemplateDefaults {
        match self {
            Self::Common(adapter_kind) => {
                upstream_template(adapter_kind.adapter(), UpstreamSiteOverride::empty())
            }
            Self::ManagedSite(site_kind) => site_kind.defaults(),
        }
    }
}

impl UpstreamCommonAdapterKind {
    fn parse(template: &str) -> Option<Self> {
        UPSTREAM_COMMON_ADAPTER_REGISTRY
            .iter()
            .find(|entry| entry.name == template)
            .map(|entry| entry.kind)
    }

    const fn adapter(self) -> UpstreamCommonAdapter {
        match self {
            Self::OpenAiCompatibleBearer => OPENAI_COMPATIBLE_BEARER,
            Self::OpenAiCompatibleApiKey => OPENAI_COMPATIBLE_API_KEY,
        }
    }
}

impl ManagedUpstreamSiteKind {
    fn parse(template: &str) -> Option<Self> {
        MANAGED_UPSTREAM_SITE_REGISTRY
            .iter()
            .find(|entry| entry.name == template)
            .map(|entry| entry.kind)
    }

    fn defaults(self) -> UpstreamTemplateDefaults {
        upstream_template(self.adapter_kind().adapter(), self.site_override())
    }

    fn adapter_kind(self) -> UpstreamCommonAdapterKind {
        self.template_entry().adapter_kind
    }

    fn template_entry(self) -> &'static ManagedUpstreamSiteTemplateEntry {
        MANAGED_UPSTREAM_SITE_REGISTRY
            .iter()
            .find(|entry| entry.kind == self)
            .expect("managed upstream site kind must be present in registry")
    }

    fn site_override(self) -> UpstreamSiteOverride {
        match self {
            Self::Ai2Hhhl => ai2_hhhl_site_override(),
            Self::DeepseekOfficial => deepseek_official_site_override(),
            Self::XiaomiMimoTokenPlanCn => xiaomi_mimo_token_plan_cn_site_override(),
        }
    }
}

fn ai2_hhhl_site_override() -> UpstreamSiteOverride {
    UpstreamSiteOverride {
        provider_id: Some("ai2_hhhl"),
        api_base: Some("https://ai2.hhhl.cc/v1"),
        policy_profile_id: Some("hhhl-openai-compatible"),
        policy_profile: Some(hhhl_policy_profile()),
        default_models: &[],
    }
}

fn deepseek_official_site_override() -> UpstreamSiteOverride {
    UpstreamSiteOverride {
        provider_id: Some("deepseek"),
        api_base: Some("https://api.deepseek.com/v1"),
        policy_profile_id: None,
        policy_profile: None,
        default_models: &["deepseek-chat", "deepseek-reasoner"],
    }
}

fn xiaomi_mimo_token_plan_cn_site_override() -> UpstreamSiteOverride {
    UpstreamSiteOverride {
        provider_id: Some("xiaomi_mimo"),
        api_base: Some("https://token-plan-cn.xiaomimimo.com/v1"),
        policy_profile_id: Some("xiaomi-mimo-token-plan"),
        policy_profile: Some(xiaomi_mimo_policy_profile()),
        default_models: &["mimo-v2.5-pro"],
    }
}

impl UpstreamSiteOverride {
    fn empty() -> Self {
        Self {
            provider_id: None,
            api_base: None,
            policy_profile_id: None,
            policy_profile: None,
            default_models: &[],
        }
    }
}

fn upstream_template(
    adapter: UpstreamCommonAdapter,
    site: UpstreamSiteOverride,
) -> UpstreamTemplateDefaults {
    UpstreamTemplateDefaults { adapter, site }
}

fn insert_unique<T: Serialize>(
    root: &mut Mapping,
    section: &str,
    id: String,
    value: T,
) -> anyhow::Result<()> {
    let section_map = section_mapping_mut(root, section)?;
    let key = Value::String(id.clone());
    anyhow::ensure!(
        !section_map.contains_key(&key),
        "upstream template expansion would overwrite {section}.{id}"
    );
    section_map.insert(key, serde_yaml::to_value(value)?);
    Ok(())
}

fn insert_model_route_target(
    root: &mut Mapping,
    model: String,
    target: ModelRouteTargetConfig,
) -> anyhow::Result<()> {
    let section_map = section_mapping_mut(root, "model_routes")?;
    let key = Value::String(model.clone());
    let Some(existing) = section_map.get_mut(&key) else {
        section_map.insert(
            key,
            serde_yaml::to_value(ModelRouteConfig {
                strategy: Some("priority".to_string()),
                targets: vec![target],
            })?,
        );
        return Ok(());
    };

    let mut route: ModelRouteConfig = serde_yaml::from_value(existing.clone())?;
    anyhow::ensure!(
        !route
            .targets
            .iter()
            .any(|existing_target| existing_target.channel == target.channel),
        "upstream template expansion would duplicate model_routes.{model} target channel {}",
        target.channel
    );
    route.targets.push(target);
    *existing = serde_yaml::to_value(route)?;
    Ok(())
}

fn insert_if_absent<T>(
    root: &mut Mapping,
    section: &str,
    id: String,
    value: T,
) -> anyhow::Result<()>
where
    T: Serialize + DeserializeOwned + PartialEq,
{
    let section_map = section_mapping_mut(root, section)?;
    let key = Value::String(id.clone());
    if let Some(existing) = section_map.get(&key) {
        let existing = serde_yaml::from_value::<T>(existing.clone()).map_err(|err| {
            anyhow::anyhow!(
                "upstream template expansion found invalid existing {section}.{id}: {err}"
            )
        })?;
        anyhow::ensure!(
            existing == value,
            "upstream template expansion would conflict with existing {section}.{id}"
        );
        return Ok(());
    }

    section_map.insert(key, serde_yaml::to_value(value)?);
    Ok(())
}

fn section_mapping_mut<'a>(
    root: &'a mut Mapping,
    section: &str,
) -> anyhow::Result<&'a mut Mapping> {
    let key = Value::String(section.to_string());
    if !root.contains_key(&key) {
        root.insert(key.clone(), Value::Mapping(Mapping::new()));
    }
    root.get_mut(&key)
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| anyhow::anyhow!("{section} must be a YAML mapping"))
}

fn hhhl_policy_profile() -> PolicyProfileConfig {
    PolicyProfileConfig {
        error_rules: ErrorRulesConfig {
            keep_codes: Some(vec!["key_switch_cooldown".to_string()]),
            switch_codes: Some(vec![
                "insufficient_quota".to_string(),
                "quota_exceeded".to_string(),
                "rate_limit_exceeded".to_string(),
                "rate_limit_cooldown".to_string(),
                "billing_hard_limit_reached".to_string(),
            ]),
            expire_codes: Some(vec!["invalid_api_key".to_string()]),
            keep_statuses: None,
            switch_statuses: Some(vec!["429".to_string(), "5xx".to_string()]),
            expire_statuses: Some(vec!["401".to_string(), "403".to_string()]),
            adaptation_rules: vec![
                relay_rate_limit_rule(
                    "relay-rate-limit-cooldown",
                    "rate_limit_cooldown",
                    "cooldown",
                ),
                relay_rate_limit_rule("relay-rate-limit-rpm", "rate_limit_exceeded", "rpm"),
                relay_rate_limit_rule("relay-rate-limit-tpm", "rate_limit_exceeded", "tpm"),
            ],
        },
        probe_result_actions: ProbeResultActionConfig::default(),
    }
}

fn xiaomi_mimo_policy_profile() -> PolicyProfileConfig {
    PolicyProfileConfig {
        error_rules: ErrorRulesConfig {
            keep_codes: None,
            switch_codes: Some(vec![
                "rate_limit_exceeded".to_string(),
                "too_many_requests".to_string(),
            ]),
            expire_codes: Some(vec![
                "invalid_key".to_string(),
                "invalid_api_key".to_string(),
            ]),
            keep_statuses: None,
            switch_statuses: Some(vec!["429".to_string(), "5xx".to_string()]),
            expire_statuses: Some(vec!["401".to_string(), "403".to_string()]),
            adaptation_rules: Vec::new(),
        },
        probe_result_actions: ProbeResultActionConfig::default(),
    }
}

fn relay_rate_limit_rule(id: &str, code: &str, limit_type: &str) -> ErrorAdaptationRuleConfig {
    ErrorAdaptationRuleConfig {
        id: id.to_string(),
        enabled: true,
        matcher: ErrorAdaptationMatcherConfig {
            codes: vec![code.to_string()],
            limit_types: vec![limit_type.to_string()],
            statuses: Vec::new(),
        },
        action: ErrorAdaptationActionConfig {
            kind: Some(FailureKind::RateLimited),
            primary_scope: Some(FailureScope::Credential),
            retryable: Some(true),
            cooldown_seconds: Some(20),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::AppConfig, test_fixtures::fixtures};

    fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
        let start = source.find(signature).unwrap();
        let body_start = start + source[start..].find('{').unwrap();
        let mut depth = 0usize;
        for (offset, ch) in source[body_start..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[body_start + 1..body_start + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("function body for {signature} is not closed");
    }

    fn local_fixture_config_yaml(body: &str) -> String {
        let fixture_values = fixtures();
        format!(
            r#"
listen: 127.0.0.1:4101
client_tokens:
  - name: local-client
    token: {}
management:
  admin_token: {}
{}"#,
            fixture_values.upstream_credentials.local_router,
            fixture_values.upstream_credentials.local_admin,
            body.trim_start_matches('\n')
        )
    }

    #[test]
    fn upstream_template_defaults_delegates_template_ids_to_typed_parser() {
        let body = function_body(
            include_str!("upstream_templates.rs"),
            "fn upstream_template_defaults",
        );

        assert!(body.contains("UpstreamTemplateKind::parse(template)"));
        for template_token in [
            "openai_compatible_bearer",
            "openai_compatible_api_key",
            "ai2_hhhl",
            "deepseek_official",
            "xiaomi_mimo_token_plan_cn",
            "ai2.hhhl.cc",
            "api.deepseek.com",
            "xiaomimimo.com",
        ] {
            assert!(
                !body.contains(template_token),
                "upstream_template_defaults still contains template token {template_token}"
            );
        }
    }

    #[test]
    fn upstream_template_expands_xiaomi_token_plan_channel() {
        let raw = local_fixture_config_yaml(
            r#"
default_pool: xiaomi_mimo_token_plan_cn
upstreams:
  xiaomi_mimo_token_plan_cn:
    template: xiaomi_mimo_token_plan_cn
    credential_set: xiaomi_token_plan
    keys_file: data/xiaomi-token-plan-key.txt
"#,
        );

        let expanded = expand_raw_yaml(&raw).unwrap();
        let cfg: AppConfig = serde_yaml::from_str(&expanded).unwrap();

        assert_eq!(
            cfg.providers["xiaomi_mimo"].provider_kind,
            ProviderKind::OpenAiCompatible
        );
        assert_eq!(
            cfg.accounts["xiaomi_mimo_token_plan_cn_account"].api_base,
            "https://token-plan-cn.xiaomimimo.com/v1"
        );
        assert_eq!(
            cfg.accounts["xiaomi_mimo_token_plan_cn_account"].auth_header,
            "api-key"
        );
        assert_eq!(
            cfg.accounts["xiaomi_mimo_token_plan_cn_account"].auth_prefix,
            ""
        );
        assert_eq!(
            cfg.pools["xiaomi_mimo_token_plan_cn"].credential_set,
            "xiaomi_token_plan"
        );
        assert_eq!(
            cfg.model_routes["mimo-v2.5-pro"].targets[0].channel,
            "xiaomi_mimo_token_plan_cn"
        );
        assert!(cfg.policy_profiles.contains_key("xiaomi-mimo-token-plan"));
    }

    #[test]
    fn upstream_template_expands_openai_compatible_api_key_channel() {
        let raw = local_fixture_config_yaml(
            r#"
default_pool: relay
upstreams:
  relay:
    template: openai_compatible_api_key
    api_base: https://relay.example/v1
    keys_file: data/relay-key.txt
    models:
      - relay-model
"#,
        );

        let expanded = expand_raw_yaml(&raw).unwrap();
        let cfg: AppConfig = serde_yaml::from_str(&expanded).unwrap();

        assert_eq!(
            cfg.providers["relay_provider"].provider_kind,
            ProviderKind::OpenAiCompatible
        );
        assert_eq!(cfg.accounts["relay_account"].auth_header, "api-key");
        assert_eq!(cfg.accounts["relay_account"].auth_prefix, "");
        assert_eq!(
            cfg.accounts["relay_account"].api_base,
            "https://relay.example/v1"
        );
        assert_eq!(cfg.pools["relay"].credential_set, "relay_credentials");
        assert_eq!(cfg.model_routes["relay-model"].targets[0].channel, "relay");
        assert!(!cfg.providers.contains_key("xiaomi_mimo"));
        assert!(!cfg.policy_profiles.contains_key("xiaomi-mimo-token-plan"));
    }

    #[test]
    fn upstream_template_expands_model_alias_target() {
        let raw = local_fixture_config_yaml(
            r#"
default_pool: relay
upstreams:
  relay:
    template: openai_compatible_bearer
    api_base: https://relay.example/v1
    keys_file: data/relay-key.txt
    models:
      - public_model: gpt-5.4-mini
        upstream_model: provider/gpt-5.4-mini
"#,
        );

        let expanded = expand_raw_yaml(&raw).unwrap();
        let cfg: AppConfig = serde_yaml::from_str(&expanded).unwrap();
        let target = &cfg.model_routes["gpt-5.4-mini"].targets[0];

        assert_eq!(target.channel, "relay");
        assert_eq!(
            target.upstream_model.as_deref(),
            Some("provider/gpt-5.4-mini")
        );
    }

    #[test]
    fn upstream_template_treats_missing_upstream_model_as_public_route_only() {
        let raw = local_fixture_config_yaml(
            r#"
default_pool: relay
upstreams:
  relay:
    template: openai_compatible_bearer
    api_base: https://relay.example/v1
    keys_file: data/relay-key.txt
    models:
      - public_model: public-only-model
"#,
        );

        let expanded = expand_raw_yaml(&raw).unwrap();
        let cfg: AppConfig = serde_yaml::from_str(&expanded).unwrap();
        let target = &cfg.model_routes["public-only-model"].targets[0];

        assert_eq!(target.channel, "relay");
        assert_eq!(target.upstream_model, None);
    }

    #[test]
    fn upstream_template_expands_mixed_string_and_structured_models() {
        let raw = local_fixture_config_yaml(
            r#"
default_pool: relay
upstreams:
  relay:
    template: openai_compatible_bearer
    api_base: https://relay.example/v1
    keys_file: data/relay-key.txt
    models:
      - direct-model
      - public_model: aliased-model
        upstream_model: provider/aliased-model
"#,
        );

        let expanded = expand_raw_yaml(&raw).unwrap();
        let cfg: AppConfig = serde_yaml::from_str(&expanded).unwrap();

        let direct_target = &cfg.model_routes["direct-model"].targets[0];
        assert_eq!(direct_target.channel, "relay");
        assert_eq!(direct_target.upstream_model, None);

        let aliased_target = &cfg.model_routes["aliased-model"].targets[0];
        assert_eq!(aliased_target.channel, "relay");
        assert_eq!(
            aliased_target.upstream_model.as_deref(),
            Some("provider/aliased-model")
        );
    }

    #[test]
    fn upstream_template_rejects_empty_model_alias_target() {
        let raw = local_fixture_config_yaml(
            r#"
default_pool: relay
upstreams:
  relay:
    template: openai_compatible_bearer
    api_base: https://relay.example/v1
    keys_file: data/relay-key.txt
    models:
      - public_model: gpt-5.4-mini
        upstream_model: " "
"#,
        );

        let err = expand_raw_yaml(&raw).unwrap_err();

        assert!(err.to_string().contains("upstream_model must not be empty"));
    }

    #[test]
    fn upstream_template_appends_shared_model_targets() {
        let raw = local_fixture_config_yaml(
            r#"
default_pool: relay_a
upstreams:
  relay_a:
    template: openai_compatible_bearer
    api_base: https://relay-a.example/v1
    keys_file: data/relay-a-key.txt
    models:
      - shared-model
  relay_b:
    template: openai_compatible_bearer
    api_base: https://relay-b.example/v1
    keys_file: data/relay-b-key.txt
    models:
      - shared-model
"#,
        );

        let expanded = expand_raw_yaml(&raw).unwrap();
        let cfg: AppConfig = serde_yaml::from_str(&expanded).unwrap();
        let route = &cfg.model_routes["shared-model"];

        assert_eq!(route.targets.len(), 2);
        let channels = route
            .targets
            .iter()
            .map(|target| target.channel.as_str())
            .collect::<Vec<_>>();
        assert_eq!(channels, vec!["relay_a", "relay_b"]);
    }

    #[test]
    fn deepseek_template_separates_site_overrides_from_common_adapter() {
        let template = upstream_template_defaults("deepseek_official").unwrap();

        assert_eq!(
            template.adapter.provider_kind,
            ProviderKind::OpenAiCompatible
        );
        assert_eq!(template.adapter.auth_header, "authorization");
        assert_eq!(template.adapter.auth_prefix, "Bearer ");
        assert_eq!(template.site.provider_id, Some("deepseek"));
        assert_eq!(template.site.api_base, Some("https://api.deepseek.com/v1"));
        assert_eq!(
            template.site.default_models,
            &["deepseek-chat", "deepseek-reasoner"]
        );
    }

    #[test]
    fn upstream_template_rejects_conflicting_managed_provider() {
        let raw = local_fixture_config_yaml(
            r#"
default_pool: deepseek_cn
providers:
  deepseek:
    provider_kind: generic_http
upstreams:
  deepseek_cn:
    template: deepseek_official
    keys_file: data/deepseek-key.txt
"#,
        );

        let err = expand_raw_yaml(&raw).unwrap_err();

        assert!(err
            .to_string()
            .contains("conflict with existing providers.deepseek"));
    }

    #[test]
    fn upstream_template_rejects_conflicting_managed_policy_profile() {
        let raw = local_fixture_config_yaml(
            r#"
default_pool: xiaomi_mimo_token_plan_cn
policy_profiles:
  xiaomi-mimo-token-plan:
    error_rules:
      switch_codes:
        - incompatible_rate_limit
upstreams:
  xiaomi_mimo_token_plan_cn:
    template: xiaomi_mimo_token_plan_cn
    credential_set: xiaomi_token_plan
    keys_file: data/xiaomi-token-plan-key.txt
"#,
        );

        let err = expand_raw_yaml(&raw).unwrap_err();

        assert!(err
            .to_string()
            .contains("conflict with existing policy_profiles.xiaomi-mimo-token-plan"));
    }

    #[test]
    fn upstream_template_rejects_unknown_template() {
        let raw = local_fixture_config_yaml(
            r#"
default_pool: broken
upstreams:
  broken:
    template: unsupported_provider
    keys_file: data/key.txt
"#,
        );

        let err = expand_raw_yaml(&raw).unwrap_err();
        assert!(err.to_string().contains("unsupported upstream template"));
    }
}

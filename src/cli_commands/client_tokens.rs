use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientTokensListOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientTokenMutationMode {
    DryRun,
    NeedsConfirmation,
    Apply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientTokenSecretSource {
    Env(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientTokenCreateOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub name: String,
    pub token_source: ClientTokenSecretSource,
    pub allowed_model_groups: Vec<String>,
    pub unrestricted_model_groups: bool,
    pub allowed_channels: Vec<String>,
    pub unrestricted_channels: bool,
    pub mode: ClientTokenMutationMode,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientTokenSetEnabledOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub token_id: String,
    pub mode: ClientTokenMutationMode,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientTokenScopeUpdateOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub token_id: String,
    pub allowed_model_groups: Vec<String>,
    pub unrestricted_model_groups: bool,
    pub allowed_channels: Vec<String>,
    pub unrestricted_channels: bool,
    pub mode: ClientTokenMutationMode,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Serialize)]
struct ClientTokenCreateRequest {
    name: String,
    token: String,
    allowed_model_groups: Vec<String>,
    allowed_channels: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ClientTokenSetEnabledRequest {}

#[derive(Debug, Serialize)]
struct ClientTokenScopeUpdateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_model_groups: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_channels: Option<Vec<String>>,
}

impl ClientTokenScopeUpdateRequest {
    fn from_options(options: &ClientTokenScopeUpdateOptions) -> Self {
        Self {
            allowed_model_groups: scope_dimension_payload(
                &options.allowed_model_groups,
                options.unrestricted_model_groups,
            ),
            allowed_channels: scope_dimension_payload(
                &options.allowed_channels,
                options.unrestricted_channels,
            ),
        }
    }

    fn is_empty(&self) -> bool {
        self.allowed_model_groups.is_none() && self.allowed_channels.is_none()
    }
}

pub async fn run_list(
    options: ClientTokensListOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let tokens = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::ClientTokens)
        .await?;
    Ok(render_client_tokens_list_report(&tokens, options.output))
}

pub async fn run_create(
    options: ClientTokenCreateOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    validate_allowed_scope_values("allowed_model_groups", &options.allowed_model_groups)?;
    validate_allowed_scope_values("allowed_channels", &options.allowed_channels)?;
    match options.mode {
        ClientTokenMutationMode::DryRun => Ok(render_client_token_create_dry_run_report(&options)),
        ClientTokenMutationMode::NeedsConfirmation => {
            Err(crate::operator_client::OperatorClientError::new(
                "confirmation_required",
                "client-tokens create requires --yes or interactive confirmation",
            ))
        }
        ClientTokenMutationMode::Apply => run_confirmed_create(options).await,
    }
}

pub async fn run_disable(
    options: ClientTokenSetEnabledOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    match options.mode {
        ClientTokenMutationMode::DryRun => Ok(render_client_token_set_enabled_dry_run_report(
            "disable", &options,
        )),
        ClientTokenMutationMode::NeedsConfirmation => {
            Err(crate::operator_client::OperatorClientError::new(
                "confirmation_required",
                "client-tokens disable requires --yes or interactive confirmation",
            ))
        }
        ClientTokenMutationMode::Apply => run_confirmed_set_enabled("disable", options).await,
    }
}

pub async fn run_enable(
    options: ClientTokenSetEnabledOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    match options.mode {
        ClientTokenMutationMode::DryRun => Ok(render_client_token_set_enabled_dry_run_report(
            "enable", &options,
        )),
        ClientTokenMutationMode::NeedsConfirmation => {
            Err(crate::operator_client::OperatorClientError::new(
                "confirmation_required",
                "client-tokens enable requires --yes or interactive confirmation",
            ))
        }
        ClientTokenMutationMode::Apply => run_confirmed_set_enabled("enable", options).await,
    }
}

pub async fn run_scope_update(
    options: ClientTokenScopeUpdateOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    validate_allowed_scope_values("allowed_model_groups", &options.allowed_model_groups)?;
    validate_allowed_scope_values("allowed_channels", &options.allowed_channels)?;
    let payload = ClientTokenScopeUpdateRequest::from_options(&options);
    if payload.is_empty() {
        return Err(crate::operator_client::OperatorClientError::new(
            "client_token_scope_update_empty",
            "client-tokens scope-update requires at least one scope dimension",
        ));
    }
    match options.mode {
        ClientTokenMutationMode::DryRun => {
            Ok(render_client_token_scope_update_dry_run_report(&options))
        }
        ClientTokenMutationMode::NeedsConfirmation => {
            Err(crate::operator_client::OperatorClientError::new(
                "confirmation_required",
                "client-tokens scope-update requires --yes or interactive confirmation",
            ))
        }
        ClientTokenMutationMode::Apply => run_confirmed_scope_update(options, payload).await,
    }
}

async fn run_confirmed_create(
    options: ClientTokenCreateOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let token = options.token_source.read_from_env()?;
    let response = client
        .post_json(
            crate::operator_client::ManagementMutationEndpoint::ClientTokenCreate,
            &ClientTokenCreateRequest {
                name: options.name.clone(),
                token,
                allowed_model_groups: normalized_unique(&options.allowed_model_groups),
                allowed_channels: normalized_unique(&options.allowed_channels),
            },
        )
        .await?;
    Ok(render_client_token_create_apply_report(&options, &response))
}

async fn run_confirmed_set_enabled(
    operation: &'static str,
    options: ClientTokenSetEnabledOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let endpoint = match operation {
        "enable" => crate::operator_client::ManagementMutationEndpoint::ClientTokenEnable {
            token_id: options.token_id.clone(),
        },
        _ => crate::operator_client::ManagementMutationEndpoint::ClientTokenDisable {
            token_id: options.token_id.clone(),
        },
    };
    let response = client
        .post_json(endpoint, &ClientTokenSetEnabledRequest {})
        .await?;
    Ok(render_client_token_set_enabled_apply_report(
        operation, &options, &response,
    ))
}

async fn run_confirmed_scope_update(
    options: ClientTokenScopeUpdateOptions,
    payload: ClientTokenScopeUpdateRequest,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let response = client
        .patch_json(
            crate::operator_client::ManagementMutationEndpoint::ClientTokenScopeUpdate {
                token_id: options.token_id.clone(),
            },
            &payload,
        )
        .await?;
    Ok(render_client_token_scope_update_apply_report(
        &options, &response,
    ))
}

impl ClientTokenSecretSource {
    fn read_from_env(&self) -> Result<String, crate::operator_client::OperatorClientError> {
        match self {
            Self::Env(env_name) => {
                let value = std::env::var(env_name).map_err(|_| {
                    crate::operator_client::OperatorClientError::new(
                        "client_token_env_missing",
                        format!("client token env var {env_name} is not set"),
                    )
                })?;
                let trimmed = value.trim().to_string();
                if trimmed.is_empty() {
                    return Err(crate::operator_client::OperatorClientError::new(
                        "client_token_empty",
                        "client token env var is empty",
                    ));
                }
                Ok(trimmed)
            }
        }
    }

    fn env_name(&self) -> &str {
        match self {
            Self::Env(env_name) => env_name,
        }
    }
}

pub fn render_client_tokens_list_report(
    tokens: &Value,
    output: crate::cli_report::OutputFormat,
) -> String {
    match output {
        crate::cli_report::OutputFormat::Json => render_client_tokens_list_json(tokens),
        crate::cli_report::OutputFormat::Table => render_client_tokens_list_table(tokens),
    }
}

pub fn render_client_token_create_dry_run_report(options: &ClientTokenCreateOptions) -> String {
    let report = client_token_create_report_envelope(
        "dry_run",
        "client_token_create_plan",
        "Client-token create plan was built without reading token material or mutating management state.",
        client_token_effect_for_mode(ClientTokenMutationMode::DryRun),
        options,
        serde_json::json!({
            "command": "client-tokens create",
            "client_token_id": crate::config::stable_id("client", options.name.trim()),
            "name": safe_report_string(&options.name),
            "token_source": "env",
            "token_env": safe_report_string(options.token_source.env_name()),
            "raw_token_read": false,
            "mutating_create_sent": false,
            "allowed_model_groups": normalized_unique(&options.allowed_model_groups),
            "unrestricted_model_groups": effective_unrestricted(&options.allowed_model_groups, options.unrestricted_model_groups),
            "allowed_channels": normalized_unique(&options.allowed_channels),
            "unrestricted_channels": effective_unrestricted(&options.allowed_channels, options.unrestricted_channels),
        }),
        serde_json::json!({
            "summary": "Review the client-token create plan, then re-run with --yes to create it through management.",
            "template_id": "client_tokens_create",
            "safe_argv": ["one-ai-key", "client-tokens", "create", "--name", "<name>", "--token-env", "<env>", "--yes"],
            "side_effect_class": "management_write",
            "requires_confirmation": true,
        }),
    );
    render_client_token_mutation_report(&report, options.output, "Client token create")
}

fn render_client_token_create_apply_report(
    options: &ClientTokenCreateOptions,
    response: &Value,
) -> String {
    let report = client_token_create_report_envelope(
        "ok",
        "client_token_create_applied",
        "Client token was created through management and the response was redacted for CLI output.",
        client_token_effect_for_mode(ClientTokenMutationMode::Apply),
        options,
        serde_json::json!({
            "command": "client-tokens create",
            "token": sanitize_mutation_token(response),
            "token_source": "env",
            "token_env": safe_report_string(options.token_source.env_name()),
            "raw_token_read": true,
            "mutating_create_sent": true,
        }),
        client_tokens_post_mutation_next_action(),
    );
    render_client_token_mutation_report(&report, options.output, "Client token create")
}

fn render_client_token_set_enabled_dry_run_report(
    operation: &'static str,
    options: &ClientTokenSetEnabledOptions,
) -> String {
    let command = format!("client-tokens {operation}");
    let report = client_token_set_enabled_report_envelope(
        "dry_run",
        if operation == "enable" {
            "client_token_enable_plan"
        } else {
            "client_token_disable_plan"
        },
        "Client-token enabled-state plan was built without mutating management state.",
        client_token_effect_for_mode(ClientTokenMutationMode::DryRun),
        options,
        serde_json::json!({
            "command": command,
            "token_id": safe_report_string(&options.token_id),
            "mutating_enable_sent": false,
            "mutating_disable_sent": false,
        }),
        serde_json::json!({
            "summary": format!("Review the client-token {operation} plan, then re-run with --yes to apply it through management."),
            "template_id": format!("client_tokens_{operation}"),
            "safe_argv": ["one-ai-key", "client-tokens", operation, "<client-token-id>", "--yes"],
            "side_effect_class": "management_write",
            "requires_confirmation": true,
        }),
    );
    render_client_token_mutation_report(
        &report,
        options.output,
        if operation == "enable" {
            "Client token enable"
        } else {
            "Client token disable"
        },
    )
}

fn render_client_token_set_enabled_apply_report(
    operation: &'static str,
    options: &ClientTokenSetEnabledOptions,
    response: &Value,
) -> String {
    let report = client_token_set_enabled_report_envelope(
        "ok",
        if operation == "enable" {
            "client_token_enable_applied"
        } else {
            "client_token_disable_applied"
        },
        "Client-token enabled state was updated through management and the response was redacted for CLI output.",
        client_token_effect_for_mode(ClientTokenMutationMode::Apply),
        options,
        serde_json::json!({
            "command": format!("client-tokens {operation}"),
            "token": sanitize_mutation_token(response),
            "token_id": safe_report_string(&options.token_id),
            "mutating_enable_sent": operation == "enable",
            "mutating_disable_sent": operation == "disable",
        }),
        client_tokens_post_mutation_next_action(),
    );
    render_client_token_mutation_report(
        &report,
        options.output,
        if operation == "enable" {
            "Client token enable"
        } else {
            "Client token disable"
        },
    )
}

fn render_client_token_scope_update_dry_run_report(
    options: &ClientTokenScopeUpdateOptions,
) -> String {
    let payload = ClientTokenScopeUpdateRequest::from_options(options);
    let report = client_token_scope_update_report_envelope(
        "dry_run",
        "client_token_scope_update_plan",
        "Client-token scope update plan was built without mutating management state.",
        client_token_effect_for_mode(ClientTokenMutationMode::DryRun),
        options,
        serde_json::json!({
            "command": "client-tokens scope-update",
            "token_id": safe_report_string(&options.token_id),
            "allowed_model_groups": payload.allowed_model_groups,
            "allowed_channels": payload.allowed_channels,
            "unrestricted_model_groups": options.unrestricted_model_groups,
            "unrestricted_channels": options.unrestricted_channels,
            "mutating_scope_update_sent": false,
        }),
        serde_json::json!({
            "summary": "Review the client-token scope update plan, then re-run with --yes to apply it through management.",
            "template_id": "client_tokens_scope_update",
            "safe_argv": ["one-ai-key", "client-tokens", "scope-update", "<client-token-id>", "--allowed-model", "<public-model-or-group>", "--yes"],
            "side_effect_class": "management_write",
            "requires_confirmation": true,
        }),
    );
    render_client_token_mutation_report(&report, options.output, "Client token scope update")
}

fn render_client_token_scope_update_apply_report(
    options: &ClientTokenScopeUpdateOptions,
    response: &Value,
) -> String {
    let report = client_token_scope_update_report_envelope(
        "ok",
        "client_token_scope_update_applied",
        "Client-token scope was updated through management and the response was redacted for CLI output.",
        client_token_effect_for_mode(ClientTokenMutationMode::Apply),
        options,
        serde_json::json!({
            "command": "client-tokens scope-update",
            "token": sanitize_mutation_token(response),
            "token_id": safe_report_string(&options.token_id),
            "mutating_scope_update_sent": true,
        }),
        client_tokens_post_mutation_next_action(),
    );
    render_client_token_mutation_report(&report, options.output, "Client token scope update")
}

fn client_token_create_report_envelope(
    status: &'static str,
    reason_code: &'static str,
    reason: &'static str,
    effect: crate::cli_effects::CommandEffect,
    options: &ClientTokenCreateOptions,
    data: Value,
    next_action: Value,
) -> Value {
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason,
        reason_code,
        effect,
        scope: serde_json::json!({
            "client_token_id": crate::config::stable_id("client", options.name.trim()),
            "name": safe_report_string(&options.name),
        }),
        window: Value::Null,
        next_action,
        data,
    })
}

fn client_token_set_enabled_report_envelope(
    status: &'static str,
    reason_code: &'static str,
    reason: &'static str,
    effect: crate::cli_effects::CommandEffect,
    options: &ClientTokenSetEnabledOptions,
    data: Value,
    next_action: Value,
) -> Value {
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason,
        reason_code,
        effect,
        scope: serde_json::json!({
            "client_token_id": safe_report_string(&options.token_id),
        }),
        window: Value::Null,
        next_action,
        data,
    })
}

fn client_token_scope_update_report_envelope(
    status: &'static str,
    reason_code: &'static str,
    reason: &'static str,
    effect: crate::cli_effects::CommandEffect,
    options: &ClientTokenScopeUpdateOptions,
    data: Value,
    next_action: Value,
) -> Value {
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason,
        reason_code,
        effect,
        scope: serde_json::json!({
            "client_token_id": safe_report_string(&options.token_id),
        }),
        window: Value::Null,
        next_action,
        data,
    })
}

fn render_client_token_mutation_report(
    report: &Value,
    output: crate::cli_report::OutputFormat,
    title: &str,
) -> String {
    match output {
        crate::cli_report::OutputFormat::Json => serde_json::to_string_pretty(report)
            .expect("client-token mutation report should serialize"),
        crate::cli_report::OutputFormat::Table => render_client_token_mutation_table(report, title),
    }
}

fn render_client_token_mutation_table(report: &Value, title: &str) -> String {
    let mut output = String::new();
    output.push_str(title);
    output.push('\n');
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    for field in [
        "command",
        "client_token_id",
        "name",
        "token_source",
        "token_env",
        "raw_token_read",
        "mutating_create_sent",
        "mutating_enable_sent",
        "mutating_disable_sent",
        "mutating_scope_update_sent",
        "unrestricted_model_groups",
        "unrestricted_channels",
    ] {
        crate::cli_report::push_table_field(&mut output, field, report.get(field));
    }
    if let Some(token) = report.get("token") {
        crate::cli_report::push_table_field(&mut output, "token.id", token.get("id"));
        crate::cli_report::push_table_field(&mut output, "token.name", token.get("name"));
        crate::cli_report::push_table_field(&mut output, "token.enabled", token.get("enabled"));
        if let Some(scope) = token.get("scope_summary") {
            crate::cli_report::push_table_field(
                &mut output,
                "token.scope.allowed_model_group_count",
                scope.get("allowed_model_group_count"),
            );
            crate::cli_report::push_table_field(
                &mut output,
                "token.scope.allowed_channel_count",
                scope.get("allowed_channel_count"),
            );
            crate::cli_report::push_table_field(
                &mut output,
                "token.scope.unrestricted_model_groups",
                scope.get("unrestricted_model_groups"),
            );
            crate::cli_report::push_table_field(
                &mut output,
                "token.scope.unrestricted_channels",
                scope.get("unrestricted_channels"),
            );
        }
    }
    output
}

fn sanitize_mutation_token(response: &Value) -> Value {
    response
        .get("token")
        .map(sanitize_client_token)
        .unwrap_or(Value::Null)
}

fn client_tokens_post_mutation_next_action() -> Value {
    serde_json::json!({
        "summary": "Review runtime client-token references after the management mutation.",
        "template_id": "client_tokens_list",
        "safe_argv": ["one-ai-key", "client-tokens", "list", "--management-url", "<url>", "--management-token-env", "<env>"],
        "side_effect_class": "runtime_readonly",
        "requires_confirmation": false,
    })
}

pub fn client_token_effect_for_mode(
    mode: ClientTokenMutationMode,
) -> crate::cli_effects::CommandEffect {
    match mode {
        ClientTokenMutationMode::DryRun => crate::cli_effects::CommandEffect {
            side_effect_class: crate::cli_effects::SideEffectClass::OfflineReadonly,
            effect_vector: crate::cli_effects::EffectVector::default(),
        },
        ClientTokenMutationMode::NeedsConfirmation | ClientTokenMutationMode::Apply => {
            crate::cli_effects::CommandEffect {
                side_effect_class: crate::cli_effects::SideEffectClass::ManagementWrite,
                effect_vector: crate::cli_effects::EffectVector {
                    writes_management_store: true,
                    mutates_runtime: true,
                    ..crate::cli_effects::EffectVector::default()
                },
            }
        }
    }
}

fn render_client_tokens_list_json(tokens: &Value) -> String {
    let report = sanitized_client_tokens_list_report(tokens);
    serde_json::to_string_pretty(&report).expect("client-tokens list json report should serialize")
}

fn sanitized_client_tokens_list_report(tokens: &Value) -> Value {
    let client_tokens = tokens
        .get("client_tokens")
        .and_then(Value::as_array)
        .map(|tokens| tokens.iter().map(sanitize_client_token).collect::<Vec<_>>())
        .unwrap_or_default();
    let disabled_count = client_tokens
        .iter()
        .filter(|token| token.get("enabled").and_then(Value::as_bool) == Some(false))
        .count();
    let status = if client_tokens.is_empty() {
        "empty"
    } else if disabled_count == client_tokens.len() {
        "blocked"
    } else {
        "ok"
    };
    let data = serde_json::json!({
        "command": "client-tokens list",
        "reload_diff_status": crate::cli_report::LIST_RELOAD_STATUS_NOT_EVALUATED,
        "capability_status": crate::cli_report::LIST_CAPABILITY_STATUS_NOT_EVALUATED,
        "token_count": client_tokens.len(),
        "disabled_token_count": disabled_count,
        "client_tokens": client_tokens,
    });
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason: client_tokens_list_reason(status),
        reason_code: client_tokens_list_reason_code(status),
        effect: crate::cli_effects::runtime_readonly_effect(),
        scope: serde_json::json!({"projection": "client_tokens"}),
        window: Value::Null,
        next_action: client_tokens_list_next_action(status),
        data,
    })
}

fn client_tokens_list_reason_code(status: &str) -> &'static str {
    match status {
        "empty" => "no_client_tokens_configured",
        "blocked" => "all_client_tokens_disabled",
        _ => "client_tokens_available",
    }
}

fn client_tokens_list_reason(status: &str) -> &'static str {
    match status {
        "empty" => "No runtime client-token references are available.",
        "blocked" => "All runtime client-token references are disabled.",
        _ => "Runtime client-token references are available.",
    }
}

fn client_tokens_list_next_action(status: &str) -> Value {
    match status {
        "empty" => serde_json::json!({
            "summary": "No runtime client-token references are available. Add configured client tokens, then run check-config. Use reload status or reload diff to inspect staged runtime changes.",
            "template_id": "check_config",
            "safe_argv": ["one-ai-key", "check-config", "--config", "<config>"],
            "side_effect_class": "offline_readonly",
            "requires_confirmation": false,
            "reload_status": crate::cli_report::LIST_RELOAD_STATUS_NOT_EVALUATED,
        }),
        "blocked" => serde_json::json!({
            "summary": "All runtime client-token references are disabled. This command is read-only; update the supported configuration or management workflow, then run check-config. Use reload status or reload diff to inspect staged runtime changes.",
            "template_id": "check_config",
            "safe_argv": ["one-ai-key", "check-config", "--config", "<config>"],
            "side_effect_class": "offline_readonly",
            "requires_confirmation": false,
            "reload_status": crate::cli_report::LIST_RELOAD_STATUS_NOT_EVALUATED,
        }),
        _ => serde_json::json!({
            "summary": "Runtime client-token references are available. Use models explain with a public model and client-token reference to inspect model visibility.",
            "template_id": "models_explain",
            "safe_argv": ["one-ai-key", "models", "explain", "--management-url", "<url>", "--management-token-env", "<env>", "--model", "<public-model>", "--client-token-ref", "<client-token-ref>"],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
    }
}

fn sanitize_client_token(token: &Value) -> Value {
    serde_json::json!({
        "id": token.get("id").and_then(Value::as_str),
        "name": token.get("name").and_then(Value::as_str),
        "enabled": token.get("enabled").and_then(Value::as_bool),
        "scope_summary": {
            "unrestricted_model_groups": token
                .get("unrestricted_model_groups")
                .and_then(Value::as_bool),
            "unrestricted_channels": token
                .get("unrestricted_channels")
                .and_then(Value::as_bool),
            "allowed_model_group_count": token
                .get("allowed_model_groups")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default(),
            "allowed_channel_count": token
                .get("allowed_channels")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default(),
        },
    })
}

fn render_client_tokens_list_table(tokens: &Value) -> String {
    let report = sanitized_client_tokens_list_report(tokens);
    let mut output = String::new();
    output.push_str("Client tokens\n");
    crate::cli_report::append_report_envelope_table_fields(&mut output, &report);
    crate::cli_report::push_table_field(
        &mut output,
        "reload_diff_status",
        report.get("reload_diff_status"),
    );
    crate::cli_report::push_table_field(
        &mut output,
        "capability_status",
        report.get("capability_status"),
    );
    output.push_str(&format!(
        "token_count: {}\n",
        report
            .get("token_count")
            .and_then(Value::as_u64)
            .unwrap_or_default()
    ));
    output.push_str("tokens:\n");
    for token in report
        .get("client_tokens")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let scope = token.get("scope_summary");
        output.push_str(&format!(
            "- id={} name={} enabled={} model_groups={} channels={}\n",
            display_value(
                token
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>")
            ),
            display_value(
                token
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown>")
            ),
            token
                .get("enabled")
                .and_then(Value::as_bool)
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            scope_count_or_unrestricted(
                scope
                    .and_then(|scope| scope.get("unrestricted_model_groups"))
                    .and_then(Value::as_bool),
                scope
                    .and_then(|scope| scope.get("allowed_model_group_count"))
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
            ),
            scope_count_or_unrestricted(
                scope
                    .and_then(|scope| scope.get("unrestricted_channels"))
                    .and_then(Value::as_bool),
                scope
                    .and_then(|scope| scope.get("allowed_channel_count"))
                    .and_then(Value::as_u64)
                    .unwrap_or_default()
            )
        ));
    }
    output
}

fn scope_count_or_unrestricted(unrestricted: Option<bool>, count: u64) -> String {
    if unrestricted == Some(true) {
        "unrestricted".to_string()
    } else {
        count.to_string()
    }
}

fn display_value(value: &str) -> String {
    crate::cli_report::escape_table_value(value)
}

fn safe_report_string(value: &str) -> String {
    value.trim().to_string()
}

fn normalized_unique(values: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if !value.is_empty() && !normalized.iter().any(|existing| existing == value) {
            normalized.push(value.to_string());
        }
    }
    normalized
}

fn validate_allowed_scope_values(
    field: &'static str,
    values: &[String],
) -> Result<(), crate::operator_client::OperatorClientError> {
    if values.iter().any(|value| value.trim().is_empty()) {
        return Err(crate::operator_client::OperatorClientError::new(
            "client_token_scope_value_empty",
            format!("{field} values must not be empty; use the explicit unrestricted flag instead"),
        ));
    }
    Ok(())
}

fn scope_dimension_payload(values: &[String], unrestricted: bool) -> Option<Vec<String>> {
    if unrestricted {
        Some(Vec::new())
    } else {
        let normalized = normalized_unique(values);
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    }
}

fn effective_unrestricted(values: &[String], unrestricted: bool) -> bool {
    unrestricted || normalized_unique(values).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{Path, State},
        http::HeaderMap,
        routing::{patch, post},
        Json, Router,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct CapturedRequests {
        requests: Arc<Mutex<Vec<Value>>>,
    }

    async fn spawn_management_fixture(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{addr}")
    }

    async fn capture_create(
        State(captured): State<CapturedRequests>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        captured.requests.lock().unwrap().push(serde_json::json!({
            "method": "POST",
            "path": "/management/client-tokens",
            "authorization": headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            "body": body,
        }));
        Json(serde_json::json!({
            "token": {
                "id": "client_local-codex",
                "name": "local-codex",
                "enabled": true,
                "token": "SHOULD_NOT_RENDER_CLIENT_TOKEN_VALUE",
                "token_hash": "SHOULD_NOT_RENDER_CLIENT_TOKEN_HASH",
                "unrestricted_model_groups": false,
                "unrestricted_channels": true,
                "allowed_model_groups": ["coding"],
                "allowed_channels": []
            }
        }))
    }

    async fn capture_scope_update(
        State(captured): State<CapturedRequests>,
        Path(token_id): Path<String>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        captured.requests.lock().unwrap().push(serde_json::json!({
            "method": "PATCH",
            "path": format!("/management/client-tokens/{token_id}"),
            "authorization": headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            "body": body,
        }));
        Json(serde_json::json!({
            "token": {
                "id": token_id,
                "name": "local-codex",
                "enabled": true,
                "token": "SHOULD_NOT_RENDER_CLIENT_TOKEN_VALUE",
                "token_hash": "SHOULD_NOT_RENDER_CLIENT_TOKEN_HASH",
                "unrestricted_model_groups": false,
                "unrestricted_channels": true,
                "allowed_model_groups": ["coding"],
                "allowed_channels": []
            }
        }))
    }

    #[test]
    fn client_tokens_list_json_redacts_raw_token_fields_and_reports_scope_summary() {
        let input = serde_json::json!({
            "client_tokens": [
                {
                    "id": "local-client",
                    "name": "Local Client",
                    "enabled": true,
                    "token": "SHOULD_NOT_RENDER_CLIENT_TOKEN_VALUE",
                    "token_hash": "SHOULD_NOT_RENDER_CLIENT_TOKEN_HASH",
                    "unrestricted_model_groups": false,
                    "unrestricted_channels": true,
                    "allowed_model_groups": ["gpt-public", "claude-public"],
                    "allowed_channels": []
                }
            ]
        });

        let rendered =
            render_client_tokens_list_report(&input, crate::cli_report::OutputFormat::Json);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason_code"], "client_tokens_available");
        assert_eq!(report["side_effect_class"], "runtime_readonly");
        assert_eq!(report["effect_vector"]["reads_management_runtime"], true);
        assert_eq!(report["window"], serde_json::Value::Null);
        assert_eq!(
            report["reload_diff_status"],
            "not_evaluated_for_list_use_reload_status"
        );
        assert_eq!(
            report["capability_status"],
            "not_evaluated_for_list_use_models_explain"
        );
        assert_eq!(report["client_tokens"][0]["id"], "local-client");
        let legacy_safe_field = ["safe", "command"].join("_");
        let legacy_dry_run_field = ["dry", "run", "command"].join("_");
        assert!(report["next_action"].get(&legacy_safe_field).is_none());
        assert!(report["next_action"].get(&legacy_dry_run_field).is_none());
        assert_eq!(
            report["next_action"]["safe_argv"],
            serde_json::json!([
                "one-ai-key",
                "models",
                "explain",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
                "--model",
                "<public-model>",
                "--client-token-ref",
                "<client-token-ref>"
            ])
        );
        assert_eq!(
            report["client_tokens"][0]["scope_summary"]["allowed_model_group_count"],
            2
        );
        assert!(rendered.contains("models explain"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_CLIENT_TOKEN_VALUE"));
        assert!(!rendered.contains("token_hash"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_CLIENT_TOKEN_HASH"));
    }

    #[test]
    fn client_tokens_list_table_includes_status_and_no_raw_token_material() {
        let input = serde_json::json!({
            "client_tokens": [
                {
                    "id": "disabled-client",
                    "name": "Disabled Client",
                    "enabled": false,
                    "token": "SHOULD_NOT_RENDER_CLIENT_TOKEN_VALUE",
                    "token_hash": "SHOULD_NOT_RENDER_CLIENT_TOKEN_HASH",
                    "unrestricted_model_groups": true,
                    "unrestricted_channels": false,
                    "allowed_model_groups": [],
                    "allowed_channels": ["primary"]
                }
            ]
        });

        let rendered =
            render_client_tokens_list_report(&input, crate::cli_report::OutputFormat::Table);

        assert!(rendered.contains("status: blocked"));
        assert!(rendered.contains("reason_code: all_client_tokens_disabled"));
        assert!(rendered.contains("side_effect_class: runtime_readonly"));
        assert!(rendered.contains("effect.reads_management_runtime: true"));
        assert!(rendered.contains("capability_status: not_evaluated_for_list_use_models_explain"));
        assert!(!rendered.contains("unavailable_until_m4"));
        assert!(!rendered.contains("until M4"));
        assert!(rendered.contains("next_action.safe_argv[0]: one-ai-key"));
        assert!(rendered.contains("model_groups=unrestricted"));
        assert!(rendered.contains("channels=1"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_CLIENT_TOKEN_VALUE"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_CLIENT_TOKEN_HASH"));
    }

    #[test]
    fn client_tokens_list_table_escapes_control_characters() {
        let input = serde_json::json!({
            "client_tokens": [
                {
                    "id": "local\nclient",
                    "name": "Local\rClient\u{1b}[31m",
                    "enabled": true,
                    "unrestricted_model_groups": true,
                    "unrestricted_channels": true,
                    "allowed_model_groups": [],
                    "allowed_channels": []
                }
            ]
        });

        let rendered =
            render_client_tokens_list_report(&input, crate::cli_report::OutputFormat::Table);

        assert!(rendered.contains("local\\nclient"));
        assert!(rendered.contains("Local\\rClient\\u{1b}[31m"));
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains("local\nclient"));
        assert!(!rendered.contains("Local\rClient"));
    }

    #[tokio::test]
    async fn client_token_create_dry_run_does_not_require_token_env_and_redacts_secret_shape() {
        let options = ClientTokenCreateOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: None,
                deprecated_base_url: None,
                management_token_env: None,
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            name: "local-codex".to_string(),
            token_source: ClientTokenSecretSource::Env("MISSING_CLIENT_TOKEN_ENV".to_string()),
            allowed_model_groups: vec!["coding".to_string()],
            unrestricted_model_groups: false,
            allowed_channels: Vec::new(),
            unrestricted_channels: true,
            mode: ClientTokenMutationMode::DryRun,
            output: crate::cli_report::OutputFormat::Json,
        };

        let rendered = run_create(options)
            .await
            .expect("dry-run must not read token env or build an operator client");
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "dry_run");
        assert_eq!(report["reason_code"], "client_token_create_plan");
        assert_eq!(report["side_effect_class"], "offline_readonly");
        assert_eq!(report["token_env"], "MISSING_CLIENT_TOKEN_ENV");
        assert_eq!(report["raw_token_read"], false);
        assert_eq!(report["mutating_create_sent"], false);
        assert_eq!(
            report["scope"]["client_token_id"],
            crate::config::stable_id("client", "local-codex")
        );
        assert_eq!(
            report["allowed_model_groups"],
            serde_json::json!(["coding"])
        );
        assert_eq!(report["unrestricted_channels"], true);
        assert!(!rendered.contains("token_hash"));
        assert!(!rendered.contains("token_value"));
    }

    #[test]
    fn client_token_mutation_apply_reports_redact_management_response() {
        let options = ClientTokenSetEnabledOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: None,
                deprecated_base_url: None,
                management_token_env: None,
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            token_id: "client_local-codex".to_string(),
            mode: ClientTokenMutationMode::Apply,
            output: crate::cli_report::OutputFormat::Json,
        };
        let response = serde_json::json!({
            "token": {
                "id": "client_local-codex",
                "name": "local-codex",
                "enabled": false,
                "token": "SHOULD_NOT_RENDER_CLIENT_TOKEN_VALUE",
                "token_hash": "SHOULD_NOT_RENDER_CLIENT_TOKEN_HASH",
                "unrestricted_model_groups": false,
                "unrestricted_channels": true,
                "allowed_model_groups": ["coding"],
                "allowed_channels": []
            }
        });

        let rendered = render_client_token_set_enabled_apply_report("disable", &options, &response);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason_code"], "client_token_disable_applied");
        assert_eq!(report["token"]["id"], "client_local-codex");
        assert_eq!(
            report["token"]["scope_summary"]["allowed_model_group_count"],
            1
        );
        assert!(!rendered.contains("SHOULD_NOT_RENDER_CLIENT_TOKEN_VALUE"));
        assert!(!rendered.contains("SHOULD_NOT_RENDER_CLIENT_TOKEN_HASH"));
        assert!(!rendered.contains("token_hash"));
    }

    #[test]
    fn client_token_scope_update_payload_distinguishes_omitted_and_unrestricted_dimensions() {
        let options = ClientTokenScopeUpdateOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: None,
                deprecated_base_url: None,
                management_token_env: None,
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            token_id: "client_local-codex".to_string(),
            allowed_model_groups: vec!["coding".to_string()],
            unrestricted_model_groups: false,
            allowed_channels: Vec::new(),
            unrestricted_channels: true,
            mode: ClientTokenMutationMode::Apply,
            output: crate::cli_report::OutputFormat::Json,
        };

        let payload = ClientTokenScopeUpdateRequest::from_options(&options);
        let encoded = serde_json::to_value(payload).unwrap();

        assert_eq!(
            encoded,
            serde_json::json!({
                "allowed_model_groups": ["coding"],
                "allowed_channels": []
            })
        );
    }

    #[tokio::test]
    async fn confirmed_client_token_create_and_scope_update_send_expected_management_requests() {
        let captured = CapturedRequests::default();
        let router = Router::new()
            .route("/management/client-tokens", post(capture_create))
            .route("/management/client-tokens/:id", patch(capture_scope_update))
            .with_state(captured.clone());
        let management_url = spawn_management_fixture(router).await;
        std::env::set_var("ONE_AI_KEY_TEST_MANAGEMENT_TOKEN", "test-management-token");
        std::env::set_var(
            "ONE_AI_KEY_TEST_NEW_CLIENT_TOKEN",
            "SIMULATED_CLIENT_TOKEN_VALUE",
        );
        let connection = crate::cli::OperatorConnectionOptions {
            management_url: Some(management_url),
            deprecated_base_url: None,
            management_token_env: Some("ONE_AI_KEY_TEST_MANAGEMENT_TOKEN".to_string()),
            management_token_stdin: false,
            timeout_seconds: 10,
        };

        let create_report = run_create(ClientTokenCreateOptions {
            connection: connection.clone(),
            name: "local-codex".to_string(),
            token_source: ClientTokenSecretSource::Env(
                "ONE_AI_KEY_TEST_NEW_CLIENT_TOKEN".to_string(),
            ),
            allowed_model_groups: vec!["coding".to_string()],
            unrestricted_model_groups: false,
            allowed_channels: Vec::new(),
            unrestricted_channels: true,
            mode: ClientTokenMutationMode::Apply,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        let scope_report = run_scope_update(ClientTokenScopeUpdateOptions {
            connection,
            token_id: "client_local-codex".to_string(),
            allowed_model_groups: vec!["coding".to_string()],
            unrestricted_model_groups: false,
            allowed_channels: Vec::new(),
            unrestricted_channels: true,
            mode: ClientTokenMutationMode::Apply,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();

        let requests = captured.requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["method"], "POST");
        assert_eq!(requests[0]["path"], "/management/client-tokens");
        assert_eq!(requests[0]["authorization"], "Bearer test-management-token");
        assert_eq!(
            requests[0]["body"],
            serde_json::json!({
                "name": "local-codex",
                "token": "SIMULATED_CLIENT_TOKEN_VALUE",
                "allowed_model_groups": ["coding"],
                "allowed_channels": []
            })
        );
        assert_eq!(requests[1]["method"], "PATCH");
        assert_eq!(
            requests[1]["path"],
            "/management/client-tokens/client_local-codex"
        );
        assert_eq!(
            requests[1]["body"],
            serde_json::json!({
                "allowed_model_groups": ["coding"],
                "allowed_channels": []
            })
        );
        assert!(!create_report.contains("SIMULATED_CLIENT_TOKEN_VALUE"));
        assert!(!create_report.contains("SHOULD_NOT_RENDER_CLIENT_TOKEN_VALUE"));
        assert!(!create_report.contains("SHOULD_NOT_RENDER_CLIENT_TOKEN_HASH"));
        assert!(!create_report.contains("token_hash"));
        assert!(!scope_report.contains("SHOULD_NOT_RENDER_CLIENT_TOKEN_VALUE"));
        assert!(!scope_report.contains("SHOULD_NOT_RENDER_CLIENT_TOKEN_HASH"));
        assert!(!scope_report.contains("token_hash"));
    }

    #[tokio::test]
    async fn confirmed_client_token_create_validates_management_source_before_reading_token_env() {
        let error = run_create(ClientTokenCreateOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("http://127.0.0.1:1".to_string()),
                deprecated_base_url: None,
                management_token_env: None,
                management_token_stdin: false,
                timeout_seconds: 1,
            },
            name: "local-codex".to_string(),
            token_source: ClientTokenSecretSource::Env(
                "ONE_AI_KEY_TEST_MISSING_CLIENT_TOKEN".to_string(),
            ),
            allowed_model_groups: Vec::new(),
            unrestricted_model_groups: true,
            allowed_channels: Vec::new(),
            unrestricted_channels: true,
            mode: ClientTokenMutationMode::Apply,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .expect_err("missing management token source should be detected before token env read");

        assert_eq!(error.reason_code(), "management_token_source_missing");
    }

    #[tokio::test]
    async fn client_token_create_rejects_explicit_empty_allowed_scope_values() {
        let error = run_create(ClientTokenCreateOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: None,
                deprecated_base_url: None,
                management_token_env: None,
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            name: "local-codex".to_string(),
            token_source: ClientTokenSecretSource::Env("MISSING_CLIENT_TOKEN_ENV".to_string()),
            allowed_model_groups: vec!["   ".to_string()],
            unrestricted_model_groups: false,
            allowed_channels: Vec::new(),
            unrestricted_channels: true,
            mode: ClientTokenMutationMode::DryRun,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .expect_err("explicit empty allowed model value must not become unrestricted");

        assert_eq!(error.reason_code(), "client_token_scope_value_empty");
    }

    #[tokio::test]
    async fn client_token_scope_update_rejects_explicit_empty_allowed_scope_values() {
        let error = run_scope_update(ClientTokenScopeUpdateOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: None,
                deprecated_base_url: None,
                management_token_env: None,
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            token_id: "client_local-codex".to_string(),
            allowed_model_groups: Vec::new(),
            unrestricted_model_groups: true,
            allowed_channels: vec!["\t".to_string()],
            unrestricted_channels: false,
            mode: ClientTokenMutationMode::DryRun,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .expect_err("explicit empty allowed channel value must not be silently ignored");

        assert_eq!(error.reason_code(), "client_token_scope_value_empty");
    }
}

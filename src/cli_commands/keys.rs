use serde::Serialize;
use serde_json::Value;
use std::{collections::HashSet, path::PathBuf};

const PROBE_SUMMARY_UNAVAILABLE: &str = "unavailable_without_summary_projection";
const MAX_CREDENTIAL_REF_LIMIT: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeysCommand {
    List(KeysListOptions),
    Stats(KeysStatsOptions),
    ReplacementPlan(KeysReplacementPlanOptions),
    Import(KeysImportOptions),
    Probe(KeysProbeOptions),
    Disable(KeysDisableOptions),
    ProbeApply(KeysProbeApplyCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysListOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysStatsOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub credential_set_id: Option<String>,
    pub include_credential_refs: bool,
    pub credential_ref_limit: usize,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysReplacementPlanOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub credential_set_id: String,
    pub model: Option<String>,
    pub client_token_ref: Option<String>,
    pub include_credential_refs: bool,
    pub credential_ref_limit: usize,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysImportOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub credential_set_id: String,
    pub source: PathBuf,
    pub mode: KeysImportMode,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeysImportMode {
    DryRun,
    NeedsConfirmation,
    Apply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysProbeOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub credential_set_id: String,
    pub credential_ref: String,
    pub model: String,
    pub kind: KeysProbeKind,
    pub expected_output: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub mode: KeysProbeMode,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum KeysProbeKind {
    ModelRetrieve,
    ChatCompletion,
}

impl KeysProbeKind {
    fn request_code(self) -> &'static str {
        match self {
            Self::ModelRetrieve => "model_retrieve",
            Self::ChatCompletion => "chat_completion",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeysProbeMode {
    DryRun,
    NeedsConfirmation,
    Apply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysDisableOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub credential_set_id: String,
    pub credential_ref: String,
    pub reason: String,
    pub mode: KeysDisableMode,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeysDisableMode {
    DryRun,
    NeedsConfirmation,
    Apply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeysProbeApplyCommand {
    Plan(KeysProbeApplyPlanOptions),
    Apply(KeysProbeApplyApplyOptions),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysProbeApplyPlanOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub credential_set_id: String,
    pub credential_ref: String,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysProbeApplyApplyOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub credential_set_id: String,
    pub credential_ref: String,
    pub probe_result_ref: Option<String>,
    pub mode: KeysProbeApplyMode,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeysProbeApplyMode {
    DryRun,
    NeedsConfirmation,
    Apply,
}

#[derive(Debug, Serialize)]
struct CredentialSetCredentialsImportRequest {
    keys: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    batch_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProbeCredentialRequest {
    model: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
struct DisableCredentialRequest {
    reason: String,
}

#[derive(Debug, Serialize)]
struct ApplyLatestProbeRequest {
    probe_result_ref: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

pub async fn run(
    command: KeysCommand,
) -> Result<String, crate::operator_client::OperatorClientError> {
    match command {
        KeysCommand::List(options) => run_list(options).await,
        KeysCommand::Stats(options) => run_stats(options).await,
        KeysCommand::ReplacementPlan(options) => run_replacement_plan(options).await,
        KeysCommand::Import(options) => run_import(options).await,
        KeysCommand::Probe(options) => run_probe(options).await,
        KeysCommand::Disable(options) => run_disable(options).await,
        KeysCommand::ProbeApply(command) => run_probe_apply(command).await,
    }
}

pub async fn run_list(
    options: KeysListOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let credential_sets = client.get_json(keys_list_endpoint()).await?;
    Ok(render_keys_list_report(&credential_sets, options.output))
}

pub async fn run_stats(
    options: KeysStatsOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let credential_sets = client.get_json(keys_stats_sets_endpoint()).await?;
    let mut report =
        sanitized_keys_stats_report(&credential_sets, options.credential_set_id.as_deref());

    if let Some(set_id) = selected_single_set_id(&report) {
        let operations = client
            .get_json(keys_stats_operations_endpoint(set_id))
            .await?;
        merge_operations_into_stats_report(&mut report, &operations);
    }
    if options.include_credential_refs {
        if let Some(set_id) = options.credential_set_id.as_deref() {
            if selected_single_set_id(&report).is_some() {
                let credentials = client
                    .get_json(keys_stats_credentials_endpoint(
                        set_id,
                        options.credential_ref_limit,
                    ))
                    .await?;
                merge_credential_refs_into_stats_report(&mut report, &credentials);
            }
        }
    }

    Ok(render_sanitized_keys_report(&report, options.output))
}

pub async fn run_replacement_plan(
    options: KeysReplacementPlanOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let credential_sets = client.get_json(keys_stats_sets_endpoint()).await?;
    let initial_report =
        sanitized_keys_stats_report(&credential_sets, Some(&options.credential_set_id));
    let selected_set_exists = selected_single_set_id(&initial_report).is_some();
    let operations = if selected_set_exists {
        Some(
            client
                .get_json(keys_stats_operations_endpoint(&options.credential_set_id))
                .await?,
        )
    } else {
        None
    };
    let credential_refs = if selected_set_exists && options.include_credential_refs {
        Some(
            client
                .get_json(keys_stats_credentials_endpoint(
                    &options.credential_set_id,
                    options.credential_ref_limit,
                ))
                .await?,
        )
    } else {
        None
    };
    let route = if let Some(model) = options.model.as_deref().and_then(safe_probe_model) {
        client
            .get_json(crate::operator_client::ReadOnlyEndpoint::RoutingPreview {
                model: model.to_string(),
                client_token_ref: options
                    .client_token_ref
                    .as_deref()
                    .and_then(safe_local_id)
                    .map(ToOwned::to_owned),
            })
            .await
            .ok()
    } else {
        None
    };

    Ok(render_keys_replacement_plan_report(
        &options,
        &credential_sets,
        operations.as_ref(),
        credential_refs.as_ref(),
        route.as_ref(),
    ))
}

pub async fn run_import(
    options: KeysImportOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    match options.mode {
        KeysImportMode::DryRun => render_keys_import_dry_run_report(&options, None),
        KeysImportMode::NeedsConfirmation => Err(crate::operator_client::OperatorClientError::new(
            "confirmation_required",
            "keys import requires --yes or interactive confirmation",
        )),
        KeysImportMode::Apply => run_confirmed_import(options).await,
    }
}

pub async fn run_probe(
    options: KeysProbeOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    match options.mode {
        KeysProbeMode::DryRun => {
            let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
            let credential_sets = client
                .get_json(crate::operator_client::ReadOnlyEndpoint::CredentialSets)
                .await?;
            Ok(render_keys_probe_dry_run_report(
                &options,
                Some(&credential_sets),
            ))
        }
        KeysProbeMode::NeedsConfirmation => Err(crate::operator_client::OperatorClientError::new(
            "confirmation_required",
            "keys probe requires --yes or interactive confirmation",
        )),
        KeysProbeMode::Apply => run_confirmed_probe(options).await,
    }
}

pub async fn run_disable(
    options: KeysDisableOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    match options.mode {
        KeysDisableMode::DryRun => Ok(render_keys_disable_dry_run_report(&options)),
        KeysDisableMode::NeedsConfirmation => {
            Err(crate::operator_client::OperatorClientError::new(
                "confirmation_required",
                "keys disable requires --yes or interactive confirmation",
            ))
        }
        KeysDisableMode::Apply => run_confirmed_disable(options).await,
    }
}

pub async fn run_probe_apply(
    command: KeysProbeApplyCommand,
) -> Result<String, crate::operator_client::OperatorClientError> {
    match command {
        KeysProbeApplyCommand::Plan(options) => run_probe_apply_plan(options).await,
        KeysProbeApplyCommand::Apply(options) => match options.mode {
            KeysProbeApplyMode::DryRun => run_probe_apply_apply_dry_run(options).await,
            KeysProbeApplyMode::NeedsConfirmation => {
                Err(crate::operator_client::OperatorClientError::new(
                    "confirmation_required",
                    "keys probe-apply apply requires --yes or interactive confirmation",
                ))
            }
            KeysProbeApplyMode::Apply => run_confirmed_probe_apply(options).await,
        },
    }
}

async fn run_probe_apply_plan(
    options: KeysProbeApplyPlanOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let plan = client
        .get_json(keys_probe_apply_plan_endpoint(
            &options.credential_set_id,
            &options.credential_ref,
        ))
        .await?;
    Ok(render_keys_probe_apply_plan_report(
        &options,
        &plan,
        "keys probe-apply plan",
    ))
}

async fn run_probe_apply_apply_dry_run(
    options: KeysProbeApplyApplyOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let plan = client
        .get_json(keys_probe_apply_plan_endpoint(
            &options.credential_set_id,
            &options.credential_ref,
        ))
        .await?;
    let plan_options = options.as_plan_options();
    Ok(render_keys_probe_apply_plan_report_with_expected(
        &plan_options,
        &plan,
        "keys probe-apply apply",
        options.probe_result_ref.as_deref(),
    ))
}

impl KeysProbeApplyApplyOptions {
    fn as_plan_options(&self) -> KeysProbeApplyPlanOptions {
        KeysProbeApplyPlanOptions {
            connection: self.connection.clone(),
            credential_set_id: self.credential_set_id.clone(),
            credential_ref: self.credential_ref.clone(),
            output: self.output,
        }
    }
}

async fn run_confirmed_probe_apply(
    options: KeysProbeApplyApplyOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let Some(probe_result_ref) = options
        .probe_result_ref
        .as_deref()
        .and_then(normalize_probe_result_ref)
    else {
        return Err(crate::operator_client::OperatorClientError::new(
            "probe_result_ref_required",
            "keys probe-apply apply requires --probe-result-ref",
        ));
    };
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let response = client
        .post_json(
            crate::operator_client::ManagementMutationEndpoint::CredentialSetCredentialProbeApply {
                credential_set_id: options.credential_set_id.clone(),
                credential_ref: options.credential_ref.clone(),
            },
            &ApplyLatestProbeRequest {
                probe_result_ref: probe_result_ref.clone(),
                reason: Some("operator probe apply".to_string()),
            },
        )
        .await?;
    Ok(render_keys_probe_apply_apply_report(
        &options,
        &probe_result_ref,
        &response,
    ))
}

async fn run_confirmed_probe(
    options: KeysProbeOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let response = client
        .post_json(
            crate::operator_client::ManagementMutationEndpoint::CredentialSetCredentialProbe {
                credential_set_id: options.credential_set_id.clone(),
                credential_ref: options.credential_ref.clone(),
            },
            &ProbeCredentialRequest {
                model: options.model.clone(),
                kind: options.kind.request_code(),
                expected_output: options.expected_output.clone(),
                timeout_seconds: options.timeout_seconds,
            },
        )
        .await?;
    Ok(render_keys_probe_apply_report(
        &options,
        &response,
        options.output,
    ))
}

async fn run_confirmed_disable(
    options: KeysDisableOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let response = client
        .post_json(
            crate::operator_client::ManagementMutationEndpoint::CredentialSetCredentialDisable {
                credential_set_id: options.credential_set_id.clone(),
                credential_ref: options.credential_ref.clone(),
            },
            &DisableCredentialRequest {
                reason: options.reason.clone(),
            },
        )
        .await?;
    Ok(render_keys_disable_apply_report(&options, &response))
}

async fn run_confirmed_import(
    options: KeysImportOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let runtime = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::ExplainRuntime)
        .await?;
    if !credential_source_writable(&runtime) {
        return Ok(render_keys_import_preflight_blocked_report(
            &options,
            &runtime,
            options.output,
        ));
    }

    let source = read_import_source(&options)?;
    if source.non_empty_lines.is_empty() {
        return Ok(render_keys_import_empty_source_report(&options, &source));
    }

    let response = client
        .post_json(
            crate::operator_client::ManagementMutationEndpoint::CredentialSetCredentialsImport {
                credential_set_id: options.credential_set_id.clone(),
            },
            &CredentialSetCredentialsImportRequest {
                keys: source.lines.clone(),
                batch_id: None,
            },
        )
        .await?;
    Ok(render_keys_import_apply_report(
        &options,
        &source,
        &response,
        options.output,
    ))
}

pub fn keys_list_endpoint() -> crate::operator_client::ReadOnlyEndpoint {
    crate::operator_client::ReadOnlyEndpoint::CredentialSets
}

pub fn keys_stats_sets_endpoint() -> crate::operator_client::ReadOnlyEndpoint {
    crate::operator_client::ReadOnlyEndpoint::CredentialSets
}

pub fn keys_stats_operations_endpoint(
    credential_set_id: &str,
) -> crate::operator_client::ReadOnlyEndpoint {
    crate::operator_client::ReadOnlyEndpoint::CredentialSetOperations {
        credential_set_id: credential_set_id.to_string(),
    }
}

pub fn keys_stats_credentials_endpoint(
    credential_set_id: &str,
    limit: usize,
) -> crate::operator_client::ReadOnlyEndpoint {
    crate::operator_client::ReadOnlyEndpoint::CredentialSetCredentials {
        credential_set_id: credential_set_id.to_string(),
        offset: Some(0),
        limit: Some(bounded_credential_ref_limit(limit)),
    }
}

pub fn keys_probe_apply_plan_endpoint(
    credential_set_id: &str,
    credential_ref: &str,
) -> crate::operator_client::ReadOnlyEndpoint {
    crate::operator_client::ReadOnlyEndpoint::CredentialSetCredentialProbeApplyPlan {
        credential_set_id: credential_set_id.to_string(),
        credential_ref: credential_ref.to_string(),
    }
}

fn bounded_credential_ref_limit(limit: usize) -> usize {
    limit.clamp(1, MAX_CREDENTIAL_REF_LIMIT)
}

pub fn render_keys_list_report(
    credential_sets: &Value,
    output: crate::cli_report::OutputFormat,
) -> String {
    let report = sanitized_keys_list_report(credential_sets);
    render_sanitized_keys_report(&report, output)
}

#[cfg(test)]
pub fn render_keys_stats_report(
    credential_sets: &Value,
    output: crate::cli_report::OutputFormat,
) -> String {
    let report = sanitized_keys_stats_report(credential_sets, None);
    render_sanitized_keys_report(&report, output)
}

fn render_sanitized_keys_report(report: &Value, output: crate::cli_report::OutputFormat) -> String {
    match output {
        crate::cli_report::OutputFormat::Json => {
            serde_json::to_string_pretty(report).expect("keys report should serialize")
        }
        crate::cli_report::OutputFormat::Table => {
            match report.get("command").and_then(Value::as_str) {
                Some("keys import") => render_keys_import_table(report),
                Some("keys probe") => render_keys_probe_table(report),
                Some("keys disable") => render_keys_disable_table(report),
                Some("keys replacement-plan") => render_keys_replacement_plan_table(report),
                Some("keys probe-apply plan") | Some("keys probe-apply apply") => {
                    render_keys_probe_apply_table(report)
                }
                _ => render_keys_table(report),
            }
        }
    }
}

pub fn render_keys_probe_dry_run_report(
    options: &KeysProbeOptions,
    credential_sets: Option<&Value>,
) -> String {
    let set = credential_sets.and_then(|projection| {
        sanitized_credential_sets(projection, Some(&options.credential_set_id))
            .into_iter()
            .next()
    });
    let channel_id = set
        .as_ref()
        .and_then(|set| set.get("channel_ids"))
        .and_then(Value::as_array)
        .and_then(|channels| channels.first())
        .and_then(Value::as_str)
        .and_then(safe_local_id);
    let status = if normalize_credential_ref(&options.credential_ref).is_some() {
        "dry_run"
    } else {
        "blocked"
    };
    let reason_code = if status == "dry_run" {
        "keys_probe_plan"
    } else {
        "credential_ref_invalid"
    };
    let report = keys_probe_report_envelope(
        status,
        reason_code,
        "Credential probe plan was built without sending an upstream request.",
        keys_probe_effect_for_mode(KeysProbeMode::DryRun),
        options,
        serde_json::json!({
            "command": "keys probe",
            "credential_ref": normalize_credential_ref(&options.credential_ref),
            "credential_count": if status == "dry_run" { 1 } else { 0 },
            "channel_id": channel_id,
            "model": safe_probe_model(&options.model),
            "probe_kind": options.kind.request_code(),
            "expected_output_configured": options.expected_output.is_some(),
            "timeout_seconds": options.timeout_seconds,
            "upstream_request_sent": false,
            "probe_evidence_persisted": false,
            "automatic_rollback": false,
            "recovery_path": "no recovery needed for dry-run; confirmed probes persist evidence only",
            "warning": "confirmed probe touches one upstream credential and persists redacted probe evidence",
        }),
        if status == "dry_run" {
            keys_probe_next_action(options)
        } else {
            keys_probe_blocked_next_action("Credential probe requires a non-secret credential_ref.")
        },
    );
    render_sanitized_keys_report(&report, options.output)
}

pub fn render_keys_disable_dry_run_report(options: &KeysDisableOptions) -> String {
    let credential_ref = normalize_credential_ref(&options.credential_ref);
    let status = if credential_ref.is_some() {
        "dry_run"
    } else {
        "blocked"
    };
    let reason_code = if credential_ref.is_some() {
        "keys_disable_plan"
    } else {
        "credential_ref_invalid"
    };
    let report = keys_disable_report_envelope(
        status,
        reason_code,
        "Credential disable plan was built without mutating management state.",
        keys_disable_effect_for_mode(KeysDisableMode::DryRun),
        options,
        serde_json::json!({
            "command": "keys disable",
            "credential_ref": credential_ref,
            "reason_configured": safe_disable_reason(&options.reason).is_some(),
            "mutating_disable_sent": false,
            "upstream_request_sent": false,
            "automatic_rollback": false,
            "recovery_path": "confirmed disable mutates credential lifecycle state through management",
        }),
        if credential_ref.is_some() {
            keys_disable_next_action(options)
        } else {
            keys_disable_blocked_next_action(
                "Credential disable requires a non-secret credential_ref.",
            )
        },
    );
    render_sanitized_keys_report(&report, options.output)
}

fn render_keys_disable_apply_report(options: &KeysDisableOptions, response: &Value) -> String {
    let report = keys_disable_report_envelope(
        "ok",
        "keys_disable_applied",
        "Credential disable was applied through management and the response was redacted for CLI output.",
        keys_disable_effect_for_mode(KeysDisableMode::Apply),
        options,
        serde_json::json!({
            "command": "keys disable",
            "credential_ref": response
                .get("credential_ref")
                .and_then(Value::as_str)
                .and_then(normalize_credential_ref)
                .or_else(|| normalize_credential_ref(&options.credential_ref)),
            "channel_id": response
                .get("channel_id")
                .and_then(Value::as_str)
                .and_then(safe_local_id),
            "selector_generation": response.get("selector_generation").and_then(Value::as_u64),
            "state_kind": response
                .get("state")
                .and_then(|state| state.get("kind"))
                .and_then(Value::as_str)
                .and_then(safe_local_id),
            "reason_configured": safe_disable_reason(&options.reason).is_some(),
            "mutating_disable_sent": true,
            "upstream_request_sent": false,
            "automatic_rollback": false,
            "recovery_path": "review credential-set state after disabling this credential",
        }),
        serde_json::json!({
            "summary": "Review credential-set serving state after disabling the credential.",
            "safe_argv": keys_stats_argv(Some(&options.credential_set_id), true),
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
    );
    render_sanitized_keys_report(&report, options.output)
}

fn keys_disable_report_envelope(
    status: &'static str,
    reason_code: &'static str,
    reason: &'static str,
    effect: crate::cli_effects::CommandEffect,
    options: &KeysDisableOptions,
    data: Value,
    next_action: Value,
) -> Value {
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason,
        reason_code,
        effect,
        scope: serde_json::json!({
            "credential_set_id": safe_local_id(&options.credential_set_id),
            "credential_ref": normalize_credential_ref(&options.credential_ref),
        }),
        window: Value::Null,
        next_action,
        data,
    })
}

fn keys_disable_effect_for_mode(mode: KeysDisableMode) -> crate::cli_effects::CommandEffect {
    match mode {
        KeysDisableMode::DryRun => crate::cli_effects::CommandEffect {
            side_effect_class: crate::cli_effects::SideEffectClass::OfflineReadonly,
            effect_vector: crate::cli_effects::EffectVector::default(),
        },
        KeysDisableMode::NeedsConfirmation | KeysDisableMode::Apply => {
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

fn keys_disable_next_action(options: &KeysDisableOptions) -> Value {
    serde_json::json!({
        "summary": "Re-run with explicit confirmation to disable this single credential.",
        "safe_argv": keys_disable_apply_argv(options),
        "side_effect_class": "management_write",
        "requires_confirmation": true,
        "automatic_rollback": false,
        "recovery_path": "review keys stats after confirmed disable",
    })
}

fn keys_disable_blocked_next_action(summary: &str) -> Value {
    serde_json::json!({
        "summary": summary,
        "safe_argv": Value::Null,
        "side_effect_class": Value::Null,
        "requires_confirmation": false,
        "automatic_rollback": false,
    })
}

fn keys_disable_apply_argv(options: &KeysDisableOptions) -> Value {
    let Some(credential_set_id) = safe_local_id(&options.credential_set_id) else {
        return Value::Null;
    };
    let Some(credential_ref) = normalize_credential_ref(&options.credential_ref) else {
        return Value::Null;
    };
    let Some(reason) = safe_disable_reason(&options.reason) else {
        return Value::Null;
    };
    serde_json::json!([
        "one-ai-key",
        "keys",
        "disable",
        "--credential-set",
        credential_set_id,
        "--credential-ref",
        credential_ref,
        "--reason",
        reason,
        "--yes"
    ])
}

fn safe_disable_reason(reason: &str) -> Option<&str> {
    let trimmed = reason.trim();
    if trimmed.is_empty() || trimmed.len() > 160 {
        return None;
    }
    if !trimmed.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b' ' | b'.' | b',' | b':' | b'_' | b'-' | b'/' | b'(' | b')'
            )
    }) {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("sk-")
        || lower.contains("http")
        || lower.contains("://")
        || looks_like_jwt(trimmed)
        || contains_instruction_marker(&lower)
    {
        return None;
    }
    Some(trimmed)
}

pub fn render_keys_probe_apply_report(
    options: &KeysProbeOptions,
    response: &Value,
    output: crate::cli_report::OutputFormat,
) -> String {
    let report = keys_probe_report_envelope(
        "ok",
        "keys_probe_recorded",
        "Credential probe was executed and redacted probe evidence was persisted.",
        keys_probe_effect_for_mode(KeysProbeMode::Apply),
        options,
        serde_json::json!({
            "command": "keys probe",
            "credential_ref": normalize_credential_ref(&options.credential_ref),
            "channel_id": response
                .get("channel_id")
                .and_then(Value::as_str)
                .and_then(safe_local_id),
            "model": safe_probe_model(&options.model),
            "probe_kind": options.kind.request_code(),
            "expected_output_configured": options.expected_output.is_some(),
            "upstream_request_sent": true,
            "probe_evidence_persisted": true,
            "automatic_rollback": false,
            "recovery_path": "review bounded failure evidence, then use keys stats or route explain before any credential lifecycle correction",
            "probe": sanitize_probe_result(response.get("result")),
        }),
        serde_json::json!({
            "summary": "Review the probe result before applying credential lifecycle changes.",
            "safe_argv": keys_stats_argv(Some(&options.credential_set_id), true),
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
    );
    render_sanitized_keys_report(&report, output)
}

pub fn render_keys_probe_apply_plan_report(
    options: &KeysProbeApplyPlanOptions,
    response: &Value,
    command_label: &'static str,
) -> String {
    render_keys_probe_apply_plan_report_with_expected(options, response, command_label, None)
}

fn render_keys_probe_apply_apply_report(
    options: &KeysProbeApplyApplyOptions,
    probe_result_ref: &str,
    response: &Value,
) -> String {
    let mutation = sanitize_probe_apply_mutation(response.get("mutation"));
    let lifecycle_mutation_applied = !mutation.is_null();
    let plan_options = options.as_plan_options();
    let report = keys_probe_apply_report_envelope(
        "ok",
        "keys_probe_apply_applied",
        "Probe apply workflow applied the planned credential lifecycle action.",
        keys_probe_apply_effect_for_mode(KeysProbeApplyMode::Apply),
        &plan_options,
        serde_json::json!({
            "command": "keys probe-apply apply",
            "credential_ref": normalize_credential_ref(&options.credential_ref),
            "probe_result_ref": normalize_probe_result_ref(probe_result_ref),
            "applied_action": response
                .get("action")
                .and_then(Value::as_str)
                .and_then(safe_probe_apply_action),
            "probe": sanitize_probe_result(response.get("probe")),
            "mutation": mutation,
            "upstream_request_sent": false,
            "mutating_apply_sent": true,
            "lifecycle_mutation_applied": lifecycle_mutation_applied,
            "automatic_rollback": false,
            "recovery_path": "review keys stats and bounded failure evidence if lifecycle state does not match expectation",
        }),
        serde_json::json!({
            "summary": "Review credential-set state after applying probe evidence.",
            "safe_argv": keys_stats_argv(Some(&options.credential_set_id), true),
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
    );
    render_sanitized_keys_report(&report, options.output)
}

fn render_keys_probe_apply_plan_report_with_expected(
    options: &KeysProbeApplyPlanOptions,
    response: &Value,
    command_label: &'static str,
    expected_probe_result_ref: Option<&str>,
) -> String {
    let credential_ref = response
        .get("credential_ref")
        .and_then(Value::as_str)
        .and_then(normalize_credential_ref)
        .or_else(|| normalize_credential_ref(&options.credential_ref));
    let probe_result_ref = response
        .get("probe_result_ref")
        .and_then(Value::as_str)
        .and_then(normalize_probe_result_ref);
    let planned_action = response
        .get("action")
        .and_then(Value::as_str)
        .and_then(safe_probe_apply_action);
    let expected_probe_result_ref = expected_probe_result_ref.and_then(normalize_probe_result_ref);
    let probe_result_ref_matches = expected_probe_result_ref
        .as_ref()
        .is_none_or(|expected| probe_result_ref.as_ref() == Some(expected));
    let (status, reason_code, reason) = if probe_result_ref_matches {
        (
            "dry_run",
            "keys_probe_apply_plan_projected",
            "Probe apply plan was projected without mutating credential lifecycle state.",
        )
    } else {
        (
            "blocked",
            "probe_result_ref_mismatch",
            "The supplied probe_result_ref does not match the latest probe plan.",
        )
    };
    let report = keys_probe_apply_report_envelope(
        status,
        reason_code,
        reason,
        crate::cli_effects::runtime_readonly_store_reads_effect(),
        options,
        serde_json::json!({
            "command": command_label,
            "credential_ref": credential_ref,
            "probe_result_ref": probe_result_ref,
            "expected_probe_result_ref": expected_probe_result_ref,
            "probe_result_ref_matches": probe_result_ref_matches,
            "planned_action": planned_action,
            "probe": sanitize_probe_result(response.get("probe")),
            "upstream_request_sent": false,
            "mutating_apply_sent": false,
            "lifecycle_mutation_applied": false,
            "automatic_rollback": false,
            "recovery_path": "review keys stats and bounded failure evidence before any lifecycle correction",
        }),
        if probe_result_ref_matches {
            keys_probe_apply_next_action(options, probe_result_ref.as_deref())
        } else {
            keys_probe_apply_blocked_next_action(
                "The supplied probe_result_ref is stale; review the current keys stats before retrying.",
            )
        },
    );
    render_sanitized_keys_report(&report, options.output)
}

fn sanitize_probe_apply_mutation(mutation: Option<&Value>) -> Value {
    let Some(mutation) = mutation.filter(|value| !value.is_null()) else {
        return Value::Null;
    };
    serde_json::json!({
        "channel_id": mutation
            .get("channel_id")
            .and_then(Value::as_str)
            .and_then(safe_local_id),
        "credential_set_id": mutation
            .get("credential_set_id")
            .and_then(Value::as_str)
            .and_then(safe_local_id),
        "selector_generation": mutation
            .get("selector_generation")
            .and_then(Value::as_u64),
        "state_kind": mutation
            .get("state")
            .and_then(|state| state.get("kind"))
            .and_then(Value::as_str),
    })
}

fn sanitize_probe_result(result: Option<&Value>) -> Value {
    let Some(result) = result else {
        return Value::Null;
    };
    serde_json::json!({
        "outcome": result.get("outcome").and_then(Value::as_str),
        "channel_id": result
            .get("channel_id")
            .and_then(Value::as_str)
            .and_then(safe_local_id),
        "upstream_status": result.get("upstream_status").and_then(Value::as_u64),
        "upstream_code": result
            .get("upstream_code")
            .and_then(Value::as_str)
            .and_then(safe_local_id),
        "upstream_limit_type": result
            .get("upstream_limit_type")
            .and_then(Value::as_str)
            .and_then(safe_local_id),
        "created_at_unix_seconds": result
            .get("created_at_unix_seconds")
            .and_then(Value::as_i64),
    })
}

fn keys_probe_apply_report_envelope(
    status: &'static str,
    reason_code: &'static str,
    reason: &'static str,
    effect: crate::cli_effects::CommandEffect,
    options: &KeysProbeApplyPlanOptions,
    data: Value,
    next_action: Value,
) -> Value {
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason,
        reason_code,
        effect,
        scope: serde_json::json!({
            "credential_set_id": safe_local_id(&options.credential_set_id),
            "credential_ref": normalize_credential_ref(&options.credential_ref),
        }),
        window: Value::Null,
        next_action,
        data,
    })
}

fn keys_probe_apply_next_action(
    options: &KeysProbeApplyPlanOptions,
    _probe_result_ref: Option<&str>,
) -> Value {
    serde_json::json!({
        "summary": "Review bounded credential state before choosing any lifecycle correction.",
        "safe_argv": keys_stats_argv(Some(&options.credential_set_id), true),
        "side_effect_class": "runtime_readonly",
        "requires_confirmation": false,
        "automatic_rollback": false,
        "recovery_path": "use keys stats with bounded credential refs to inspect current state",
    })
}

fn keys_probe_apply_effect_for_mode(mode: KeysProbeApplyMode) -> crate::cli_effects::CommandEffect {
    match mode {
        KeysProbeApplyMode::DryRun => crate::cli_effects::runtime_readonly_store_reads_effect(),
        KeysProbeApplyMode::NeedsConfirmation | KeysProbeApplyMode::Apply => {
            crate::cli_effects::CommandEffect {
                side_effect_class: crate::cli_effects::SideEffectClass::ManagementWrite,
                effect_vector: crate::cli_effects::EffectVector {
                    reads_management_runtime: true,
                    reads_management_store: true,
                    writes_management_store: true,
                    mutates_runtime: true,
                    ..crate::cli_effects::EffectVector::default()
                },
            }
        }
    }
}

fn keys_probe_apply_blocked_next_action(summary: &str) -> Value {
    serde_json::json!({
        "summary": summary,
        "safe_argv": Value::Null,
        "side_effect_class": "runtime_readonly",
        "requires_confirmation": false,
        "automatic_rollback": false,
    })
}

fn keys_probe_report_envelope(
    status: &'static str,
    reason_code: &'static str,
    reason: &'static str,
    effect: crate::cli_effects::CommandEffect,
    options: &KeysProbeOptions,
    data: Value,
    next_action: Value,
) -> Value {
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason,
        reason_code,
        effect,
        scope: serde_json::json!({
            "credential_set_id": safe_local_id(&options.credential_set_id),
            "credential_ref": normalize_credential_ref(&options.credential_ref),
            "model": safe_probe_model(&options.model),
        }),
        window: Value::Null,
        next_action,
        data,
    })
}

fn keys_probe_effect_for_mode(mode: KeysProbeMode) -> crate::cli_effects::CommandEffect {
    match mode {
        KeysProbeMode::DryRun => crate::cli_effects::runtime_readonly_store_reads_effect(),
        KeysProbeMode::NeedsConfirmation | KeysProbeMode::Apply => {
            crate::cli_effects::CommandEffect {
                side_effect_class: crate::cli_effects::SideEffectClass::UpstreamTouching,
                effect_vector: crate::cli_effects::EffectVector {
                    reads_management_runtime: true,
                    writes_management_store: true,
                    calls_upstream: true,
                    ..crate::cli_effects::EffectVector::default()
                },
            }
        }
    }
}

fn keys_probe_next_action(options: &KeysProbeOptions) -> Value {
    serde_json::json!({
        "summary": "Re-run with explicit confirmation to probe this single credential.",
        "safe_argv": keys_probe_apply_argv(options),
        "side_effect_class": "upstream_touching",
        "requires_confirmation": true,
        "automatic_rollback": false,
        "recovery_path": "review persisted probe evidence with keys stats before any credential lifecycle correction",
    })
}

fn keys_probe_blocked_next_action(summary: &str) -> Value {
    serde_json::json!({
        "summary": summary,
        "safe_argv": Value::Null,
        "side_effect_class": Value::Null,
        "requires_confirmation": false,
    })
}

fn keys_probe_apply_argv(options: &KeysProbeOptions) -> Value {
    let Some(credential_set_id) = safe_local_id(&options.credential_set_id) else {
        return Value::Null;
    };
    let Some(credential_ref) = normalize_credential_ref(&options.credential_ref) else {
        return Value::Null;
    };
    let Some(model) = safe_probe_model(&options.model) else {
        return Value::Null;
    };
    if options.expected_output.is_some() {
        return Value::Null;
    }
    serde_json::json!([
        "one-ai-key",
        "keys",
        "probe",
        "--credential-set",
        credential_set_id,
        "--credential-ref",
        credential_ref,
        "--model",
        model,
        "--kind",
        options.kind.request_code().replace('_', "-"),
        "--yes"
    ])
}

fn safe_probe_model(model: &str) -> Option<&str> {
    if model.is_empty() || model.len() > 128 {
        return None;
    }
    if !model.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/' | b':')
    }) {
        return None;
    }
    let lower = model.to_ascii_lowercase();
    if lower.contains("sk-")
        || lower.contains("http")
        || lower.contains("www.")
        || looks_like_jwt(model)
        || contains_instruction_marker(&lower)
    {
        return None;
    }
    Some(model)
}

pub fn render_keys_import_dry_run_report(
    options: &KeysImportOptions,
    runtime: Option<&Value>,
) -> Result<String, crate::operator_client::OperatorClientError> {
    if let Some(runtime) = runtime {
        if !credential_source_writable(runtime) {
            return Ok(render_keys_import_preflight_blocked_report(
                options,
                runtime,
                options.output,
            ));
        }
    }
    let source = read_import_source(options)?;
    let report = keys_import_report_envelope(
        "dry_run",
        "keys_import_local_preview",
        "Credential import source was inspected locally without sending source secrets to management.",
        keys_import_local_preview_effect(),
        options,
        serde_json::json!({
            "command": "keys import",
            "line_count": source.line_count,
            "non_empty_line_count": source.non_empty_lines.len(),
            "local_duplicate_line_count": source.local_duplicate_line_count,
            "store_duplicate_status": "unknown_local_preview",
            "confirmed_import_route_eligible": true,
            "source_secrets_sent_to_management": false,
            "automatic_rollback": false,
            "recovery_path": "manual credential lifecycle correction plus keys stats",
        }),
        keys_import_next_action("dry_run", options),
    );
    Ok(render_sanitized_keys_report(&report, options.output))
}

struct ImportSource {
    line_count: usize,
    lines: Vec<String>,
    non_empty_lines: Vec<String>,
    local_duplicate_line_count: usize,
}

fn read_import_source(
    options: &KeysImportOptions,
) -> Result<ImportSource, crate::operator_client::OperatorClientError> {
    let raw = std::fs::read_to_string(&options.source).map_err(|_| {
        crate::operator_client::OperatorClientError::new(
            "keys_import_source_read_failed",
            "failed to read credential import source",
        )
    })?;
    let lines = raw.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let mut seen = HashSet::new();
    let mut non_empty_lines = Vec::new();
    let mut local_duplicate_line_count = 0usize;
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !seen.insert(trimmed.to_string()) {
            local_duplicate_line_count += 1;
        }
        non_empty_lines.push(trimmed.to_string());
    }
    Ok(ImportSource {
        line_count: lines.len(),
        lines,
        non_empty_lines,
        local_duplicate_line_count,
    })
}

fn render_keys_import_preflight_blocked_report(
    options: &KeysImportOptions,
    runtime: &Value,
    output: crate::cli_report::OutputFormat,
) -> String {
    let report = keys_import_report_envelope(
        "blocked",
        "credential_store_readonly",
        "Credential import requires a writable credential store and was stopped before reading or sending source secrets.",
        keys_import_effect_for_mode(options.mode),
        options,
        serde_json::json!({
            "command": "keys import",
            "credential_source": {
                "writable": runtime
                    .get("credential_source")
                    .and_then(|source| source.get("writable"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                "source_kind": runtime
                    .get("credential_source")
                    .and_then(|source| source.get("source_kind"))
                    .and_then(Value::as_str),
            },
            "source_secrets_sent_to_management": false,
            "confirmed_import_route_eligible": false,
            "automatic_rollback": false,
            "recovery_path": "configure a writable credential store before importing credentials",
        }),
        keys_import_blocked_next_action(
            "Credential import is blocked until the credential store is writable.",
        ),
    );
    render_sanitized_keys_report(&report, output)
}

fn render_keys_import_empty_source_report(
    options: &KeysImportOptions,
    source: &ImportSource,
) -> String {
    let report = keys_import_report_envelope(
        "blocked",
        "keys_import_source_empty",
        "Credential import source did not contain any non-empty credential lines.",
        keys_import_effect_for_mode(options.mode),
        options,
        serde_json::json!({
            "command": "keys import",
            "line_count": source.line_count,
            "non_empty_line_count": 0,
            "local_duplicate_line_count": source.local_duplicate_line_count,
            "source_secrets_sent_to_management": false,
            "confirmed_import_route_eligible": false,
            "automatic_rollback": false,
            "recovery_path": "provide a source file with at least one non-empty credential line",
        }),
        keys_import_blocked_next_action("Credential import source is empty."),
    );
    render_sanitized_keys_report(&report, options.output)
}

fn render_keys_import_apply_report(
    options: &KeysImportOptions,
    source: &ImportSource,
    response: &Value,
    output: crate::cli_report::OutputFormat,
) -> String {
    let imported_credentials = response
        .get("imported_credentials")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let report = keys_import_report_envelope(
        "ok",
        "keys_import_applied",
        "Credential import was applied through management and the response was redacted for CLI output.",
        keys_import_effect_for_mode(KeysImportMode::Apply),
        options,
        serde_json::json!({
            "command": "keys import",
            "line_count": source.line_count,
            "non_empty_line_count": source.non_empty_lines.len(),
            "local_duplicate_line_count": source.local_duplicate_line_count,
            "source_secrets_sent_to_management": true,
            "confirmed_import_route_eligible": imported_credentials > 0,
            "automatic_rollback": false,
            "recovery_path": "manual credential lifecycle correction plus keys stats",
            "management_response": sanitize_import_response(response),
        }),
        serde_json::json!({
            "summary": "Review credential-set serving state after import.",
            "safe_argv": keys_stats_argv(Some(&options.credential_set_id), true),
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
    );
    render_sanitized_keys_report(&report, output)
}

fn sanitize_import_response(response: &Value) -> Value {
    serde_json::json!({
        "credential_set_id": response
            .get("credential_set_id")
            .and_then(Value::as_str)
            .and_then(safe_local_id),
        "channel_ids": safe_string_array(response.get("channel_ids")),
        "selector_generation": response.get("selector_generation").and_then(Value::as_u64),
        "requested_credentials": response.get("requested_credentials").and_then(Value::as_u64),
        "imported_credentials": response.get("imported_credentials").and_then(Value::as_u64),
        "duplicate_credentials": response.get("duplicate_credentials").and_then(Value::as_u64),
        "ignored_empty_credentials": response
            .get("ignored_empty_credentials")
            .and_then(Value::as_u64),
        "credential_statuses_returned": response
            .get("credentials")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0),
        "operations": sanitize_import_operations(response.get("operations")),
    })
}

fn sanitize_import_operations(operations: Option<&Value>) -> Value {
    let Some(operations) = operations else {
        return Value::Null;
    };
    serde_json::json!({
        "status": operations.get("status").and_then(Value::as_str),
        "serving_mode": operations.get("serving_mode").and_then(Value::as_str),
        "accepting_requests": operations.get("accepting_requests").and_then(Value::as_bool),
        "needs_operator_input": operations
            .get("needs_operator_input")
            .and_then(Value::as_bool),
        "required_action": operations.get("required_action").and_then(Value::as_str),
        "credentials": sanitize_lifecycle_counts(operations.get("credentials")),
    })
}

fn keys_import_report_envelope(
    status: &'static str,
    reason_code: &'static str,
    reason: &'static str,
    effect: crate::cli_effects::CommandEffect,
    options: &KeysImportOptions,
    data: Value,
    next_action: Value,
) -> Value {
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason,
        reason_code,
        effect,
        scope: serde_json::json!({
            "credential_set_id": safe_local_id(&options.credential_set_id),
            "source_ref": "redacted",
        }),
        window: Value::Null,
        next_action,
        data,
    })
}

fn keys_import_effect_for_mode(mode: KeysImportMode) -> crate::cli_effects::CommandEffect {
    match mode {
        KeysImportMode::DryRun => keys_import_local_preview_effect(),
        KeysImportMode::NeedsConfirmation | KeysImportMode::Apply => {
            keys_import_management_write_effect()
        }
    }
}

fn keys_import_local_preview_effect() -> crate::cli_effects::CommandEffect {
    crate::cli_effects::CommandEffect {
        side_effect_class: crate::cli_effects::SideEffectClass::LocalPreview,
        effect_vector: crate::cli_effects::EffectVector {
            reads_local_files: true,
            ..crate::cli_effects::EffectVector::default()
        },
    }
}

fn keys_import_management_write_effect() -> crate::cli_effects::CommandEffect {
    crate::cli_effects::CommandEffect {
        side_effect_class: crate::cli_effects::SideEffectClass::ManagementWrite,
        effect_vector: crate::cli_effects::EffectVector {
            reads_local_files: true,
            writes_management_store: true,
            mutates_runtime: true,
            ..crate::cli_effects::EffectVector::default()
        },
    }
}

fn keys_import_next_action(status: &str, options: &KeysImportOptions) -> Value {
    if status == "dry_run" {
        serde_json::json!({
            "summary": "Re-run with explicit confirmation to import these credentials.",
            "safe_argv": keys_import_apply_argv(options),
            "side_effect_class": "management_write",
            "requires_confirmation": true,
            "automatic_rollback": false,
            "recovery_path": "manual credential lifecycle correction plus keys stats",
        })
    } else {
        keys_import_blocked_next_action("Credential import is blocked.")
    }
}

fn keys_import_blocked_next_action(summary: &str) -> Value {
    serde_json::json!({
        "summary": summary,
        "safe_argv": Value::Null,
        "side_effect_class": Value::Null,
        "requires_confirmation": false,
    })
}

fn keys_import_apply_argv(options: &KeysImportOptions) -> Value {
    let Some(credential_set_id) = safe_local_id(&options.credential_set_id) else {
        return Value::Null;
    };
    let Some(source) = safe_relative_source_arg(&options.source) else {
        return Value::Null;
    };
    serde_json::json!([
        "one-ai-key",
        "keys",
        "import",
        "--credential-set",
        credential_set_id,
        "--source",
        source,
        "--yes"
    ])
}

fn safe_relative_source_arg(source: &std::path::Path) -> Option<String> {
    if source.is_absolute() {
        return None;
    }
    let rendered = source.to_string_lossy();
    if rendered.is_empty()
        || rendered.len() > 160
        || rendered.contains('\\')
        || rendered.contains("..")
        || rendered.to_ascii_lowercase().contains("sk-")
        || rendered.to_ascii_lowercase().contains("http")
        || contains_instruction_marker(&rendered.to_ascii_lowercase())
    {
        return None;
    }
    if rendered
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
    {
        Some(rendered.to_string())
    } else {
        None
    }
}

fn credential_source_writable(runtime: &Value) -> bool {
    runtime
        .get("credential_source")
        .and_then(|source| source.get("writable"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn sanitized_keys_list_report(credential_sets: &Value) -> Value {
    let sets = sanitized_credential_sets(credential_sets, None);
    let status = if sets.is_empty() { "blocked" } else { "ok" };
    let reason_code = if sets.is_empty() {
        "no_credential_sets_projected"
    } else {
        "credential_sets_projected"
    };

    let data = serde_json::json!({
        "command": "keys list",
        "scan_mode": "summary_projection_only",
        "credential_sets": sets,
    });
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason: keys_reason(reason_code),
        reason_code,
        effect: crate::cli_effects::runtime_readonly_store_reads_effect(),
        scope: serde_json::json!({"projection": "credential_sets"}),
        window: Value::Null,
        next_action: keys_next_action(status, None),
        data,
    })
}

fn sanitized_keys_stats_report(credential_sets: &Value, credential_set_id: Option<&str>) -> Value {
    let sets = sanitized_credential_sets(credential_sets, credential_set_id);
    let status = if credential_set_id.is_some() && sets.is_empty() {
        "blocked"
    } else {
        "ok"
    };
    let reason_code = if credential_set_id.is_some() && sets.is_empty() {
        "credential_set_not_projected"
    } else {
        "credential_set_stats_projected"
    };

    let data = serde_json::json!({
        "command": "keys stats",
        "scan_mode": "summary_projection_only",
        "credential_sets": sets,
    });
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason: keys_reason(reason_code),
        reason_code,
        effect: crate::cli_effects::runtime_readonly_store_reads_effect(),
        scope: serde_json::json!({
            "projection": "credential_sets",
            "credential_set_id": credential_set_id.and_then(safe_local_id),
        }),
        window: Value::Null,
        next_action: keys_next_action(status, credential_set_id),
        data,
    })
}

pub fn render_keys_replacement_plan_report(
    options: &KeysReplacementPlanOptions,
    credential_sets: &Value,
    operations: Option<&Value>,
    credential_refs: Option<&Value>,
    route: Option<&Value>,
) -> String {
    let mut stats_report =
        sanitized_keys_stats_report(credential_sets, Some(&options.credential_set_id));
    if let Some(operations) = operations {
        merge_operations_into_stats_report(&mut stats_report, operations);
    }
    if let Some(credential_refs) = credential_refs {
        merge_credential_refs_into_stats_report(&mut stats_report, credential_refs);
    }

    let set = stats_report
        .get("credential_sets")
        .and_then(Value::as_array)
        .and_then(|sets| sets.first())
        .cloned();
    let set_found = set.is_some();
    let capacity_summary = set
        .as_ref()
        .and_then(|set| {
            set.get("operations")
                .and_then(|operations| operations.get("credentials"))
                .or_else(|| set.get("credentials"))
        })
        .cloned()
        .unwrap_or(Value::Null);
    let replacement_need = replacement_need_summary(set.as_ref());
    let route_impact = route_impact_summary(options, route);
    let status = if set_found { "ok" } else { "blocked" };
    let reason_code = if set_found {
        "keys_replacement_plan_projected"
    } else {
        "credential_set_not_projected"
    };

    let data = serde_json::json!({
        "command": "keys replacement-plan",
        "scan_mode": "management_projection_only",
        "credential_set_id": safe_local_id(&options.credential_set_id),
        "capacity_summary": capacity_summary,
        "replacement_need": replacement_need,
        "route_impact": route_impact,
        "credential_set": set,
        "safe_next_actions": replacement_plan_safe_next_actions(options, route.is_some()),
    });
    let report =
        crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
            status,
            reason: if set_found {
                "Credential replacement plan was projected without mutation."
            } else {
                "The requested credential set was not projected."
            },
            reason_code,
            effect: crate::cli_effects::runtime_readonly_store_reads_effect(),
            scope: serde_json::json!({
                "credential_set_id": safe_local_id(&options.credential_set_id),
                "model": options.model.as_deref().and_then(safe_probe_model),
                "client_token_ref": options.client_token_ref.as_deref().and_then(safe_local_id),
            }),
            window: Value::Null,
            next_action: replacement_plan_next_action(options, set_found),
            data,
        });
    render_sanitized_keys_report(&report, options.output)
}

fn replacement_need_summary(set: Option<&Value>) -> Value {
    let Some(set) = set else {
        return serde_json::json!({
            "status": "unknown",
            "reason_code": "credential_set_not_projected",
            "summary": "Replacement need cannot be assessed without a credential-set projection.",
        });
    };
    let operations = set.get("operations");
    let operations_needs_input = operations
        .and_then(|operations| operations.get("needs_operator_input"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let required_action = operations
        .and_then(|operations| operations.get("required_action"))
        .and_then(Value::as_str)
        .and_then(safe_local_id);
    if operations_needs_input || required_action.is_some() {
        return serde_json::json!({
            "status": "replacement_recommended",
            "reason_code": required_action.unwrap_or("credential_set_needs_operator_input"),
            "summary": "Credential-set operations projection requests operator review before replacement.",
        });
    }

    let credentials = operations
        .and_then(|operations| operations.get("credentials"))
        .or_else(|| set.get("credentials"));
    let count = |field: &str| {
        credentials
            .and_then(|credentials| credentials.get(field))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let total = count("total");
    let available = count("available");
    let blocked =
        count("cooling_down") + count("expired") + count("quota_exhausted") + count("disabled");
    if total == 0 {
        serde_json::json!({
            "status": "replacement_recommended",
            "reason_code": "credential_set_empty",
            "summary": "Credential set has no projected credentials.",
        })
    } else if available == 0 {
        serde_json::json!({
            "status": "replacement_recommended",
            "reason_code": "no_available_credentials",
            "summary": "Credential set has no projected available credentials.",
        })
    } else if blocked > 0 {
        serde_json::json!({
            "status": "monitor",
            "reason_code": "partial_capacity_loss",
            "summary": "Credential set still has available capacity but some credentials are unavailable.",
        })
    } else {
        serde_json::json!({
            "status": "not_required",
            "reason_code": "sufficient_available_capacity",
            "summary": "Credential set has projected available capacity.",
        })
    }
}

fn route_impact_summary(options: &KeysReplacementPlanOptions, route: Option<&Value>) -> Value {
    let model = options.model.as_deref().and_then(safe_probe_model);
    let client_token_ref = options.client_token_ref.as_deref().and_then(safe_local_id);
    if model.is_none() {
        return serde_json::json!({
            "status": "unavailable_without_model_context",
            "model": Value::Null,
            "client_token_ref": client_token_ref,
            "admission_status": Value::Null,
            "reason_code": Value::Null,
            "selected_target": Value::Null,
            "requested_credential_set": route_requested_credential_set_unknown(options),
            "summary": "Route impact was not requested because no public model context was supplied.",
        });
    }
    let Some(route) = route else {
        return serde_json::json!({
            "status": "unavailable_without_route_projection",
            "model": model,
            "client_token_ref": client_token_ref,
            "admission_status": Value::Null,
            "reason_code": Value::Null,
            "selected_target": Value::Null,
            "requested_credential_set": route_requested_credential_set_unknown(options),
            "summary": "Route impact projection was not available from the management API.",
        });
    };
    let selected_candidate = route_selected_candidate(route);
    let requested_credential_set =
        route_requested_credential_set_summary(options, route, selected_candidate);
    serde_json::json!({
        "status": "available",
        "model": route
            .get("model")
            .and_then(Value::as_str)
            .and_then(safe_probe_model)
            .or(model),
        "client_token_ref": route
            .get("client_token")
            .and_then(|client_token| client_token.get("name"))
            .and_then(Value::as_str)
            .and_then(safe_local_id)
            .or(client_token_ref),
        "route_kind": route
            .get("route_kind")
            .and_then(Value::as_str)
            .and_then(safe_local_id),
        "admission_status": route
            .get("admission_summary")
            .and_then(|summary| summary.get("status"))
            .and_then(Value::as_str)
            .and_then(safe_local_id),
        "reason_code": route
            .get("admission_summary")
            .and_then(|summary| summary.get("reason_code"))
            .and_then(Value::as_str)
            .and_then(safe_local_id),
        "candidate_count": route
            .get("admission_summary")
            .and_then(|summary| summary.get("candidate_count"))
            .and_then(Value::as_u64),
        "included_count": route
            .get("admission_summary")
            .and_then(|summary| summary.get("included_count"))
            .and_then(Value::as_u64),
        "blocked_count": route
            .get("admission_summary")
            .and_then(|summary| summary.get("blocked_count"))
            .and_then(Value::as_u64),
        "last_resort_used": route
            .get("admission_summary")
            .and_then(|summary| summary.get("last_resort_used"))
            .and_then(Value::as_bool),
        "selected_target": sanitize_route_selected_target(route.get("selected_target"), selected_candidate),
        "requested_credential_set": requested_credential_set,
        "summary": "Route impact was projected from the read-only routing preview schema; credential-set impact is candidate-level context, not an automatic replacement decision.",
    })
}

fn route_requested_credential_set_unknown(options: &KeysReplacementPlanOptions) -> Value {
    serde_json::json!({
        "credential_set_id": safe_local_id(&options.credential_set_id),
        "candidate_presence": "unknown",
        "selected_candidate": Value::Null,
        "matching_candidate_count": Value::Null,
        "summary": "Requested credential-set participation cannot be determined without a route candidate projection.",
    })
}

fn route_requested_credential_set_summary(
    options: &KeysReplacementPlanOptions,
    route: &Value,
    selected_candidate: Option<&Value>,
) -> Value {
    let Some(requested_set_id) = safe_local_id(&options.credential_set_id) else {
        return route_requested_credential_set_unknown(options);
    };
    let Some(candidates) = route.get("candidates").and_then(Value::as_array) else {
        return route_requested_credential_set_unknown(options);
    };

    let mut matching_candidate_count = 0usize;
    let mut saw_safe_credential_set_id = candidates.is_empty();
    let mut requested_selected = false;
    for candidate in candidates {
        let candidate_set_id = candidate
            .get("credential_set_id")
            .and_then(Value::as_str)
            .and_then(safe_local_id);
        if candidate_set_id.is_some() {
            saw_safe_credential_set_id = true;
        }
        if candidate_set_id == Some(requested_set_id) {
            matching_candidate_count += 1;
            if candidate
                .get("selected")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || selected_candidate.is_some_and(|selected| std::ptr::eq(selected, candidate))
            {
                requested_selected = true;
            }
        }
    }

    if !saw_safe_credential_set_id {
        return route_requested_credential_set_unknown(options);
    }

    let candidate_presence = if requested_selected {
        "selected_candidate"
    } else if matching_candidate_count > 0 {
        "candidate_not_selected"
    } else {
        "not_candidate"
    };
    let selected_candidate = if requested_selected {
        Some(true)
    } else if matching_candidate_count > 0 {
        Some(false)
    } else {
        None
    };
    let summary = match candidate_presence {
        "selected_candidate" => "Requested credential set appears on the selected route candidate.",
        "candidate_not_selected" => {
            "Requested credential set appears in route candidates but not on the selected candidate."
        }
        "not_candidate" => "Requested credential set does not appear in the route candidates.",
        _ => "Requested credential-set participation is unknown.",
    };

    serde_json::json!({
        "credential_set_id": requested_set_id,
        "candidate_presence": candidate_presence,
        "selected_candidate": selected_candidate,
        "matching_candidate_count": matching_candidate_count,
        "summary": summary,
    })
}

fn route_selected_candidate(route: &Value) -> Option<&Value> {
    let candidates = route.get("candidates").and_then(Value::as_array)?;
    candidates
        .iter()
        .find(|candidate| {
            candidate
                .get("selected")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .or_else(|| {
            let selected_target = route.get("selected_target")?;
            candidates.iter().find(|candidate| {
                route_candidate_matches_selected_target(candidate, selected_target)
            })
        })
}

fn route_candidate_matches_selected_target(candidate: &Value, target: &Value) -> bool {
    let channel_matches = candidate.get("channel_id").and_then(Value::as_str)
        == target.get("channel_id").and_then(Value::as_str);
    let position_matches = candidate.get("plan_position").and_then(Value::as_u64)
        == target.get("plan_position").and_then(Value::as_u64);
    channel_matches && position_matches
}

fn sanitize_route_selected_target(
    target: Option<&Value>,
    selected_candidate: Option<&Value>,
) -> Value {
    if target.is_none() && selected_candidate.is_none() {
        return Value::Null;
    };
    let channel_id = target
        .and_then(|target| target.get("channel_id"))
        .or_else(|| selected_candidate.and_then(|candidate| candidate.get("channel_id")))
        .and_then(Value::as_str)
        .and_then(safe_local_id);
    let plan_position = target
        .and_then(|target| target.get("plan_position"))
        .or_else(|| selected_candidate.and_then(|candidate| candidate.get("plan_position")))
        .and_then(Value::as_u64);
    let credential_set_id = selected_candidate.and_then(|candidate| {
        candidate
            .get("credential_set_id")
            .and_then(Value::as_str)
            .and_then(safe_local_id)
    });

    serde_json::json!({
        "channel_id": channel_id,
        "plan_position": plan_position,
        "credential_set_id": credential_set_id,
    })
}

fn replacement_plan_safe_next_actions(
    options: &KeysReplacementPlanOptions,
    route_projection_available: bool,
) -> Value {
    let mut actions = vec![serde_json::json!({
        "summary": "Refresh credential-set stats from read-only management projections.",
        "safe_argv": keys_stats_argv(Some(&options.credential_set_id), options.include_credential_refs),
        "side_effect_class": "runtime_readonly",
        "requires_confirmation": false,
    })];
    if route_projection_available {
        actions.push(serde_json::json!({
            "summary": "Re-check route impact through the read-only route explain projection.",
            "safe_argv": route_explain_argv(options),
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }));
    }
    actions.push(serde_json::json!({
        "summary": "Preview a replacement import locally; this does not send credentials to management.",
        "safe_argv": keys_import_dry_run_argv(&options.credential_set_id),
        "side_effect_class": "local_preview",
        "requires_confirmation": false,
    }));
    Value::Array(actions)
}

fn replacement_plan_next_action(options: &KeysReplacementPlanOptions, set_found: bool) -> Value {
    if set_found {
        serde_json::json!({
            "summary": "Review the safe next actions before any credential import or lifecycle change.",
            "safe_argv": keys_stats_argv(Some(&options.credential_set_id), options.include_credential_refs),
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
            "automatic_rollback": false,
        })
    } else {
        serde_json::json!({
            "summary": "List credential sets from the read-only management projection.",
            "safe_argv": ["one-ai-key", "keys", "list"],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
            "automatic_rollback": false,
        })
    }
}

fn route_explain_argv(options: &KeysReplacementPlanOptions) -> Value {
    let Some(model) = options.model.as_deref().and_then(safe_probe_model) else {
        return Value::Null;
    };
    let mut argv = vec![
        Value::from("one-ai-key"),
        Value::from("route"),
        Value::from("explain"),
        Value::from(model),
    ];
    if let Some(client_token_ref) = options.client_token_ref.as_deref().and_then(safe_local_id) {
        argv.push(Value::from("--client-token-ref"));
        argv.push(Value::from(client_token_ref));
    }
    Value::Array(argv)
}

fn keys_import_dry_run_argv(credential_set_id: &str) -> Value {
    let Some(credential_set_id) = safe_local_id(credential_set_id) else {
        return Value::Null;
    };
    serde_json::json!([
        "one-ai-key",
        "keys",
        "import",
        "--credential-set",
        credential_set_id,
        "--source",
        "replacement.keys",
        "--dry-run"
    ])
}

fn keys_reason(reason_code: &str) -> &'static str {
    match reason_code {
        "no_credential_sets_projected" => "No credential-set projection is available.",
        "credential_set_not_projected" => "The requested credential set was not projected.",
        "credential_set_stats_projected" => "Credential-set stats were projected.",
        _ => "Credential sets were projected.",
    }
}

fn sanitized_credential_sets(credential_sets: &Value, filter_id: Option<&str>) -> Vec<Value> {
    credential_sets
        .get("credential_sets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|set| match filter_id {
            Some(expected) => set
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id == expected),
            None => true,
        })
        .map(sanitize_credential_set)
        .collect()
}

fn sanitize_credential_set(set: &Value) -> Value {
    let probe_summary = sanitize_probe_summary(set);
    let credential_refs = bounded_credential_refs(set);
    let credential_ref_status = if credential_refs.is_empty() {
        "not_requested"
    } else {
        "available_from_summary_projection"
    };
    serde_json::json!({
        "credential_set_id": set.get("id").and_then(Value::as_str).and_then(safe_local_id),
        "channels": set.get("channels").and_then(Value::as_u64),
        "channel_ids": string_array(set.get("channel_ids")),
        "account_ids": string_array(set.get("account_ids")),
        "provider_ids": string_array(set.get("provider_ids")),
        "credentials": sanitize_lifecycle_counts(set.get("credentials")),
        "key_import": sanitize_key_import(set.get("key_import")),
        "latest_probe_summary": probe_summary.latest_probe_summary,
        "probe_summary_status": probe_summary.status,
        "credential_ref_status": credential_ref_status,
        "credential_refs": credential_refs,
    })
}

fn sanitize_lifecycle_counts(counts: Option<&Value>) -> Value {
    let Some(counts) = counts else {
        return Value::Null;
    };
    let mut sanitized = serde_json::Map::new();
    for field in [
        "total",
        "available",
        "cooling_down",
        "expired",
        "quota_exhausted",
        "disabled",
    ] {
        if let Some(value) = counts.get(field).and_then(Value::as_u64) {
            sanitized.insert(field.to_string(), Value::from(value));
        }
    }
    Value::Object(sanitized)
}

fn sanitize_key_import(key_import: Option<&Value>) -> Value {
    let Some(key_import) = key_import else {
        return Value::Null;
    };
    serde_json::json!({
        "source_kind": key_import.get("source_kind").and_then(Value::as_str),
        "unique_count": key_import.get("unique_count").and_then(Value::as_u64),
        "duplicate_occurrence_count": key_import
            .get("duplicate_occurrence_count")
            .and_then(Value::as_u64),
        "ignored_empty_count": key_import.get("ignored_empty_count").and_then(Value::as_u64),
        "invalid_line_count": key_import.get("invalid_line_count").and_then(Value::as_u64),
        "created_at_unix_seconds": key_import
            .get("created_at_unix_seconds")
            .and_then(Value::as_i64),
    })
}

struct ProbeSummaryProjection {
    status: Value,
    latest_probe_summary: Value,
}

fn sanitize_probe_summary(set: &Value) -> ProbeSummaryProjection {
    if let Some(summary) = set.get("probe_summary").filter(|value| !value.is_null()) {
        return ProbeSummaryProjection {
            status: summary
                .get("status")
                .and_then(Value::as_str)
                .map(Value::from)
                .unwrap_or_else(|| Value::from("available")),
            latest_probe_summary: sanitize_probe_summary_body(summary),
        };
    }

    ProbeSummaryProjection {
        status: Value::from(PROBE_SUMMARY_UNAVAILABLE),
        latest_probe_summary: Value::Null,
    }
}

fn sanitize_probe_summary_body(summary: &Value) -> Value {
    let mut sanitized = serde_json::Map::new();
    if let Some(status) = summary.get("status").and_then(Value::as_str) {
        sanitized.insert("status".to_string(), Value::from(status));
    }
    for field in [
        "total_credentials",
        "probed_credentials",
        "available_credentials",
        "cooling_down_credentials",
        "failed_credentials",
        "success",
        "invalid",
        "quota_exhausted",
        "rate_limited",
        "provider_unavailable",
        "unsupported_model",
        "unknown",
        "unprobed_credentials",
    ] {
        if let Some(value) = summary.get(field).and_then(Value::as_u64) {
            sanitized.insert(field.to_string(), Value::from(value));
        }
    }
    if let Some(value) = summary
        .get("latest_probe_at_unix_seconds")
        .and_then(Value::as_i64)
    {
        sanitized.insert(
            "latest_probe_at_unix_seconds".to_string(),
            Value::from(value),
        );
    }
    Value::Object(sanitized)
}

fn bounded_credential_refs(set: &Value) -> Vec<String> {
    set.get("credentials")
        .and_then(|credentials| credentials.get("refs"))
        .or_else(|| set.get("credential_refs"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(normalize_credential_ref)
        .collect()
}

fn normalize_credential_ref(value: &str) -> Option<String> {
    let position = value.strip_prefix("cr:v1:pos:")?;
    if position.is_empty() || !position.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let position = position.parse::<usize>().ok()?;
    Some(format!("cr:v1:pos:{position}"))
}

fn normalize_probe_result_ref(value: &str) -> Option<String> {
    let id = value.strip_prefix("pr:v1:id:")?;
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let id = id.parse::<i64>().ok()?;
    if id < 0 {
        return None;
    }
    Some(format!("pr:v1:id:{id}"))
}

fn safe_probe_apply_action(value: &str) -> Option<&str> {
    match value {
        "noop" | "expire" | "quota_exhaust" | "restore" | "cooldown" => Some(value),
        _ => None,
    }
}

fn merge_operations_into_stats_report(report: &mut Value, operations: &Value) {
    let Some(first_set) = report
        .get_mut("credential_sets")
        .and_then(Value::as_array_mut)
        .and_then(|sets| sets.first_mut())
    else {
        return;
    };

    let sanitized = serde_json::json!({
        "credential_set_id": operations
            .get("credential_set_id")
            .and_then(Value::as_str)
            .and_then(safe_local_id),
        "status": operations.get("status").and_then(Value::as_str),
        "serving_mode": operations.get("serving_mode").and_then(Value::as_str),
        "accepting_requests": operations.get("accepting_requests").and_then(Value::as_bool),
        "needs_operator_input": operations
            .get("needs_operator_input")
            .and_then(Value::as_bool),
        "required_action": operations.get("required_action").and_then(Value::as_str),
        "channels": operations.get("channels").and_then(Value::as_u64),
        "channel_ids": string_array(operations.get("channel_ids")),
        "account_ids": string_array(operations.get("account_ids")),
        "provider_ids": string_array(operations.get("provider_ids")),
        "credentials": sanitize_lifecycle_counts(operations.get("credentials")),
        "alerts": sanitize_operations_alerts(operations.get("alerts")),
        "next_action": operations_next_action(operations),
    });
    if let Some(object) = first_set.as_object_mut() {
        object.insert("operations".to_string(), sanitized);
    }
}

fn merge_credential_refs_into_stats_report(report: &mut Value, credentials_page: &Value) {
    let Some(first_set) = report
        .get_mut("credential_sets")
        .and_then(Value::as_array_mut)
        .and_then(|sets| sets.first_mut())
    else {
        return;
    };
    let expected_set_id = first_set
        .get("credential_set_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if credentials_page
        .get("credential_set_id")
        .and_then(Value::as_str)
        .is_some_and(|page_set_id| page_set_id != expected_set_id)
    {
        if let Some(object) = first_set.as_object_mut() {
            object.insert(
                "credential_ref_status".to_string(),
                Value::from("credential_ref_page_mismatch"),
            );
            object.insert(
                "credential_ref_window".to_string(),
                credential_ref_window(credentials_page, 0, 0),
            );
        }
        return;
    }
    let credentials = credentials_page
        .get("credentials")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let credential_refs = credentials
        .iter()
        .filter_map(|credential| credential.get("credential_ref").and_then(Value::as_str))
        .filter_map(normalize_credential_ref)
        .map(Value::from)
        .collect::<Vec<_>>();
    let returned_refs = credential_refs.len();
    let returned_credentials = credentials.len();
    let dropped_credentials = returned_credentials.saturating_sub(returned_refs);
    let status = if !credential_refs.is_empty() && dropped_credentials == 0 {
        "available_bounded_page"
    } else if !credential_refs.is_empty() {
        "partial_bounded_page"
    } else if credentials.is_empty() {
        "empty_bounded_page"
    } else {
        "unavailable_without_writable_resource_position"
    };
    let window = credential_ref_window(credentials_page, returned_refs, dropped_credentials);
    if let Some(object) = first_set.as_object_mut() {
        object.insert(
            "credential_refs".to_string(),
            Value::Array(credential_refs.clone()),
        );
        object.insert("credential_ref_status".to_string(), Value::from(status));
        object.insert("credential_ref_window".to_string(), window);
    }
}

fn credential_ref_window(page: &Value, returned_refs: usize, dropped_credentials: usize) -> Value {
    let returned_credentials = page
        .get("credentials")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    serde_json::json!({
        "offset": page.get("offset").and_then(Value::as_u64),
        "limit": page.get("limit").and_then(Value::as_u64),
        "returned_credentials": returned_credentials,
        "returned_credential_refs": returned_refs,
        "dropped_credentials": dropped_credentials,
        "total_credentials": page
            .get("total_credentials")
            .and_then(Value::as_u64),
        "filtered_credentials": page
            .get("filtered_credentials")
            .and_then(Value::as_u64),
    })
}

fn selected_single_set_id(report: &Value) -> Option<&str> {
    let sets = report.get("credential_sets")?.as_array()?;
    if sets.len() != 1 {
        return None;
    }
    sets.first()?
        .get("credential_set_id")
        .and_then(Value::as_str)
}

fn keys_next_action(status: &str, credential_set_id: Option<&str>) -> Value {
    if status == "ok" {
        serde_json::json!({
            "summary": "Credential-set state is reported from bounded management projections.",
            "dry_run_argv_status": "keys_probe_unavailable_until_m3",
            "safe_argv": keys_stats_argv(credential_set_id, false),
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        })
    } else {
        serde_json::json!({
            "summary": "No matching credential-set projection is available from the management API.",
            "dry_run_argv_status": Value::Null,
            "safe_argv": ["one-ai-key", "keys", "list"],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        })
    }
}

fn keys_stats_argv(credential_set_id: Option<&str>, include_refs: bool) -> Value {
    let mut argv = vec![
        Value::from("one-ai-key"),
        Value::from("keys"),
        Value::from("stats"),
    ];
    if let Some(credential_set_id) = credential_set_id {
        if !is_safe_argv_arg(credential_set_id) {
            return Value::Null;
        }
        argv.push(Value::from("--credential-set"));
        argv.push(Value::from(credential_set_id));
    }
    if include_refs {
        argv.push(Value::from("--include-credential-refs"));
    }
    Value::Array(argv)
}

fn is_safe_argv_arg(arg: &str) -> bool {
    if arg.is_empty() || arg.len() > 128 {
        return false;
    }
    if !arg
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
    {
        return false;
    }
    let lower = arg.to_ascii_lowercase();
    if lower.contains("sk-")
        || lower.contains("://")
        || lower.contains("http")
        || lower.contains("www.")
        || looks_like_jwt(arg)
        || looks_like_long_token(arg)
        || contains_instruction_marker(&lower)
    {
        return false;
    }
    true
}

fn looks_like_jwt(arg: &str) -> bool {
    let segments = arg.split('.').collect::<Vec<_>>();
    segments.len() == 3
        && segments
            .iter()
            .all(|segment| segment.len() >= 8 && is_base64url_like(segment))
}

fn looks_like_long_token(arg: &str) -> bool {
    arg.len() >= 32
        && arg
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        && arg.bytes().any(|byte| byte.is_ascii_uppercase())
        && arg.bytes().any(|byte| byte.is_ascii_digit())
}

fn is_base64url_like(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn contains_instruction_marker(lower: &str) -> bool {
    let normalized = lower
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    [
        "ignoreinstruction",
        "systemprompt",
        "promptinjection",
        "developerinstruction",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn safe_local_id(value: &str) -> Option<&str> {
    if is_safe_argv_arg(value) {
        Some(value)
    } else {
        None
    }
}

fn operations_next_action(operations: &Value) -> Value {
    let needs_operator_input = operations
        .get("needs_operator_input")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !needs_operator_input {
        serde_json::json!({
            "summary": "Credential-set operations projection does not require operator input.",
            "dry_run_argv_status": Value::Null,
            "safe_argv": Value::Null,
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        })
    } else {
        serde_json::json!({
            "summary": "Some credentials are blocked. M2.4b is read-only; request bounded credential_refs with keys stats before M3 probe workflows.",
            "dry_run_argv_status": "keys_probe_unavailable_until_m3",
            "safe_argv": keys_stats_argv(operations.get("credential_set_id").and_then(Value::as_str), true),
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        })
    }
}

fn sanitize_operations_alerts(alerts: Option<&Value>) -> Vec<Value> {
    alerts
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|alert| {
            let kind = alert
                .get("kind")
                .and_then(Value::as_str)
                .and_then(safe_local_id);
            let severity = alert
                .get("severity")
                .and_then(Value::as_str)
                .and_then(safe_alert_severity);
            let reason_code = alert
                .get("reason_code")
                .and_then(Value::as_str)
                .and_then(safe_local_id);
            serde_json::json!({
                "kind": kind,
                "severity": severity,
                "reason_code": reason_code,
                "summary": operations_alert_summary(kind, severity, reason_code),
            })
        })
        .collect()
}

fn safe_alert_severity(value: &str) -> Option<&str> {
    match value {
        "info" | "warning" | "error" | "critical" => Some(value),
        _ => None,
    }
}

fn operations_alert_summary(
    kind: Option<&str>,
    severity: Option<&str>,
    reason_code: Option<&str>,
) -> &'static str {
    match kind.or(reason_code) {
        Some("credential_pool_exhausted") => {
            "Credential pool is exhausted; inspect bounded credential refs before lifecycle correction."
        }
        Some("credential_pool_degraded") => {
            "Credential pool is degraded; inspect bounded credential refs before lifecycle correction."
        }
        Some("credential_rotation_required") => {
            "Credential rotation is required; inspect bounded credential refs before lifecycle correction."
        }
        _ if matches!(severity, Some("critical" | "error")) => {
            "Credential-set operations alert requires operator review; details were redacted."
        }
        _ => "Credential-set operations alert was reported; details were redacted.",
    }
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn safe_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(safe_local_id)
        .map(ToOwned::to_owned)
        .collect()
}

fn render_keys_import_table(report: &Value) -> String {
    let mut output = String::new();
    output.push_str("keys import\n");
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    for field in [
        "line_count",
        "non_empty_line_count",
        "local_duplicate_line_count",
        "store_duplicate_status",
        "confirmed_import_route_eligible",
        "source_secrets_sent_to_management",
        "automatic_rollback",
        "recovery_path",
    ] {
        crate::cli_report::push_table_field(&mut output, field, report.get(field));
    }
    if let Some(response) = report.get("management_response") {
        for field in [
            "selector_generation",
            "requested_credentials",
            "imported_credentials",
            "duplicate_credentials",
            "ignored_empty_credentials",
            "credential_statuses_returned",
        ] {
            crate::cli_report::push_table_field(
                &mut output,
                &format!("management_response.{field}"),
                response.get(field),
            );
        }
    }
    output
}

fn render_keys_probe_table(report: &Value) -> String {
    let mut output = String::new();
    output.push_str("keys probe\n");
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    for field in [
        "credential_ref",
        "channel_id",
        "model",
        "probe_kind",
        "credential_count",
        "upstream_request_sent",
        "probe_evidence_persisted",
        "automatic_rollback",
        "recovery_path",
        "warning",
    ] {
        crate::cli_report::push_table_field(&mut output, field, report.get(field));
    }
    if let Some(probe) = report.get("probe") {
        for field in [
            "outcome",
            "channel_id",
            "upstream_status",
            "upstream_code",
            "upstream_limit_type",
            "created_at_unix_seconds",
        ] {
            crate::cli_report::push_table_field(
                &mut output,
                &format!("probe.{field}"),
                probe.get(field),
            );
        }
    }
    output
}

fn render_keys_disable_table(report: &Value) -> String {
    let mut output = String::new();
    output.push_str("keys disable\n");
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    for field in [
        "credential_ref",
        "channel_id",
        "selector_generation",
        "state_kind",
        "reason_configured",
        "mutating_disable_sent",
        "upstream_request_sent",
        "automatic_rollback",
        "recovery_path",
    ] {
        crate::cli_report::push_table_field(&mut output, field, report.get(field));
    }
    output
}

fn render_keys_probe_apply_table(report: &Value) -> String {
    let mut output = String::new();
    output.push_str("keys probe-apply\n");
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    for field in [
        "credential_ref",
        "probe_result_ref",
        "planned_action",
        "applied_action",
        "upstream_request_sent",
        "mutating_apply_sent",
        "lifecycle_mutation_applied",
        "automatic_rollback",
        "recovery_path",
    ] {
        crate::cli_report::push_table_field(&mut output, field, report.get(field));
    }
    if let Some(probe) = report.get("probe") {
        for field in [
            "outcome",
            "channel_id",
            "upstream_status",
            "upstream_code",
            "upstream_limit_type",
            "created_at_unix_seconds",
        ] {
            crate::cli_report::push_table_field(
                &mut output,
                &format!("probe.{field}"),
                probe.get(field),
            );
        }
    }
    output
}

fn render_keys_replacement_plan_table(report: &Value) -> String {
    let mut output = String::new();
    output.push_str("keys replacement-plan\n");
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    for field in [
        "credential_set_id",
        "replacement_need.status",
        "replacement_need.reason_code",
        "route_impact.status",
        "route_impact.model",
        "route_impact.client_token_ref",
        "route_impact.reason_code",
        "route_impact.requested_credential_set.candidate_presence",
    ] {
        push_nested_table_field(&mut output, field, report);
    }
    if let Some(capacity) = report.get("capacity_summary") {
        for field in [
            "total",
            "available",
            "cooling_down",
            "expired",
            "quota_exhausted",
            "disabled",
        ] {
            crate::cli_report::push_table_field(
                &mut output,
                &format!("capacity_summary.{field}"),
                capacity.get(field),
            );
        }
    }
    output
}

fn push_nested_table_field(output: &mut String, path: &str, report: &Value) {
    let mut value = report;
    for segment in path.split('.') {
        value = value.get(segment).unwrap_or(&Value::Null);
    }
    crate::cli_report::push_table_field(output, path, Some(value));
}

fn render_keys_table(report: &Value) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "{}\n",
        report
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("keys")
    ));
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    output.push_str("scan_mode: summary_projection_only\n");
    output.push_str("credential_sets:\n");
    for set in report
        .get("credential_sets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        output.push_str(&render_credential_set_line(set));
    }
    output
}

fn render_credential_set_line(set: &Value) -> String {
    let id = set
        .get("credential_set_id")
        .and_then(Value::as_str)
        .unwrap_or("<unknown>");
    let credentials = set.get("credentials").unwrap_or(&Value::Null);
    let refs = set
        .get("credential_refs")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(display_value)
                .collect::<Vec<_>>()
                .join(",")
        })
        .filter(|refs| !refs.is_empty())
        .unwrap_or_else(|| "none".to_string());
    let probe_summary_status = set
        .get("probe_summary_status")
        .and_then(Value::as_str)
        .unwrap_or(PROBE_SUMMARY_UNAVAILABLE);

    format!(
        "- {} channels={} total={} available={} cooling_down={} expired={} quota_exhausted={} disabled={} probe_summary_status={} credential_refs={}\n",
        display_value(id),
        set.get("channels").and_then(Value::as_u64).unwrap_or(0),
        credentials
            .get("total")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        credentials
            .get("available")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        credentials
            .get("cooling_down")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        credentials
            .get("expired")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        credentials
            .get("quota_exhausted")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        credentials
            .get("disabled")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        display_value(probe_summary_status),
        refs,
    )
}

fn display_value(value: &str) -> String {
    crate::cli_report::escape_table_value(value)
}

#[cfg(test)]
mod tests {
    use axum::{
        http::StatusCode,
        routing::{get, post},
        Json, Router,
    };
    use serde_json::{json, Value};
    use std::{
        fs,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex,
        },
    };

    fn temp_keys_file(name: &str, content: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir()
            .join("one-ai-key-test-output")
            .join(format!("keys-import-{name}-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("replacement.keys");
        fs::write(&path, content).unwrap();
        path
    }

    async fn spawn_management_fixture(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[test]
    fn keys_import_dry_run_reports_counts_without_secret_output() {
        let source = temp_keys_file(
            "dry-run",
            "replacement-one\n\nreplacement-two\nreplacement-one\n",
        );

        let rendered = super::render_keys_import_dry_run_report(
            &super::KeysImportOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-credentials".to_string(),
                source,
                mode: super::KeysImportMode::DryRun,
                output: crate::cli_report::OutputFormat::Json,
            },
            None,
        )
        .unwrap();
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "dry_run");
        assert_eq!(report["reason_code"], "keys_import_local_preview");
        assert_eq!(report["side_effect_class"], "local_preview");
        assert_eq!(report["effect_vector"]["reads_local_files"], true);
        assert_eq!(report["effect_vector"]["writes_management_store"], false);
        assert_eq!(report["effect_vector"]["mutates_runtime"], false);
        assert_eq!(report["scope"]["credential_set_id"], "relay-credentials");
        assert_eq!(report["data"]["line_count"], 4);
        assert_eq!(report["data"]["non_empty_line_count"], 3);
        assert_eq!(report["data"]["local_duplicate_line_count"], 1);
        assert_eq!(
            report["data"]["store_duplicate_status"],
            "unknown_local_preview"
        );
        assert_eq!(report["data"]["confirmed_import_route_eligible"], true);
        assert_eq!(report["data"]["automatic_rollback"], false);
        assert_eq!(
            report["data"]["recovery_path"],
            "manual credential lifecycle correction plus keys stats"
        );
        assert_eq!(report["next_action"]["requires_confirmation"], true);
        assert_eq!(
            report["next_action"]["side_effect_class"],
            "management_write"
        );
        assert!(!rendered.contains("replacement-one"));
        assert!(!rendered.contains("replacement-two"));
    }

    #[test]
    fn keys_import_dry_run_reports_readonly_store_preflight_without_source_secrets() {
        let source = temp_keys_file("readonly", "replacement-one\n");
        let runtime = json!({
            "credential_source": {
                "writable": false,
                "source_kind": "file"
            }
        });

        let rendered = super::render_keys_import_dry_run_report(
            &super::KeysImportOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-credentials".to_string(),
                source,
                mode: super::KeysImportMode::DryRun,
                output: crate::cli_report::OutputFormat::Json,
            },
            Some(&runtime),
        )
        .unwrap();
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["reason_code"], "credential_store_readonly");
        assert_eq!(report["data"]["source_secrets_sent_to_management"], false);
        assert_eq!(report["next_action"]["safe_argv"], Value::Null);
        assert!(!rendered.contains("replacement-one"));
    }

    #[tokio::test]
    async fn keys_import_confirmed_readonly_preflight_does_not_call_import_endpoint() {
        let source = temp_keys_file("confirmed-readonly", "replacement-one\n");
        let import_called = Arc::new(AtomicBool::new(false));
        let route_called = Arc::clone(&import_called);
        let router = Router::new()
            .route(
                "/management/explain/runtime",
                get(|| async {
                    Json(json!({
                        "credential_source": {
                            "writable": false,
                            "source_kind": "file"
                        }
                    }))
                }),
            )
            .route(
                "/management/credential-sets/relay-credentials/credentials/import",
                post(move |Json(_body): Json<Value>| {
                    let route_called = Arc::clone(&route_called);
                    async move {
                        route_called.store(true, Ordering::SeqCst);
                        Json(json!({"unexpected": true}))
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!("ONE_AI_KEY_TEST_TOKEN_{}", std::process::id());
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run_import(super::KeysImportOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            credential_set_id: "relay-credentials".to_string(),
            source,
            mode: super::KeysImportMode::Apply,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["reason_code"], "credential_store_readonly");
        assert_eq!(report["data"]["source_secrets_sent_to_management"], false);
        assert!(!import_called.load(Ordering::SeqCst));
        assert!(!rendered.contains("replacement-one"));
    }

    #[test]
    fn keys_import_apply_report_drops_backend_credentials_and_fingerprints() {
        let source = temp_keys_file("apply-redaction", "replacement-one\n");
        let source = super::read_import_source(&super::KeysImportOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            credential_set_id: "relay-credentials".to_string(),
            source,
            mode: super::KeysImportMode::Apply,
            output: crate::cli_report::OutputFormat::Json,
        })
        .unwrap();
        let rendered = super::render_keys_import_apply_report(
            &super::KeysImportOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-credentials".to_string(),
                source: "data/replacement.keys".into(),
                mode: super::KeysImportMode::Apply,
                output: crate::cli_report::OutputFormat::Json,
            },
            &source,
            &json!({
                "credential_set_id": "relay-credentials",
                "channel_ids": ["relay-channel"],
                "selector_generation": 7,
                "requested_credentials": 1,
                "imported_credentials": 1,
                "duplicate_credentials": 0,
                "ignored_empty_credentials": 0,
                "credentials": [
                    {
                        "id": "internal-derived-id",
                        "fingerprint": "fingerprint-fixture",
                        "state": "available"
                    }
                ],
                "operations": {
                    "status": "healthy",
                    "serving_mode": "serving",
                    "accepting_requests": true,
                    "needs_operator_input": false,
                    "required_action": null,
                    "credentials": {
                        "total": 1,
                        "available": 1
                    }
                }
            }),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason_code"], "keys_import_applied");
        assert_eq!(
            report["data"]["management_response"]["credential_statuses_returned"],
            1
        );
        assert!(!rendered.contains("replacement-one"));
        assert!(!rendered.contains("internal-derived-id"));
        assert!(!rendered.contains("fingerprint-fixture"));
    }

    #[test]
    fn keys_probe_dry_run_reports_single_credential_plan_without_upstream_call() {
        let credential_sets = json!({
            "credential_sets": [
                {
                    "id": "relay-credentials",
                    "channel_ids": ["relay-channel"],
                    "credentials": {"total": 2, "available": 1, "rate_limited": 1}
                }
            ]
        });

        let rendered = super::render_keys_probe_dry_run_report(
            &super::KeysProbeOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-credentials".to_string(),
                credential_ref: "cr:v1:pos:0".to_string(),
                model: "gpt-example".to_string(),
                kind: super::KeysProbeKind::ModelRetrieve,
                expected_output: None,
                timeout_seconds: None,
                mode: super::KeysProbeMode::DryRun,
                output: crate::cli_report::OutputFormat::Json,
            },
            Some(&credential_sets),
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "dry_run");
        assert_eq!(report["reason_code"], "keys_probe_plan");
        assert_eq!(report["side_effect_class"], "runtime_readonly");
        assert_eq!(report["effect_vector"]["calls_upstream"], false);
        assert_eq!(report["effect_vector"]["writes_management_store"], false);
        assert_eq!(report["scope"]["credential_ref"], "cr:v1:pos:0");
        assert_eq!(report["data"]["channel_id"], "relay-channel");
        assert_eq!(report["data"]["probe_kind"], "model_retrieve");
        assert_eq!(report["data"]["credential_count"], 1);
        assert_eq!(report["data"]["upstream_request_sent"], false);
        assert_eq!(report["next_action"]["requires_confirmation"], true);
        assert_eq!(
            report["next_action"]["side_effect_class"],
            "upstream_touching"
        );
        assert_no_probe_apply_recommendation(&report);
    }

    #[tokio::test]
    async fn keys_probe_dry_run_does_not_call_probe_endpoint() {
        let probe_called = Arc::new(AtomicBool::new(false));
        let route_called = Arc::clone(&probe_called);
        let router = Router::new()
            .route(
                "/management/credential-sets",
                get(|| async {
                    Json(json!({
                        "credential_sets": [
                            {
                                "id": "relay-credentials",
                                "channel_ids": ["relay-channel"],
                                "credentials": {"total": 1, "available": 1}
                            }
                        ]
                    }))
                }),
            )
            .route(
                "/management/credential-sets/relay-credentials/credentials/:credential_ref/probe",
                post(move |Json(_body): Json<Value>| {
                    let route_called = Arc::clone(&route_called);
                    async move {
                        route_called.store(true, Ordering::SeqCst);
                        Json(json!({"unexpected": true}))
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!("ONE_AI_KEY_TEST_PROBE_TOKEN_{}", std::process::id());
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run_probe(super::KeysProbeOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            credential_set_id: "relay-credentials".to_string(),
            credential_ref: "cr:v1:pos:0".to_string(),
            model: "gpt-example".to_string(),
            kind: super::KeysProbeKind::ModelRetrieve,
            expected_output: None,
            timeout_seconds: None,
            mode: super::KeysProbeMode::DryRun,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "dry_run");
        assert_eq!(report["data"]["upstream_request_sent"], false);
        assert!(!probe_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn keys_probe_confirmed_posts_probe_request_and_sanitizes_response() {
        let captured_body = Arc::new(Mutex::new(None::<Value>));
        let route_body = Arc::clone(&captured_body);
        let router = Router::new().route(
            "/management/credential-sets/relay-credentials/credentials/:credential_ref/probe",
            post(move |Json(body): Json<Value>| {
                let route_body = Arc::clone(&route_body);
                async move {
                    *route_body.lock().unwrap() = Some(body);
                    Json(json!({
                        "credential_set_id": "relay-credentials",
                        "channel_id": "relay-channel",
                        "credential_id": "internal-derived-id",
                        "result": {
                            "outcome": "success",
                            "channel_id": "relay-channel",
                            "provider_id": "relay-provider",
                            "account_id": "relay-account",
                            "classifier_id": null,
                            "adaptation_rule_id": null,
                            "upstream_status": 200,
                            "upstream_code": null,
                            "upstream_limit_type": null,
                            "latency_ms": 12,
                            "created_at_unix_seconds": 123
                        },
                        "credential": {
                            "id": "internal-derived-id",
                            "fingerprint": "fingerprint-fixture",
                            "state": "available"
                        },
                        "raw_key": "sk-probe-secret",
                        "token_hash": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                        "source_path": "/tmp/raw-probe-source.keys",
                        "upstream_url": "https://upstream.example/v1/chat?api_key=sk-url-secret"
                    }))
                }
            }),
        );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!("ONE_AI_KEY_TEST_PROBE_APPLY_TOKEN_{}", std::process::id());
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run_probe(super::KeysProbeOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            credential_set_id: "relay-credentials".to_string(),
            credential_ref: "cr:v1:pos:0".to_string(),
            model: "gpt-example".to_string(),
            kind: super::KeysProbeKind::ChatCompletion,
            expected_output: Some("ok".to_string()),
            timeout_seconds: Some(2),
            mode: super::KeysProbeMode::Apply,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let body = captured_body
            .lock()
            .unwrap()
            .clone()
            .expect("probe endpoint should receive a body");
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(body["model"], "gpt-example");
        assert_eq!(body["kind"], "chat_completion");
        assert_eq!(body["expected_output"], "ok");
        assert_eq!(body["timeout_seconds"], 2);
        assert_eq!(report["status"], "ok");
        assert_eq!(report["data"]["probe"]["outcome"], "success");
        assert_no_probe_apply_recommendation(&report);
        assert!(!rendered.contains("internal-derived-id"));
        assert!(!rendered.contains("fingerprint-fixture"));
        assert!(!rendered.contains("sk-probe-secret"));
        assert!(
            !rendered.contains("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
        assert!(!rendered.contains("/tmp/raw-probe-source.keys"));
        assert!(!rendered.contains("https://upstream.example"));
    }

    #[test]
    fn keys_disable_dry_run_reports_local_plan_without_secret_output() {
        let rendered = super::render_keys_disable_dry_run_report(&super::KeysDisableOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            credential_set_id: "relay-credentials".to_string(),
            credential_ref: "cr:v1:pos:0".to_string(),
            reason: "operator verified bad key".to_string(),
            mode: super::KeysDisableMode::DryRun,
            output: crate::cli_report::OutputFormat::Json,
        });
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "dry_run");
        assert_eq!(report["reason_code"], "keys_disable_plan");
        assert_eq!(report["side_effect_class"], "offline_readonly");
        assert_eq!(report["effect_vector"]["writes_management_store"], false);
        assert_eq!(report["effect_vector"]["calls_upstream"], false);
        assert_eq!(report["effect_vector"]["mutates_runtime"], false);
        assert_eq!(report["scope"]["credential_set_id"], "relay-credentials");
        assert_eq!(report["scope"]["credential_ref"], "cr:v1:pos:0");
        assert_eq!(report["data"]["credential_ref"], "cr:v1:pos:0");
        assert_eq!(report["data"]["reason_configured"], true);
        assert_eq!(report["data"]["mutating_disable_sent"], false);
        assert_eq!(report["data"]["upstream_request_sent"], false);
        assert_eq!(report["next_action"]["requires_confirmation"], true);
        assert_eq!(
            report["next_action"]["side_effect_class"],
            "management_write"
        );
        assert_eq!(
            report["next_action"]["safe_argv"],
            json!([
                "one-ai-key",
                "keys",
                "disable",
                "--credential-set",
                "relay-credentials",
                "--credential-ref",
                "cr:v1:pos:0",
                "--reason",
                "operator verified bad key",
                "--yes"
            ])
        );
        assert!(!rendered.contains("sk-"));
        assert!(!rendered.contains("token_hash"));
    }

    #[tokio::test]
    async fn keys_disable_dry_run_does_not_call_disable_endpoint_or_require_token() {
        let disable_called = Arc::new(AtomicBool::new(false));
        let route_called = Arc::clone(&disable_called);
        let router = Router::new().route(
            "/management/credential-sets/relay-credentials/credentials/:credential_ref/disable",
            post(move |Json(_body): Json<Value>| {
                let route_called = Arc::clone(&route_called);
                async move {
                    route_called.store(true, Ordering::SeqCst);
                    Json(json!({"unexpected": true}))
                }
            }),
        );
        let management_url = spawn_management_fixture(router).await;

        let rendered = super::run(super::KeysCommand::Disable(super::KeysDisableOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: None,
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            credential_set_id: "relay-credentials".to_string(),
            credential_ref: "cr:v1:pos:0".to_string(),
            reason: "operator verified bad key".to_string(),
            mode: super::KeysDisableMode::DryRun,
            output: crate::cli_report::OutputFormat::Json,
        }))
        .await
        .unwrap();
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "dry_run");
        assert_eq!(report["data"]["mutating_disable_sent"], false);
        assert!(!disable_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn keys_disable_confirmed_posts_disable_request_and_sanitizes_response() {
        let captured_body = Arc::new(Mutex::new(None::<Value>));
        let route_body = Arc::clone(&captured_body);
        let router = Router::new().route(
            "/management/credential-sets/relay-credentials/credentials/:credential_ref/disable",
            post(move |Json(body): Json<Value>| {
                let route_body = Arc::clone(&route_body);
                async move {
                    *route_body.lock().unwrap() = Some(body);
                    Json(json!({
                        "credential_set_id": "relay-credentials",
                        "credential_ref": "cr:v1:pos:0",
                        "credential_id": "internal-derived-id",
                        "channel_id": "relay-channel",
                        "selector_generation": 12,
                        "state": {"kind": "disabled", "reason": "operator verified bad key"},
                        "credential": {
                            "id": "internal-derived-id",
                            "fingerprint": "fingerprint-fixture",
                            "state": {"kind": "disabled", "reason": "operator verified bad key"}
                        },
                        "raw_request_body": "request-body-fixture",
                        "raw_response_body": "response-body-fixture",
                        "raw_key": "RAW_DISABLE_SECRET",
                        "token_hash": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                        "source_path": "/tmp/raw-disable-source.keys"
                    }))
                }
            }),
        );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!("ONE_AI_KEY_TEST_DISABLE_POST_TOKEN_{}", std::process::id());
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run(super::KeysCommand::Disable(super::KeysDisableOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            credential_set_id: "relay-credentials".to_string(),
            credential_ref: "cr:v1:pos:0".to_string(),
            reason: "operator verified bad key".to_string(),
            mode: super::KeysDisableMode::Apply,
            output: crate::cli_report::OutputFormat::Json,
        }))
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let body = captured_body
            .lock()
            .unwrap()
            .clone()
            .expect("disable endpoint should receive a body");
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(body["reason"], "operator verified bad key");
        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason_code"], "keys_disable_applied");
        assert_eq!(report["side_effect_class"], "management_write");
        assert_eq!(report["data"]["credential_ref"], "cr:v1:pos:0");
        assert_eq!(report["data"]["state_kind"], "disabled");
        assert_eq!(report["data"]["selector_generation"], 12);
        assert_eq!(report["data"]["mutating_disable_sent"], true);
        assert_eq!(report["data"]["upstream_request_sent"], false);
        assert_eq!(report["next_action"]["requires_confirmation"], false);
        assert!(!rendered.contains("internal-derived-id"));
        assert!(!rendered.contains("fingerprint-fixture"));
        assert!(!rendered.contains("request-body-fixture"));
        assert!(!rendered.contains("response-body-fixture"));
        assert!(!rendered.contains("RAW_DISABLE_SECRET"));
        assert!(
            !rendered.contains("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
        assert!(!rendered.contains("/tmp/raw-disable-source.keys"));
    }

    #[test]
    fn keys_probe_apply_report_drops_backend_credentials_and_request_bodies() {
        let rendered = super::render_keys_probe_apply_report(
            &super::KeysProbeOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-credentials".to_string(),
                credential_ref: "cr:v1:pos:0".to_string(),
                model: "gpt-example".to_string(),
                kind: super::KeysProbeKind::ModelRetrieve,
                expected_output: None,
                timeout_seconds: None,
                mode: super::KeysProbeMode::Apply,
                output: crate::cli_report::OutputFormat::Json,
            },
            &json!({
                "credential_set_id": "relay-credentials",
                "channel_id": "relay-channel",
                "credential_id": "internal-derived-id",
                "result": {
                    "outcome": "success",
                    "channel_id": "relay-channel",
                    "provider_id": "relay-provider",
                    "account_id": "relay-account",
                    "classifier_id": null,
                    "adaptation_rule_id": null,
                    "upstream_status": 200,
                    "upstream_code": null,
                    "upstream_limit_type": null,
                    "latency_ms": 12,
                    "created_at_unix_seconds": 123
                },
                "credential": {
                    "id": "internal-derived-id",
                    "fingerprint": "fingerprint-fixture",
                    "state": "available"
                },
                "raw_request_body": "request-body-fixture",
                "raw_response_body": "response-body-fixture",
                "raw_key": "sk-probe-secret",
                "token_hash": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "source_path": "/tmp/raw-probe-source.keys",
                "upstream_url": "https://upstream.example/v1/models?token=sk-url-secret"
            }),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason_code"], "keys_probe_recorded");
        assert_eq!(report["scope"]["credential_ref"], "cr:v1:pos:0");
        assert_eq!(report["data"]["probe"]["outcome"], "success");
        assert_eq!(report["data"]["probe"]["upstream_status"], 200);
        assert_eq!(report["data"]["probe"]["channel_id"], "relay-channel");
        assert_eq!(report["data"]["probe"]["created_at_unix_seconds"], 123);
        assert!(report["data"]["probe"].get("provider_id").is_none());
        assert!(report["data"]["probe"].get("account_id").is_none());
        assert!(report["data"]["probe"].get("classifier_id").is_none());
        assert_eq!(report["data"]["upstream_request_sent"], true);
        assert_no_probe_apply_recommendation(&report);
        assert!(!rendered.contains("internal-derived-id"));
        assert!(!rendered.contains("fingerprint-fixture"));
        assert!(!rendered.contains("request-body-fixture"));
        assert!(!rendered.contains("response-body-fixture"));
        assert!(!rendered.contains("sk-probe-secret"));
        assert!(
            !rendered.contains("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
        assert!(!rendered.contains("/tmp/raw-probe-source.keys"));
        assert!(!rendered.contains("https://upstream.example"));
    }

    #[test]
    fn keys_probe_apply_dry_run_report_uses_backend_plan_without_secret_output() {
        let rendered = super::render_keys_probe_apply_plan_report(
            &super::KeysProbeApplyPlanOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some("https://router.example".to_string()),
                    deprecated_base_url: None,
                    management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-credentials".to_string(),
                credential_ref: "cr:v1:pos:0".to_string(),
                output: crate::cli_report::OutputFormat::Json,
            },
            &json!({
                "credential_set_id": "relay-credentials",
                "credential_ref": "cr:v1:pos:0",
                "probe_result_ref": "pr:v1:id:7",
                "action": "restore",
                "probe": {
                    "outcome": "success",
                    "channel_id": "relay-channel",
                    "provider_id": "relay-provider",
                    "account_id": "relay-account",
                    "classifier_id": "classifier-internal",
                    "adaptation_rule_id": null,
                    "upstream_status": 200,
                    "upstream_code": null,
                    "upstream_limit_type": null,
                    "latency_ms": 12,
                    "created_at_unix_seconds": 123
                },
                "credential_id": "internal-derived-id",
                "fingerprint": "fingerprint-fixture",
                "raw_request_body": "request-body-fixture",
                "raw_response_body": "response-body-fixture"
            }),
            "keys probe-apply plan",
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "dry_run");
        assert_eq!(report["reason_code"], "keys_probe_apply_plan_projected");
        assert_eq!(report["side_effect_class"], "runtime_readonly");
        assert_eq!(report["effect_vector"]["calls_upstream"], false);
        assert_eq!(report["effect_vector"]["writes_management_store"], false);
        assert_eq!(report["effect_vector"]["mutates_runtime"], false);
        assert_eq!(report["scope"]["credential_ref"], "cr:v1:pos:0");
        assert_eq!(report["data"]["credential_ref"], "cr:v1:pos:0");
        assert_eq!(report["data"]["probe_result_ref"], "pr:v1:id:7");
        assert_eq!(report["data"]["planned_action"], "restore");
        assert_eq!(report["data"]["probe"]["outcome"], "success");
        assert_eq!(report["data"]["probe"]["channel_id"], "relay-channel");
        assert_eq!(report["data"]["mutating_apply_sent"], false);
        assert_eq!(report["data"]["lifecycle_mutation_applied"], false);
        assert_eq!(report["next_action"]["requires_confirmation"], false);
        assert_eq!(
            report["next_action"]["side_effect_class"],
            "runtime_readonly"
        );
        assert_eq!(
            report["next_action"]["safe_argv"],
            json!([
                "one-ai-key",
                "keys",
                "stats",
                "--credential-set",
                "relay-credentials",
                "--include-credential-refs"
            ])
        );
        assert_no_probe_apply_recommendation(&report);
        assert!(!rendered.contains("internal-derived-id"));
        assert!(!rendered.contains("fingerprint-fixture"));
        assert!(!rendered.contains("request-body-fixture"));
        assert!(!rendered.contains("response-body-fixture"));
        assert!(!rendered.contains("relay-provider"));
        assert!(!rendered.contains("relay-account"));
        assert!(!rendered.contains("classifier-internal"));
    }

    #[tokio::test]
    async fn keys_probe_apply_dry_run_calls_only_readonly_plan_endpoint() {
        let post_called = Arc::new(AtomicBool::new(false));
        let route_called = Arc::clone(&post_called);
        let router = Router::new()
            .route(
                "/management/credential-sets/relay-credentials/credentials/:credential_ref/apply-latest-probe/plan",
                get(|| async {
                    Json(json!({
                        "credential_set_id": "relay-credentials",
                        "credential_ref": "cr:v1:pos:0",
                        "probe_result_ref": "pr:v1:id:7",
                        "action": "restore",
                        "probe": {
                            "outcome": "success",
                            "channel_id": "relay-channel",
                            "upstream_status": 200,
                            "upstream_code": null,
                            "upstream_limit_type": null,
                            "created_at_unix_seconds": 123
                        }
                    }))
                }),
            )
            .route(
                "/management/credential-sets/relay-credentials/credentials/:credential_ref/apply-latest-probe",
                post(move |Json(_body): Json<Value>| {
                    let route_called = Arc::clone(&route_called);
                    async move {
                        route_called.store(true, Ordering::SeqCst);
                        Json(json!({"unexpected": true}))
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!(
            "ONE_AI_KEY_TEST_PROBE_APPLY_PLAN_TOKEN_{}",
            std::process::id()
        );
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run(super::KeysCommand::ProbeApply(
            super::KeysProbeApplyCommand::Apply(super::KeysProbeApplyApplyOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some(management_url),
                    deprecated_base_url: None,
                    management_token_env: Some(env_name.clone()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-credentials".to_string(),
                credential_ref: "cr:v1:pos:0".to_string(),
                probe_result_ref: None,
                mode: super::KeysProbeApplyMode::DryRun,
                output: crate::cli_report::OutputFormat::Json,
            }),
        ))
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "dry_run");
        assert_eq!(report["data"]["mutating_apply_sent"], false);
        assert!(!post_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn keys_probe_apply_dry_run_blocks_probe_result_ref_mismatch_without_post() {
        let post_called = Arc::new(AtomicBool::new(false));
        let route_called = Arc::clone(&post_called);
        let router = Router::new()
            .route(
                "/management/credential-sets/relay-credentials/credentials/:credential_ref/apply-latest-probe/plan",
                get(|| async {
                    Json(json!({
                        "credential_set_id": "relay-credentials",
                        "credential_ref": "cr:v1:pos:0",
                        "probe_result_ref": "pr:v1:id:9",
                        "action": "restore",
                        "probe": {
                            "outcome": "success",
                            "channel_id": "relay-channel",
                            "upstream_status": 200,
                            "created_at_unix_seconds": 123
                        }
                    }))
                }),
            )
            .route(
                "/management/credential-sets/relay-credentials/credentials/:credential_ref/apply-latest-probe",
                post(move |Json(_body): Json<Value>| {
                    let route_called = Arc::clone(&route_called);
                    async move {
                        route_called.store(true, Ordering::SeqCst);
                        Json(json!({"unexpected": true}))
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!(
            "ONE_AI_KEY_TEST_PROBE_APPLY_REF_TOKEN_{}",
            std::process::id()
        );
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run(super::KeysCommand::ProbeApply(
            super::KeysProbeApplyCommand::Apply(super::KeysProbeApplyApplyOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some(management_url),
                    deprecated_base_url: None,
                    management_token_env: Some(env_name.clone()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-credentials".to_string(),
                credential_ref: "cr:v1:pos:0".to_string(),
                probe_result_ref: Some("pr:v1:id:7".to_string()),
                mode: super::KeysProbeApplyMode::DryRun,
                output: crate::cli_report::OutputFormat::Json,
            }),
        ))
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["reason_code"], "probe_result_ref_mismatch");
        assert_eq!(report["data"]["probe_result_ref"], "pr:v1:id:9");
        assert_eq!(report["data"]["expected_probe_result_ref"], "pr:v1:id:7");
        assert_eq!(report["data"]["probe_result_ref_matches"], false);
        assert_eq!(report["next_action"]["safe_argv"], Value::Null);
        assert!(!post_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn keys_probe_apply_confirmed_posts_precondition_and_sanitizes_response() {
        let captured_body = Arc::new(Mutex::new(None::<Value>));
        let route_body = Arc::clone(&captured_body);
        let router = Router::new().route(
            "/management/credential-sets/relay-credentials/credentials/:credential_ref/apply-latest-probe",
            post(move |Json(body): Json<Value>| {
                let route_body = Arc::clone(&route_body);
                async move {
                    *route_body.lock().unwrap() = Some(body);
                    Json(json!({
                        "credential_set_id": "relay-credentials",
                        "credential_id": "internal-derived-id",
                        "action": "expire",
                        "probe": {
                            "outcome": "invalid",
                            "channel_id": "relay-channel",
                            "provider_id": "relay-provider",
                            "account_id": "relay-account",
                            "upstream_status": 401,
                            "upstream_code": "invalid_api_key",
                            "upstream_limit_type": null,
                            "created_at_unix_seconds": 123
                        },
                        "mutation": {
                            "channel_id": "relay-channel",
                            "credential_set_id": "relay-credentials",
                            "credential_id": "internal-derived-id",
                            "selector_generation": 9,
                            "state": {"kind": "expired", "reason": "redacted"},
                            "credential": {
                                "id": "internal-derived-id",
                                "fingerprint": "fingerprint-fixture",
                                "state": {"kind": "expired", "reason": "redacted"}
                            }
                        },
                        "raw_request_body": "request-body-fixture",
                        "raw_response_body": "response-body-fixture"
                    }))
                }
            }),
        );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!(
            "ONE_AI_KEY_TEST_PROBE_APPLY_POST_TOKEN_{}",
            std::process::id()
        );
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run(super::KeysCommand::ProbeApply(
            super::KeysProbeApplyCommand::Apply(super::KeysProbeApplyApplyOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some(management_url),
                    deprecated_base_url: None,
                    management_token_env: Some(env_name.clone()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-credentials".to_string(),
                credential_ref: "cr:v1:pos:0".to_string(),
                probe_result_ref: Some("pr:v1:id:7".to_string()),
                mode: super::KeysProbeApplyMode::Apply,
                output: crate::cli_report::OutputFormat::Json,
            }),
        ))
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let body = captured_body
            .lock()
            .unwrap()
            .clone()
            .expect("apply endpoint should receive a body");
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(body["probe_result_ref"], "pr:v1:id:7");
        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason_code"], "keys_probe_apply_applied");
        assert_eq!(report["side_effect_class"], "management_write");
        assert_eq!(report["data"]["credential_ref"], "cr:v1:pos:0");
        assert_eq!(report["data"]["probe_result_ref"], "pr:v1:id:7");
        assert_eq!(report["data"]["applied_action"], "expire");
        assert_eq!(report["data"]["probe"]["outcome"], "invalid");
        assert_eq!(report["data"]["mutation"]["selector_generation"], 9);
        assert_eq!(report["data"]["mutation"]["state_kind"], "expired");
        assert_eq!(report["data"]["lifecycle_mutation_applied"], true);
        assert!(!rendered.contains("internal-derived-id"));
        assert!(!rendered.contains("fingerprint-fixture"));
        assert!(!rendered.contains("request-body-fixture"));
        assert!(!rendered.contains("response-body-fixture"));
        assert!(!rendered.contains("relay-provider"));
        assert!(!rendered.contains("relay-account"));
    }

    #[tokio::test]
    async fn keys_probe_apply_confirmed_requires_probe_result_ref_before_post() {
        let post_called = Arc::new(AtomicBool::new(false));
        let route_called = Arc::clone(&post_called);
        let router = Router::new().route(
            "/management/credential-sets/relay-credentials/credentials/:credential_ref/apply-latest-probe",
            post(move |Json(_body): Json<Value>| {
                let route_called = Arc::clone(&route_called);
                async move {
                    route_called.store(true, Ordering::SeqCst);
                    Json(json!({"unexpected": true}))
                }
            }),
        );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!(
            "ONE_AI_KEY_TEST_PROBE_APPLY_MISSING_REF_TOKEN_{}",
            std::process::id()
        );
        std::env::set_var(&env_name, "opaque-management-fixture");

        let error = super::run(super::KeysCommand::ProbeApply(
            super::KeysProbeApplyCommand::Apply(super::KeysProbeApplyApplyOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some(management_url),
                    deprecated_base_url: None,
                    management_token_env: Some(env_name.clone()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-credentials".to_string(),
                credential_ref: "cr:v1:pos:0".to_string(),
                probe_result_ref: None,
                mode: super::KeysProbeApplyMode::Apply,
                output: crate::cli_report::OutputFormat::Json,
            }),
        ))
        .await
        .expect_err("confirmed apply should require probe_result_ref");
        std::env::remove_var(env_name);

        assert_eq!(error.reason_code(), "probe_result_ref_required");
        assert!(!post_called.load(Ordering::SeqCst));
    }

    #[test]
    fn keys_stats_report_uses_summary_projection_and_drops_secret_derived_fields() {
        let projection = json!({
            "credential_sets": [{
                "id": "shared-credentials",
                "channels": 2,
                "channel_ids": ["relay-a", "relay-b"],
                "account_ids": ["account-a"],
                "provider_ids": ["provider-a"],
                "credentials": {
                    "total": 3,
                    "available": 1,
                    "cooling_down": 1,
                    "expired": 0,
                    "quota_exhausted": 1,
                    "disabled": 0,
                    "refs": ["cr:v1:pos:0", "cr:v1:pos:1"],
                    "credential_id": "cred_secret_internal",
                    "fingerprint": "fp-secret",
                    "raw_hash": "hash-secret",
                    "prefix": "RAW_PREFIX_SECRET",
                    "suffix": "RAW_SUFFIX_SECRET",
                    "hmac": "hmac-secret",
                    "raw_key": "RAW_KEY_SECRET"
                },
                "probe_summary": {
                    "status": "available",
                    "total_credentials": 3,
                    "available_credentials": 1,
                    "latest_probe_at_unix_seconds": 1710000000,
                    "fingerprint": "probe-fp-secret",
                    "raw_hash": "probe-hash-secret"
                },
                "latest_probe": {
                    "credential_id": "cred_probe_secret",
                    "fingerprint": "probe-latest-fp-secret"
                },
                "key_import": {
                    "source_kind": "management_api",
                    "unique_count": 3,
                    "duplicate_occurrence_count": 0,
                    "ignored_empty_count": 0,
                    "invalid_line_count": 0,
                    "created_at_unix_seconds": 1700000000,
                    "source_path": "/tmp/raw-secret-source"
                },
                "credential_id": "cred_top_secret",
                "token_hash": "token-secret"
            }]
        });

        let rendered =
            super::render_keys_stats_report(&projection, crate::cli_report::OutputFormat::Json);

        assert!(rendered.contains("\"command\": \"keys stats\""));
        assert!(rendered.contains("\"status\": \"ok\""));
        assert!(rendered.contains("\"reason_code\": \"credential_set_stats_projected\""));
        assert!(rendered.contains("\"side_effect_class\": \"runtime_readonly\""));
        assert!(rendered.contains("\"effect_vector\""));
        assert!(rendered.contains("\"reads_management_runtime\": true"));
        assert!(rendered.contains("\"reads_management_store\": true"));
        assert!(rendered.contains("\"window\": null"));
        assert!(rendered.contains("\"scan_mode\": \"summary_projection_only\""));
        assert!(rendered.contains("\"credential_set_id\": \"shared-credentials\""));
        assert!(rendered.contains("\"available\": 1"));
        assert!(rendered.contains("\"quota_exhausted\": 1"));
        assert!(rendered.contains("\"probe_summary_status\": \"available\""));
        assert!(rendered.contains("\"credential_refs\""));
        assert!(rendered.contains("cr:v1:pos:0"));
        assert!(rendered.contains("cr:v1:pos:1"));
        assert!(rendered.contains("keys_probe_unavailable_until_m3"));
        assert!(!rendered.contains("cred_secret_internal"));
        assert!(!rendered.contains("cred_probe_secret"));
        assert!(!rendered.contains("fp-secret"));
        assert!(!rendered.contains("hash-secret"));
        assert!(!rendered.contains("RAW_PREFIX_SECRET"));
        assert!(!rendered.contains("RAW_SUFFIX_SECRET"));
        assert!(!rendered.contains("hmac-secret"));
        assert!(!rendered.contains("RAW_KEY_SECRET"));
        assert!(!rendered.contains("/tmp/raw-secret-source"));
        assert!(!rendered.contains("token-secret"));
    }

    #[test]
    fn keys_list_marks_probe_summary_unavailable_without_bounded_summary_projection() {
        let projection = json!({
            "credential_sets": [{
                "id": "shared-credentials",
                "channels": 1,
                "credentials": {
                    "total": 1,
                    "available": 1,
                    "refs": ["cr:v1:pos:0\nsecret", "cr:v1:pos:1/extra", "cr:v1:pos:2"]
                }
            }]
        });

        let rendered =
            super::render_keys_list_report(&projection, crate::cli_report::OutputFormat::Json);

        assert!(rendered.contains("\"command\": \"keys list\""));
        assert!(rendered.contains("\"side_effect_class\": \"runtime_readonly\""));
        assert!(rendered.contains("\"effect_vector\""));
        assert!(rendered
            .contains("\"probe_summary_status\": \"unavailable_without_summary_projection\""));
        assert!(
            rendered.contains("\"credential_ref_status\": \"available_from_summary_projection\"")
        );
        assert!(!rendered.contains("cr:v1:pos:0\\nsecret"));
        assert!(!rendered.contains("cr:v1:pos:1/extra"));
        assert!(rendered.contains("cr:v1:pos:2"));
        assert!(rendered.contains("\"latest_probe_summary\": null"));
        assert!(rendered.contains("\"next_action\""));
        assert!(!rendered.contains("keys probe"));
    }

    #[test]
    fn keys_stats_probe_summary_whitelist_enforces_field_types() {
        let projection = json!({
            "credential_sets": [{
                "id": "shared-credentials",
                "channels": 1,
                "credentials": {
                    "total": 1,
                    "available": 1
                },
                "probe_summary": {
                    "status": {"raw_key": "RAW_STATUS_SECRET"},
                    "success": "RAW_SUCCESS_SECRET",
                    "invalid": 1,
                    "latest_probe_at_unix_seconds": "RAW_TIME_SECRET"
                }
            }]
        });

        let rendered =
            super::render_keys_stats_report(&projection, crate::cli_report::OutputFormat::Json);

        assert!(rendered.contains("\"invalid\": 1"));
        assert!(!rendered.contains("RAW_STATUS_SECRET"));
        assert!(!rendered.contains("RAW_SUCCESS_SECRET"));
        assert!(!rendered.contains("RAW_TIME_SECRET"));
        assert!(!rendered.contains("raw_key"));
    }

    #[test]
    fn keys_replacement_plan_render_summarizes_capacity_and_sanitizes_route_context() {
        let projection = json!({
            "credential_sets": [{
                "id": "shared-credentials",
                "channels": 1,
                "channel_ids": ["relay-a"],
                "account_ids": ["account-a"],
                "provider_ids": ["provider-a"],
                "credentials": {
                    "total": 3,
                    "available": 0,
                    "cooling_down": 1,
                    "expired": 1,
                    "quota_exhausted": 1,
                    "disabled": 0,
                    "refs": ["cr:v1:pos:0", "cr:v1:pos:1"],
                    "raw_key": "RAW_KEY_SECRET"
                },
                "probe_summary": {
                    "status": "available",
                    "total_credentials": 3,
                    "quota_exhausted": 1,
                    "invalid": 1,
                    "fingerprint": "probe-fp-secret"
                },
                "key_import": {
                    "source_kind": "management_api",
                    "source_path": "/Users/rtoc/private.keys"
                },
                "token_hash": "token-secret"
            }]
        });
        let operations = json!({
            "credential_set_id": "shared-credentials",
            "status": "blocked",
            "serving_mode": "unavailable",
            "accepting_requests": false,
            "needs_operator_input": true,
            "required_action": "import_replacement_credentials",
            "credentials": {
                "total": 3,
                "available": 0,
                "cooling_down": 1,
                "expired": 1,
                "quota_exhausted": 1
            },
            "raw_response_body": "response-secret"
        });
        let route = json!({
            "request_id": "preview-local",
            "model": "gpt-example",
            "route_kind": "explicit_model_route",
            "registry_generation": 8,
            "candidate_limit": 16,
            "policy_summary": {
                "route_target_retry_enabled": true,
                "same_request_credential_retry_enabled": false,
                "max_same_request_retries": 0,
                "candidate_limit": 16
            },
            "client_token": {
                "id": "client-local",
                "name": "local-client",
                "unrestricted_model_groups": true,
                "unrestricted_channels": true,
                "raw_token": "CLIENT_SECRET"
            },
            "admission_summary": {
                "status": "unavailable",
                "reason_code": "no_usable_key_or_target",
                "selected_target": null,
                "candidate_count": 2,
                "included_count": 0,
                "blocked_count": 2,
                "soft_suppressed_count": 0,
                "hard_blocked_count": 2,
                "last_resort_used": false,
                "last_resort_reason": null
            },
            "selected_target": {
                "channel_id": "relay-a",
                "plan_position": 0
            },
            "candidates": [
                {
                    "target_index": 0,
                    "channel_id": "relay-a",
                    "provider_kind": "openai_compatible",
                    "plan_position": 0,
                    "included": false,
                    "selected": true,
                    "reasons": ["credential_pool_exhausted"],
                    "credential_set_id": "shared-credentials",
                    "selector_generation": 7,
                    "credentials": {"total": 3, "available": 0},
                    "raw_key": "RAW_ROUTE_KEY_SECRET"
                },
                {
                    "target_index": 1,
                    "channel_id": "relay-b",
                    "provider_kind": "openai_compatible",
                    "plan_position": 1,
                    "included": false,
                    "selected": false,
                    "reasons": ["channel_cooling_down"],
                    "credential_set_id": "backup-credentials",
                    "selector_generation": 8,
                    "credentials": {"total": 1, "available": 0},
                    "credential_id": "internal-credential-secret"
                }
            ],
            "raw_request_body": "request-secret"
        });
        let options = super::KeysReplacementPlanOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            credential_set_id: "shared-credentials".to_string(),
            model: Some("gpt-example".to_string()),
            client_token_ref: Some("local-client".to_string()),
            include_credential_refs: true,
            credential_ref_limit: 2,
            output: crate::cli_report::OutputFormat::Json,
        };

        let rendered = super::render_keys_replacement_plan_report(
            &options,
            &projection,
            Some(&operations),
            None,
            Some(&route),
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason_code"], "keys_replacement_plan_projected");
        assert_eq!(report["side_effect_class"], "runtime_readonly");
        assert_eq!(report["effect_vector"]["reads_management_runtime"], true);
        assert_eq!(report["effect_vector"]["reads_management_store"], true);
        assert_eq!(report["scope"]["credential_set_id"], "shared-credentials");
        assert_eq!(report["data"]["capacity_summary"]["available"], json!(0));
        assert_eq!(
            report["data"]["replacement_need"]["status"],
            "replacement_recommended"
        );
        assert_eq!(
            report["data"]["replacement_need"]["reason_code"],
            "import_replacement_credentials"
        );
        assert_eq!(report["data"]["route_impact"]["status"], "available");
        assert_eq!(report["data"]["route_impact"]["model"], "gpt-example");
        assert_eq!(
            report["data"]["route_impact"]["client_token_ref"],
            "local-client"
        );
        assert_eq!(
            report["data"]["route_impact"]["admission_status"],
            "unavailable"
        );
        assert_eq!(
            report["data"]["route_impact"]["reason_code"],
            "no_usable_key_or_target"
        );
        assert_eq!(
            report["data"]["route_impact"]["requested_credential_set"]["candidate_presence"],
            "selected_candidate"
        );
        assert_eq!(
            report["data"]["route_impact"]["requested_credential_set"]["selected_candidate"],
            true
        );
        assert_eq!(
            report["data"]["route_impact"]["selected_target"]["credential_set_id"],
            "shared-credentials"
        );
        assert_eq!(
            report["data"]["safe_next_actions"][0]["safe_argv"][0],
            "one-ai-key"
        );
        assert!(report["data"]["safe_next_actions"]
            .to_string()
            .contains("import"));
        assert!(report["data"]["safe_next_actions"]
            .to_string()
            .contains("dry-run"));
        assert!(!report["data"]["safe_next_actions"]
            .to_string()
            .contains("--yes"));
        assert!(!rendered.contains("RAW_KEY_SECRET"));
        assert!(!rendered.contains("RAW_ROUTE_KEY_SECRET"));
        assert!(!rendered.contains("CLIENT_SECRET"));
        assert!(!rendered.contains("internal-credential-secret"));
        assert!(!rendered.contains("token-secret"));
        assert!(!rendered.contains("/Users/rtoc/private.keys"));
        assert!(!rendered.contains("request-secret"));
        assert!(!rendered.contains("response-secret"));

        let table_options = super::KeysReplacementPlanOptions {
            output: crate::cli_report::OutputFormat::Table,
            ..options
        };
        let table_rendered = super::render_keys_replacement_plan_report(
            &table_options,
            &projection,
            Some(&operations),
            None,
            Some(&route),
        );

        assert!(table_rendered.contains(
            "route_impact.requested_credential_set.candidate_presence: selected_candidate"
        ));
        assert!(!table_rendered.contains("RAW_KEY_SECRET"));
        assert!(!table_rendered.contains("RAW_ROUTE_KEY_SECRET"));
        assert!(!table_rendered.contains("CLIENT_SECRET"));
        assert!(!table_rendered.contains("internal-credential-secret"));
        assert!(!table_rendered.contains("token-secret"));
        assert!(!table_rendered.contains("/Users/rtoc/private.keys"));
        assert!(!table_rendered.contains("request-secret"));
        assert!(!table_rendered.contains("response-secret"));
    }

    #[test]
    fn keys_replacement_plan_without_model_marks_route_impact_unavailable() {
        let options = super::KeysReplacementPlanOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            credential_set_id: "shared-credentials".to_string(),
            model: None,
            client_token_ref: Some("local-client".to_string()),
            include_credential_refs: false,
            credential_ref_limit: 20,
            output: crate::cli_report::OutputFormat::Json,
        };

        let rendered = super::render_keys_replacement_plan_report(
            &options,
            &json!({
                "credential_sets": [{
                    "id": "shared-credentials",
                    "credentials": {"total": 1, "available": 1}
                }]
            }),
            None,
            None,
            None,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(
            report["data"]["route_impact"]["status"],
            "unavailable_without_model_context"
        );
    }

    #[tokio::test]
    async fn keys_replacement_plan_uses_existing_readonly_endpoints_without_posting() {
        let called_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let post_called = Arc::new(AtomicBool::new(false));
        let paths = Arc::clone(&called_paths);
        let router = Router::new()
            .route(
                "/management/credential-sets",
                get(move || {
                    let paths = Arc::clone(&paths);
                    async move {
                        paths
                            .lock()
                            .unwrap()
                            .push("/management/credential-sets".to_string());
                        Json(json!({
                            "credential_sets": [{
                                "id": "relay-credentials",
                                "channels": 1,
                                "credentials": {"total": 2, "available": 0, "quota_exhausted": 2}
                            }]
                        }))
                    }
                }),
            )
            .route(
                "/management/credential-sets/relay-credentials/operations",
                get({
                    let paths = Arc::clone(&called_paths);
                    move || {
                        let paths = Arc::clone(&paths);
                        async move {
                            paths.lock().unwrap().push(
                                "/management/credential-sets/relay-credentials/operations"
                                    .to_string(),
                            );
                            Json(json!({
                                "credential_set_id": "relay-credentials",
                                "needs_operator_input": true,
                                "required_action": "import_replacement_credentials",
                                "credentials": {"total": 2, "available": 0, "quota_exhausted": 2}
                            }))
                        }
                    }
                }),
            )
            .route(
                "/management/credential-sets/relay-credentials/credentials",
                get({
                    let paths = Arc::clone(&called_paths);
                    move || {
                        let paths = Arc::clone(&paths);
                        async move {
                            paths.lock().unwrap().push(
                                "/management/credential-sets/relay-credentials/credentials"
                                    .to_string(),
                            );
                            Json(json!({
                                "credential_set_id": "relay-credentials",
                                "offset": 0,
                                "limit": 2,
                                "total_credentials": 2,
                                "credentials": [{"credential_ref": "cr:v1:pos:0"}]
                            }))
                        }
                    }
                }),
            )
            .route(
                "/management/routing/preview",
                get({
                    let paths = Arc::clone(&called_paths);
                    move || {
                        let paths = Arc::clone(&paths);
                        async move {
                            paths
                                .lock()
                                .unwrap()
                                .push("/management/routing/preview".to_string());
                            Json(json!({
                                "request_id": "preview-local",
                                "model": "gpt-example",
                                "route_kind": "explicit_model_route",
                                "registry_generation": 8,
                                "candidate_limit": 16,
                                "policy_summary": {
                                    "route_target_retry_enabled": true,
                                    "same_request_credential_retry_enabled": false,
                                    "max_same_request_retries": 0,
                                    "candidate_limit": 16
                                },
                                "client_token": {
                                    "id": "client-local",
                                    "name": "local-client",
                                    "unrestricted_model_groups": true,
                                    "unrestricted_channels": true
                                },
                                "admission_summary": {
                                    "status": "unavailable",
                                    "reason_code": "no_usable_key_or_target",
                                    "selected_target": null,
                                    "candidate_count": 1,
                                    "included_count": 0,
                                    "blocked_count": 1,
                                    "soft_suppressed_count": 0,
                                    "hard_blocked_count": 1,
                                    "last_resort_used": false,
                                    "last_resort_reason": null
                                },
                                "selected_target": null,
                                "candidates": [{
                                    "target_index": 0,
                                    "channel_id": "relay-a",
                                    "provider_kind": "openai_compatible",
                                    "plan_position": 0,
                                    "included": false,
                                    "selected": false,
                                    "reasons": ["credential_pool_exhausted"],
                                    "credential_set_id": "relay-credentials",
                                    "selector_generation": 7,
                                    "credentials": {"total": 2, "available": 0}
                                }]
                            }))
                        }
                    }
                }),
            )
            .route(
                "/management/credential-sets/relay-credentials/credentials/import",
                post({
                    let post_called = Arc::clone(&post_called);
                    move |Json(_body): Json<Value>| {
                        let post_called = Arc::clone(&post_called);
                        async move {
                            post_called.store(true, Ordering::SeqCst);
                            Json(json!({"unexpected": true}))
                        }
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!(
            "ONE_AI_KEY_TEST_REPLACEMENT_PLAN_TOKEN_{}",
            std::process::id()
        );
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run(super::KeysCommand::ReplacementPlan(
            super::KeysReplacementPlanOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some(management_url),
                    deprecated_base_url: None,
                    management_token_env: Some(env_name.clone()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-credentials".to_string(),
                model: Some("gpt-example".to_string()),
                client_token_ref: Some("local-client".to_string()),
                include_credential_refs: true,
                credential_ref_limit: 2,
                output: crate::cli_report::OutputFormat::Json,
            },
        ))
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let paths = called_paths.lock().unwrap().clone();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["data"]["route_impact"]["status"], "available");
        assert_eq!(
            report["data"]["route_impact"]["requested_credential_set"]["candidate_presence"],
            "candidate_not_selected"
        );
        assert_eq!(
            paths,
            vec![
                "/management/credential-sets",
                "/management/credential-sets/relay-credentials/operations",
                "/management/credential-sets/relay-credentials/credentials",
                "/management/routing/preview",
            ]
        );
        assert!(!post_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn keys_replacement_plan_route_preview_failure_degrades_without_posting_or_raw_body() {
        let called_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let post_called = Arc::new(AtomicBool::new(false));
        let paths = Arc::clone(&called_paths);
        let router = Router::new()
            .route(
                "/management/credential-sets",
                get(move || {
                    let paths = Arc::clone(&paths);
                    async move {
                        paths
                            .lock()
                            .unwrap()
                            .push("/management/credential-sets".to_string());
                        Json(json!({
                            "credential_sets": [{
                                "id": "relay-credentials",
                                "channels": 1,
                                "credentials": {"total": 2, "available": 1, "quota_exhausted": 1}
                            }]
                        }))
                    }
                }),
            )
            .route(
                "/management/credential-sets/relay-credentials/operations",
                get({
                    let paths = Arc::clone(&called_paths);
                    move || {
                        let paths = Arc::clone(&paths);
                        async move {
                            paths.lock().unwrap().push(
                                "/management/credential-sets/relay-credentials/operations"
                                    .to_string(),
                            );
                            Json(json!({
                                "credential_set_id": "relay-credentials",
                                "needs_operator_input": false,
                                "credentials": {"total": 2, "available": 1, "quota_exhausted": 1}
                            }))
                        }
                    }
                }),
            )
            .route(
                "/management/routing/preview",
                get({
                    let paths = Arc::clone(&called_paths);
                    move || {
                        let paths = Arc::clone(&paths);
                        async move {
                            paths
                                .lock()
                                .unwrap()
                                .push("/management/routing/preview".to_string());
                            (StatusCode::NOT_FOUND, "ROUTE_PREVIEW_RAW_BODY_SECRET")
                        }
                    }
                }),
            )
            .route(
                "/management/credential-sets/relay-credentials/credentials/import",
                post({
                    let post_called = Arc::clone(&post_called);
                    move |Json(_body): Json<Value>| {
                        let post_called = Arc::clone(&post_called);
                        async move {
                            post_called.store(true, Ordering::SeqCst);
                            Json(json!({"unexpected": true}))
                        }
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!(
            "ONE_AI_KEY_TEST_REPLACEMENT_PLAN_ROUTE_FAIL_TOKEN_{}",
            std::process::id()
        );
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run(super::KeysCommand::ReplacementPlan(
            super::KeysReplacementPlanOptions {
                connection: crate::cli::OperatorConnectionOptions {
                    management_url: Some(management_url),
                    deprecated_base_url: None,
                    management_token_env: Some(env_name.clone()),
                    management_token_stdin: false,
                    timeout_seconds: 10,
                },
                credential_set_id: "relay-credentials".to_string(),
                model: Some("gpt-example".to_string()),
                client_token_ref: Some("local-client".to_string()),
                include_credential_refs: false,
                credential_ref_limit: 2,
                output: crate::cli_report::OutputFormat::Json,
            },
        ))
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();
        let paths = called_paths.lock().unwrap().clone();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason_code"], "keys_replacement_plan_projected");
        assert_eq!(
            report["data"]["route_impact"]["status"],
            "unavailable_without_route_projection"
        );
        assert_eq!(
            report["data"]["route_impact"]["requested_credential_set"]["candidate_presence"],
            "unknown"
        );
        assert_eq!(
            paths,
            vec![
                "/management/credential-sets",
                "/management/credential-sets/relay-credentials/operations",
                "/management/routing/preview",
            ]
        );
        assert!(!post_called.load(Ordering::SeqCst));
        assert!(!rendered.contains("ROUTE_PREVIEW_RAW_BODY_SECRET"));
    }

    #[test]
    fn keys_stats_endpoint_plan_does_not_page_scan_credentials_by_default() {
        assert_eq!(
            super::keys_stats_sets_endpoint(),
            crate::operator_client::ReadOnlyEndpoint::CredentialSets
        );
        assert_eq!(
            super::keys_stats_operations_endpoint("shared-credentials"),
            crate::operator_client::ReadOnlyEndpoint::CredentialSetOperations {
                credential_set_id: "shared-credentials".to_string()
            }
        );
        assert_eq!(
            super::keys_stats_credentials_endpoint("shared-credentials", 500),
            crate::operator_client::ReadOnlyEndpoint::CredentialSetCredentials {
                credential_set_id: "shared-credentials".to_string(),
                offset: Some(0),
                limit: Some(50),
            }
        );
    }

    #[test]
    fn keys_stats_operations_merge_uses_runtime_projection_shape_and_redacts_alert_noise() {
        let projection = json!({
            "credential_sets": [{
                "id": "shared-credentials",
                "channels": 1,
                "credentials": {
                    "total": 2,
                    "available": 0,
                    "cooling_down": 1,
                    "quota_exhausted": 1
                },
                "probe_summary": {
                    "total_credentials": 2,
                    "probed_credentials": 2,
                    "quota_exhausted": 1,
                    "rate_limited": 1
                }
            }]
        });
        let operations = json!({
            "credential_set_id": "shared-credentials",
            "channels": 1,
            "channel_ids": ["relay-a"],
            "account_ids": ["account-a"],
            "provider_ids": ["provider-a"],
            "status": "blocked",
            "serving_mode": "unavailable",
            "accepting_requests": false,
            "needs_operator_input": true,
            "required_action": "probe_or_import_credentials",
            "credentials": {
                "total": 2,
                "available": 0,
                "cooling_down": 1,
                "quota_exhausted": 1,
                "fingerprint": "ops-fp-secret"
            },
            "alerts": [{
                "kind": "credential_pool_exhausted",
                "severity": "critical",
                "message": "No credentials are available: sk-alert-secret https://host.example/path?token=sk-url-secret /Users/rtoc/.one-ai-key/keys.txt request body {\"Authorization\":\"Bearer raw-token\"} response body {\"token\":\"secret\"} 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "credential_id": "cred-alert-secret"
            }],
            "raw_key": "RAW_OPS_SECRET"
        });

        let mut report =
            super::sanitized_keys_stats_report(&projection, Some("shared-credentials"));
        super::merge_operations_into_stats_report(&mut report, &operations);
        let rendered = serde_json::to_string_pretty(&report).unwrap();

        assert!(rendered.contains("\"operations\""));
        assert!(rendered.contains("\"status\": \"blocked\""));
        assert!(rendered.contains("\"required_action\": \"probe_or_import_credentials\""));
        assert!(rendered.contains("\"kind\": \"credential_pool_exhausted\""));
        assert!(rendered.contains("\"summary\""));
        assert!(rendered.contains("\"next_action\""));
        assert!(rendered.contains("\"safe_argv\""));
        assert!(rendered.contains("keys_probe_unavailable_until_m3"));
        assert!(rendered.contains("\"shared-credentials\""));
        assert!(!rendered.contains("<credential-set-id>"));
        assert!(!rendered.contains("keys probe"));
        assert!(!rendered.contains("probe-apply"));
        assert!(!rendered.contains("No credentials are available"));
        assert!(!rendered.contains("sk-alert-secret"));
        assert!(!rendered.contains("sk-url-secret"));
        assert!(!rendered.contains("https://host.example/path"));
        assert!(!rendered.contains("/Users/rtoc/.one-ai-key/keys.txt"));
        assert!(!rendered.contains("Authorization"));
        assert!(!rendered.contains("raw-token"));
        assert!(!rendered.contains("response body"));
        assert!(!rendered.contains("0123456789abcdef0123456789abcdef"));
        assert!(!rendered.contains("ops-fp-secret"));
        assert!(!rendered.contains("cred-alert-secret"));
        assert!(!rendered.contains("RAW_OPS_SECRET"));
    }

    #[test]
    fn keys_next_action_rejects_unsafe_dynamic_safe_argv_args() {
        let report = super::sanitized_keys_stats_report(
            &json!({
                "credential_sets": [{
                    "id": "unsafe/credential-set",
                    "channels": 1,
                    "credentials": {"total": 1, "available": 0}
                }]
            }),
            Some("unsafe/credential-set"),
        );

        assert_eq!(report["next_action"]["safe_argv"], Value::Null);
        assert!(!report.to_string().contains("unsafe/credential-set"));

        let token_like_id = ["sk", "secret"].join("-");
        let report = super::sanitized_keys_stats_report(
            &json!({
                "credential_sets": [{
                    "id": token_like_id,
                    "channels": 1,
                    "credentials": {"total": 1, "available": 0}
                }]
            }),
            Some(&token_like_id),
        );

        assert_eq!(report["next_action"]["safe_argv"], Value::Null);
        assert!(!report.to_string().contains(&token_like_id));

        let report = super::sanitized_keys_stats_report(
            &json!({
                "credential_sets": [{
                    "id": "https:relay",
                    "channels": 1,
                    "credentials": {"total": 1, "available": 0}
                }]
            }),
            Some("https:relay"),
        );

        assert_eq!(report["next_action"]["safe_argv"], Value::Null);
        assert!(!report.to_string().contains("https:relay"));

        for unsafe_id in [
            [
                "Aa0", "Bb1", "Cc2", "Dd3", "Ee4", "Ff5", "Gg6", "Hh7", "Ii8", "Jj9", "Kk0",
            ]
            .join(""),
            ["aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc"].join("."),
            ["ignore", "instruction"].join("_"),
        ] {
            let report = super::sanitized_keys_stats_report(
                &json!({
                    "credential_sets": [{
                        "id": unsafe_id,
                        "channels": 1,
                        "credentials": {"total": 1, "available": 0}
                    }]
                }),
                Some(&unsafe_id),
            );

            assert_eq!(report["next_action"]["safe_argv"], Value::Null);
            assert!(!report.to_string().contains(&unsafe_id));
        }
    }

    #[test]
    fn keys_stats_merges_explicit_bounded_credential_ref_page_without_internal_ids() {
        let projection = json!({
            "credential_sets": [{
                "id": "shared-credentials",
                "channels": 1,
                "credentials": {
                    "total": 2,
                    "available": 2
                },
                "probe_summary": {
                    "total_credentials": 2,
                    "unprobed_credentials": 2
                }
            }]
        });
        let credentials_page = json!({
            "credential_set_id": "shared-credentials",
            "channel_ids": ["relay-a"],
            "total_credentials": 2,
            "filtered_credentials": 2,
            "offset": 0,
            "limit": 2,
            "credentials": [
                {
                    "id": "cred_internal_secret",
                    "credential_ref": "cr:v1:pos:0",
                    "fingerprint": "fp-secret",
                    "source": {"source_id": "source-secret"}
                },
                {
                    "id": "cred_internal_secret_2",
                    "credential_ref": "cr:v1:pos:1\nsecret",
                    "fingerprint": "fp-secret-2"
                }
            ]
        });

        let mut report =
            super::sanitized_keys_stats_report(&projection, Some("shared-credentials"));
        super::merge_credential_refs_into_stats_report(&mut report, &credentials_page);
        let rendered = serde_json::to_string_pretty(&report).unwrap();

        assert!(rendered.contains("\"credential_ref_status\": \"partial_bounded_page\""));
        assert!(rendered.contains("\"credential_ref_window\""));
        assert!(rendered.contains("\"returned_credential_refs\": 1"));
        assert!(rendered.contains("\"dropped_credentials\": 1"));
        assert!(rendered.contains("cr:v1:pos:0"));
        assert!(!rendered.contains("cr:v1:pos:1\\nsecret"));
        assert!(!rendered.contains("cred_internal_secret"));
        assert!(!rendered.contains("fp-secret"));
        assert!(!rendered.contains("source-secret"));
    }

    #[test]
    fn keys_stats_rejects_mismatched_credential_ref_page() {
        let projection = json!({
            "credential_sets": [{
                "id": "shared-credentials",
                "channels": 1,
                "credentials": {
                    "total": 1,
                    "available": 1
                }
            }]
        });
        let credentials_page = json!({
            "credential_set_id": "other-credentials",
            "total_credentials": 1,
            "filtered_credentials": 1,
            "offset": 0,
            "limit": 1,
            "credentials": [
                {
                    "id": "cred_internal_secret",
                    "credential_ref": "cr:v1:pos:0",
                    "fingerprint": "fp-secret"
                }
            ]
        });

        let mut report =
            super::sanitized_keys_stats_report(&projection, Some("shared-credentials"));
        super::merge_credential_refs_into_stats_report(&mut report, &credentials_page);
        let rendered = serde_json::to_string_pretty(&report).unwrap();

        assert!(rendered.contains("\"credential_ref_status\": \"credential_ref_page_mismatch\""));
        assert!(!rendered.contains("cr:v1:pos:0"));
        assert!(!rendered.contains("cred_internal_secret"));
        assert!(!rendered.contains("fp-secret"));
    }

    #[test]
    fn keys_table_escapes_control_characters() {
        let projection = json!({
            "credential_sets": [{
                "id": "shared\ncredentials\u{1b}[31m",
                "channels": 1,
                "credentials": {
                    "total": 1,
                    "available": 1,
                    "refs": ["cr:v1:pos:0"]
                },
                "probe_summary": {
                    "status": "ready\rnow"
                }
            }]
        });

        let rendered =
            super::render_keys_list_report(&projection, crate::cli_report::OutputFormat::Table);

        assert!(rendered.contains("side_effect_class: runtime_readonly"));
        assert!(rendered.contains("effect.reads_management_runtime: true"));
        assert!(rendered.contains("next_action.safe_argv[0]: one-ai-key"));
        assert!(!rendered.contains("shared\\ncredentials\\u{1b}[31m"));
        assert!(!rendered.contains("shared\ncredentials"));
        assert!(rendered.contains("ready\\rnow"));
        assert!(!rendered.contains('\u{1b}'));
    }

    fn assert_no_probe_apply_recommendation(report: &Value) {
        for field in [
            &report["next_action"],
            &report["data"]["recovery_path"],
            &report["next_action"]["recovery_path"],
        ] {
            assert!(
                !field.to_string().contains("probe-apply"),
                "probe-apply recommendation leaked: {field}"
            );
        }
    }
}

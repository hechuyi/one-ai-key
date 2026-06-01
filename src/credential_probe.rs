use futures_util::StreamExt;
use serde::Serialize;
use std::time::{Duration, Instant};

use crate::{
    config::{ProbeResultActionKind, ProbeResultPolicy},
    credential_repository::{
        CredentialProbeOutcome, CredentialProbeResultRecord, CredentialProbeResultRecordInput,
        CredentialProbeSummaryRecord, CredentialSetId,
    },
    credentials::CredentialId,
    error::{ClassifiedFailure, FailureKind, FailureScope},
    pool::SelectedKey,
    provider::{ChatProbeSuccessOutcome, CredentialProbeTarget, ProviderAdapter},
    state::PoolState,
};

#[derive(Debug, Clone)]
pub struct CredentialProbeCommand {
    pub credential_set_id: String,
    pub credential_id: CredentialId,
    pub model: String,
    pub kind: CredentialProbeKind,
    pub expected_output: Option<String>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialProbeKind {
    ModelRetrieve,
    ChatCompletion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialProbeFilter {
    All,
    Success,
    Invalid,
    QuotaExhausted,
    RateLimited,
    ProviderUnavailable,
    UnsupportedModel,
    Unknown,
    Unprobed,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialProbeApplyActionStatus {
    Expire,
    QuotaExhaust,
    Restore,
    Cooldown,
    Noop,
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialProbeResultStatus {
    pub outcome: CredentialProbeOutcomeStatus,
    pub channel_id: String,
    pub provider_id: String,
    pub account_id: String,
    pub classifier_id: Option<String>,
    pub adaptation_rule_id: Option<String>,
    pub upstream_status: Option<u16>,
    pub upstream_code: Option<String>,
    pub upstream_limit_type: Option<String>,
    pub latency_ms: u64,
    pub created_at_unix_seconds: i64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialProbeOutcomeStatus {
    Success,
    Invalid,
    QuotaExhausted,
    RateLimited,
    ProviderUnavailable,
    UnsupportedModel,
    Unknown,
}

#[derive(Debug, Serialize, Default)]
pub struct CredentialProbeSummaryStatus {
    pub total_credentials: usize,
    pub probed_credentials: usize,
    pub unprobed_credentials: usize,
    pub success: usize,
    pub invalid: usize,
    pub quota_exhausted: usize,
    pub rate_limited: usize,
    pub provider_unavailable: usize,
    pub unsupported_model: usize,
    pub unknown: usize,
}

pub fn probe_filter_matches(
    filter: CredentialProbeFilter,
    latest_probe: Option<&CredentialProbeResultStatus>,
) -> bool {
    match filter {
        CredentialProbeFilter::All => true,
        CredentialProbeFilter::Unprobed => latest_probe.is_none(),
        CredentialProbeFilter::Success => latest_probe
            .is_some_and(|probe| matches!(probe.outcome, CredentialProbeOutcomeStatus::Success)),
        CredentialProbeFilter::Invalid => latest_probe
            .is_some_and(|probe| matches!(probe.outcome, CredentialProbeOutcomeStatus::Invalid)),
        CredentialProbeFilter::QuotaExhausted => latest_probe.is_some_and(|probe| {
            matches!(probe.outcome, CredentialProbeOutcomeStatus::QuotaExhausted)
        }),
        CredentialProbeFilter::RateLimited => latest_probe.is_some_and(|probe| {
            matches!(probe.outcome, CredentialProbeOutcomeStatus::RateLimited)
        }),
        CredentialProbeFilter::ProviderUnavailable => latest_probe.is_some_and(|probe| {
            matches!(
                probe.outcome,
                CredentialProbeOutcomeStatus::ProviderUnavailable
            )
        }),
        CredentialProbeFilter::UnsupportedModel => latest_probe.is_some_and(|probe| {
            matches!(
                probe.outcome,
                CredentialProbeOutcomeStatus::UnsupportedModel
            )
        }),
        CredentialProbeFilter::Unknown => latest_probe
            .is_some_and(|probe| matches!(probe.outcome, CredentialProbeOutcomeStatus::Unknown)),
    }
}

pub fn probe_filter_outcome(filter: CredentialProbeFilter) -> Option<CredentialProbeOutcome> {
    match filter {
        CredentialProbeFilter::Success => Some(CredentialProbeOutcome::Success),
        CredentialProbeFilter::Invalid => Some(CredentialProbeOutcome::Invalid),
        CredentialProbeFilter::QuotaExhausted => Some(CredentialProbeOutcome::QuotaExhausted),
        CredentialProbeFilter::RateLimited => Some(CredentialProbeOutcome::RateLimited),
        CredentialProbeFilter::ProviderUnavailable => {
            Some(CredentialProbeOutcome::ProviderUnavailable)
        }
        CredentialProbeFilter::UnsupportedModel => Some(CredentialProbeOutcome::UnsupportedModel),
        CredentialProbeFilter::Unknown => Some(CredentialProbeOutcome::Unknown),
        CredentialProbeFilter::All | CredentialProbeFilter::Unprobed => None,
    }
}

pub fn probe_apply_action(
    outcome: CredentialProbeOutcome,
    policy: &ProbeResultPolicy,
) -> CredentialProbeApplyActionStatus {
    let action = match outcome {
        CredentialProbeOutcome::Success => policy.success,
        CredentialProbeOutcome::Invalid => policy.invalid,
        CredentialProbeOutcome::QuotaExhausted => policy.quota_exhausted,
        CredentialProbeOutcome::RateLimited => policy.rate_limited,
        CredentialProbeOutcome::ProviderUnavailable => policy.provider_unavailable,
        CredentialProbeOutcome::UnsupportedModel => policy.unsupported_model,
        CredentialProbeOutcome::Unknown => policy.unknown,
    };
    match action {
        ProbeResultActionKind::Noop => CredentialProbeApplyActionStatus::Noop,
        ProbeResultActionKind::Expire => CredentialProbeApplyActionStatus::Expire,
        ProbeResultActionKind::QuotaExhaust => CredentialProbeApplyActionStatus::QuotaExhaust,
        ProbeResultActionKind::Restore => CredentialProbeApplyActionStatus::Restore,
        ProbeResultActionKind::Cooldown => CredentialProbeApplyActionStatus::Cooldown,
    }
}

pub fn probe_result_is_default_key_switch_cooldown(
    probe: &CredentialProbeResultRecord,
    classifier_id: &str,
) -> bool {
    matches!(probe.outcome, CredentialProbeOutcome::Unknown)
        && probe.upstream_code.as_deref() == Some("key_switch_cooldown")
        && probe.classifier_id.as_deref() == Some(classifier_id)
        && probe.adaptation_rule_id.is_none()
}

pub fn probe_outcome_for_failure(failure: &ClassifiedFailure) -> CredentialProbeOutcome {
    match (failure.kind, failure.primary_scope) {
        (FailureKind::AuthInvalid, FailureScope::Credential) => CredentialProbeOutcome::Invalid,
        (FailureKind::QuotaExhausted, FailureScope::Credential) => {
            CredentialProbeOutcome::QuotaExhausted
        }
        (FailureKind::RateLimited, FailureScope::Credential) => CredentialProbeOutcome::RateLimited,
        (FailureKind::KeySwitchCooldown, FailureScope::Credential) => {
            CredentialProbeOutcome::Unknown
        }
        (FailureKind::ProviderUnavailable, _) => CredentialProbeOutcome::ProviderUnavailable,
        (FailureKind::ClientError, FailureScope::ModelGroup) => {
            CredentialProbeOutcome::UnsupportedModel
        }
        _ => CredentialProbeOutcome::Unknown,
    }
}

pub fn probe_result_input_from_failure(
    command: &CredentialProbeCommand,
    pool_state: &PoolState,
    channel_id: &str,
    failure: ClassifiedFailure,
    latency_ms: u64,
) -> CredentialProbeResultRecordInput {
    CredentialProbeResultRecordInput {
        credential_set_id: CredentialSetId(command.credential_set_id.clone()),
        credential_id: command.credential_id.clone(),
        channel_id: channel_id.to_string(),
        provider_id: pool_state.provider_id.clone(),
        account_id: pool_state.account_id.clone(),
        outcome: probe_outcome_for_failure(&failure),
        classifier_id: Some(failure.classifier_id),
        adaptation_rule_id: failure.adaptation_rule_id,
        upstream_status: failure.upstream_status,
        upstream_code: failure.upstream_code,
        upstream_limit_type: failure.upstream_limit_type,
        latency_ms,
    }
}

pub fn transport_failure_probe_result_input(
    command: &CredentialProbeCommand,
    pool_state: &PoolState,
    channel_id: &str,
    latency_ms: u64,
) -> CredentialProbeResultRecordInput {
    probe_result_input_from_failure(
        command,
        pool_state,
        channel_id,
        pool_state.error_classifier.classify_transport_failure(),
        latency_ms,
    )
}

pub struct HttpFailureProbeResultContext<'a> {
    pub adapter: &'a ProviderAdapter,
    pub command: &'a CredentialProbeCommand,
    pub pool_state: &'a PoolState,
    pub channel_id: &'a str,
    pub status: u16,
    pub headers: &'a reqwest::header::HeaderMap,
    pub body: &'a [u8],
    pub latency_ms: u64,
}

pub fn http_failure_probe_result_input(
    context: HttpFailureProbeResultContext<'_>,
) -> CredentialProbeResultRecordInput {
    let failure = context.adapter.classify_failure(
        &context.pool_state.error_classifier,
        context.status,
        context.headers,
        context.body,
    );
    probe_result_input_from_failure(
        context.command,
        context.pool_state,
        context.channel_id,
        failure,
        context.latency_ms,
    )
}

pub fn successful_probe_result_input(
    command: &CredentialProbeCommand,
    pool_state: &PoolState,
    channel_id: &str,
    outcome: CredentialProbeOutcome,
    upstream_status: u16,
    latency_ms: u64,
) -> CredentialProbeResultRecordInput {
    CredentialProbeResultRecordInput {
        credential_set_id: CredentialSetId(command.credential_set_id.clone()),
        credential_id: command.credential_id.clone(),
        channel_id: channel_id.to_string(),
        provider_id: pool_state.provider_id.clone(),
        account_id: pool_state.account_id.clone(),
        outcome,
        classifier_id: None,
        adaptation_rule_id: None,
        upstream_status: Some(upstream_status),
        upstream_code: None,
        upstream_limit_type: None,
        latency_ms,
    }
}

pub fn model_retrieve_success_probe_result_input(
    command: &CredentialProbeCommand,
    pool_state: &PoolState,
    channel_id: &str,
    upstream_status: u16,
    latency_ms: u64,
) -> CredentialProbeResultRecordInput {
    successful_probe_result_input(
        command,
        pool_state,
        channel_id,
        CredentialProbeOutcome::Success,
        upstream_status,
        latency_ms,
    )
}

pub struct ChatSuccessProbeResultContext<'a> {
    pub adapter: &'a ProviderAdapter,
    pub command: &'a CredentialProbeCommand,
    pub pool_state: &'a PoolState,
    pub channel_id: &'a str,
    pub upstream_status: u16,
    pub body: &'a [u8],
    pub expected_output: Option<&'a str>,
    pub latency_ms: u64,
}

pub fn chat_success_probe_result_input(
    context: ChatSuccessProbeResultContext<'_>,
) -> CredentialProbeResultRecordInput {
    let outcome = match context
        .adapter
        .credential_chat_probe_success_outcome(context.body, context.expected_output)
    {
        ChatProbeSuccessOutcome::MatchesExpectedOutput => CredentialProbeOutcome::Success,
        ChatProbeSuccessOutcome::UnexpectedOutput => CredentialProbeOutcome::Unknown,
    };
    successful_probe_result_input(
        context.command,
        context.pool_state,
        context.channel_id,
        outcome,
        context.upstream_status,
        context.latency_ms,
    )
}

pub fn unsupported_model_probe_result_input(
    command: &CredentialProbeCommand,
    pool_state: &PoolState,
    channel_id: &str,
    latency_ms: u64,
) -> CredentialProbeResultRecordInput {
    CredentialProbeResultRecordInput {
        credential_set_id: CredentialSetId(command.credential_set_id.clone()),
        credential_id: command.credential_id.clone(),
        channel_id: channel_id.to_string(),
        provider_id: pool_state.provider_id.clone(),
        account_id: pool_state.account_id.clone(),
        outcome: CredentialProbeOutcome::UnsupportedModel,
        classifier_id: None,
        adaptation_rule_id: None,
        upstream_status: None,
        upstream_code: None,
        upstream_limit_type: None,
        latency_ms,
    }
}

pub fn credential_probe_target_for_command(
    adapter: &ProviderAdapter,
    command: &CredentialProbeCommand,
) -> CredentialProbeTarget {
    let expected_output = command.expected_output.as_deref();
    match command.kind {
        CredentialProbeKind::ModelRetrieve => adapter.credential_probe_target(&command.model),
        CredentialProbeKind::ChatCompletion => {
            adapter.credential_chat_probe_target(&command.model, expected_output)
        }
    }
}

pub async fn send_model_retrieve_probe_request(
    http_client: &reqwest::Client,
    adapter: &ProviderAdapter,
    pool_state: &PoolState,
    selected: &SelectedKey,
    path: &str,
    timeout: Duration,
) -> reqwest::Result<reqwest::Response> {
    let url = adapter.upstream_url(&pool_state.api_base, path, None);
    adapter
        .apply_upstream_auth_and_headers(
            http_client.get(url).timeout(timeout),
            &axum::http::HeaderMap::new(),
            pool_state.auth_header.as_str(),
            pool_state.auth_prefix.as_str(),
            &selected.key,
        )
        .send()
        .await
}

pub async fn send_chat_completion_probe_request(
    http_client: &reqwest::Client,
    adapter: &ProviderAdapter,
    pool_state: &PoolState,
    selected: &SelectedKey,
    path: &str,
    timeout: Duration,
    body: &serde_json::Value,
) -> reqwest::Result<reqwest::Response> {
    let url = adapter.upstream_url(&pool_state.api_base, path, None);
    adapter
        .apply_upstream_auth_and_headers(
            http_client.post(url).timeout(timeout),
            &axum::http::HeaderMap::new(),
            pool_state.auth_header.as_str(),
            pool_state.auth_prefix.as_str(),
            &selected.key,
        )
        .json(body)
        .send()
        .await
}

pub async fn model_retrieve_probe_result_input_from_response(
    adapter: &ProviderAdapter,
    command: &CredentialProbeCommand,
    pool_state: &PoolState,
    channel_id: &str,
    response: reqwest::Response,
    max_error_body_bytes: usize,
    latency_ms: u64,
) -> CredentialProbeResultRecordInput {
    let status = response.status();
    if status.is_success() {
        model_retrieve_success_probe_result_input(
            command,
            pool_state,
            channel_id,
            status.as_u16(),
            latency_ms,
        )
    } else {
        let headers = response.headers().clone();
        let body = read_limited_upstream_body(response, max_error_body_bytes)
            .await
            .unwrap_or_default();
        http_failure_probe_result_input(HttpFailureProbeResultContext {
            adapter,
            command,
            pool_state,
            channel_id,
            status: status.as_u16(),
            headers: &headers,
            body: body.as_ref(),
            latency_ms,
        })
    }
}

pub struct ChatCompletionProbeResponseContext<'a> {
    pub adapter: &'a ProviderAdapter,
    pub command: &'a CredentialProbeCommand,
    pub pool_state: &'a PoolState,
    pub channel_id: &'a str,
    pub max_error_body_bytes: usize,
    pub expected_output: Option<&'a str>,
    pub latency_ms: u64,
}

pub async fn chat_completion_probe_result_input_from_response(
    context: ChatCompletionProbeResponseContext<'_>,
    response: reqwest::Response,
) -> CredentialProbeResultRecordInput {
    let status = response.status();
    let headers = response.headers().clone();
    let body = read_limited_upstream_body(response, context.max_error_body_bytes)
        .await
        .unwrap_or_default();
    if status.is_success() {
        chat_success_probe_result_input(ChatSuccessProbeResultContext {
            adapter: context.adapter,
            command: context.command,
            pool_state: context.pool_state,
            channel_id: context.channel_id,
            upstream_status: status.as_u16(),
            body: &body,
            expected_output: context.expected_output,
            latency_ms: context.latency_ms,
        })
    } else {
        http_failure_probe_result_input(HttpFailureProbeResultContext {
            adapter: context.adapter,
            command: context.command,
            pool_state: context.pool_state,
            channel_id: context.channel_id,
            status: status.as_u16(),
            headers: &headers,
            body: &body,
            latency_ms: context.latency_ms,
        })
    }
}

pub struct ExecuteCredentialProbeContext<'a> {
    pub http_client: &'a reqwest::Client,
    pub command: &'a CredentialProbeCommand,
    pub pool_state: &'a PoolState,
    pub channel_id: &'a str,
    pub selected: &'a SelectedKey,
    pub max_error_body_bytes: usize,
}

pub async fn execute_credential_probe(
    context: ExecuteCredentialProbeContext<'_>,
) -> CredentialProbeResultRecordInput {
    let started = Instant::now();
    let adapter = ProviderAdapter::new(context.pool_state.provider_kind);
    let expected_output = context.command.expected_output.as_deref();
    let target = credential_probe_target_for_command(&adapter, context.command);
    match target {
        CredentialProbeTarget::ModelRetrieve { path } => {
            let response = send_model_retrieve_probe_request(
                context.http_client,
                &adapter,
                context.pool_state,
                context.selected,
                &path,
                context.command.timeout,
            )
            .await;
            let latency_ms = probe_latency_ms(started);
            match response {
                Ok(response) => {
                    model_retrieve_probe_result_input_from_response(
                        &adapter,
                        context.command,
                        context.pool_state,
                        context.channel_id,
                        response,
                        context.max_error_body_bytes,
                        latency_ms,
                    )
                    .await
                }
                Err(_) => transport_failure_probe_result_input(
                    context.command,
                    context.pool_state,
                    context.channel_id,
                    latency_ms,
                ),
            }
        }
        CredentialProbeTarget::ChatCompletion { path, body } => {
            let response = send_chat_completion_probe_request(
                context.http_client,
                &adapter,
                context.pool_state,
                context.selected,
                &path,
                context.command.timeout,
                &body,
            )
            .await;
            let latency_ms = probe_latency_ms(started);
            match response {
                Ok(response) => {
                    chat_completion_probe_result_input_from_response(
                        ChatCompletionProbeResponseContext {
                            adapter: &adapter,
                            command: context.command,
                            pool_state: context.pool_state,
                            channel_id: context.channel_id,
                            max_error_body_bytes: context.max_error_body_bytes,
                            expected_output,
                            latency_ms,
                        },
                        response,
                    )
                    .await
                }
                Err(_) => transport_failure_probe_result_input(
                    context.command,
                    context.pool_state,
                    context.channel_id,
                    latency_ms,
                ),
            }
        }
        CredentialProbeTarget::UnsupportedModel => unsupported_model_probe_result_input(
            context.command,
            context.pool_state,
            context.channel_id,
            probe_latency_ms(started),
        ),
    }
}

pub fn probe_latency_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

pub async fn read_limited_upstream_body(
    response: reqwest::Response,
    limit: usize,
) -> anyhow::Result<bytes::Bytes> {
    let mut stream = response.bytes_stream();
    let mut body = bytes::BytesMut::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len() + chunk.len() > limit {
            anyhow::bail!("upstream probe body exceeded {limit} bytes");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

pub fn credential_probe_result_status(
    record: CredentialProbeResultRecord,
) -> CredentialProbeResultStatus {
    CredentialProbeResultStatus {
        outcome: credential_probe_outcome_status(record.outcome),
        channel_id: record.channel_id,
        provider_id: record.provider_id,
        account_id: record.account_id,
        classifier_id: record.classifier_id,
        adaptation_rule_id: record.adaptation_rule_id,
        upstream_status: record.upstream_status,
        upstream_code: record.upstream_code,
        upstream_limit_type: record.upstream_limit_type,
        latency_ms: record.latency_ms,
        created_at_unix_seconds: record.created_at_unix_seconds,
    }
}

pub fn credential_probe_summary_status(
    total_credentials: usize,
    summary: CredentialProbeSummaryRecord,
) -> CredentialProbeSummaryStatus {
    let probed_credentials = summary.success
        + summary.invalid
        + summary.quota_exhausted
        + summary.rate_limited
        + summary.provider_unavailable
        + summary.unsupported_model
        + summary.unknown;
    CredentialProbeSummaryStatus {
        total_credentials,
        probed_credentials,
        unprobed_credentials: total_credentials.saturating_sub(probed_credentials),
        success: summary.success,
        invalid: summary.invalid,
        quota_exhausted: summary.quota_exhausted,
        rate_limited: summary.rate_limited,
        provider_unavailable: summary.provider_unavailable,
        unsupported_model: summary.unsupported_model,
        unknown: summary.unknown,
    }
}

pub fn credential_probe_outcome_status(
    outcome: CredentialProbeOutcome,
) -> CredentialProbeOutcomeStatus {
    match outcome {
        CredentialProbeOutcome::Success => CredentialProbeOutcomeStatus::Success,
        CredentialProbeOutcome::Invalid => CredentialProbeOutcomeStatus::Invalid,
        CredentialProbeOutcome::QuotaExhausted => CredentialProbeOutcomeStatus::QuotaExhausted,
        CredentialProbeOutcome::RateLimited => CredentialProbeOutcomeStatus::RateLimited,
        CredentialProbeOutcome::ProviderUnavailable => {
            CredentialProbeOutcomeStatus::ProviderUnavailable
        }
        CredentialProbeOutcome::UnsupportedModel => CredentialProbeOutcomeStatus::UnsupportedModel,
        CredentialProbeOutcome::Unknown => CredentialProbeOutcomeStatus::Unknown,
    }
}

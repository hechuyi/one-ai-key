use serde_json::Value;

const RELOAD_DIFF_STATUS: &str = "unavailable";
const RELOAD_DIFF_REASON_CODE: &str = "unavailable_without_staged_projection";
const RELOAD_DIFF_APPLY_STATUS: &str = "dry_run_available";
const RELOAD_DIFF_UNAVAILABLE_APPLY_STATUS: &str = "unavailable_without_staged_projection";
const RELOAD_DIFF_TEMPLATE_ID: &str = "reload_diff_unavailable";
const RELOAD_DIFF_MAX_RESOURCE_CHANGES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadStatusOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadDiffOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadApplyMode {
    DryRun,
    NeedsConfirmation,
    Apply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReloadApplyOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub mode: ReloadApplyMode,
    pub expected_staged_registry_version: Option<u64>,
    pub output: crate::cli_report::OutputFormat,
}

pub async fn run_status(
    options: ReloadStatusOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let runtime = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::Runtime)
        .await?;
    let explain_runtime = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::ExplainRuntime)
        .await?;
    Ok(render_reload_status_report(
        Some(&runtime),
        Some(&explain_runtime),
        options.output,
    ))
}

pub async fn run_diff(
    options: ReloadDiffOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let diff = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::RuntimeReloadDiff)
        .await?;
    Ok(render_reload_diff_report(&diff, options.output))
}

pub async fn run_apply(
    options: ReloadApplyOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    match options.mode {
        ReloadApplyMode::DryRun => run_apply_dry_run(options).await,
        ReloadApplyMode::NeedsConfirmation => {
            Err(crate::operator_client::OperatorClientError::new(
                "confirmation_required",
                "reload apply requires --yes or interactive confirmation",
            ))
        }
        ReloadApplyMode::Apply => run_apply_confirmed(options).await,
    }
}

async fn run_apply_dry_run(
    options: ReloadApplyOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let runtime = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::Runtime)
        .await?;
    let diff = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::RuntimeReloadDiff)
        .await?;
    Ok(render_reload_apply_dry_run_report(
        &options,
        &runtime,
        Some(&diff),
    ))
}

async fn run_apply_confirmed(
    options: ReloadApplyOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let Some(expected_staged_registry_version) = options.expected_staged_registry_version else {
        return Ok(render_reload_apply_blocked_report(
            &options,
            "reload_apply_precondition_unavailable",
            "Confirmed runtime reload requires an expected staged registry version from reload apply --dry-run.",
            Value::Null,
        ));
    };
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let runtime = client
        .get_json(crate::operator_client::ReadOnlyEndpoint::Runtime)
        .await?;
    let staged_registry_version = runtime
        .get("staged_registry_version")
        .and_then(Value::as_u64);
    if staged_registry_version != Some(expected_staged_registry_version) {
        return Ok(render_reload_apply_blocked_report(
            &options,
            "reload_apply_generation_changed_after_plan",
            "Staged registry version changed after the apply plan; rerun reload status or reload apply --dry-run.",
            serde_json::json!({
                "current_staged_registry_version": staged_registry_version,
            }),
        ));
    }

    let response = client
        .post_json(
            crate::operator_client::ManagementMutationEndpoint::RuntimeReload {
                expected_staged_registry_version,
            },
            &serde_json::json!({}),
        )
        .await;
    match response {
        Ok(response) => Ok(render_reload_apply_success_report(
            &options, &runtime, &response,
        )),
        Err(error) => Ok(render_reload_apply_failure_report(
            &options,
            &runtime,
            error.reason_code(),
        )),
    }
}

pub fn render_reload_diff_report(diff: &Value, output: crate::cli_report::OutputFormat) -> String {
    let report = sanitized_reload_diff_report(diff);
    match output {
        crate::cli_report::OutputFormat::Json => {
            serde_json::to_string_pretty(&report).expect("reload diff should serialize")
        }
        crate::cli_report::OutputFormat::Table => render_reload_diff_table(&report),
    }
}

fn render_reload_apply_dry_run_report(
    options: &ReloadApplyOptions,
    runtime: &Value,
    diff: Option<&Value>,
) -> String {
    let expected_staged_registry_version = runtime
        .get("staged_registry_version")
        .and_then(Value::as_u64);
    let report = reload_apply_report_envelope(
        "planned",
        "reload_apply_dry_run",
        "Runtime reload apply plan was produced without mutating active runtime.",
        crate::cli_effects::runtime_readonly_effect(),
        serde_json::json!({
            "command": "reload apply",
            "mode": "dry_run",
            "active_registry_generation": runtime.get("active_registry_generation").and_then(Value::as_u64),
            "active_registry_version": runtime.get("active_registry_version").and_then(Value::as_u64),
            "staged_registry_version": expected_staged_registry_version,
            "runtime_reload_required": runtime.get("runtime_reload_required").and_then(Value::as_bool),
            "expected_staged_registry_version": expected_staged_registry_version,
            "reload_diff_status": diff.and_then(|diff| diff.get("status")).and_then(Value::as_str).and_then(safe_reload_diff_status_without_reason).unwrap_or("unknown"),
            "reload_diff_reason_code": diff.and_then(|diff| diff.get("reason_code")).and_then(Value::as_str).and_then(safe_reload_diff_reason_code).unwrap_or("unknown"),
            "mutating_reload_sent": false,
            "precondition_supported": true,
            "last_reload": Value::Null,
        }),
        reload_apply_next_action(expected_staged_registry_version),
    );
    render_reload_apply_report(&report, options.output)
}

fn render_reload_apply_blocked_report(
    options: &ReloadApplyOptions,
    reason_code: &'static str,
    reason: &'static str,
    extra: Value,
) -> String {
    let data = serde_json::json!({
        "command": "reload apply",
        "mode": "apply",
        "expected_staged_registry_version": options.expected_staged_registry_version,
        "mutating_reload_sent": false,
        "precondition_supported": true,
        "block": extra,
        "last_reload": Value::Null,
    });
    let report = reload_apply_report_envelope(
        "blocked",
        reason_code,
        reason,
        reload_apply_management_effect(),
        data,
        serde_json::json!({
            "summary": "Rerun reload status and reload apply --dry-run, then retry with the reported expected staged registry version.",
            "template_id": "reload_apply_rerun_plan",
            "safe_argv": ["one-ai-key", "reload", "apply", "--dry-run"],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
    );
    render_reload_apply_report(&report, options.output)
}

fn render_reload_apply_success_report(
    options: &ReloadApplyOptions,
    before_runtime: &Value,
    response: &Value,
) -> String {
    let data = serde_json::json!({
        "command": "reload apply",
        "mode": "apply",
        "expected_staged_registry_version": options.expected_staged_registry_version,
        "before": sanitize_reload_runtime_projection(before_runtime),
        "management_response": sanitize_reload_runtime_projection(response),
        "mutating_reload_sent": true,
        "precondition_supported": true,
        "last_reload": {
            "last_reload_error_reason_code": Value::Null,
        },
    });
    let report = reload_apply_report_envelope(
        "ok",
        "runtime_reload_applied",
        "Runtime reload was applied through management with an expected staged registry version precondition.",
        reload_apply_management_effect(),
        data,
        serde_json::json!({
            "summary": "Inspect reload status after apply.",
            "template_id": "reload_status",
            "safe_argv": ["one-ai-key", "reload", "status"],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
    );
    render_reload_apply_report(&report, options.output)
}

fn render_reload_apply_failure_report(
    options: &ReloadApplyOptions,
    before_runtime: &Value,
    last_reload_error_reason_code: &'static str,
) -> String {
    let data = serde_json::json!({
        "command": "reload apply",
        "mode": "apply",
        "expected_staged_registry_version": options.expected_staged_registry_version,
        "before": sanitize_reload_runtime_projection(before_runtime),
        "mutating_reload_sent": true,
        "precondition_supported": true,
        "last_reload": {
            "last_reload_error_reason_code": last_reload_error_reason_code,
        },
    });
    let report = reload_apply_report_envelope(
        "failed",
        "runtime_reload_failed",
        "Runtime reload request failed; raw management error output was not rendered.",
        reload_apply_management_effect(),
        data,
        serde_json::json!({
            "summary": "Inspect reload status and management events before retrying.",
            "template_id": "reload_status",
            "safe_argv": ["one-ai-key", "reload", "status"],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
    );
    render_reload_apply_report(&report, options.output)
}

fn reload_apply_report_envelope(
    status: &'static str,
    reason_code: &'static str,
    reason: &'static str,
    effect: crate::cli_effects::CommandEffect,
    data: Value,
    next_action: Value,
) -> Value {
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason,
        reason_code,
        effect,
        scope: serde_json::json!({
            "projection": "runtime_reload_apply",
        }),
        window: Value::Null,
        next_action,
        data,
    })
}

fn render_reload_apply_report(report: &Value, output: crate::cli_report::OutputFormat) -> String {
    match output {
        crate::cli_report::OutputFormat::Json => {
            serde_json::to_string_pretty(report).expect("reload apply should serialize")
        }
        crate::cli_report::OutputFormat::Table => render_reload_apply_table(report),
    }
}

fn reload_apply_next_action(expected_staged_registry_version: Option<u64>) -> Value {
    let Some(expected_staged_registry_version) = expected_staged_registry_version else {
        return serde_json::json!({
            "summary": "No staged registry version is available to use as an apply precondition.",
            "template_id": "reload_apply_unavailable",
            "safe_argv": [],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        });
    };
    serde_json::json!({
        "summary": "Apply the planned runtime reload only if the staged registry version still matches.",
        "template_id": "reload_apply",
        "safe_argv": [
            "one-ai-key",
            "reload",
            "apply",
            "--expected-staged-registry-version",
            expected_staged_registry_version.to_string(),
            "--yes"
        ],
        "side_effect_class": "management_write",
        "requires_confirmation": true,
    })
}

fn sanitize_reload_runtime_projection(value: &Value) -> Value {
    serde_json::json!({
        "active_registry_generation": value.get("active_registry_generation").and_then(Value::as_u64),
        "active_registry_version": value.get("active_registry_version").and_then(Value::as_u64),
        "staged_registry_version": value.get("staged_registry_version").and_then(Value::as_u64),
        "runtime_reload_required": value.get("runtime_reload_required").and_then(Value::as_bool),
        "channels": value.get("channels").and_then(Value::as_u64),
    })
}

pub fn reload_apply_management_effect() -> crate::cli_effects::CommandEffect {
    crate::cli_effects::CommandEffect {
        side_effect_class: crate::cli_effects::SideEffectClass::ManagementWrite,
        effect_vector: crate::cli_effects::EffectVector {
            reads_management_runtime: true,
            writes_management_store: true,
            mutates_runtime: true,
            ..crate::cli_effects::EffectVector::default()
        },
    }
}

pub fn render_reload_status_report(
    runtime: Option<&Value>,
    explain_runtime: Option<&Value>,
    output: crate::cli_report::OutputFormat,
) -> String {
    let report = sanitized_reload_status_report(runtime, explain_runtime);
    match output {
        crate::cli_report::OutputFormat::Json => {
            serde_json::to_string_pretty(&report).expect("reload status should serialize")
        }
        crate::cli_report::OutputFormat::Table => render_reload_status_table(&report),
    }
}

fn sanitized_reload_diff_report(diff: &Value) -> Value {
    let reason_code = diff
        .get("reason_code")
        .and_then(Value::as_str)
        .and_then(safe_reload_diff_reason_code)
        .unwrap_or(RELOAD_DIFF_REASON_CODE);
    let status = diff
        .get("status")
        .and_then(Value::as_str)
        .and_then(|status| safe_reload_diff_status(status, reason_code))
        .unwrap_or(RELOAD_DIFF_STATUS);
    let data = serde_json::json!({
        "command": "reload diff",
        "active_registry_generation": diff.get("active_registry_generation").and_then(Value::as_u64),
        "active_registry_version": diff.get("active_registry_version").and_then(Value::as_u64),
        "staged_registry_version": diff.get("staged_registry_version").and_then(Value::as_u64),
        "runtime_reload_required": diff.get("runtime_reload_required").and_then(Value::as_bool),
        "mutating_reload_sent": false,
        "reload_apply_status": reload_diff_apply_status(reason_code),
        "budget": sanitize_reload_diff_budget(diff.get("budget")),
        "resource_changes": sanitize_reload_diff_resource_changes(diff.get("resource_changes")),
    });
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason: reload_diff_reason(reason_code),
        reason_code,
        effect: crate::cli_effects::runtime_readonly_effect(),
        scope: serde_json::json!({
            "projection": "runtime_reload_diff",
        }),
        window: Value::Null,
        next_action: reload_diff_next_action(reason_code),
        data,
    })
}

fn sanitized_reload_status_report(
    runtime: Option<&Value>,
    explain_runtime: Option<&Value>,
) -> Value {
    let reload_required = first_bool(&[
        runtime.and_then(|value| value.get("runtime_reload_required")),
        runtime.and_then(|value| value.get("reload_required")),
        explain_runtime.and_then(|value| value.get("runtime_reload_required")),
        explain_runtime.and_then(|value| value.get("reload_required")),
    ]);
    let status = if runtime.is_none() && explain_runtime.is_none() {
        "partial"
    } else if reload_required == Some(true) {
        "pending_reload"
    } else {
        "ok"
    };
    let reason_code = match status {
        "pending_reload" => "runtime_reload_required",
        "partial" => "reload_status_projection_unavailable",
        _ => "runtime_reload_not_required",
    };
    let active_registry_generation = first_u64(&[
        runtime.and_then(|value| value.get("active_registry_generation")),
        explain_runtime.and_then(|value| value.get("active_registry_generation")),
    ]);
    let active_registry_version = first_u64(&[
        runtime.and_then(|value| value.get("active_registry_version")),
        explain_runtime.and_then(|value| value.get("active_registry_version")),
    ]);
    let staged_registry_version = first_u64(&[
        runtime.and_then(|value| value.get("staged_registry_version")),
        explain_runtime.and_then(|value| value.get("staged_registry_version")),
    ]);
    let staged_vs_runtime = summarize_staged_vs_runtime(explain_runtime, reload_required);
    let last_reload = summarize_last_reload(explain_runtime);
    let (reload_diff_status, reload_apply_status) = match reload_required {
        Some(true) => ("available", "dry_run_available"),
        Some(false) => ("not_required", "not_required"),
        None => ("unknown", "unknown"),
    };
    let data = serde_json::json!({
        "command": "reload status",
        "active_registry_generation": active_registry_generation,
        "active_registry_version": active_registry_version,
        "staged_registry_version": staged_registry_version,
        "runtime_reload_required": reload_required,
        "staged_vs_runtime": staged_vs_runtime,
        "last_reload": last_reload,
        "reload_diff_status": reload_diff_status,
        "reload_apply_status": reload_apply_status,
        "mutating_reload_sent": false,
    });
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason: reload_status_reason(reason_code),
        reason_code,
        effect: crate::cli_effects::runtime_readonly_effect(),
        scope: serde_json::json!({
            "projection": "runtime_and_explain_runtime",
        }),
        window: Value::Null,
        next_action: reload_status_next_action(status),
        data,
    })
}

fn summarize_staged_vs_runtime(
    explain_runtime: Option<&Value>,
    reload_required: Option<bool>,
) -> Value {
    let staged = explain_runtime.and_then(|value| value.get("staged_vs_runtime"));
    serde_json::json!({
        "active_matches_staged": first_bool(&[
            staged.and_then(|value| value.get("active_matches_staged")),
            reload_required.map(|required| Value::Bool(!required)).as_ref(),
        ]),
        "reload_required_reason": staged
            .and_then(|value| value.get("reload_required_reason"))
            .and_then(Value::as_str)
            .and_then(safe_reason_code),
    })
}

fn summarize_last_reload(explain_runtime: Option<&Value>) -> Value {
    let last_reload = explain_runtime.and_then(|value| value.get("last_reload"));
    serde_json::json!({
        "last_reload_at_unix_seconds": last_reload
            .and_then(|value| value.get("last_reload_at_unix_seconds"))
            .and_then(Value::as_u64),
        "last_reload_error_reason_code": last_reload
            .and_then(|value| value.get("last_reload_error_reason_code"))
            .and_then(Value::as_str)
            .and_then(safe_reason_code),
    })
}

fn reload_status_reason(reason_code: &str) -> &'static str {
    match reason_code {
        "runtime_reload_required" => "Runtime has staged registry changes that are not active yet.",
        "reload_status_projection_unavailable" => {
            "Reload status could not read the runtime projections."
        }
        _ => {
            "Runtime active and staged registry versions match, or no staged registry is available."
        }
    }
}

fn reload_status_next_action(_status: &str) -> Value {
    if _status == "pending_reload" {
        serde_json::json!({
            "summary": "Reload status is read-only. Inspect the typed reload diff before any explicit reload planning.",
            "template_id": "reload_diff",
            "safe_argv": ["one-ai-key", "reload", "diff"],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
            "reload_diff_status": "available",
            "reload_apply_status": "dry_run_available",
        })
    } else {
        serde_json::json!({
            "summary": "Reload status is read-only. No runtime reload is currently required by the active projections.",
            "template_id": "reload_status_only",
            "safe_argv": ["one-ai-key", "reload", "status"],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
            "reload_diff_status": "not_required",
            "reload_apply_status": "not_required",
        })
    }
}

fn reload_diff_reason(reason_code: &str) -> &'static str {
    match reason_code {
        "reload_diff_available" => "A typed reload diff is available.",
        "reload_diff_truncated" => "A typed reload diff is available but truncated.",
        "reload_diff_empty" => "A typed reload diff is available and empty.",
        _ => "A typed reload diff projection is unavailable for the current runtime.",
    }
}

fn reload_diff_next_action(reason_code: &str) -> Value {
    match reason_code {
        "reload_diff_available" | "reload_diff_truncated" | "reload_diff_empty" => {
            serde_json::json!({
                "summary": "Reload diff is read-only and complete for this request. No further diagnostic action is required.",
                "template_id": "no_action_required",
                "safe_argv": [],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false,
            })
        }
        _ => serde_json::json!({
            "summary": "Reload diff is read-only and currently unavailable; inspect reload status or prepare explicit staged registry changes.",
            "template_id": RELOAD_DIFF_TEMPLATE_ID,
            "safe_argv": [],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
    }
}

fn reload_diff_apply_status(reason_code: &str) -> &'static str {
    match reason_code {
        "reload_diff_available" | "reload_diff_truncated" | "reload_diff_empty" => {
            RELOAD_DIFF_APPLY_STATUS
        }
        _ => RELOAD_DIFF_UNAVAILABLE_APPLY_STATUS,
    }
}

fn render_reload_status_table(report: &Value) -> String {
    let mut output = String::new();
    output.push_str("Reload status\n");
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    for field in [
        "active_registry_generation",
        "active_registry_version",
        "staged_registry_version",
        "runtime_reload_required",
        "reload_diff_status",
        "reload_apply_status",
    ] {
        crate::cli_report::push_table_field(&mut output, field, report.get(field));
    }
    if let Some(staged) = report.get("staged_vs_runtime") {
        crate::cli_report::push_table_field(
            &mut output,
            "staged_vs_runtime.active_matches_staged",
            staged.get("active_matches_staged"),
        );
        crate::cli_report::push_table_field(
            &mut output,
            "staged_vs_runtime.reload_required_reason",
            staged.get("reload_required_reason"),
        );
    }
    if let Some(last_reload) = report.get("last_reload") {
        crate::cli_report::push_table_field(
            &mut output,
            "last_reload.last_reload_at_unix_seconds",
            last_reload.get("last_reload_at_unix_seconds"),
        );
        crate::cli_report::push_table_field(
            &mut output,
            "last_reload.last_reload_error_reason_code",
            last_reload.get("last_reload_error_reason_code"),
        );
    }
    output
}

fn render_reload_diff_table(report: &Value) -> String {
    let mut output = String::new();
    output.push_str("Reload diff\n");
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    for field in [
        "active_registry_generation",
        "active_registry_version",
        "staged_registry_version",
        "runtime_reload_required",
        "reload_apply_status",
        "mutating_reload_sent",
    ] {
        crate::cli_report::push_table_field(&mut output, field, report.get(field));
    }
    if let Some(budget) = report.get("budget") {
        crate::cli_report::push_table_field(
            &mut output,
            "budget.total_resource_changes",
            budget.get("total_resource_changes"),
        );
        crate::cli_report::push_table_field(
            &mut output,
            "budget.omitted_resource_changes",
            budget.get("omitted_resource_changes"),
        );
        crate::cli_report::push_table_field(
            &mut output,
            "budget.truncated",
            budget.get("truncated"),
        );
    }
    if let Some(resource_changes) = report.get("resource_changes").and_then(Value::as_array) {
        for section in resource_changes {
            if let Some(resource_type) = section.get("resource_type").and_then(Value::as_str) {
                crate::cli_report::push_table_field(
                    &mut output,
                    &format!("resource_changes.{resource_type}.added"),
                    section.get("added"),
                );
                crate::cli_report::push_table_field(
                    &mut output,
                    &format!("resource_changes.{resource_type}.removed"),
                    section.get("removed"),
                );
                crate::cli_report::push_table_field(
                    &mut output,
                    &format!("resource_changes.{resource_type}.changed"),
                    section.get("changed"),
                );
            }
        }
    }
    output
}

fn render_reload_apply_table(report: &Value) -> String {
    let mut output = String::new();
    output.push_str("Reload apply\n");
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    for field in [
        "mode",
        "active_registry_generation",
        "active_registry_version",
        "staged_registry_version",
        "runtime_reload_required",
        "expected_staged_registry_version",
        "reload_diff_status",
        "reload_diff_reason_code",
        "mutating_reload_sent",
        "precondition_supported",
    ] {
        crate::cli_report::push_table_field(&mut output, field, report.get(field));
    }
    if let Some(last_reload) = report.get("last_reload") {
        crate::cli_report::push_table_field(
            &mut output,
            "last_reload.last_reload_error_reason_code",
            last_reload.get("last_reload_error_reason_code"),
        );
    }
    output
}

fn first_bool(values: &[Option<&Value>]) -> Option<bool> {
    values
        .iter()
        .find_map(|value| value.and_then(Value::as_bool))
}

fn first_u64(values: &[Option<&Value>]) -> Option<u64> {
    values
        .iter()
        .find_map(|value| value.and_then(Value::as_u64))
}

fn safe_reason_code(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return None;
    }
    Some(value.to_string())
}

fn safe_reload_diff_reason_code(value: &str) -> Option<&'static str> {
    match value {
        "reload_diff_available" => Some("reload_diff_available"),
        "reload_diff_truncated" => Some("reload_diff_truncated"),
        "reload_diff_empty" => Some("reload_diff_empty"),
        "unavailable_without_staged_projection" => Some("unavailable_without_staged_projection"),
        _ => None,
    }
}

fn safe_reload_diff_status(value: &str, reason_code: &str) -> Option<&'static str> {
    match (value, reason_code) {
        ("ok", "reload_diff_available" | "reload_diff_truncated" | "reload_diff_empty") => {
            Some("ok")
        }
        ("unavailable", "unavailable_without_staged_projection") => Some("unavailable"),
        _ => None,
    }
}

fn safe_reload_diff_status_without_reason(value: &str) -> Option<&'static str> {
    match value {
        "ok" => Some("ok"),
        "unavailable" => Some("unavailable"),
        _ => None,
    }
}

fn sanitize_reload_diff_budget(value: Option<&Value>) -> Value {
    let budget = value.unwrap_or(&Value::Null);
    serde_json::json!({
        "max_resource_changes": budget
            .get("max_resource_changes")
            .and_then(Value::as_u64)
            .filter(|value| *value <= RELOAD_DIFF_MAX_RESOURCE_CHANGES as u64),
        "total_resource_changes": budget
            .get("total_resource_changes")
            .and_then(Value::as_u64),
        "omitted_resource_changes": budget
            .get("omitted_resource_changes")
            .and_then(Value::as_u64),
        "truncated": budget
            .get("truncated")
            .and_then(Value::as_bool),
    })
}

fn sanitize_reload_diff_resource_changes(value: Option<&Value>) -> Value {
    let sections = value
        .and_then(Value::as_array)
        .map(|sections| {
            sections
                .iter()
                .filter_map(sanitize_reload_diff_resource_section)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Value::Array(sections)
}

fn sanitize_reload_diff_resource_section(section: &Value) -> Option<Value> {
    let resource_type = section
        .get("resource_type")
        .and_then(Value::as_str)
        .and_then(safe_reload_diff_resource_type)?;
    Some(serde_json::json!({
        "resource_type": resource_type,
        "added": sanitize_string_array(section.get("added"), RELOAD_DIFF_MAX_RESOURCE_CHANGES),
        "removed": sanitize_string_array(section.get("removed"), RELOAD_DIFF_MAX_RESOURCE_CHANGES),
        "changed": sanitize_reload_diff_changed(section.get("changed")),
    }))
}

fn safe_reload_diff_resource_type(value: &str) -> Option<&'static str> {
    match value {
        "providers" => Some("providers"),
        "accounts" => Some("accounts"),
        "credential_sets" => Some("credential_sets"),
        "channels" => Some("channels"),
        "model_routes" => Some("model_routes"),
        "policy_profiles" => Some("policy_profiles"),
        "routing_profiles" => Some("routing_profiles"),
        _ => None,
    }
}

fn sanitize_string_array(value: Option<&Value>, limit: usize) -> Value {
    let values = value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(limit)
                .filter_map(Value::as_str)
                .filter_map(safe_reload_diff_identifier)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Value::Array(values.into_iter().map(Value::String).collect())
}

fn sanitize_reload_diff_changed(value: Option<&Value>) -> Value {
    let changes = value
        .and_then(Value::as_array)
        .map(|changes| {
            changes
                .iter()
                .take(RELOAD_DIFF_MAX_RESOURCE_CHANGES)
                .filter_map(|change| {
                    let id = change
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(safe_reload_diff_identifier)?;
                    let changed_fields = sanitize_string_array(
                        change.get("changed_fields"),
                        RELOAD_DIFF_MAX_RESOURCE_CHANGES,
                    );
                    Some(serde_json::json!({
                        "id": id,
                        "changed_fields": changed_fields,
                    }))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Value::Array(changes)
}

fn safe_reload_diff_identifier(value: &str) -> Option<String> {
    if value.starts_with("sk-")
        || value.is_empty()
        || value.len() > 120
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        return None;
    }
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use axum::{
        extract::Query,
        http::StatusCode,
        routing::{get, post},
        Json, Router,
    };
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    async fn spawn_management_fixture(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{addr}")
    }

    fn reload_apply_connection(
        management_url: String,
        env_name: &str,
    ) -> crate::cli::OperatorConnectionOptions {
        std::env::set_var(env_name, "opaque-management-fixture");
        crate::cli::OperatorConnectionOptions {
            management_url: Some(management_url),
            deprecated_base_url: None,
            management_token_env: Some(env_name.to_string()),
            management_token_stdin: false,
            timeout_seconds: 10,
        }
    }

    fn runtime_projection(staged_registry_version: u64) -> Value {
        json!({
            "active_registry_generation": 11,
            "active_registry_version": staged_registry_version - 1,
            "staged_registry_version": staged_registry_version,
            "runtime_reload_required": true,
            "channels": 2
        })
    }

    #[tokio::test]
    async fn reload_status_cli_calls_runtime_projection_only() {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let runtime_seen = Arc::clone(&seen_paths);
        let explain_seen = Arc::clone(&seen_paths);
        let router = Router::new()
            .route(
                "/management/runtime",
                get(move || {
                    let runtime_seen = Arc::clone(&runtime_seen);
                    async move {
                        runtime_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime".to_string());
                        Json(json!({
                            "active_registry_generation": 11,
                            "active_registry_version": 3,
                            "staged_registry_version": 4,
                            "runtime_reload_required": true
                        }))
                    }
                }),
            )
            .route(
                "/management/explain/runtime",
                get(move || {
                    let explain_seen = Arc::clone(&explain_seen);
                    async move {
                        explain_seen
                            .lock()
                            .unwrap()
                            .push("/management/explain/runtime".to_string());
                        Json(json!({
                            "active_registry_generation": 11,
                            "active_registry_version": 3,
                            "staged_registry_version": 4,
                            "runtime_reload_required": true,
                            "staged_vs_runtime": {
                                "active_matches_staged": false,
                                "reload_required_reason": "staged_registry_differs"
                            },
                            "last_reload": {
                                "last_reload_at_unix_seconds": null,
                                "last_reload_error_reason_code": null,
                                "raw_error": "SHOULD_NOT_RENDER_RAW_ERROR"
                            }
                        }))
                    }
                }),
            );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!("ONE_AI_KEY_TEST_RELOAD_STATUS_TOKEN_{}", std::process::id());
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run_status(super::ReloadStatusOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec![
                "/management/runtime".to_string(),
                "/management/explain/runtime".to_string(),
            ]
        );
        assert_eq!(report["effect_vector"]["calls_upstream"], false);
        assert_eq!(report["effect_vector"]["writes_management_store"], false);
        assert_eq!(report["effect_vector"]["mutates_runtime"], false);
        assert!(!rendered.contains("SHOULD_NOT_RENDER_RAW_ERROR"));
    }

    #[test]
    fn reload_status_cli_reports_active_and_staged_generation() {
        let rendered = super::render_reload_status_report(
            Some(&json!({
                "active_registry_generation": 11,
                "active_registry_version": 3,
                "staged_registry_version": 4,
                "runtime_reload_required": true,
                "raw_config_path": "SHOULD_NOT_RENDER_PATH"
            })),
            Some(&json!({
                "active_registry_generation": 11,
                "active_registry_version": 3,
                "staged_registry_version": 4,
                "runtime_reload_required": true,
                "staged_vs_runtime": {
                    "active_matches_staged": false,
                    "reload_required_reason": "staged_registry_differs"
                },
                "last_reload": {
                    "last_reload_at_unix_seconds": 123,
                    "last_reload_error_reason_code": null
                }
            })),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "pending_reload");
        assert_eq!(report["reason_code"], "runtime_reload_required");
        assert_eq!(report["active_registry_generation"], 11);
        assert_eq!(report["active_registry_version"], 3);
        assert_eq!(report["staged_registry_version"], 4);
        assert_eq!(report["runtime_reload_required"], true);
        assert_eq!(
            report["staged_vs_runtime"]["reload_required_reason"],
            "staged_registry_differs"
        );
        assert!(!rendered.contains("SHOULD_NOT_RENDER_PATH"));
    }

    #[test]
    fn reload_status_cli_reports_last_reload_result_without_raw_errors() {
        let rendered = super::render_reload_status_report(
            Some(&json!({
                "active_registry_generation": 11,
                "active_registry_version": 3,
                "staged_registry_version": 3,
                "runtime_reload_required": false
            })),
            Some(&json!({
                "last_reload": {
                    "last_reload_at_unix_seconds": 456,
                    "last_reload_error_reason_code": "runtime_reload_failed",
                    "error": "SHOULD_NOT_RENDER_RAW_ERROR"
                }
            })),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["last_reload"]["last_reload_at_unix_seconds"], 456);
        assert_eq!(
            report["last_reload"]["last_reload_error_reason_code"],
            "runtime_reload_failed"
        );
        assert!(!rendered.contains("SHOULD_NOT_RENDER_RAW_ERROR"));
    }

    #[test]
    fn reload_status_cli_suggests_only_readonly_reload_investigation_without_mutating() {
        let rendered = super::render_reload_status_report(
            Some(&json!({
                "active_registry_generation": 11,
                "active_registry_version": 3,
                "staged_registry_version": 4,
                "runtime_reload_required": true
            })),
            Some(&json!({})),
            crate::cli_report::OutputFormat::Json,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["next_action"]["template_id"], "reload_diff");
        assert_eq!(
            report["next_action"]["safe_argv"],
            json!(["one-ai-key", "reload", "diff"])
        );
        assert_eq!(
            report["next_action"]["side_effect_class"],
            "runtime_readonly"
        );
        assert_eq!(report["next_action"]["requires_confirmation"], false);
        assert_eq!(report["reload_diff_status"], "available");
        assert_eq!(report["reload_apply_status"], "dry_run_available");
        assert_eq!(report["data"]["mutating_reload_sent"], false);
        let argv = report["next_action"]["safe_argv"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(!argv.contains(&"apply"));
        assert!(!argv.contains(&"--yes"));
        assert_ne!(argv, ["one-ai-key", "reload", "apply", "--dry-run"]);
    }

    #[tokio::test]
    async fn reload_diff_cli_reports_unavailable_without_suggesting_apply() {
        let router = Router::new().route(
            "/management/runtime/reload-diff",
            get(|| async {
                Json(json!({
                    "status": "unavailable",
                    "reason_code": "unavailable_without_staged_projection",
                    "active_registry_generation": 11,
                    "active_registry_version": 3,
                    "staged_registry_version": null,
                    "runtime_reload_required": false,
                    "mutating_reload_sent": false,
                    "reload_apply_status": "unavailable_without_staged_projection",
                    "next_action": {
                        "template_id": "reload_diff_unavailable",
                        "safe_argv": [],
                        "side_effect_class": "runtime_readonly",
                        "requires_confirmation": false
                    },
                    "raw_yaml": "SHOULD_NOT_RENDER_RAW_YAML"
                }))
            }),
        );
        let management_url = spawn_management_fixture(router).await;
        let env_name = format!("ONE_AI_KEY_TEST_RELOAD_DIFF_TOKEN_{}", std::process::id());
        std::env::set_var(&env_name, "opaque-management-fixture");

        let rendered = super::run_diff(super::ReloadDiffOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some(management_url),
                deprecated_base_url: None,
                management_token_env: Some(env_name.clone()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "unavailable");
        assert_eq!(
            report["reason_code"],
            "unavailable_without_staged_projection"
        );
        assert_eq!(
            report["next_action"]["template_id"],
            "reload_diff_unavailable"
        );
        assert_eq!(report["next_action"]["safe_argv"], json!([]));
        assert_eq!(
            report["reload_apply_status"],
            "unavailable_without_staged_projection"
        );
        assert_eq!(report["mutating_reload_sent"], false);
        assert!(!rendered.contains("SHOULD_NOT_RENDER_RAW_YAML"));
    }

    #[test]
    fn reload_diff_cli_clamps_contract_fields_and_drops_backend_actions() {
        let backend = json!({
            "status": "ok",
            "reason_code": "runtime_reload_applied",
            "active_registry_generation": 11,
            "active_registry_version": 3,
            "staged_registry_version": 4,
            "runtime_reload_required": true,
            "mutating_reload_sent": true,
            "reload_apply_status": "available_now",
            "next_action": {
                "template_id": "reload_apply_now",
                "safe_argv": ["one-ai-key", "reload", "apply"],
                "side_effect_class": "management_write",
                "requires_confirmation": true
            },
            "raw_yaml": "SHOULD_NOT_RENDER_RAW_YAML",
            "raw_path": "SHOULD_NOT_RENDER_PATH"
        });

        let rendered_json =
            super::render_reload_diff_report(&backend, crate::cli_report::OutputFormat::Json);
        let report: Value = serde_json::from_str(&rendered_json).unwrap();

        assert_eq!(report["status"], "unavailable");
        assert_eq!(
            report["reason_code"],
            "unavailable_without_staged_projection"
        );
        assert_eq!(
            report["next_action"]["template_id"],
            "reload_diff_unavailable"
        );
        assert_eq!(report["next_action"]["safe_argv"], json!([]));
        assert_eq!(
            report["next_action"]["side_effect_class"],
            "runtime_readonly"
        );
        assert_eq!(report["next_action"]["requires_confirmation"], false);
        assert_eq!(
            report["reload_apply_status"],
            "unavailable_without_staged_projection"
        );
        assert_eq!(report["mutating_reload_sent"], false);
        for forbidden in [
            "runtime_reload_applied",
            "available_now",
            "reload_apply_now",
            "management_write",
            "SHOULD_NOT_RENDER_RAW_YAML",
            "SHOULD_NOT_RENDER_PATH",
        ] {
            assert!(!rendered_json.contains(forbidden));
        }

        let rendered_table =
            super::render_reload_diff_report(&backend, crate::cli_report::OutputFormat::Table);
        for forbidden in [
            "runtime_reload_applied",
            "available_now",
            "reload_apply_now",
            "management_write",
            "SHOULD_NOT_RENDER_RAW_YAML",
            "SHOULD_NOT_RENDER_PATH",
        ] {
            assert!(!rendered_table.contains(forbidden));
        }
    }

    #[test]
    fn reload_diff_cli_wraps_backend_projection_only() {
        let token_like_value = format!("sk-{}", "SHOULD_NOT_RENDER_TOKEN");
        let backend = json!({
            "status": "ok",
            "reason_code": "reload_diff_available",
            "active_registry_generation": 11,
            "active_registry_version": 3,
            "staged_registry_version": 4,
            "runtime_reload_required": true,
            "mutating_reload_sent": true,
            "reload_apply_status": "available_now",
            "next_action": {
                "template_id": "reload_apply_now",
                "safe_argv": ["one-ai-key", "reload", "apply"],
                "side_effect_class": "management_write",
                "requires_confirmation": true
            },
            "budget": {
                "max_resource_changes": 64,
                "total_resource_changes": 3,
                "omitted_resource_changes": 0,
                "truncated": false
            },
            "resource_changes": [
                {
                    "resource_type": "providers",
                    "added": ["provider-a", token_like_value],
                    "removed": ["provider-b"],
                    "changed": [
                        {
                            "id": "provider-c",
                            "changed_fields": ["api_base", "raw/path"]
                        }
                    ],
                    "raw_yaml": "SHOULD_NOT_RENDER_RESOURCE_RAW_YAML"
                },
                {
                    "resource_type": "model_routes",
                    "added": [],
                    "removed": [],
                    "changed": [
                        {
                            "id": "model_route:0",
                            "changed_fields": ["targets", token_like_value]
                        }
                    ]
                },
                {
                    "resource_type": "unknown_raw_type",
                    "added": ["SHOULD_NOT_RENDER_UNKNOWN_SECTION"]
                }
            ],
            "raw_yaml": "SHOULD_NOT_RENDER_RAW_YAML",
            "raw_path": "SHOULD_NOT_RENDER_PATH"
        });

        let rendered_json =
            super::render_reload_diff_report(&backend, crate::cli_report::OutputFormat::Json);
        let report: Value = serde_json::from_str(&rendered_json).unwrap();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason_code"], "reload_diff_available");
        assert_eq!(report["next_action"]["template_id"], "no_action_required");
        assert_eq!(report["next_action"]["safe_argv"], json!([]));
        let next_action_summary = report["next_action"]["summary"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase();
        for forbidden in [
            "reload apply",
            "apply --dry-run",
            "--yes",
            "mutation",
            "confirmed apply",
            "apply",
        ] {
            assert!(
                !next_action_summary.contains(forbidden),
                "reload diff next_action summary contains forbidden text {forbidden}"
            );
        }
        assert_eq!(report["reload_apply_status"], "dry_run_available");
        assert_eq!(report["mutating_reload_sent"], false);
        let argv = report["next_action"]["safe_argv"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert!(!argv.contains(&"diff"));
        assert!(!argv.contains(&"apply"));
        assert!(!argv.contains(&"--yes"));
        assert_ne!(argv, ["one-ai-key", "reload", "apply", "--dry-run"]);
        assert_eq!(report["budget"]["max_resource_changes"], 64);
        assert_eq!(report["budget"]["total_resource_changes"], 3);
        assert_eq!(report["budget"]["omitted_resource_changes"], 0);
        assert_eq!(report["budget"]["truncated"], false);
        assert_eq!(report["resource_changes"][0]["resource_type"], "providers");
        assert_eq!(
            report["resource_changes"][0]["added"],
            json!(["provider-a"])
        );
        assert_eq!(
            report["resource_changes"][0]["removed"],
            json!(["provider-b"])
        );
        assert_eq!(
            report["resource_changes"][0]["changed"][0]["changed_fields"],
            json!(["api_base"])
        );
        assert_eq!(
            report["resource_changes"][1]["changed"][0]["id"],
            "model_route:0"
        );
        assert_eq!(
            report["resource_changes"][1]["changed"][0]["changed_fields"],
            json!(["targets"])
        );
        assert_eq!(report["resource_changes"].as_array().unwrap().len(), 2);

        for forbidden in [
            "available_now",
            "reload_apply_now",
            "management_write",
            "SHOULD_NOT_RENDER_RESOURCE_RAW_YAML",
            "SHOULD_NOT_RENDER_UNKNOWN_SECTION",
            "SHOULD_NOT_RENDER_RAW_YAML",
            "SHOULD_NOT_RENDER_PATH",
            token_like_value.as_str(),
        ] {
            assert!(!rendered_json.contains(forbidden));
        }

        let rendered_table =
            super::render_reload_diff_report(&backend, crate::cli_report::OutputFormat::Table);
        assert!(rendered_table.contains("status: ok"));
        assert!(rendered_table.contains("reason_code: reload_diff_available"));
        assert!(rendered_table.contains("resource_changes.model_routes.changed"));
        assert!(rendered_table.contains("model_route:0"));
        for forbidden in [
            "reload apply",
            "apply --dry-run",
            "--yes",
            "mutation",
            "confirmed apply",
        ] {
            assert!(
                !rendered_table.to_ascii_lowercase().contains(forbidden),
                "reload diff table next_action contains forbidden text {forbidden}"
            );
        }
        for forbidden in [
            "available_now",
            "reload_apply_now",
            "management_write",
            "SHOULD_NOT_RENDER_RESOURCE_RAW_YAML",
            "SHOULD_NOT_RENDER_UNKNOWN_SECTION",
            "SHOULD_NOT_RENDER_RAW_YAML",
            "SHOULD_NOT_RENDER_PATH",
            token_like_value.as_str(),
        ] {
            assert!(!rendered_table.contains(forbidden));
        }
    }

    #[tokio::test]
    async fn reload_apply_dry_run_does_not_post_reload() {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let runtime_seen = Arc::clone(&seen_paths);
        let diff_seen = Arc::clone(&seen_paths);
        let post_seen = Arc::clone(&seen_paths);
        let router = Router::new()
            .route(
                "/management/runtime",
                get(move || {
                    let runtime_seen = Arc::clone(&runtime_seen);
                    async move {
                        runtime_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime".to_string());
                        Json(runtime_projection(4))
                    }
                }),
            )
            .route(
                "/management/runtime/reload-diff",
                get(move || {
                    let diff_seen = Arc::clone(&diff_seen);
                    async move {
                        diff_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime/reload-diff".to_string());
                        Json(json!({
                            "status": "ok",
                            "reason_code": "reload_diff_available",
                            "active_registry_generation": 11,
                            "active_registry_version": 3,
                            "staged_registry_version": 4,
                            "runtime_reload_required": true
                        }))
                    }
                }),
            )
            .route(
                "/management/runtime/reload",
                post(move || {
                    let post_seen = Arc::clone(&post_seen);
                    async move {
                        post_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime/reload".to_string());
                        Json(json!({}))
                    }
                }),
            );
        let env_name = format!(
            "ONE_AI_KEY_TEST_RELOAD_APPLY_DRY_RUN_TOKEN_{}",
            std::process::id()
        );
        let connection = reload_apply_connection(spawn_management_fixture(router).await, &env_name);

        let rendered = super::run_apply(super::ReloadApplyOptions {
            connection,
            mode: super::ReloadApplyMode::DryRun,
            expected_staged_registry_version: None,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "planned");
        assert_eq!(report["reason_code"], "reload_apply_dry_run");
        assert_eq!(report["side_effect_class"], "runtime_readonly");
        assert_eq!(report["effect_vector"]["mutates_runtime"], false);
        assert_eq!(report["expected_staged_registry_version"], 4);
        assert_eq!(report["mutating_reload_sent"], false);
        assert_eq!(
            report["next_action"]["safe_argv"],
            json!([
                "one-ai-key",
                "reload",
                "apply",
                "--expected-staged-registry-version",
                "4",
                "--yes"
            ])
        );
        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec![
                "/management/runtime".to_string(),
                "/management/runtime/reload-diff".to_string(),
            ]
        );
    }

    #[test]
    fn reload_apply_requires_confirmation_in_non_tty() {
        let action = crate::cli::CliAction::ReloadApply(super::ReloadApplyOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            mode: super::ReloadApplyMode::NeedsConfirmation,
            expected_staged_registry_version: Some(4),
            output: crate::cli_report::OutputFormat::Json,
        });

        assert_eq!(
            crate::cli_effects::confirmation_outcome(&action, false),
            crate::cli_effects::ConfirmationOutcome::Denied {
                exit_code: 3,
                reason_code: "confirmation_required"
            }
        );
    }

    #[tokio::test]
    async fn reload_apply_yes_posts_runtime_reload_once_when_precondition_supported() {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let runtime_seen = Arc::clone(&seen_paths);
        let post_seen = Arc::clone(&seen_paths);
        let router = Router::new()
            .route(
                "/management/runtime",
                get(move || {
                    let runtime_seen = Arc::clone(&runtime_seen);
                    async move {
                        runtime_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime".to_string());
                        Json(runtime_projection(4))
                    }
                }),
            )
            .route(
                "/management/runtime/reload",
                post(move |Query(query): Query<HashMap<String, String>>| {
                    let post_seen = Arc::clone(&post_seen);
                    async move {
                        post_seen.lock().unwrap().push(format!(
                            "/management/runtime/reload?expected_staged_registry_version={}",
                            query
                                .get("expected_staged_registry_version")
                                .cloned()
                                .unwrap_or_default()
                        ));
                        Json(json!({
                            "active_registry_generation": 12,
                            "active_registry_version": 4,
                            "staged_registry_version": 4,
                            "runtime_reload_required": false,
                            "channels": 2,
                            "raw_yaml": "SHOULD_NOT_RENDER_RAW_YAML"
                        }))
                    }
                }),
            );
        let env_name = format!(
            "ONE_AI_KEY_TEST_RELOAD_APPLY_YES_TOKEN_{}",
            std::process::id()
        );
        let connection = reload_apply_connection(spawn_management_fixture(router).await, &env_name);

        let rendered = super::run_apply(super::ReloadApplyOptions {
            connection,
            mode: super::ReloadApplyMode::Apply,
            expected_staged_registry_version: Some(4),
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason_code"], "runtime_reload_applied");
        assert_eq!(report["side_effect_class"], "management_write");
        assert_eq!(report["effect_vector"]["mutates_runtime"], true);
        assert_eq!(report["mutating_reload_sent"], true);
        assert_eq!(report["management_response"]["active_registry_version"], 4);
        assert!(!rendered.contains("SHOULD_NOT_RENDER_RAW_YAML"));
        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec![
                "/management/runtime".to_string(),
                "/management/runtime/reload?expected_staged_registry_version=4".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn reload_apply_fails_closed_without_generation_precondition() {
        let router = Router::new().route(
            "/management/runtime/reload",
            post(|| async { Json(json!({"unexpected": true})) }),
        );
        let env_name = format!(
            "ONE_AI_KEY_TEST_RELOAD_APPLY_NO_PRECONDITION_TOKEN_{}",
            std::process::id()
        );
        let connection = reload_apply_connection(spawn_management_fixture(router).await, &env_name);

        let rendered = super::run_apply(super::ReloadApplyOptions {
            connection,
            mode: super::ReloadApplyMode::Apply,
            expected_staged_registry_version: None,
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(
            report["reason_code"],
            "reload_apply_precondition_unavailable"
        );
        assert_eq!(report["mutating_reload_sent"], false);
    }

    #[tokio::test]
    async fn reload_apply_fails_closed_when_generation_changed_after_plan() {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let runtime_seen = Arc::clone(&seen_paths);
        let post_seen = Arc::clone(&seen_paths);
        let router = Router::new()
            .route(
                "/management/runtime",
                get(move || {
                    let runtime_seen = Arc::clone(&runtime_seen);
                    async move {
                        runtime_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime".to_string());
                        Json(runtime_projection(5))
                    }
                }),
            )
            .route(
                "/management/runtime/reload",
                post(move || {
                    let post_seen = Arc::clone(&post_seen);
                    async move {
                        post_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime/reload".to_string());
                        Json(json!({}))
                    }
                }),
            );
        let env_name = format!(
            "ONE_AI_KEY_TEST_RELOAD_APPLY_CHANGED_TOKEN_{}",
            std::process::id()
        );
        let connection = reload_apply_connection(spawn_management_fixture(router).await, &env_name);

        let rendered = super::run_apply(super::ReloadApplyOptions {
            connection,
            mode: super::ReloadApplyMode::Apply,
            expected_staged_registry_version: Some(4),
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(
            report["reason_code"],
            "reload_apply_generation_changed_after_plan"
        );
        assert_eq!(report["mutating_reload_sent"], false);
        assert_eq!(report["block"]["current_staged_registry_version"], 5);
        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec!["/management/runtime".to_string()]
        );
    }

    #[test]
    fn reload_apply_reports_mutates_runtime_effect_vector() {
        let action = crate::cli::CliAction::ReloadApply(super::ReloadApplyOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            mode: super::ReloadApplyMode::Apply,
            expected_staged_registry_version: Some(4),
            output: crate::cli_report::OutputFormat::Json,
        });

        let effect = crate::cli_effects::classify_action(&action);

        assert_eq!(
            effect.side_effect_class,
            crate::cli_effects::SideEffectClass::ManagementWrite
        );
        assert_eq!(effect.effect_vector.reads_management_runtime, true);
        assert_eq!(effect.effect_vector.writes_management_store, true);
        assert_eq!(effect.effect_vector.mutates_runtime, true);
    }

    #[tokio::test]
    async fn reload_apply_reports_last_reload_failure_redacted() {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let runtime_seen = Arc::clone(&seen_paths);
        let post_seen = Arc::clone(&seen_paths);
        let router = Router::new()
            .route(
                "/management/runtime",
                get(move || {
                    let runtime_seen = Arc::clone(&runtime_seen);
                    async move {
                        runtime_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime".to_string());
                        Json(runtime_projection(4))
                    }
                }),
            )
            .route(
                "/management/runtime/reload",
                post(move |Query(query): Query<HashMap<String, String>>| {
                    let post_seen = Arc::clone(&post_seen);
                    async move {
                        post_seen.lock().unwrap().push(format!(
                            "/management/runtime/reload?expected_staged_registry_version={}",
                            query
                                .get("expected_staged_registry_version")
                                .cloned()
                                .unwrap_or_default()
                        ));
                        (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({
                                "error": "RAW_FAILURE_SHOULD_NOT_RENDER",
                                "raw_path": "/private/runtime/input.yaml",
                                "credential": "TOKEN_RAW_FAILURE_SHOULD_NOT_RENDER"
                            })),
                        )
                    }
                }),
            );
        let env_name = format!(
            "ONE_AI_KEY_TEST_RELOAD_APPLY_FAILURE_TOKEN_{}",
            std::process::id()
        );
        let connection = reload_apply_connection(spawn_management_fixture(router).await, &env_name);

        let rendered = super::run_apply(super::ReloadApplyOptions {
            connection,
            mode: super::ReloadApplyMode::Apply,
            expected_staged_registry_version: Some(4),
            output: crate::cli_report::OutputFormat::Json,
        })
        .await
        .unwrap();
        std::env::remove_var(env_name);
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "failed");
        assert_eq!(report["reason_code"], "runtime_reload_failed");
        assert_eq!(
            report["last_reload"]["last_reload_error_reason_code"],
            "management_http_error"
        );
        assert_eq!(report["mutating_reload_sent"], true);
        assert!(!rendered.contains("RAW_FAILURE_SHOULD_NOT_RENDER"));
        assert!(!rendered.contains("/private/runtime/input.yaml"));
        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec![
                "/management/runtime".to_string(),
                "/management/runtime/reload?expected_staged_registry_version=4".to_string(),
            ]
        );
    }
}

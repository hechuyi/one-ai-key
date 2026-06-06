use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientTokensListOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub output: crate::cli_report::OutputFormat,
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

pub fn render_client_tokens_list_report(
    tokens: &Value,
    output: crate::cli_report::OutputFormat,
) -> String {
    match output {
        crate::cli_report::OutputFormat::Json => render_client_tokens_list_json(tokens),
        crate::cli_report::OutputFormat::Table => render_client_tokens_list_table(tokens),
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
        "reload_diff_status": "unavailable_until_m4",
        "capability_status": "unavailable_until_m4",
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
            "summary": "No runtime client-token references are available. Add configured client tokens, then run check-config. Reload status is unavailable until M4.",
            "template_id": "check_config",
            "safe_argv": ["one-ai-key", "check-config", "--config", "<config>"],
            "side_effect_class": "offline_readonly",
            "requires_confirmation": false,
            "reload_status": "unavailable_until_m4",
        }),
        "blocked" => serde_json::json!({
            "summary": "All runtime client-token references are disabled. M2.3 is read-only; update the supported configuration or management workflow, then run check-config. Reload status is unavailable until M4.",
            "template_id": "check_config",
            "safe_argv": ["one-ai-key", "check-config", "--config", "<config>"],
            "side_effect_class": "offline_readonly",
            "requires_confirmation": false,
            "reload_status": "unavailable_until_m4",
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
    output.push_str("reload_diff_status: unavailable_until_m4\n");
    output.push_str("capability_status: unavailable_until_m4\n");
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(report["reload_diff_status"], "unavailable_until_m4");
        assert_eq!(report["capability_status"], "unavailable_until_m4");
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
        assert!(rendered.contains("capability_status: unavailable_until_m4"));
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
}

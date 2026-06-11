use serde_json::Value;

pub const DEFAULT_LAST: usize = 50;
pub const MAX_LAST: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseFilterEventsOptions {
    pub connection: crate::cli::OperatorConnectionOptions,
    pub last: Option<usize>,
    pub request_id: Option<String>,
    pub public_model: Option<String>,
    pub channel_id: Option<String>,
    pub action: Option<String>,
    pub output: crate::cli_report::OutputFormat,
}

pub fn parse_response_filter_events_last(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("last must be an integer from 1 to {MAX_LAST}"))?;
    if (1..=MAX_LAST).contains(&parsed) {
        Ok(parsed)
    } else {
        Err(format!("last must be from 1 to {MAX_LAST}"))
    }
}

pub async fn run_events(
    options: ResponseFilterEventsOptions,
) -> Result<String, crate::operator_client::OperatorClientError> {
    let client = crate::cli_commands::operator_client_from_connection(&options.connection)?;
    let (snapshot, window) = fetch_response_filter_events_window(&client, options.last).await?;
    let report = response_filter_events_report(&snapshot, window, &options);
    Ok(render_response_filter_events_report(
        &report,
        options.output,
    ))
}

async fn fetch_response_filter_events_window(
    client: &crate::operator_client::OperatorClient,
    last: Option<usize>,
) -> Result<(Value, ResponseFilterEventsWindow), crate::operator_client::OperatorClientError> {
    let requested_last = last.unwrap_or(DEFAULT_LAST);
    let effective_last = requested_last.clamp(1, MAX_LAST);
    let metadata = client
        .get_json(
            crate::operator_client::ReadOnlyEndpoint::ResponseFilterEvents {
                offset: Some(0),
                limit: Some(0),
            },
        )
        .await?;
    let buffered_events = metadata_usize(&metadata, "buffered_events");
    let offset = buffered_events.saturating_sub(effective_last);
    let snapshot = client
        .get_json(
            crate::operator_client::ReadOnlyEndpoint::ResponseFilterEvents {
                offset: Some(offset),
                limit: Some(effective_last),
            },
        )
        .await?;
    let window = ResponseFilterEventsWindow {
        requested_last,
        effective_last,
        buffered_events,
        capacity: metadata_usize(&metadata, "capacity"),
        dropped_events: metadata_u64(&metadata, "dropped_events"),
        offset,
    };
    Ok((snapshot, window))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResponseFilterEventsWindow {
    requested_last: usize,
    effective_last: usize,
    buffered_events: usize,
    capacity: usize,
    dropped_events: u64,
    offset: usize,
}

impl ResponseFilterEventsWindow {
    fn metadata(self, returned: usize) -> Value {
        serde_json::json!({
            "kind": "bounded_recent_response_filter_events",
            "limit": self.effective_last,
            "capacity": self.capacity,
            "dropped_events": self.dropped_events,
            "buffered_events": self.buffered_events,
            "offset": self.offset,
            "returned": returned,
            "truncated": self.offset > 0,
            "cursor": Value::Null,
            "bounded_reason": "latest_window",
            "requested_last": self.requested_last,
        })
    }
}

fn response_filter_events_report(
    snapshot: &Value,
    window: ResponseFilterEventsWindow,
    options: &ResponseFilterEventsOptions,
) -> Value {
    let projected_events = snapshot
        .get("events")
        .and_then(Value::as_array)
        .map(|events| {
            events
                .iter()
                .map(project_response_filter_event)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let fetched_count = projected_events.len();
    let events = projected_events
        .into_iter()
        .filter(|event| event_matches_filters(event, options))
        .collect::<Vec<_>>();
    let status = if events.is_empty() { "ok" } else { "degraded" };
    let reason_code = if events.is_empty() {
        "no_response_filter_events_found"
    } else {
        "response_filter_events_found"
    };

    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason: if events.is_empty() {
            "No response-filter events matched the bounded recent window."
        } else {
            "Response-filter events matched the bounded recent window."
        },
        reason_code,
        effect: crate::cli_effects::runtime_readonly_store_reads_effect(),
        scope: filters_metadata(options),
        window: window.metadata(events.len()),
        next_action: serde_json::json!({
            "summary": "Inspect bounded response-filter events and correlated failures before changing rules or credentials.",
            "template_id": "response_filter_events_review",
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
            "safe_argv": [
                "one-ai-key",
                "response-filters",
                "events",
                "--management-url",
                "<url>",
                "--management-token-env",
                "<env>",
                "--last",
                DEFAULT_LAST.to_string(),
            ],
        }),
        data: serde_json::json!({
            "fetched_count": fetched_count,
            "event_count": events.len(),
            "matched_count": events.len(),
            "events": events,
        }),
    })
}

pub fn render_response_filter_events_report(
    report: &Value,
    output: crate::cli_report::OutputFormat,
) -> String {
    match output {
        crate::cli_report::OutputFormat::Json => {
            serde_json::to_string_pretty(report).expect("response-filter event report serializes")
        }
        crate::cli_report::OutputFormat::Table => render_response_filter_events_table(report),
    }
}

fn render_response_filter_events_table(report: &Value) -> String {
    let mut output = String::new();
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    if let Some(window) = report.get("window") {
        for field in [
            "requested_last",
            "buffered_events",
            "capacity",
            "dropped_events",
            "offset",
        ] {
            crate::cli_report::push_table_field(
                &mut output,
                &format!("window.{field}"),
                window.get(field),
            );
        }
    }
    if let Some(events) = report
        .get("data")
        .and_then(|data| data.get("events"))
        .and_then(Value::as_array)
    {
        for (index, event) in events.iter().enumerate() {
            output.push_str(&format!(
                "event[{index}]: event_id={} request_id={} public_model={} channel_id={} rule_id={} action={} outcome={} content_kind={} reason_code={} body_committed={}\n",
                table_str(event.get("event_id")),
                table_str(event.get("request_id")),
                table_str(event.get("public_model")),
                table_str(event.get("channel_id")),
                table_str(event.get("rule_id")),
                table_str(event.get("action")),
                table_str(event.get("outcome")),
                table_str(event.get("content_kind")),
                table_str(event.get("reason_code")),
                table_str(event.get("body_committed")),
            ));
        }
    }
    output
}

fn table_str(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => crate::cli_report::escape_table_value(value),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Null) | None => "unknown".to_string(),
        Some(value) => crate::cli_report::escape_table_value(&value.to_string()),
    }
}

fn event_matches_filters(event: &Value, options: &ResponseFilterEventsOptions) -> bool {
    matches_string(event, "request_id", options.request_id.as_deref())
        && matches_string(event, "public_model", options.public_model.as_deref())
        && matches_string(event, "channel_id", options.channel_id.as_deref())
        && matches_string(event, "action", options.action.as_deref())
}

fn matches_string(event: &Value, field: &str, expected: Option<&str>) -> bool {
    match expected {
        Some(expected) => event.get(field).and_then(Value::as_str) == Some(expected),
        None => true,
    }
}

fn project_response_filter_event(event: &Value) -> Value {
    serde_json::json!({
        "event_id": event.get("event_id").and_then(Value::as_u64),
        "created_at_unix_seconds": event.get("created_at_unix_seconds").and_then(Value::as_u64),
        "request_id": sanitize_value(event.get("request_id")),
        "channel_id": sanitize_value(event.get("channel_id")),
        "public_model": sanitize_value(event.get("public_model")),
        "rule_id": sanitize_value(event.get("rule_id")),
        "action": sanitize_enum(event.get("action"), &["redact", "reject", "reject_and_expire_credential", "reject_and_cooldown_channel"]),
        "content_kind": sanitize_enum(event.get("content_kind"), &["json", "sse", "bytes", "unknown"]),
        "reason_code": sanitize_enum(event.get("reason_code"), &["rule_matched", "required_rule_missing"]),
        "outcome": sanitize_enum(event.get("outcome"), &["redacted", "rejected"]),
        "body_committed": event.get("body_committed").and_then(Value::as_bool),
    })
}

fn sanitize_enum(value: Option<&Value>, allowed: &[&str]) -> Value {
    let Some(value) = value.and_then(Value::as_str) else {
        return Value::Null;
    };
    if allowed.contains(&value) {
        Value::String(value.to_string())
    } else {
        Value::Null
    }
}

fn sanitize_value(value: Option<&Value>) -> Value {
    value
        .and_then(Value::as_str)
        .and_then(sanitize_local_string)
        .map(Value::String)
        .unwrap_or(Value::Null)
}

fn filters_metadata(options: &ResponseFilterEventsOptions) -> Value {
    serde_json::json!({
        "request_id": options.request_id.as_deref().and_then(sanitize_local_string),
        "public_model": options.public_model.as_deref().and_then(sanitize_local_string),
        "channel_id": options.channel_id.as_deref().and_then(sanitize_local_string),
        "action": options.action.as_deref().and_then(sanitize_local_string),
    })
}

fn sanitize_local_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return None;
    }
    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("sk-")
        || lower.contains("://")
        || lower.contains("http")
        || lower.contains("telegram")
        || lower.contains("promo")
        || lower.contains("invite")
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn metadata_usize(snapshot: &Value, key: &str) -> usize {
    snapshot
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

fn metadata_u64(snapshot: &Value, key: &str) -> u64 {
    snapshot.get(key).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
pub fn events_endpoint_sequence(
    last: Option<usize>,
    buffered_events: usize,
) -> Vec<crate::operator_client::ReadOnlyEndpoint> {
    let effective_last = last.unwrap_or(DEFAULT_LAST).clamp(1, MAX_LAST);
    vec![
        crate::operator_client::ReadOnlyEndpoint::ResponseFilterEvents {
            offset: Some(0),
            limit: Some(0),
        },
        crate::operator_client::ReadOnlyEndpoint::ResponseFilterEvents {
            offset: Some(buffered_events.saturating_sub(effective_last)),
            limit: Some(effective_last),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> ResponseFilterEventsOptions {
        ResponseFilterEventsOptions {
            connection: crate::cli::OperatorConnectionOptions {
                management_url: Some("https://router.example".to_string()),
                deprecated_base_url: None,
                management_token_env: Some("ONE_AI_KEY_MANAGEMENT_TOKEN".to_string()),
                management_token_stdin: false,
                timeout_seconds: 10,
            },
            last: Some(25),
            request_id: None,
            public_model: None,
            channel_id: None,
            action: None,
            output: crate::cli_report::OutputFormat::Json,
        }
    }

    fn event_fixture() -> Value {
        serde_json::json!({
            "buffered_events": 90,
            "capacity": 1024,
            "dropped_events": 3,
            "offset": 65,
            "limit": 25,
            "events": [
                {
                    "event_id": 41,
                    "created_at_unix_seconds": 1770000000,
                    "request_id": "req-filter",
                    "channel_id": "relay-a",
                    "public_model": "gpt-example",
                    "rule_id": "relay_ad_guard",
                    "action": "reject",
                    "content_kind": "json",
                    "reason_code": "rule_matched",
                    "outcome": "rejected",
                    "body_committed": false,
                    "matched_text": "sk-should-not-leak",
                    "response_body": "https://example.invalid/should-not-leak"
                },
                {
                    "event_id": 42,
                    "created_at_unix_seconds": 1770000001,
                    "request_id": "req-other",
                    "channel_id": "relay-b",
                    "public_model": "gpt-example",
                    "rule_id": "https://unsafe.invalid/private?token=secret",
                    "action": "redact",
                    "content_kind": "sse",
                    "reason_code": "rule_matched",
                    "outcome": "redacted",
                    "body_committed": true
                }
            ]
        })
    }

    #[test]
    fn response_filter_events_endpoint_sequence_fetches_metadata_then_latest_window() {
        assert_eq!(
            events_endpoint_sequence(Some(25), 90),
            vec![
                crate::operator_client::ReadOnlyEndpoint::ResponseFilterEvents {
                    offset: Some(0),
                    limit: Some(0),
                },
                crate::operator_client::ReadOnlyEndpoint::ResponseFilterEvents {
                    offset: Some(65),
                    limit: Some(25),
                },
            ]
        );
    }

    #[test]
    fn response_filter_events_json_filters_and_sanitizes_bounded_window() {
        let mut options = options();
        options.request_id = Some("req-filter".to_string());
        options.action = Some("reject".to_string());
        let report = response_filter_events_report(
            &event_fixture(),
            ResponseFilterEventsWindow {
                requested_last: 25,
                effective_last: 25,
                buffered_events: 90,
                capacity: 1024,
                dropped_events: 3,
                offset: 65,
            },
            &options,
        );

        assert_eq!(report["status"], "degraded");
        assert_eq!(report["reason_code"], "response_filter_events_found");
        assert_eq!(
            report["window"]["kind"],
            "bounded_recent_response_filter_events"
        );
        assert_eq!(report["window"]["returned"], 1);
        assert_eq!(report["window"]["truncated"], true);
        assert_eq!(report["data"]["fetched_count"], 2);
        assert_eq!(report["data"]["event_count"], 1);
        assert_eq!(report["data"]["matched_count"], 1);
        assert_eq!(report["data"]["events"][0]["request_id"], "req-filter");
        assert_eq!(report["data"]["events"][0]["action"], "reject");
        assert_eq!(report["data"]["events"][0]["rule_id"], "relay_ad_guard");
        assert_eq!(report["effect_vector"]["calls_upstream"], false);
        assert_eq!(report["effect_vector"]["writes_management_store"], false);
        assert_eq!(
            report["next_action"]["side_effect_class"],
            "runtime_readonly"
        );
        assert!(crate::diagnostic_contract::is_valid_safe_next_action(
            &report["next_action"]
        ));

        let rendered =
            render_response_filter_events_report(&report, crate::cli_report::OutputFormat::Json);
        for forbidden in [
            "matched_text",
            "response_body",
            "sk-should-not-leak",
            "example.invalid",
            "unsafe.invalid",
            "token=secret",
        ] {
            assert!(!rendered.contains(forbidden), "leaked {forbidden}");
        }
    }

    #[test]
    fn response_filter_events_table_escapes_and_reports_bounded_metadata() {
        let report = response_filter_events_report(
            &event_fixture(),
            ResponseFilterEventsWindow {
                requested_last: 25,
                effective_last: 25,
                buffered_events: 90,
                capacity: 1024,
                dropped_events: 3,
                offset: 65,
            },
            &options(),
        );
        let rendered =
            render_response_filter_events_report(&report, crate::cli_report::OutputFormat::Table);

        assert!(rendered.contains("window.buffered_events: 90"));
        assert!(rendered.contains("window.dropped_events: 3"));
        assert!(rendered.contains("event[0]: event_id=41"));
        assert!(rendered.contains("rule_id=relay_ad_guard"));
        assert!(rendered.contains("event[1]: event_id=42"));
        assert!(rendered.contains("rule_id=unknown"));
        assert!(!rendered.contains("unsafe.invalid"));
    }
}

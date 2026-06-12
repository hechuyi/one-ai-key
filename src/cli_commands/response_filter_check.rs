use std::{fs, path::PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::registry::RegistryRepository;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseFilterCheckOptions {
    pub config_path: PathBuf,
    pub samples_path: PathBuf,
    pub output: crate::cli_report::OutputFormat,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseFilterCheckSample {
    id: String,
    content_kind: ResponseFilterSampleKind,
    body: String,
    expect_outcome: ResponseFilterOutcome,
    #[serde(default)]
    expect_rule_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ResponseFilterSampleKind {
    Plain,
    ErrorJson,
}

impl ResponseFilterSampleKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::ErrorJson => "error_json",
        }
    }

    fn allow_required_missing(self) -> bool {
        matches!(self, Self::Plain)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ResponseFilterOutcome {
    Unchanged,
    Redacted,
    Rejected,
}

impl ResponseFilterOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Redacted => "redacted",
            Self::Rejected => "rejected",
        }
    }
}

pub fn run(options: ResponseFilterCheckOptions) -> anyhow::Result<String> {
    let report = response_filter_check_report(&options);
    Ok(render_response_filter_check_report(&report, options.output))
}

fn response_filter_check_report(options: &ResponseFilterCheckOptions) -> Value {
    let policy = match load_response_filter_policy(&options.config_path) {
        Ok(policy) => policy,
        Err(error) => {
            return response_filter_check_error_report(response_filter_check_error_reason(
                &error.to_string(),
                "config",
            ));
        }
    };
    let raw_samples = match fs::read_to_string(&options.samples_path) {
        Ok(raw) => raw,
        Err(_) => {
            return response_filter_check_error_report("local_io_error");
        }
    };
    let samples = match serde_yaml::from_str::<Vec<ResponseFilterCheckSample>>(&raw_samples) {
        Ok(samples) => samples,
        Err(_) => {
            return response_filter_check_error_report("response_filter_sample_schema_invalid");
        }
    };

    let sample_results = samples
        .iter()
        .map(|sample| evaluate_sample(&policy, sample))
        .collect::<Vec<_>>();
    let failed_count = sample_results
        .iter()
        .filter(|sample| {
            sample.get("reason_code").and_then(Value::as_str) != Some("sample_expectation_met")
        })
        .count();
    let status = if failed_count == 0 { "ok" } else { "blocked" };
    let reason_code = if failed_count == 0 {
        "response_filter_samples_passed"
    } else {
        "response_filter_sample_mismatch"
    };
    response_filter_check_envelope(
        status,
        reason_code,
        json!({
            "command": "response-filters check",
            "sample_count": sample_results.len(),
            "failed_count": failed_count,
            "samples": sample_results,
        }),
    )
}

fn load_response_filter_policy(
    config_path: &PathBuf,
) -> anyhow::Result<crate::response_filter::ResponseFilterPolicy> {
    let document = crate::registry::YamlRegistryRepository::new(config_path).load_registry()?;
    Ok(document.response_filter.resolve_for_offline_check()?.policy)
}

fn evaluate_sample(
    policy: &crate::response_filter::ResponseFilterPolicy,
    sample: &ResponseFilterCheckSample,
) -> Value {
    let decision = policy
        .inspect_text_for_precommit(&sample.body, sample.content_kind.allow_required_missing());
    let (actual_outcome, matches) = match decision {
        crate::response_filter::ResponseFilterDecision::Unchanged => {
            (ResponseFilterOutcome::Unchanged, Vec::new())
        }
        crate::response_filter::ResponseFilterDecision::Redacted { matches, .. } => {
            (ResponseFilterOutcome::Redacted, matches)
        }
        crate::response_filter::ResponseFilterDecision::Rejected { matches } => {
            (ResponseFilterOutcome::Rejected, matches)
        }
    };
    let sanitized_matched_rule_ids =
        sanitize_rule_ids(matches.iter().map(|matched| &matched.rule_id));
    let sanitized_expected_rule_ids = sanitize_rule_ids(sample.expect_rule_ids.iter());
    let matched_rule_ids = sanitized_matched_rule_ids.clone().unwrap_or_default();
    let expected_rule_ids = sanitized_expected_rule_ids.clone().unwrap_or_default();
    let rule_ids_match = sample.expect_rule_ids.is_empty()
        || sorted_values(&matched_rule_ids) == sorted_values(&expected_rule_ids);
    let outcome_matches = actual_outcome == sample.expect_outcome;
    let reason_code = if sanitized_expected_rule_ids.is_err() {
        "sample_expected_rule_id_invalid"
    } else if sanitized_matched_rule_ids.is_err() {
        "sample_matched_rule_id_invalid"
    } else if outcome_matches && rule_ids_match {
        "sample_expectation_met"
    } else if !outcome_matches {
        "sample_outcome_mismatch"
    } else {
        "sample_rule_ids_mismatch"
    };

    json!({
        "sample_id": safe_local_code(&sample.id).unwrap_or_else(|| "unknown".to_string()),
        "content_kind": sample.content_kind.as_str(),
        "expected_outcome": sample.expect_outcome.as_str(),
        "actual_outcome": actual_outcome.as_str(),
        "matched_rule_ids": matched_rule_ids,
        "reason_code": reason_code,
    })
}

fn sanitize_rule_ids<'a>(
    rule_ids: impl IntoIterator<Item = &'a String>,
) -> Result<Vec<String>, ()> {
    rule_ids
        .into_iter()
        .map(|rule_id| safe_local_code(rule_id).ok_or(()))
        .collect()
}

fn response_filter_check_error_report(reason_code: &str) -> Value {
    response_filter_check_envelope(
        "blocked",
        reason_code,
        json!({
            "command": "response-filters check",
            "sample_count": 0,
            "failed_count": 0,
            "samples": [],
        }),
    )
}

fn response_filter_check_envelope(status: &str, reason_code: &str, data: Value) -> Value {
    crate::cli_report::report_envelope_with_legacy_fields(crate::cli_report::ReportEnvelope {
        status,
        reason: response_filter_check_reason(status, reason_code),
        reason_code,
        effect: crate::cli_effects::CommandEffect {
            side_effect_class: crate::cli_effects::SideEffectClass::OfflineReadonly,
            effect_vector: crate::cli_effects::EffectVector {
                reads_local_files: true,
                ..crate::cli_effects::EffectVector::default()
            },
        },
        scope: json!({
            "input": "local_config_and_samples",
            "sample_source": "local_file",
        }),
        window: Value::Null,
        next_action: json!({
            "summary": "Update local response-filter samples or configured rules, then rerun the offline check.",
            "template_id": "response_filters_check",
            "side_effect_class": "offline_readonly",
            "requires_confirmation": false,
            "safe_argv": [
                "one-ai-key",
                "--config",
                "<config>",
                "response-filters",
                "check",
                "--samples",
                "<samples>"
            ],
        }),
        data,
    })
}

fn response_filter_check_reason(status: &str, reason_code: &str) -> &'static str {
    match (status, reason_code) {
        ("ok", "response_filter_samples_passed") => {
            "All response-filter samples matched configured rules."
        }
        ("blocked", "response_filter_sample_mismatch") => {
            "One or more response-filter samples did not match expectations."
        }
        ("blocked", "response_filter_sample_schema_invalid") => {
            "Response-filter sample input is invalid."
        }
        ("blocked", "local_io_error") => "Response-filter local input could not be read.",
        ("blocked", "malformed_config") | ("blocked", "response_filter_config_invalid") => {
            "Response-filter configuration could not be loaded."
        }
        _ => "Response-filter sample check did not complete successfully.",
    }
}

fn response_filter_check_error_reason(message: &str, source: &str) -> &'static str {
    if message.contains("read registry config") || source == "samples" {
        "local_io_error"
    } else if message.contains("parse registry config YAML") {
        "malformed_config"
    } else {
        "response_filter_config_invalid"
    }
}

pub fn render_response_filter_check_report(
    report: &Value,
    output: crate::cli_report::OutputFormat,
) -> String {
    match output {
        crate::cli_report::OutputFormat::Json => {
            serde_json::to_string_pretty(report).expect("response filter check report serializes")
        }
        crate::cli_report::OutputFormat::Table => render_response_filter_check_table(report),
    }
}

fn render_response_filter_check_table(report: &Value) -> String {
    let mut output = String::new();
    crate::cli_report::append_report_envelope_table_fields(&mut output, report);
    for field in ["sample_count", "failed_count"] {
        crate::cli_report::push_table_field(
            &mut output,
            &format!("data.{field}"),
            report.get("data").and_then(|data| data.get(field)),
        );
    }
    if let Some(samples) = report
        .get("data")
        .and_then(|data| data.get("samples"))
        .and_then(Value::as_array)
    {
        for (index, sample) in samples.iter().enumerate() {
            output.push_str(&format!(
                "sample[{index}]: sample_id={} content_kind={} expected_outcome={} actual_outcome={} matched_rule_ids={} reason_code={}\n",
                table_str(sample.get("sample_id")),
                table_str(sample.get("content_kind")),
                table_str(sample.get("expected_outcome")),
                table_str(sample.get("actual_outcome")),
                table_str(sample.get("matched_rule_ids")),
                table_str(sample.get("reason_code")),
            ));
        }
    }
    output
}

fn table_str(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => crate::cli_report::escape_table_value(value),
        Some(Value::Array(values)) => crate::cli_report::escape_table_value(
            &values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(","),
        ),
        Some(value) => crate::cli_report::escape_table_value(&value.to_string()),
        None => "unknown".to_string(),
    }
}

fn safe_local_code(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return None;
    }
    if !trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
    {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("sk-")
        || lower.contains("://")
        || lower.contains("http")
        || lower.contains("www.")
        || lower.contains("token")
        || lower.contains("secret")
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn sorted_values(values: &[String]) -> Vec<&str> {
    let mut sorted = values.iter().map(String::as_str).collect::<Vec<_>>();
    sorted.sort_unstable();
    sorted
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn temp_path(name: &str) -> PathBuf {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "one-ai-key-response-filter-check-{}-{unique}-{name}",
            std::process::id()
        ))
    }

    fn write_file(path: &Path, contents: &str) {
        fs::write(path, contents).unwrap();
    }

    fn sample_config() -> String {
        r#"
pools: {}
response_filter:
  enabled: true
  replacement: "[redacted]"
  rules:
    - id: literal-redact
      kind: literal
      action: redact
      value: PRIVATE_MARKER
    - id: reject-marker
      kind: literal
      action: reject
      value: BLOCK_MARKER
    - id: required-success-marker
      kind: required_literal
      action: reject
      value: REQUIRED_OK
"#
        .to_string()
    }

    fn run_json(config: &str, samples: &str) -> String {
        let config_path = temp_path("config.yaml");
        let samples_path = temp_path("samples.yaml");
        write_file(&config_path, config);
        write_file(&samples_path, samples);
        let rendered = run(ResponseFilterCheckOptions {
            config_path: config_path.clone(),
            samples_path: samples_path.clone(),
            output: crate::cli_report::OutputFormat::Json,
        })
        .unwrap();
        let _ = fs::remove_file(config_path);
        let _ = fs::remove_file(samples_path);
        rendered
    }

    #[test]
    fn response_filter_check_reports_redaction_and_rejection_without_sample_body() {
        let rendered = run_json(
            &sample_config(),
            r#"
- id: redacts-private-marker
  content_kind: plain
  body: "hello PRIVATE_MARKER REQUIRED_OK"
  expect_outcome: redacted
  expect_rule_ids: ["literal-redact"]
- id: rejects-block-marker
  content_kind: plain
  body: "BLOCK_MARKER REQUIRED_OK"
  expect_outcome: rejected
  expect_rule_ids: ["reject-marker"]
"#,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["reason_code"], "response_filter_samples_passed");
        assert_eq!(report["data"]["sample_count"], json!(2));
        assert_eq!(
            report["data"]["samples"][0]["matched_rule_ids"],
            json!(["literal-redact"])
        );
        assert_eq!(
            report["data"]["samples"][1]["matched_rule_ids"],
            json!(["reject-marker"])
        );
        assert!(!rendered.contains("PRIVATE_MARKER"));
        assert!(!rendered.contains("BLOCK_MARKER"));
        assert!(!rendered.contains("REQUIRED_OK"));
    }

    #[test]
    fn response_filter_check_does_not_reject_error_json_for_required_rule_missing() {
        let rendered = run_json(
            &sample_config(),
            r#"
- id: retained-error-json
  content_kind: error_json
  body: '{"error":{"message":"upstream said no required marker"}}'
  expect_outcome: unchanged
"#,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "ok");
        assert_eq!(report["data"]["samples"][0]["actual_outcome"], "unchanged");
        assert!(!rendered.contains("upstream said"));
    }

    #[test]
    fn response_filter_check_invalid_sample_schema_uses_stable_reason_without_body() {
        let rendered = run_json(
            &sample_config(),
            r#"
- id: invalid-sample
  content_kind: plain
  body: "PRIVATE_BODY_SECRET"
"#,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(
            report["reason_code"],
            "response_filter_sample_schema_invalid"
        );
        assert!(!rendered.contains("PRIVATE_BODY_SECRET"));
    }

    #[test]
    fn response_filter_check_mismatch_does_not_leak_sample_body_or_private_url() {
        let rendered = run_json(
            &sample_config(),
            r#"
- id: mismatch-sample
  content_kind: plain
  body: "PRIVATE_MISMATCH_BODY https://private.example/path REQUIRED_OK"
  expect_outcome: rejected
  expect_rule_ids: ["reject-marker"]
"#,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["reason_code"], "response_filter_sample_mismatch");
        assert_eq!(
            report["data"]["samples"][0]["reason_code"],
            "sample_outcome_mismatch"
        );
        assert!(!rendered.contains("PRIVATE_MISMATCH_BODY"));
        assert!(!rendered.contains("private.example"));
        assert!(!rendered.contains("REQUIRED_OK"));
    }

    #[test]
    fn response_filter_check_invalid_expected_rule_id_fails_closed_without_leaking_value() {
        let rendered = run_json(
            &sample_config(),
            r#"
- id: unsafe-expected-rule
  content_kind: plain
  body: "hello REQUIRED_OK"
  expect_outcome: unchanged
  expect_rule_ids: ["secret-token-rule"]
"#,
        );
        let report: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(report["status"], "blocked");
        assert_eq!(report["reason_code"], "response_filter_sample_mismatch");
        assert_eq!(
            report["data"]["samples"][0]["reason_code"],
            "sample_expected_rule_id_invalid"
        );
        assert_eq!(report["data"]["samples"][0]["matched_rule_ids"], json!([]));
        assert!(!rendered.contains("secret-token-rule"));
        assert!(!rendered.contains("REQUIRED_OK"));
    }
}

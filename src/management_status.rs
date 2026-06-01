use serde::Serialize;
use std::collections::{BTreeSet, HashMap};

use crate::{
    pool::KeyPoolSnapshot,
    state::{ChannelHealth, PoolState},
};

#[derive(Debug, Clone, Serialize)]
pub struct ChannelHealthStatus {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RuntimeCredentialCounts {
    pub total: usize,
    pub available: usize,
    pub cooling_down: usize,
    pub expired: usize,
    pub quota_exhausted: usize,
    pub disabled: usize,
}

#[derive(Debug, Serialize, Default)]
pub struct RuntimeChannelHealthCounts {
    pub total: usize,
    pub available: usize,
    pub cooling_down: usize,
    pub degraded: usize,
    pub disabled: usize,
}

pub fn add_runtime_channel_health_count(
    counts: &mut RuntimeChannelHealthCounts,
    pool_state: &PoolState,
) {
    counts.total += 1;
    match channel_health_status(pool_state).kind {
        "available" => counts.available += 1,
        "cooling_down" => counts.cooling_down += 1,
        "degraded" => counts.degraded += 1,
        "disabled" => counts.disabled += 1,
        _ => {}
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CredentialPoolAlertStatus {
    pub kind: &'static str,
    pub severity: &'static str,
    pub total_credentials: usize,
    pub available_credentials: usize,
    pub cooling_down_credentials: usize,
    pub expired_credentials: usize,
    pub quota_exhausted_credentials: usize,
    pub disabled_credentials: usize,
}

#[derive(Debug, Serialize)]
pub struct CredentialSetOperationalAlert {
    pub kind: &'static str,
    pub severity: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct CredentialSetOperationalState {
    pub status: &'static str,
    pub serving_mode: &'static str,
    pub accepting_requests: bool,
    pub needs_operator_input: bool,
    pub required_action: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct CredentialSetReadinessProjection {
    pub needs_operator_input: bool,
    pub blocking: bool,
}

#[derive(Debug, Clone)]
pub struct CredentialSetRuntimeSample {
    pub counts: RuntimeCredentialCounts,
    pub readiness: CredentialSetReadinessProjection,
}

impl CredentialSetRuntimeSample {
    pub fn has_available_credentials(&self) -> bool {
        self.counts.available > 0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RuntimeReadinessProjection {
    pub status: &'static str,
    pub blocking_alerts: usize,
}

#[derive(Default)]
pub struct CredentialSetSnapshotCache {
    snapshots: HashMap<String, KeyPoolSnapshot>,
}

#[derive(Default)]
pub struct CredentialSetRuntimeSampler {
    snapshot_cache: CredentialSetSnapshotCache,
}

impl CredentialSetRuntimeSampler {
    pub async fn sample_pool(&mut self, pool_state: &PoolState) -> CredentialSetRuntimeSample {
        self.snapshot_cache.runtime_sample_for(pool_state).await
    }

    pub fn accumulator(&mut self) -> CredentialSetRuntimeAccumulator<'_> {
        CredentialSetRuntimeAccumulator {
            snapshot_cache: &mut self.snapshot_cache,
            seen: BTreeSet::new(),
        }
    }
}

pub struct CredentialSetRuntimeAccumulator<'a> {
    snapshot_cache: &'a mut CredentialSetSnapshotCache,
    seen: BTreeSet<String>,
}

impl CredentialSetRuntimeAccumulator<'_> {
    pub async fn add_pool_counts(
        &mut self,
        pool_state: &PoolState,
        counts: &mut RuntimeCredentialCounts,
    ) -> bool {
        self.snapshot_cache
            .add_counts_once(&mut self.seen, pool_state, counts)
            .await
    }

    pub fn seen_credential_set_ids(&self) -> &BTreeSet<String> {
        &self.seen
    }
}

impl CredentialSetSnapshotCache {
    async fn snapshot_for(&mut self, pool_state: &PoolState) -> KeyPoolSnapshot {
        match self.snapshots.get(&pool_state.credential_set_id.0) {
            Some(snapshot) => snapshot.clone(),
            None => {
                let pool = pool_state.pool.lock().await;
                let snapshot = pool.snapshot();
                self.snapshots
                    .insert(pool_state.credential_set_id.0.clone(), snapshot.clone());
                snapshot
            }
        }
    }

    pub async fn runtime_sample_for(
        &mut self,
        pool_state: &PoolState,
    ) -> CredentialSetRuntimeSample {
        let snapshot = self.snapshot_for(pool_state).await;
        credential_set_runtime_sample_from_snapshot(snapshot)
    }

    pub async fn add_counts_once(
        &mut self,
        seen: &mut BTreeSet<String>,
        pool_state: &PoolState,
        counts: &mut RuntimeCredentialCounts,
    ) -> bool {
        if seen.insert(pool_state.credential_set_id.0.clone()) {
            let snapshot = self.snapshot_for(pool_state).await;
            add_key_pool_snapshot_counts(counts, snapshot);
            true
        } else {
            false
        }
    }
}

pub fn add_key_pool_snapshot_counts(
    counts: &mut RuntimeCredentialCounts,
    snapshot: KeyPoolSnapshot,
) {
    counts.total += snapshot.total_credentials;
    counts.available += snapshot.available_credentials;
    counts.cooling_down += snapshot.cooling_down_credentials;
    counts.expired += snapshot.expired_credentials;
    counts.quota_exhausted += snapshot.quota_exhausted_credentials;
    counts.disabled += snapshot.disabled_credentials;
}

pub fn credential_set_runtime_sample_from_snapshot(
    snapshot: KeyPoolSnapshot,
) -> CredentialSetRuntimeSample {
    let mut counts = RuntimeCredentialCounts::default();
    add_key_pool_snapshot_counts(&mut counts, snapshot);
    let readiness = credential_set_readiness_projection(&counts);
    CredentialSetRuntimeSample { counts, readiness }
}

pub fn credential_pool_alerts(counts: &RuntimeCredentialCounts) -> Vec<CredentialPoolAlertStatus> {
    if counts.total == 0 {
        return Vec::new();
    }

    let alert = |kind, severity| CredentialPoolAlertStatus {
        kind,
        severity,
        total_credentials: counts.total,
        available_credentials: counts.available,
        cooling_down_credentials: counts.cooling_down,
        expired_credentials: counts.expired,
        quota_exhausted_credentials: counts.quota_exhausted,
        disabled_credentials: counts.disabled,
    };

    if counts.available == 0 {
        vec![alert("credential_pool_exhausted", "critical")]
    } else if counts.total > 1 && counts.available == 1 {
        vec![alert("credential_pool_degraded", "warning")]
    } else {
        Vec::new()
    }
}

pub fn credential_set_operational_state(
    counts: &RuntimeCredentialCounts,
) -> CredentialSetOperationalState {
    if counts.total == 0 || counts.available == 0 {
        CredentialSetOperationalState {
            status: "exhausted",
            serving_mode: "stopped",
            accepting_requests: false,
            needs_operator_input: true,
            required_action: "import_valid_credentials",
        }
    } else if counts.available == 1 {
        CredentialSetOperationalState {
            status: "transition_required",
            serving_mode: "serving_degraded",
            accepting_requests: true,
            needs_operator_input: true,
            required_action: "import_valid_credentials",
        }
    } else if counts.expired > 0 || counts.quota_exhausted > 0 || counts.disabled > 0 {
        CredentialSetOperationalState {
            status: "degraded",
            serving_mode: "serving",
            accepting_requests: true,
            needs_operator_input: false,
            required_action: "monitor_or_restore_credentials",
        }
    } else {
        CredentialSetOperationalState {
            status: "healthy",
            serving_mode: "serving",
            accepting_requests: true,
            needs_operator_input: false,
            required_action: "none",
        }
    }
}

pub fn credential_set_readiness_projection(
    counts: &RuntimeCredentialCounts,
) -> CredentialSetReadinessProjection {
    let operational_state = credential_set_operational_state(counts);
    CredentialSetReadinessProjection {
        needs_operator_input: operational_state.needs_operator_input,
        blocking: !operational_state.accepting_requests,
    }
}

pub fn runtime_readiness_projection(
    serving_channels: usize,
    credential_set_blocking_alerts: usize,
) -> RuntimeReadinessProjection {
    let ready = serving_channels > 0;
    RuntimeReadinessProjection {
        status: if ready { "ready" } else { "not_ready" },
        blocking_alerts: credential_set_blocking_alerts.max(usize::from(!ready)),
    }
}

pub fn credential_set_operational_alerts(
    counts: &RuntimeCredentialCounts,
) -> Vec<CredentialSetOperationalAlert> {
    if counts.total == 0 || counts.available == 0 {
        vec![CredentialSetOperationalAlert {
            kind: "credential_set_exhausted",
            severity: "critical",
            message: "credential set has no available credentials; import validated credentials before serving requests",
        }]
    } else if counts.available == 1 {
        vec![CredentialSetOperationalAlert {
            kind: "credential_set_transition_required",
            severity: "warning",
            message: "credential set is serving on a single available credential; import validated replacement credentials",
        }]
    } else if counts.expired > 0 || counts.quota_exhausted > 0 || counts.disabled > 0 {
        vec![CredentialSetOperationalAlert {
            kind: "credential_set_degraded",
            severity: "info",
            message:
                "credential set contains unavailable credentials but still has replacement capacity",
        }]
    } else {
        Vec::new()
    }
}

pub fn redact_management_reason(reason: &str) -> String {
    redact_secret_segments(&redact_bearer_secret_values(&redact_labeled_secret_values(
        &redact_path_segments(reason),
    )))
}

pub fn channel_health_status(pool_state: &PoolState) -> ChannelHealthStatus {
    channel_health_status_from_health(
        pool_state
            .health
            .lock()
            .expect("channel health mutex poisoned")
            .clone(),
    )
}

pub fn channel_health_status_from_health(health: ChannelHealth) -> ChannelHealthStatus {
    match health {
        ChannelHealth::Available => ChannelHealthStatus {
            kind: "available",
            reason: None,
            remaining_seconds: None,
        },
        ChannelHealth::CoolingDown { until, reason } if std::time::Instant::now() < until => {
            ChannelHealthStatus {
                kind: "cooling_down",
                reason: Some(reason),
                remaining_seconds: Some(
                    until
                        .saturating_duration_since(std::time::Instant::now())
                        .as_secs()
                        .max(1),
                ),
            }
        }
        ChannelHealth::CoolingDown { .. } => ChannelHealthStatus {
            kind: "available",
            reason: None,
            remaining_seconds: None,
        },
        ChannelHealth::Degraded { reason } => ChannelHealthStatus {
            kind: "degraded",
            reason: Some(reason),
            remaining_seconds: None,
        },
        ChannelHealth::Disabled { reason } => ChannelHealthStatus {
            kind: "disabled",
            reason: Some(redact_management_reason(&reason)),
            remaining_seconds: None,
        },
    }
}
pub fn redacted_api_base(api_base: &str) -> String {
    match reqwest::Url::parse(api_base) {
        Ok(url) => {
            let mut redacted = String::new();
            redacted.push_str(url.scheme());
            redacted.push_str("://");
            if let Some(host) = url.host_str() {
                redacted.push_str(host);
            }
            if let Some(port) = url.port() {
                redacted.push(':');
                redacted.push_str(&port.to_string());
            }
            redacted.push_str(url.path());
            redacted
        }
        Err(_) => redact_unparsed_api_base(api_base),
    }
}

fn redact_secret_segments(input: &str) -> String {
    redact_segments(
        input,
        |remaining| remaining.starts_with("sk-"),
        "[redacted]",
    )
}

fn redact_unparsed_api_base(api_base: &str) -> String {
    let without_query = api_base
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .to_string();
    let Some(scheme_end) = without_query.find("://") else {
        return without_query
            .rsplit_once('@')
            .map(|(_, rest)| rest.to_string())
            .unwrap_or(without_query);
    };
    let authority_start = scheme_end + 3;
    let authority_and_path = &without_query[authority_start..];
    let authority_len = authority_and_path
        .find('/')
        .unwrap_or(authority_and_path.len());
    let authority = &authority_and_path[..authority_len];
    let Some((_, redacted_authority)) = authority.rsplit_once('@') else {
        return without_query;
    };
    let mut redacted = String::new();
    redacted.push_str(&without_query[..authority_start]);
    redacted.push_str(redacted_authority);
    redacted.push_str(&authority_and_path[authority_len..]);
    redacted
}

fn redact_path_segments(input: &str) -> String {
    redact_segments(
        input,
        |remaining| {
            remaining.starts_with("/Users/")
                || remaining.starts_with("/etc/")
                || remaining.starts_with("/home/")
                || remaining.starts_with("/opt/")
                || remaining.starts_with("/private/tmp/")
                || remaining.starts_with("/root/")
                || remaining.starts_with("/tmp/")
                || remaining.starts_with("/usr/")
                || remaining.starts_with("/var/")
                || remaining.starts_with("file:///etc/")
                || remaining.starts_with("file:///Users/")
                || remaining.starts_with("file:///home/")
                || remaining.starts_with("file:///opt/")
                || remaining.starts_with("file:///root/")
                || remaining.starts_with("file:///usr/")
                || remaining.starts_with("file:///var/")
                || remaining.starts_with("~/")
        },
        "[redacted-path]",
    )
}

fn redact_labeled_secret_values(input: &str) -> String {
    redact_key_value_secret_values(
        input,
        &[
            "authorization",
            "access_token",
            "api_key",
            "api-key",
            "apikey",
            "token",
            "key",
        ],
    )
}

fn redact_bearer_secret_values(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut index = 0usize;
    while index < input.len() {
        let remaining = &input[index..];
        if starts_with_ignore_ascii_case(remaining, "bearer") {
            let after_label = &remaining["bearer".len()..];
            let whitespace_len = leading_whitespace_len(after_label);
            if whitespace_len > 0 {
                if after_label[whitespace_len..].starts_with("[redacted]") {
                    output.push_str(
                        &remaining[.."bearer".len() + whitespace_len + "[redacted]".len()],
                    );
                    index += "bearer".len() + whitespace_len + "[redacted]".len();
                    continue;
                }
                output.push_str(&remaining[.."bearer".len() + whitespace_len]);
                output.push_str("[redacted]");
                index += "bearer".len()
                    + whitespace_len
                    + sensitive_segment_len(&after_label[whitespace_len..]);
                continue;
            }
        }
        let ch = remaining
            .chars()
            .next()
            .expect("index is inside utf-8 boundary");
        output.push(ch);
        index += ch.len_utf8();
    }
    output
}

fn redact_key_value_secret_values(input: &str, labels: &[&str]) -> String {
    let mut output = String::with_capacity(input.len());
    let mut index = 0usize;
    while index < input.len() {
        let remaining = &input[index..];
        if let Some(label) = labels
            .iter()
            .find(|label| starts_with_ignore_ascii_case(remaining, label))
        {
            let label_len = label.len();
            let after_label = &remaining[label_len..];
            let before_separator_len = leading_whitespace_len(after_label);
            let after_whitespace = &after_label[before_separator_len..];
            if let Some(separator) = after_whitespace
                .chars()
                .next()
                .filter(|ch| matches!(ch, ':' | '='))
            {
                let separator_len = separator.len_utf8();
                let after_separator = &after_whitespace[separator_len..];
                let after_separator_whitespace_len = leading_whitespace_len(after_separator);
                let value = &after_separator[after_separator_whitespace_len..];
                output.push_str(&remaining[..label_len + before_separator_len + separator_len]);
                output.push_str(&after_separator[..after_separator_whitespace_len]);
                if label.eq_ignore_ascii_case("authorization")
                    && starts_with_ignore_ascii_case(value, "bearer")
                {
                    let after_bearer = &value["bearer".len()..];
                    let bearer_whitespace_len = leading_whitespace_len(after_bearer);
                    output.push_str(&value[.."bearer".len()]);
                    output.push(' ');
                    output.push_str("[redacted]");
                    index += label_len
                        + before_separator_len
                        + separator_len
                        + after_separator_whitespace_len
                        + "bearer".len()
                        + bearer_whitespace_len
                        + sensitive_segment_len(&after_bearer[bearer_whitespace_len..]);
                } else if label.eq_ignore_ascii_case("authorization") {
                    output.push_str("[redacted]");
                    index += label_len
                        + before_separator_len
                        + separator_len
                        + after_separator_whitespace_len
                        + authorization_value_len(value);
                } else {
                    output.push_str("[redacted]");
                    index += label_len
                        + before_separator_len
                        + separator_len
                        + after_separator_whitespace_len
                        + sensitive_segment_len(value);
                }
                continue;
            } else {
                let after_label_whitespace_len = leading_whitespace_len(after_label);
                if after_label_whitespace_len > 0 {
                    let value = &after_label[after_label_whitespace_len..];
                    let value_len = sensitive_segment_len(value);
                    if value.get(..value_len).is_some_and(looks_like_secret_token) {
                        output.push_str(&remaining[..label_len]);
                        output.push_str(&after_label[..after_label_whitespace_len]);
                        output.push_str("[redacted]");
                        index += label_len + after_label_whitespace_len + value_len;
                        continue;
                    }
                }
            }
        }
        let ch = remaining
            .chars()
            .next()
            .expect("index is inside utf-8 boundary");
        output.push(ch);
        index += ch.len_utf8();
    }
    output
}

fn leading_whitespace_len(value: &str) -> usize {
    value
        .char_indices()
        .find_map(|(index, ch)| (!ch.is_whitespace()).then_some(index))
        .unwrap_or(value.len())
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|value_prefix| value_prefix.eq_ignore_ascii_case(prefix))
}

fn looks_like_secret_token(value: &str) -> bool {
    value.starts_with("sk-")
        || value.contains('-')
        || value.contains('_')
        || value.chars().any(|ch| ch.is_ascii_digit())
}

fn authorization_value_len(value: &str) -> usize {
    let scheme_len = sensitive_segment_len(value);
    let after_scheme = &value[scheme_len..];
    let whitespace_len = leading_whitespace_len(after_scheme);
    if whitespace_len == 0 {
        return scheme_len;
    }
    let credential = &after_scheme[whitespace_len..];
    scheme_len + whitespace_len + sensitive_segment_len(credential)
}

fn redact_segments(
    input: &str,
    is_segment_start: impl Fn(&str) -> bool,
    replacement: &str,
) -> String {
    let mut output = String::with_capacity(input.len());
    let mut index = 0usize;
    while index < input.len() {
        let remaining = &input[index..];
        if is_segment_start(remaining) {
            output.push_str(replacement);
            index += sensitive_segment_len(remaining);
            continue;
        }
        let ch = remaining
            .chars()
            .next()
            .expect("index is inside utf-8 boundary");
        output.push(ch);
        index += ch.len_utf8();
    }
    output
}

fn sensitive_segment_len(segment: &str) -> usize {
    segment
        .char_indices()
        .find_map(|(index, ch)| sensitive_segment_delimiter(ch).then_some(index))
        .unwrap_or(segment.len())
}

fn sensitive_segment_delimiter(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '"' | '\'' | ')' | ']' | '}' | ',' | ';')
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    fn production_source(path: &str) -> String {
        strip_cfg_test_items(
            &fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap(),
        )
    }

    fn retired_management_service_source() -> String {
        String::new()
    }

    #[test]
    fn management_status_projection_lives_outside_management_service() {
        let service_source = retired_management_service_source();
        let status_source = production_source("src/management_status.rs");
        let operations_source = production_source("src/management_operations.rs");
        let runtime_source = production_source("src/management_runtime.rs");
        let main_source = production_source("src/main.rs");

        assert!(main_source.contains("mod management_status;"));

        for token in [
            "pub struct RuntimeCredentialCounts",
            "pub struct CredentialPoolAlertStatus",
            "pub struct CredentialSetOperationalState",
            "pub struct CredentialSetReadinessProjection",
            "pub struct CredentialSetRuntimeSample",
            "pub struct RuntimeReadinessProjection",
            "pub struct CredentialSetOperationalAlert",
            "pub struct ChannelHealthStatus",
            "pub fn add_key_pool_snapshot_counts",
            "pub fn credential_set_runtime_sample_from_snapshot",
            "pub fn credential_pool_alerts",
            "pub fn credential_set_operational_state",
            "pub fn credential_set_readiness_projection",
            "pub fn runtime_readiness_projection",
            "pub fn credential_set_operational_alerts",
            "pub fn channel_health_status",
            "pub fn redacted_api_base",
        ] {
            assert!(
                status_source.contains(token),
                "management status module must own {token}"
            );
            assert!(
                !service_source.contains(token),
                "management service must not own status projection token {token}"
            );
        }
        assert!(
            status_source.contains(
                "pub fn credential_set_operational_state(\n    counts: &RuntimeCredentialCounts,\n) -> CredentialSetOperationalState"
            ),
            "credential_set_operational_state must return CredentialSetOperationalState"
        );
        for field_index in 0..=4 {
            let token = format!("credential_set_operational_state(&credentials).{field_index}");
            assert!(
                !operations_source.contains(&token),
                "management operations must not read credential set operational state by tuple position: {token}"
            );
            let token =
                format!("credential_set_operational_state(&credential_set_counts).{field_index}");
            assert!(
                !runtime_source.contains(&token),
                "management runtime must not read credential set operational state by tuple position: {token}"
            );
        }
        for token in [
            "let (status, serving_mode, accepting_requests, needs_operator_input, required_action)",
            "let (_, _, accepting_requests, needs_operator_input, _)",
            "let (status, serving_mode, accepting_requests, needs_operator_input, required_action) = credential_set_operational_state(",
            "let (_, _, accepting_requests, needs_operator_input, _) = credential_set_operational_state(",
        ] {
            assert!(
                !operations_source.contains(token),
                "management operations must not destructure credential set operational state by position: {token}"
            );
            assert!(
                !runtime_source.contains(token),
                "management runtime must not destructure credential set operational state by position: {token}"
            );
        }
        for token in [
            "status: operational_state.status",
            "serving_mode: operational_state.serving_mode",
            "accepting_requests: operational_state.accepting_requests",
            "needs_operator_input: operational_state.needs_operator_input",
            "required_action: operational_state.required_action",
        ] {
            assert!(
                operations_source.contains(token),
                "management operations must use named operational state field: {token}"
            );
        }
        for token in [
            "operational_state.needs_operator_input",
            "operational_state.accepting_requests",
        ] {
            assert!(
                !runtime_source.contains(token),
                "management runtime readiness projection must not directly read credential set operational state field: {token}"
            );
        }
    }

    fn strip_cfg_test_items(source: &str) -> String {
        let mut output = String::new();
        let mut cursor = 0;

        while let Some(relative_start) = source[cursor..].find("#[cfg(test)]") {
            let attr_start = cursor + relative_start;
            output.push_str(&source[cursor..attr_start]);

            let mut item_start = line_end(source, attr_start);
            loop {
                item_start = skip_whitespace(source, item_start);
                if source[item_start..].starts_with("#[") {
                    item_start = line_end(source, item_start);
                    continue;
                }
                break;
            }

            if source[item_start..].starts_with("mod tests")
                || source[item_start..].starts_with("pub mod tests")
            {
                cursor = source.len();
                break;
            }

            cursor = item_end(source, item_start);
        }

        output.push_str(&source[cursor..]);
        output
    }

    fn line_end(source: &str, start: usize) -> usize {
        source[start..]
            .find('\n')
            .map(|offset| start + offset + 1)
            .unwrap_or(source.len())
    }

    fn skip_whitespace(source: &str, start: usize) -> usize {
        source[start..]
            .find(|ch: char| !ch.is_whitespace())
            .map(|offset| start + offset)
            .unwrap_or(source.len())
    }

    fn item_end(source: &str, start: usize) -> usize {
        let Some(relative_open_brace) = source[start..].find('{') else {
            return line_end(source, start);
        };
        let open_brace = start + relative_open_brace;
        let semicolon = source[start..].find(';').map(|offset| start + offset);
        if semicolon.is_some_and(|semicolon| semicolon < open_brace) {
            return semicolon.unwrap() + 1;
        }

        let mut depth = 0usize;
        for (offset, ch) in source[open_brace..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return open_brace + offset + ch.len_utf8();
                    }
                }
                _ => {}
            }
        }

        source.len()
    }
}

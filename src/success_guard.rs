use crate::error::{ClassifiedFailure, ErrorClassifier};
use crate::provider::EndpointKind;
use crate::upstream_response::{prefixed_body_stream, UpstreamByteStream};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardOutcome {
    Pass,
    #[cfg(test)]
    Classified,
    CapExhausted,
    DeadlineExhausted,
    ParseUnsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardContentKind {
    Json,
    Sse,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredErrorShape {
    TopLevelErrorObject,
    TopLevelCodeAndMessage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardEvidence {
    pub original_status: u16,
    pub content_type: Option<String>,
    pub endpoint: EndpointKind,
    pub prefix_len: usize,
    pub content_kind: GuardContentKind,
    pub parsed_bytes: usize,
    pub error_shape: Option<StructuredErrorShape>,
    pub sse_first_data_len: Option<usize>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardDecision {
    pub outcome: GuardOutcome,
    pub evidence: GuardEvidence,
    pub classified_failure: Option<ClassifiedFailure>,
}

#[derive(Debug)]
pub struct GuardInspection<'a> {
    pub original_status: u16,
    pub headers: &'a [(&'a str, &'a str)],
    pub content_type: Option<&'a str>,
    pub endpoint: EndpointKind,
    pub prefix: &'a [u8],
    pub cap_exhausted: bool,
    pub deadline_exhausted: bool,
    pub classifier: &'a ErrorClassifier,
}

pub struct GuardStreamInput<'a> {
    pub status: u16,
    pub headers: &'a [(&'a str, &'a str)],
    pub expected_body_len: Option<usize>,
    pub content_type: Option<&'a str>,
    pub endpoint: EndpointKind,
    pub classifier: &'a ErrorClassifier,
    pub max_bytes: usize,
    pub max_duration: Duration,
    pub stream: UpstreamByteStream,
}

pub enum GuardResult {
    PassThrough {
        outcome: GuardOutcome,
        prefix: Bytes,
        stream: UpstreamByteStream,
    },
    Classified {
        failure: ClassifiedFailure,
        #[cfg(test)]
        evidence: GuardEvidence,
    },
}

pub fn should_guard_success_status(status: u16) -> bool {
    (200..=299).contains(&status) && !matches!(status, 204 | 205)
}

pub async fn guard_success_response(
    response: reqwest::Response,
    endpoint: EndpointKind,
    classifier: &ErrorClassifier,
    max_bytes: usize,
    max_duration: Duration,
) -> Result<GuardResult, std::io::Error> {
    let status = response.status().as_u16();
    let header_pairs = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect::<Vec<_>>();
    let header_refs = header_pairs
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let response_content_length = response
        .content_length()
        .and_then(|len| usize::try_from(len).ok());
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let expected_body_len =
        response_content_length.or_else(|| expected_body_len_from_headers(&header_refs));
    let stream = response
        .bytes_stream()
        .map(|item| item.map_err(std::io::Error::other));

    guard_success_byte_stream(GuardStreamInput {
        status,
        headers: &header_refs,
        expected_body_len,
        content_type: content_type.as_deref(),
        endpoint,
        classifier,
        max_bytes,
        max_duration,
        stream: Box::pin(stream),
    })
    .await
}

pub async fn guard_success_byte_stream(
    input: GuardStreamInput<'_>,
) -> Result<GuardResult, std::io::Error> {
    let GuardStreamInput {
        status,
        headers,
        expected_body_len,
        content_type,
        endpoint,
        classifier,
        max_bytes,
        max_duration,
        mut stream,
    } = input;

    if !should_guard_success_status(status) || has_zero_content_length(headers) || max_bytes == 0 {
        return Ok(pass_through(
            GuardOutcome::Pass,
            Bytes::new(),
            stream,
            expected_body_len,
            headers,
            content_type,
            max_bytes,
        ));
    }

    let started = tokio::time::Instant::now();
    let mut prefix = BytesMut::new();

    loop {
        let inspection = GuardInspection {
            original_status: status,
            headers,
            content_type,
            endpoint,
            prefix: &prefix,
            cap_exhausted: prefix.len() >= max_bytes,
            deadline_exhausted: false,
            classifier,
        };
        match inspect_success_progress(&inspection, false) {
            GuardProgress::Classified(classification) => {
                let GuardClassification { failure, evidence } = *classification;
                return Ok(classified_result(failure, evidence));
            }
            GuardProgress::Pass { outcome, .. } => {
                return Ok(pass_through(
                    outcome,
                    prefix.freeze(),
                    stream,
                    expected_body_len,
                    headers,
                    content_type,
                    max_bytes,
                ));
            }
            GuardProgress::NeedMore => {}
        }

        if prefix.len() >= max_bytes {
            return Ok(pass_through(
                GuardOutcome::CapExhausted,
                prefix.freeze(),
                stream,
                expected_body_len,
                headers,
                content_type,
                max_bytes,
            ));
        }

        let Some(remaining_duration) = max_duration.checked_sub(started.elapsed()) else {
            return Ok(pass_through(
                GuardOutcome::DeadlineExhausted,
                prefix.freeze(),
                stream,
                expected_body_len,
                headers,
                content_type,
                max_bytes,
            ));
        };

        let next = match tokio::time::timeout(remaining_duration, stream.next()).await {
            Ok(next) => next,
            Err(_) => {
                return Ok(pass_through(
                    GuardOutcome::DeadlineExhausted,
                    prefix.freeze(),
                    stream,
                    expected_body_len,
                    headers,
                    content_type,
                    max_bytes,
                ))
            }
        };

        match next {
            Some(Ok(chunk)) => {
                let available = max_bytes.saturating_sub(prefix.len());
                if chunk.len() <= available {
                    prefix.extend_from_slice(&chunk);
                } else {
                    prefix.extend_from_slice(&chunk[..available]);
                    let overflow = chunk.slice(available..);
                    let stream = Box::pin(prefixed_body_stream(overflow, stream));
                    let prefix = prefix.freeze();
                    let inspection = GuardInspection {
                        original_status: status,
                        headers,
                        content_type,
                        endpoint,
                        prefix: &prefix,
                        cap_exhausted: true,
                        deadline_exhausted: false,
                        classifier,
                    };
                    if let GuardProgress::Classified(classification) =
                        inspect_success_progress(&inspection, false)
                    {
                        let GuardClassification { failure, evidence } = *classification;
                        return Ok(classified_result(failure, evidence));
                    }
                    return Ok(pass_through(
                        GuardOutcome::CapExhausted,
                        prefix,
                        stream,
                        expected_body_len,
                        headers,
                        content_type,
                        max_bytes,
                    ));
                }
            }
            Some(Err(err)) if prefix.is_empty() => return Err(err),
            Some(Err(err)) => {
                let stream = Box::pin(futures_util::stream::once(async move { Err(err) }));
                return Ok(pass_through(
                    GuardOutcome::ParseUnsupported,
                    prefix.freeze(),
                    stream,
                    expected_body_len,
                    headers,
                    content_type,
                    max_bytes,
                ));
            }
            None => {
                let prefix = prefix.freeze();
                let inspection = GuardInspection {
                    original_status: status,
                    headers,
                    content_type,
                    endpoint,
                    prefix: &prefix,
                    cap_exhausted: false,
                    deadline_exhausted: false,
                    classifier,
                };
                return match inspect_success_progress(&inspection, true) {
                    GuardProgress::Classified(classification) => {
                        let GuardClassification { failure, evidence } = *classification;
                        Ok(classified_result(failure, evidence))
                    }
                    GuardProgress::Pass { outcome, .. } => Ok(pass_through(
                        outcome,
                        prefix,
                        stream,
                        expected_body_len,
                        headers,
                        content_type,
                        max_bytes,
                    )),
                    GuardProgress::NeedMore => Ok(pass_through(
                        outcome_from_eof(&inspection),
                        prefix,
                        stream,
                        expected_body_len,
                        headers,
                        content_type,
                        max_bytes,
                    )),
                };
            }
        }
    }
}

#[cfg(test)]
fn classified_result(failure: ClassifiedFailure, evidence: GuardEvidence) -> GuardResult {
    GuardResult::Classified { failure, evidence }
}

#[cfg(not(test))]
fn classified_result(failure: ClassifiedFailure, _evidence: GuardEvidence) -> GuardResult {
    GuardResult::Classified { failure }
}

fn pass_through(
    outcome: GuardOutcome,
    prefix: Bytes,
    stream: UpstreamByteStream,
    expected_body_len: Option<usize>,
    headers: &[(&str, &str)],
    content_type: Option<&str>,
    max_bytes: usize,
) -> GuardResult {
    let mut stream = match expected_body_len {
        Some(expected) => enforce_content_length(stream, prefix.len(), expected),
        None => stream,
    };
    if should_check_incomplete_structured_eof(outcome, headers, content_type) {
        stream = enforce_structured_eof(
            stream,
            prefix.clone(),
            classify_content_type(content_type),
            max_bytes,
        );
    }
    GuardResult::PassThrough {
        outcome,
        prefix,
        stream,
    }
}

fn should_check_incomplete_structured_eof(
    outcome: GuardOutcome,
    headers: &[(&str, &str)],
    content_type: Option<&str>,
) -> bool {
    if matches!(outcome, GuardOutcome::CapExhausted) || has_non_identity_content_encoding(headers) {
        return false;
    }
    matches!(
        classify_content_type(content_type),
        GuardContentKind::Json | GuardContentKind::Sse
    )
}

fn expected_body_len_from_headers(headers: &[(&str, &str)]) -> Option<usize> {
    if has_non_identity_content_encoding(headers) {
        return None;
    }

    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
}

fn has_non_identity_content_encoding(headers: &[(&str, &str)]) -> bool {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
        .map(|(_, value)| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .any(|value| !value.eq_ignore_ascii_case("identity"))
        })
        .unwrap_or(false)
}

fn enforce_content_length(
    stream: UpstreamByteStream,
    initial_seen: usize,
    expected: usize,
) -> UpstreamByteStream {
    if initial_seen >= expected {
        return stream;
    }

    Box::pin(futures_util::stream::unfold(
        ContentLengthState {
            stream,
            seen: initial_seen,
            expected,
            reported_short: false,
        },
        |mut state| async move {
            match state.stream.next().await {
                Some(Ok(chunk)) => {
                    state.seen = state.seen.saturating_add(chunk.len());
                    Some((Ok(chunk), state))
                }
                Some(Err(err)) => Some((Err(err), state)),
                None if !state.reported_short && state.seen < state.expected => {
                    state.reported_short = true;
                    Some((
                        Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            format!(
                                "upstream body ended before declared content-length: read {} of {} bytes",
                                state.seen, state.expected
                            ),
                        )),
                        state,
                    ))
                }
                None => None,
            }
        },
    ))
}

struct ContentLengthState {
    stream: UpstreamByteStream,
    seen: usize,
    expected: usize,
    reported_short: bool,
}

fn enforce_structured_eof(
    stream: UpstreamByteStream,
    prefix: Bytes,
    content_kind: GuardContentKind,
    max_bytes: usize,
) -> UpstreamByteStream {
    Box::pin(futures_util::stream::unfold(
        StructuredEofState {
            stream,
            pending: BytesMut::from(&prefix[..prefix.len().min(max_bytes)]),
            content_kind,
            max_bytes,
            capped: prefix.len() >= max_bytes,
            reported_incomplete: false,
        },
        |mut state| async move {
            match state.stream.next().await {
                Some(Ok(chunk)) => {
                    if !state.capped {
                        let available = state.max_bytes.saturating_sub(state.pending.len());
                        if chunk.len() <= available {
                            state.pending.extend_from_slice(&chunk);
                        } else {
                            state.pending.extend_from_slice(&chunk[..available]);
                            state.capped = true;
                        }
                    }
                    Some((Ok(chunk), state))
                }
                Some(Err(err)) => Some((Err(err), state)),
                None if !state.reported_incomplete
                    && !state.capped
                    && structured_prefix_is_incomplete(state.content_kind, &state.pending) =>
                {
                    state.reported_incomplete = true;
                    Some((
                        Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "upstream body ended before structured success envelope completed",
                        )),
                        state,
                    ))
                }
                None => None,
            }
        },
    ))
}

struct StructuredEofState {
    stream: UpstreamByteStream,
    pending: BytesMut,
    content_kind: GuardContentKind,
    max_bytes: usize,
    capped: bool,
    reported_incomplete: bool,
}

fn structured_prefix_is_incomplete(content_kind: GuardContentKind, pending: &[u8]) -> bool {
    if pending.is_empty() {
        return false;
    }
    match content_kind {
        GuardContentKind::Json => {
            serde_json::from_slice::<Value>(pending).is_err_and(|error| error.is_eof())
        }
        GuardContentKind::Sse => {
            first_complete_data_event(pending).is_none() && has_partial_sse_data_event(pending)
        }
        GuardContentKind::Other => false,
    }
}

fn has_partial_sse_data_event(prefix: &[u8]) -> bool {
    prefix
        .split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .any(|line| {
            let Some(colon) = line.iter().position(|byte| *byte == b':') else {
                return line == b"data";
            };
            &line[..colon] == b"data"
        })
}

fn has_zero_content_length(headers: &[(&str, &str)]) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-length") && value.trim().parse::<u64>() == Ok(0)
    })
}

fn outcome_from_eof(input: &GuardInspection<'_>) -> GuardOutcome {
    match classify_content_type(input.content_type) {
        GuardContentKind::Json | GuardContentKind::Sse if input.prefix.is_empty() => {
            GuardOutcome::Pass
        }
        GuardContentKind::Json | GuardContentKind::Sse => GuardOutcome::ParseUnsupported,
        GuardContentKind::Other => GuardOutcome::ParseUnsupported,
    }
}

#[cfg(test)]
pub fn inspect_success_prefix(input: &GuardInspection<'_>) -> GuardDecision {
    match inspect_success_progress(input, false) {
        GuardProgress::Classified(classification) => {
            let GuardClassification { failure, evidence } = *classification;
            decision(GuardOutcome::Classified, evidence, Some(failure))
        }
        GuardProgress::Pass { outcome, evidence } => decision(outcome, evidence, None),
        GuardProgress::NeedMore => {
            let evidence = guard_evidence(input, classify_content_type(input.content_type));
            decision(GuardOutcome::Pass, evidence, None)
        }
    }
}

enum GuardProgress {
    Classified(Box<GuardClassification>),
    Pass {
        outcome: GuardOutcome,
        #[cfg(test)]
        evidence: GuardEvidence,
    },
    NeedMore,
}

struct GuardClassification {
    failure: ClassifiedFailure,
    evidence: GuardEvidence,
}

fn inspect_success_progress(input: &GuardInspection<'_>, body_complete: bool) -> GuardProgress {
    let content_kind = classify_content_type(input.content_type);
    let evidence = guard_evidence(input, content_kind);

    if !should_guard_success_status(input.original_status) {
        return pass_progress(GuardOutcome::Pass, evidence);
    }

    if input.deadline_exhausted {
        return pass_progress(GuardOutcome::DeadlineExhausted, evidence);
    }

    match content_kind {
        GuardContentKind::Json => inspect_json_prefix(input, evidence, body_complete),
        GuardContentKind::Sse => inspect_sse_prefix(input, evidence, body_complete),
        GuardContentKind::Other => inspect_other_prefix(input, evidence),
    }
}

fn guard_evidence(input: &GuardInspection<'_>, content_kind: GuardContentKind) -> GuardEvidence {
    GuardEvidence {
        original_status: input.original_status,
        content_type: input.content_type.map(ToOwned::to_owned),
        endpoint: input.endpoint,
        prefix_len: input.prefix.len(),
        content_kind,
        parsed_bytes: 0,
        error_shape: None,
        sse_first_data_len: None,
    }
}

fn inspect_json_prefix(
    input: &GuardInspection<'_>,
    mut evidence: GuardEvidence,
    body_complete: bool,
) -> GuardProgress {
    match serde_json::from_slice::<Value>(input.prefix) {
        Ok(value) => {
            evidence.parsed_bytes = input.prefix.len();
            classify_structured_json(input, evidence, &value, input.prefix).unwrap_or_else(
                |evidence| {
                    if input.cap_exhausted {
                        pass_progress(GuardOutcome::CapExhausted, evidence)
                    } else {
                        pass_progress(GuardOutcome::Pass, evidence)
                    }
                },
            )
        }
        Err(error) if error.is_eof() && !input.cap_exhausted && !body_complete => {
            GuardProgress::NeedMore
        }
        Err(_) if input.cap_exhausted => pass_progress(GuardOutcome::CapExhausted, evidence),
        Err(_) => pass_progress(GuardOutcome::ParseUnsupported, evidence),
    }
}

fn inspect_sse_prefix(
    input: &GuardInspection<'_>,
    mut evidence: GuardEvidence,
    body_complete: bool,
) -> GuardProgress {
    let Some(event) = first_complete_data_event(input.prefix) else {
        return if input.cap_exhausted {
            pass_progress(GuardOutcome::CapExhausted, evidence)
        } else if body_complete {
            pass_progress(GuardOutcome::ParseUnsupported, evidence)
        } else {
            GuardProgress::NeedMore
        };
    };

    evidence.parsed_bytes = event.parsed_bytes;
    evidence.sse_first_data_len = Some(event.data.len());

    let trimmed = trim_ascii(&event.data);
    if trimmed == b"[DONE]" {
        return pass_progress(GuardOutcome::Pass, evidence);
    }

    match serde_json::from_slice::<Value>(trimmed) {
        Ok(value) => classify_structured_json(input, evidence, &value, trimmed)
            .unwrap_or_else(|evidence| pass_progress(GuardOutcome::Pass, evidence)),
        Err(_) => pass_progress(GuardOutcome::ParseUnsupported, evidence),
    }
}

fn inspect_other_prefix(input: &GuardInspection<'_>, _evidence: GuardEvidence) -> GuardProgress {
    if input.cap_exhausted {
        pass_progress(GuardOutcome::CapExhausted, _evidence)
    } else {
        pass_progress(GuardOutcome::ParseUnsupported, _evidence)
    }
}

#[cfg(test)]
fn pass_progress(outcome: GuardOutcome, evidence: GuardEvidence) -> GuardProgress {
    GuardProgress::Pass { outcome, evidence }
}

#[cfg(not(test))]
fn pass_progress(outcome: GuardOutcome, _evidence: GuardEvidence) -> GuardProgress {
    GuardProgress::Pass { outcome }
}

fn classify_structured_json(
    input: &GuardInspection<'_>,
    mut evidence: GuardEvidence,
    value: &Value,
    body: &[u8],
) -> Result<GuardProgress, GuardEvidence> {
    let Some(shape) = structured_error_shape(value) else {
        return Err(evidence);
    };

    evidence.error_shape = Some(shape);
    let failure = input
        .classifier
        .classify_failure(input.original_status, input.headers, body);
    Ok(GuardProgress::Classified(Box::new(GuardClassification {
        failure,
        evidence,
    })))
}

fn structured_error_shape(value: &Value) -> Option<StructuredErrorShape> {
    if value.get("error").is_some_and(Value::is_object) {
        return Some(StructuredErrorShape::TopLevelErrorObject);
    }

    if value.get("code").is_some_and(Value::is_string)
        && value.get("message").is_some_and(Value::is_string)
    {
        return Some(StructuredErrorShape::TopLevelCodeAndMessage);
    }

    None
}

#[cfg(test)]
fn decision(
    outcome: GuardOutcome,
    evidence: GuardEvidence,
    classified_failure: Option<ClassifiedFailure>,
) -> GuardDecision {
    GuardDecision {
        outcome,
        evidence,
        classified_failure,
    }
}

fn classify_content_type(content_type: Option<&str>) -> GuardContentKind {
    let Some(content_type) = content_type else {
        return GuardContentKind::Other;
    };
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();

    if mime == "text/event-stream" {
        GuardContentKind::Sse
    } else if mime == "application/json" || mime.ends_with("+json") {
        GuardContentKind::Json
    } else {
        GuardContentKind::Other
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SseDataEvent {
    data: Vec<u8>,
    parsed_bytes: usize,
}

fn first_complete_data_event(prefix: &[u8]) -> Option<SseDataEvent> {
    let mut offset = 0;
    let mut data_lines: Vec<Vec<u8>> = Vec::new();

    while offset < prefix.len() {
        let newline_relative = prefix[offset..].iter().position(|byte| *byte == b'\n')?;
        let line_end = offset + newline_relative;
        let mut line = &prefix[offset..line_end];
        offset = line_end + 1;

        if let Some(stripped) = line.strip_suffix(b"\r") {
            line = stripped;
        }

        if line.is_empty() {
            if !data_lines.is_empty() {
                return Some(SseDataEvent {
                    data: join_sse_data_lines(&data_lines),
                    parsed_bytes: offset,
                });
            }
            continue;
        }

        if line.starts_with(b":") {
            continue;
        }

        let (field, value) = match line.iter().position(|byte| *byte == b':') {
            Some(colon) => {
                let mut value = &line[colon + 1..];
                if let Some(stripped) = value.strip_prefix(b" ") {
                    value = stripped;
                }
                (&line[..colon], value)
            }
            None => (line, &b""[..]),
        };

        if field == b"data" {
            data_lines.push(value.to_vec());
        }
    }

    None
}

fn join_sse_data_lines(lines: &[Vec<u8>]) -> Vec<u8> {
    let mut data = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            data.push(b'\n');
        }
        data.extend_from_slice(line);
    }
    data
}

fn trim_ascii(value: &[u8]) -> &[u8] {
    let mut value = value;
    while let Some((first, rest)) = value.split_first() {
        if first.is_ascii_whitespace() {
            value = rest;
        } else {
            break;
        }
    }
    while let Some((last, rest)) = value.split_last() {
        if last.is_ascii_whitespace() {
            value = rest;
        } else {
            break;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ErrorClassifier, FailureKind, FailureScope};
    use crate::provider::EndpointKind;
    use bytes::Bytes;
    use futures_util::{stream, StreamExt};
    use std::time::Duration;

    fn inspect_json(body: &[u8]) -> GuardDecision {
        inspect_success_prefix(&GuardInspection {
            original_status: 200,
            headers: &[],
            content_type: Some("application/json"),
            endpoint: EndpointKind::ChatCompletions,
            prefix: body,
            cap_exhausted: false,
            deadline_exhausted: false,
            classifier: &ErrorClassifier::default(),
        })
    }

    fn inspect_sse(prefix: &[u8]) -> GuardDecision {
        inspect_success_prefix(&GuardInspection {
            original_status: 200,
            headers: &[],
            content_type: Some("text/event-stream"),
            endpoint: EndpointKind::Responses,
            prefix,
            cap_exhausted: false,
            deadline_exhausted: false,
            classifier: &ErrorClassifier::default(),
        })
    }

    #[test]
    fn json_top_level_error_object_classifies() {
        let decision =
            inspect_json(br#"{"error":{"code":"invalid_api_key","message":"synthetic"}}"#);

        assert_eq!(decision.outcome, GuardOutcome::Classified);
        let failure = decision.classified_failure.unwrap();
        assert_eq!(failure.kind, FailureKind::AuthInvalid);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert_eq!(failure.upstream_status, Some(200));
        assert_eq!(failure.upstream_code.as_deref(), Some("invalid_api_key"));
        assert_eq!(
            decision.evidence.error_shape,
            Some(StructuredErrorShape::TopLevelErrorObject)
        );
    }

    #[test]
    fn json_top_level_code_and_message_classifies() {
        let decision =
            inspect_json(br#"{"code":"rate_limit_exceeded","message":"synthetic limit"}"#);

        assert_eq!(decision.outcome, GuardOutcome::Classified);
        let failure = decision.classified_failure.unwrap();
        assert_eq!(failure.kind, FailureKind::RateLimited);
        assert_eq!(failure.primary_scope, FailureScope::Credential);
        assert!(failure.retryable);
        assert_eq!(
            failure.upstream_code.as_deref(),
            Some("rate_limit_exceeded")
        );
        assert_eq!(
            decision.evidence.error_shape,
            Some(StructuredErrorShape::TopLevelCodeAndMessage)
        );
    }

    #[test]
    fn code_less_error_object_is_request_only() {
        let decision = inspect_json(br#"{"error":{"message":"synthetic client issue"}}"#);

        assert_eq!(decision.outcome, GuardOutcome::Classified);
        let failure = decision.classified_failure.unwrap();
        assert_eq!(failure.kind, FailureKind::ClientError);
        assert_eq!(failure.primary_scope, FailureScope::RequestOnly);
        assert!(!failure.retryable);
        assert_eq!(failure.upstream_code, None);
    }

    #[test]
    fn sse_waits_for_first_complete_data_event_across_split_prefix() {
        let decision = inspect_sse(b"event: error\ndata: {\"error\":{\"code\":\"invalid_api_key\"");

        assert_eq!(decision.outcome, GuardOutcome::Pass);
        assert_eq!(decision.evidence.parsed_bytes, 0);

        let decision = inspect_sse(
            b"event: error\ndata: {\"error\":{\"code\":\"invalid_api_key\",\"message\":\"synthetic\"}}\n\n",
        );

        assert_eq!(decision.outcome, GuardOutcome::Classified);
        assert_eq!(
            decision
                .classified_failure
                .unwrap()
                .upstream_code
                .as_deref(),
            Some("invalid_api_key")
        );
    }

    #[test]
    fn sse_handles_crlf_comments_and_multiple_data_lines() {
        let decision = inspect_sse(
            b": comment only\r\n\r\nevent: error\r\ndata: {\"error\":\r\ndata: {\"code\":\"rate_limit_exceeded\",\"message\":\"synthetic\"}}\r\n\r\n",
        );

        assert_eq!(decision.outcome, GuardOutcome::Classified);
        let failure = decision.classified_failure.unwrap();
        assert_eq!(failure.kind, FailureKind::RateLimited);
        assert_eq!(
            failure.upstream_code.as_deref(),
            Some("rate_limit_exceeded")
        );
    }

    #[test]
    fn sse_done_passes() {
        let decision = inspect_sse(b"data: [DONE]\n\n");

        assert_eq!(decision.outcome, GuardOutcome::Pass);
        assert!(decision.classified_failure.is_none());
        assert_eq!(decision.evidence.sse_first_data_len, Some(6));
    }

    #[test]
    fn valid_responses_style_sse_does_not_reject() {
        let decision = inspect_sse(
            b"event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n",
        );

        assert_eq!(decision.outcome, GuardOutcome::Pass);
        assert!(decision.classified_failure.is_none());
    }

    #[test]
    fn other_content_type_is_parse_unsupported_without_json_sniffing() {
        let decision = inspect_success_prefix(&GuardInspection {
            original_status: 203,
            headers: &[],
            content_type: Some("text/plain"),
            endpoint: EndpointKind::Generic,
            prefix: br#"  {"error":{"code":"invalid_api_key","message":"synthetic"}} trailing"#,
            cap_exhausted: false,
            deadline_exhausted: false,
            classifier: &ErrorClassifier::default(),
        });

        assert_eq!(decision.outcome, GuardOutcome::ParseUnsupported);
        assert!(decision.classified_failure.is_none());
    }

    #[test]
    fn no_body_success_status_passes_without_classification() {
        let decision = inspect_success_prefix(&GuardInspection {
            original_status: 204,
            headers: &[],
            content_type: Some("application/json"),
            endpoint: EndpointKind::ChatCompletions,
            prefix: br#"{"error":{"code":"invalid_api_key","message":"synthetic"}}"#,
            cap_exhausted: false,
            deadline_exhausted: false,
            classifier: &ErrorClassifier::default(),
        });

        assert_eq!(decision.outcome, GuardOutcome::Pass);
        assert!(decision.classified_failure.is_none());
    }

    #[test]
    fn cap_exhausted_without_decisive_json_passes_with_cap_outcome() {
        let decision = inspect_success_prefix(&GuardInspection {
            original_status: 200,
            headers: &[],
            content_type: Some("application/json"),
            endpoint: EndpointKind::ChatCompletions,
            prefix: br#"{"choices":["#,
            cap_exhausted: true,
            deadline_exhausted: false,
            classifier: &ErrorClassifier::default(),
        });

        assert_eq!(decision.outcome, GuardOutcome::CapExhausted);
        assert!(decision.classified_failure.is_none());
    }

    #[test]
    fn deadline_exhausted_skips_parsing_and_passes_with_deadline_outcome() {
        let decision = inspect_success_prefix(&GuardInspection {
            original_status: 200,
            headers: &[],
            content_type: Some("application/json"),
            endpoint: EndpointKind::ChatCompletions,
            prefix: br#"{"error":{"code":"invalid_api_key","message":"synthetic"}}"#,
            cap_exhausted: false,
            deadline_exhausted: true,
            classifier: &ErrorClassifier::default(),
        });

        assert_eq!(decision.outcome, GuardOutcome::DeadlineExhausted);
        assert!(decision.classified_failure.is_none());
    }

    async fn collect_pass_through(result: GuardResult) -> (GuardOutcome, Bytes) {
        match result {
            GuardResult::PassThrough {
                outcome,
                prefix,
                mut stream,
            } => {
                let mut body = prefix.to_vec();
                while let Some(chunk) = stream.next().await {
                    body.extend_from_slice(&chunk.unwrap());
                }
                (outcome, Bytes::from(body))
            }
            GuardResult::Classified { .. } => panic!("expected pass-through result"),
        }
    }

    #[tokio::test]
    async fn async_json_error_classifies_from_byte_stream() {
        let stream = stream::iter([Ok::<Bytes, std::io::Error>(Bytes::from_static(
            br#"{"error":{"code":"invalid_api_key","message":"synthetic"}}"#,
        ))]);

        let result = guard_success_byte_stream(GuardStreamInput {
            status: 200,
            headers: &[],
            expected_body_len: None,
            content_type: Some("application/json"),
            endpoint: EndpointKind::ChatCompletions,
            classifier: &ErrorClassifier::default(),
            max_bytes: 8192,
            max_duration: Duration::from_millis(200),
            stream: Box::pin(stream),
        })
        .await
        .unwrap();

        match result {
            GuardResult::Classified { failure, evidence } => {
                assert_eq!(failure.kind, FailureKind::AuthInvalid);
                assert_eq!(failure.upstream_status, Some(200));
                assert!(evidence.prefix_len > 0);
                assert_eq!(
                    evidence.error_shape,
                    Some(StructuredErrorShape::TopLevelErrorObject)
                );
            }
            GuardResult::PassThrough { .. } => panic!("expected classified guarded success"),
        }
    }

    #[tokio::test]
    async fn async_sse_split_chunk_first_data_classifies() {
        let stream = stream::iter([
            Ok::<Bytes, std::io::Error>(Bytes::from_static(
                b"event: error\ndata: {\"error\":{\"code\":\"upstream_",
            )),
            Ok(Bytes::from_static(
                b"unavailable\",\"message\":\"synthetic\"}}\n\n",
            )),
        ]);

        let result = guard_success_byte_stream(GuardStreamInput {
            status: 200,
            headers: &[],
            expected_body_len: None,
            content_type: Some("text/event-stream"),
            endpoint: EndpointKind::ChatCompletions,
            classifier: &ErrorClassifier::default(),
            max_bytes: 8192,
            max_duration: Duration::from_millis(200),
            stream: Box::pin(stream),
        })
        .await
        .unwrap();

        assert!(matches!(result, GuardResult::Classified { .. }));
    }

    #[tokio::test]
    async fn async_cap_pass_through_replays_prefix_and_remaining() {
        let stream = stream::iter([
            Ok::<Bytes, std::io::Error>(Bytes::from_static(b"0123456789")),
            Ok(Bytes::from_static(b"abcdef")),
        ]);

        let result = guard_success_byte_stream(GuardStreamInput {
            status: 200,
            headers: &[],
            expected_body_len: None,
            content_type: Some("application/json"),
            endpoint: EndpointKind::ChatCompletions,
            classifier: &ErrorClassifier::default(),
            max_bytes: 8,
            max_duration: Duration::from_millis(200),
            stream: Box::pin(stream),
        })
        .await
        .unwrap();

        let (outcome, body) = collect_pass_through(result).await;
        assert_eq!(outcome, GuardOutcome::CapExhausted);
        assert_eq!(&body[..], b"0123456789abcdef");
    }

    #[tokio::test]
    async fn async_deadline_pass_through_replays_available_prefix() {
        let stream = stream::once(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<Bytes, std::io::Error>(Bytes::from_static(b"late"))
        });

        let result = guard_success_byte_stream(GuardStreamInput {
            status: 200,
            headers: &[],
            expected_body_len: None,
            content_type: Some("application/json"),
            endpoint: EndpointKind::ChatCompletions,
            classifier: &ErrorClassifier::default(),
            max_bytes: 8192,
            max_duration: Duration::from_millis(1),
            stream: Box::pin(stream),
        })
        .await
        .unwrap();

        let (outcome, body) = collect_pass_through(result).await;
        assert_eq!(outcome, GuardOutcome::DeadlineExhausted);
        assert_eq!(&body[..], b"late");
    }

    #[tokio::test]
    async fn async_pass_through_reports_declared_length_short_body() {
        let stream = stream::once(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok::<Bytes, std::io::Error>(Bytes::from_static(br#"{"id":"partial""#))
        });

        let result = guard_success_byte_stream(GuardStreamInput {
            status: 200,
            headers: &[("content-length", "128")],
            expected_body_len: Some(128),
            content_type: Some("application/json"),
            endpoint: EndpointKind::ChatCompletions,
            classifier: &ErrorClassifier::default(),
            max_bytes: 8192,
            max_duration: Duration::from_millis(1),
            stream: Box::pin(stream),
        })
        .await
        .unwrap();

        match result {
            GuardResult::PassThrough {
                outcome,
                prefix,
                mut stream,
            } => {
                assert_eq!(outcome, GuardOutcome::DeadlineExhausted);
                assert!(prefix.is_empty());
                let first = stream.next().await.unwrap().unwrap();
                assert_eq!(&first[..], br#"{"id":"partial""#);
                let err = stream.next().await.unwrap().unwrap_err();
                assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
            }
            GuardResult::Classified { .. } => panic!("expected pass-through result"),
        }
    }

    #[tokio::test]
    async fn async_body_error_after_prefix_remains_in_pass_through_stream() {
        let stream = stream::iter([
            Ok::<Bytes, std::io::Error>(Bytes::from_static(br#"{"id":"partial""#)),
            Err(std::io::Error::other("synthetic body error")),
        ]);

        let result = guard_success_byte_stream(GuardStreamInput {
            status: 200,
            headers: &[("content-type", "application/json")],
            expected_body_len: None,
            content_type: Some("application/json"),
            endpoint: EndpointKind::ChatCompletions,
            classifier: &ErrorClassifier::default(),
            max_bytes: 8192,
            max_duration: Duration::from_millis(200),
            stream: Box::pin(stream),
        })
        .await
        .unwrap();

        match result {
            GuardResult::PassThrough {
                prefix, mut stream, ..
            } => {
                assert_eq!(&prefix[..], br#"{"id":"partial""#);
                let err = stream.next().await.unwrap().unwrap_err();
                assert!(err.to_string().contains("synthetic body error"));
            }
            GuardResult::Classified { .. } => panic!("expected pass-through result"),
        }
    }
}

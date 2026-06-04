use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock},
};

use axum::{
    body::Body,
    http::{header, HeaderMap, StatusCode},
    response::Response,
};
use bytes::{Bytes, BytesMut};
use futures_util::{stream, Stream, StreamExt};

use crate::{
    auth::json_error,
    events::ResponseFilterEventInput,
    provider::HeaderForwardPolicy,
    response_filter::{ResponseFilterAction, ResponseFilterDecision, ResponseFilterPolicy},
};

const MAX_FILTER_PENDING_BYTES: usize = 8192;
const FILTER_OVERLAP_BYTES: usize = 1024;
const RESPONSE_FILTER_REJECTION_JSON: &[u8] =
    br#"{"error":{"type":"response_filter_rejected","message":"upstream response content was blocked by response filter"}}"#;
const RESPONSE_FILTER_REJECTION_SSE: &[u8] =
    b"event: error\ndata: {\"error\":{\"type\":\"response_filter_rejected\",\"message\":\"upstream response content was blocked by response filter\"}}\n\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseContentKind {
    Json,
    Sse,
    Unknown,
}

pub type BodyStreamFailureObserver =
    Arc<dyn Fn(bool) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static>;
pub type UpstreamByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>>;
pub type ResponseFilterEventSink = Arc<dyn Fn(ResponseFilterEventInput) + Send + Sync + 'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseFilterEventContext {
    pub request_id: String,
    pub channel_id: String,
    pub public_model: String,
}

#[derive(Clone)]
pub struct ResponseFilterEventOptions {
    pub context: ResponseFilterEventContext,
    pub sink: ResponseFilterEventSink,
}

pub fn stream_response_with_filter_events(
    status: StatusCode,
    headers: HeaderMap,
    upstream_resp: reqwest::Response,
    response_filter: Arc<RwLock<ResponseFilterPolicy>>,
    body_failure_observer: Option<BodyStreamFailureObserver>,
    response_filter_event_options: Option<ResponseFilterEventOptions>,
) -> Response {
    let stream = upstream_resp
        .bytes_stream()
        .map(|item| item.map_err(std::io::Error::other));
    let stream = prefixed_body_stream(Bytes::new(), stream);
    stream_response_from_byte_stream_with_filter_events(
        status,
        headers,
        Box::pin(stream),
        response_filter,
        body_failure_observer,
        false,
        response_filter_event_options,
    )
}

#[cfg(test)]
pub fn stream_response_from_byte_stream(
    status: StatusCode,
    headers: HeaderMap,
    stream: UpstreamByteStream,
    response_filter: Arc<RwLock<ResponseFilterPolicy>>,
    body_failure_observer: Option<BodyStreamFailureObserver>,
    strip_body_headers: bool,
) -> Response {
    stream_response_from_byte_stream_with_filter_events(
        status,
        headers,
        stream,
        response_filter,
        body_failure_observer,
        strip_body_headers,
        None,
    )
}

pub fn stream_response_from_byte_stream_with_filter_events(
    status: StatusCode,
    mut headers: HeaderMap,
    stream: UpstreamByteStream,
    response_filter: Arc<RwLock<ResponseFilterPolicy>>,
    body_failure_observer: Option<BodyStreamFailureObserver>,
    strip_body_headers: bool,
    response_filter_event_options: Option<ResponseFilterEventOptions>,
) -> Response {
    let response_filter = response_filter
        .read()
        .expect("response filter lock poisoned")
        .clone();
    let configured_filter = response_filter.is_effective();
    let non_identity_encoded = has_non_identity_content_encoding(&headers);
    let effective_filter = configured_filter && !non_identity_encoded;
    let content_kind = response_content_kind(&headers);
    if strip_body_headers || configured_filter {
        strip_stale_body_headers(&mut headers);
    }
    if effective_filter && matches!(content_kind, ResponseContentKind::Json) {
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    }
    let stream: UpstreamByteStream = if effective_filter {
        Box::pin(filter_response_stream(
            stream,
            response_filter,
            content_kind,
            response_filter_event_options,
        ))
    } else {
        stream
    };
    let stream = observe_body_stream_failures(stream, body_failure_observer);
    response_with_headers(status, headers, Body::from_stream(stream))
}

pub fn prefixed_body_stream<S>(
    prefix: Bytes,
    remaining: S,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    stream::unfold(
        PrefixedBodyStreamState {
            prefix: (!prefix.is_empty()).then_some(prefix),
            remaining: Box::pin(remaining),
        },
        |mut state| async move {
            if let Some(prefix) = state.prefix.take() {
                return Some((Ok(prefix), state));
            }
            state.remaining.next().await.map(|item| (item, state))
        },
    )
}

struct PrefixedBodyStreamState<S> {
    prefix: Option<Bytes>,
    remaining: Pin<Box<S>>,
}

fn strip_stale_body_headers(headers: &mut HeaderMap) {
    headers.remove(header::CONTENT_LENGTH);
    if !has_non_identity_content_encoding(headers) {
        headers.remove(header::CONTENT_ENCODING);
    }
}

fn has_non_identity_content_encoding(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .any(|value| !value.eq_ignore_ascii_case("identity"))
        })
        .unwrap_or(false)
}

fn response_content_kind(headers: &HeaderMap) -> ResponseContentKind {
    let Some(value) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return ResponseContentKind::Unknown;
    };
    let media_type = value
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if media_type == "text/event-stream" {
        ResponseContentKind::Sse
    } else if media_type == "application/json" || media_type.ends_with("+json") {
        ResponseContentKind::Json
    } else {
        ResponseContentKind::Unknown
    }
}

fn observe_body_stream_failures<S>(
    stream: S,
    observer: Option<BodyStreamFailureObserver>,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    stream::unfold(
        BodyStreamFailureState {
            stream: Box::pin(stream),
            observer,
            partial_output_started: true,
        },
        |mut state| async move {
            match state.stream.next().await {
                Some(Ok(chunk)) => {
                    state.partial_output_started = true;
                    Some((Ok(chunk), state))
                }
                Some(Err(err)) => {
                    if let Some(observer) = state.observer.take() {
                        observer(state.partial_output_started).await;
                    }
                    Some((Err(err), state))
                }
                None => None,
            }
        },
    )
}

struct BodyStreamFailureState<S> {
    stream: Pin<Box<S>>,
    observer: Option<BodyStreamFailureObserver>,
    partial_output_started: bool,
}

fn filter_response_stream<S>(
    stream: S,
    response_filter: ResponseFilterPolicy,
    content_kind: ResponseContentKind,
    response_filter_event_options: Option<ResponseFilterEventOptions>,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static
where
    S: futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    stream::unfold(
        FilterStreamState {
            stream: Box::pin(stream),
            response_filter,
            content_kind,
            response_filter_event_options,
            pending: BytesMut::new(),
            finished: false,
        },
        |mut state| async move {
            loop {
                if let Some(event_end) = sse_event_end(&state.pending) {
                    let event = state.pending.split_to(event_end);
                    let filtered = filter_response_bytes(
                        event.freeze(),
                        &state.response_filter,
                        state.content_kind,
                        state.response_filter_event_options.as_ref(),
                    );
                    return Some((filtered, state));
                }
                if state.pending.len() > MAX_FILTER_PENDING_BYTES {
                    let release_len = state.pending.len().saturating_sub(FILTER_OVERLAP_BYTES);
                    let prefix = state.pending.split_to(release_len);
                    let filtered = filter_response_bytes(
                        prefix.freeze(),
                        &state.response_filter,
                        state.content_kind,
                        state.response_filter_event_options.as_ref(),
                    );
                    return Some((filtered, state));
                }
                if state.finished {
                    if state.pending.is_empty() {
                        return None;
                    }
                    let remaining = state.pending.split().freeze();
                    let filtered = filter_response_bytes(
                        remaining,
                        &state.response_filter,
                        state.content_kind,
                        state.response_filter_event_options.as_ref(),
                    );
                    return Some((filtered, state));
                }
                match state.stream.next().await {
                    Some(Ok(chunk)) => state.pending.extend_from_slice(&chunk),
                    Some(Err(err)) => return Some((Err(err), state)),
                    None => state.finished = true,
                }
            }
        },
    )
}

struct FilterStreamState<S> {
    stream: std::pin::Pin<Box<S>>,
    response_filter: ResponseFilterPolicy,
    content_kind: ResponseContentKind,
    response_filter_event_options: Option<ResponseFilterEventOptions>,
    pending: BytesMut,
    finished: bool,
}

fn sse_event_end(bytes: &[u8]) -> Option<usize> {
    let lf_end = bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| position + 2);
    let crlf_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4);
    match (lf_end, crlf_end) {
        (Some(lf), Some(crlf)) => Some(lf.min(crlf)),
        (Some(lf), None) => Some(lf),
        (None, Some(crlf)) => Some(crlf),
        (None, None) => None,
    }
}

fn filter_response_bytes(
    chunk: Bytes,
    response_filter: &ResponseFilterPolicy,
    content_kind: ResponseContentKind,
    response_filter_event_options: Option<&ResponseFilterEventOptions>,
) -> Result<Bytes, std::io::Error> {
    let Ok(text) = std::str::from_utf8(&chunk) else {
        return Ok(chunk);
    };
    match response_filter.inspect_text(text) {
        ResponseFilterDecision::Unchanged => Ok(chunk),
        ResponseFilterDecision::Redacted { text, matches } => {
            emit_response_filter_events(
                &matches,
                content_kind,
                "redacted",
                response_filter_event_options,
            );
            Ok(Bytes::from(text))
        }
        ResponseFilterDecision::Rejected { matches } => {
            emit_response_filter_events(
                &matches,
                content_kind,
                "rejected",
                response_filter_event_options,
            );
            match content_kind {
                ResponseContentKind::Sse => Ok(Bytes::from_static(RESPONSE_FILTER_REJECTION_SSE)),
                ResponseContentKind::Json | ResponseContentKind::Unknown => {
                    Ok(Bytes::from_static(RESPONSE_FILTER_REJECTION_JSON))
                }
            }
        }
    }
}

fn emit_response_filter_events(
    matches: &[crate::response_filter::ResponseFilterMatch],
    content_kind: ResponseContentKind,
    outcome: &'static str,
    response_filter_event_options: Option<&ResponseFilterEventOptions>,
) {
    let Some(options) = response_filter_event_options else {
        return;
    };
    for matched_rule in matches {
        (options.sink)(ResponseFilterEventInput {
            request_id: options.context.request_id.clone(),
            channel_id: options.context.channel_id.clone(),
            public_model: options.context.public_model.clone(),
            rule_id: matched_rule.rule_id.clone(),
            action: response_filter_action_name(matched_rule.action).to_string(),
            content_kind: content_kind.event_name().to_string(),
            reason_code: matched_rule.reason_code.to_string(),
            outcome: outcome.to_string(),
            body_committed: true,
        });
    }
}

fn response_filter_action_name(action: ResponseFilterAction) -> &'static str {
    action.as_str()
}

impl ResponseContentKind {
    fn event_name(self) -> &'static str {
        match self {
            ResponseContentKind::Json => "json",
            ResponseContentKind::Sse => "sse",
            ResponseContentKind::Unknown => "unknown",
        }
    }
}

pub fn response_with_headers(status: StatusCode, headers: HeaderMap, body: Body) -> Response {
    let header_policy = HeaderForwardPolicy::from_headers(&headers);
    let mut resp = Response::builder().status(status);
    for (name, value) in headers {
        if let Some(name) = name {
            if !header_policy.allows(name.as_str()) {
                continue;
            }
            resp = resp.header(name, value);
        }
    }
    resp.body(body).unwrap_or_else(|err| {
        json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("response build error: {err}"),
        )
    })
}

pub async fn read_limited_body(resp: reqwest::Response, limit: usize) -> anyhow::Result<Bytes> {
    let mut stream = resp.bytes_stream();
    let mut body = bytes::BytesMut::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len() + chunk.len() > limit {
            anyhow::bail!("upstream body exceeded {limit} bytes");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    use crate::response_filter::{
        ResponseFilterAction, ResponseFilterRuleKind, ResponseFilterRuleSpec, ResponseFilterSpec,
    };

    fn byte_stream(
        chunks: Vec<Result<&'static [u8], std::io::Error>>,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> {
        Box::pin(stream::iter(
            chunks
                .into_iter()
                .map(|chunk| chunk.map(Bytes::from_static)),
        ))
    }

    async fn response_bytes(response: Response) -> Result<Bytes, axum::Error> {
        axum::body::to_bytes(response.into_body(), usize::MAX).await
    }

    fn disabled_filter() -> Arc<RwLock<ResponseFilterPolicy>> {
        Arc::new(RwLock::new(ResponseFilterPolicy::disabled()))
    }

    fn reject_filter() -> Arc<RwLock<ResponseFilterPolicy>> {
        Arc::new(RwLock::new(
            ResponseFilterPolicy::compile(ResponseFilterSpec {
                enabled: true,
                replacement: "[removed]".to_string(),
                rules: vec![ResponseFilterRuleSpec {
                    id: "synthetic-rule".to_string(),
                    kind: ResponseFilterRuleKind::Literal {
                        value: "synthetic-marker".to_string(),
                        case_sensitive: true,
                    },
                    action: ResponseFilterAction::Reject,
                }],
            })
            .unwrap(),
        ))
    }

    #[tokio::test]
    async fn prefixed_body_stream_replays_prefix_once_before_remaining() {
        let stream = prefixed_body_stream(
            Bytes::from_static(b"peeked-"),
            byte_stream(vec![Ok(b"remaining"), Ok(b"-tail")]),
        );
        let response = stream_response_from_byte_stream(
            StatusCode::OK,
            HeaderMap::new(),
            Box::pin(stream),
            disabled_filter(),
            None,
            true,
        );

        let body = response_bytes(response).await.unwrap();

        assert_eq!(body, Bytes::from_static(b"peeked-remaining-tail"));
    }

    #[tokio::test]
    async fn prefixed_body_stream_skips_empty_prefix_chunk() {
        let stream =
            prefixed_body_stream(Bytes::new(), byte_stream(vec![Ok(b"first"), Ok(b"second")]));
        let chunks = stream.collect::<Vec<_>>().await;

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].as_ref().unwrap(), &Bytes::from_static(b"first"));
        assert_eq!(chunks[1].as_ref().unwrap(), &Bytes::from_static(b"second"));
    }

    #[tokio::test]
    async fn byte_stream_response_preserves_body_failure_observer() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let observed_for_observer = observed.clone();
        let observer: BodyStreamFailureObserver = Arc::new(move |partial_output_started| {
            let observed = observed_for_observer.clone();
            Box::pin(async move {
                observed.lock().unwrap().push(partial_output_started);
            })
        });
        let stream = prefixed_body_stream(
            Bytes::from_static(b"prefix"),
            byte_stream(vec![Err(std::io::Error::other("synthetic stream error"))]),
        );
        let response = stream_response_from_byte_stream(
            StatusCode::OK,
            HeaderMap::new(),
            Box::pin(stream),
            disabled_filter(),
            Some(observer),
            true,
        );

        let err = response_bytes(response).await.unwrap_err();

        assert!(err.to_string().contains("synthetic stream error"));
        assert_eq!(*observed.lock().unwrap(), vec![true]);
    }

    #[tokio::test]
    async fn byte_stream_response_strips_stale_length_and_identity_encoding() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_LENGTH, "128".parse().unwrap());
        headers.insert(header::CONTENT_ENCODING, "identity".parse().unwrap());
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        headers.insert("x-request-id", "trace-1".parse().unwrap());
        let response = stream_response_from_byte_stream(
            StatusCode::OK,
            headers,
            byte_stream(vec![Ok(br#"{"ok":true}"#)]),
            disabled_filter(),
            None,
            true,
        );

        assert!(response.headers().get(header::CONTENT_LENGTH).is_none());
        assert!(response.headers().get(header::CONTENT_ENCODING).is_none());
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        assert_eq!(response.headers().get("x-request-id").unwrap(), "trace-1");
    }

    #[tokio::test]
    async fn byte_stream_response_preserves_non_identity_encoding_and_opaque_body() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_LENGTH, "128".parse().unwrap());
        headers.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
        headers.insert(
            header::CONTENT_TYPE,
            "application/octet-stream".parse().unwrap(),
        );
        let filter = ResponseFilterPolicy::compile(ResponseFilterSpec {
            enabled: true,
            replacement: "[removed]".to_string(),
            rules: vec![ResponseFilterRuleSpec {
                id: "synthetic-marker".to_string(),
                kind: ResponseFilterRuleKind::Literal {
                    value: "synthetic-marker".to_string(),
                    case_sensitive: true,
                },
                action: ResponseFilterAction::Redact,
            }],
        })
        .unwrap();
        let response = stream_response_from_byte_stream(
            StatusCode::OK,
            headers,
            byte_stream(vec![Ok(b"synthetic-marker-compressed-bytes")]),
            Arc::new(RwLock::new(filter)),
            None,
            true,
        );

        assert!(response.headers().get(header::CONTENT_LENGTH).is_none());
        assert_eq!(
            response.headers().get(header::CONTENT_ENCODING).unwrap(),
            "gzip"
        );
        let body = response_bytes(response).await.unwrap();
        assert_eq!(
            body,
            Bytes::from_static(b"synthetic-marker-compressed-bytes")
        );
    }

    #[tokio::test]
    async fn response_filter_sees_prefixed_and_remaining_stream_once() {
        let filter = ResponseFilterPolicy::compile(ResponseFilterSpec {
            enabled: true,
            replacement: "[removed]".to_string(),
            rules: vec![ResponseFilterRuleSpec {
                id: "synthetic-marker".to_string(),
                kind: ResponseFilterRuleKind::Literal {
                    value: "synthetic-marker".to_string(),
                    case_sensitive: true,
                },
                action: ResponseFilterAction::Redact,
            }],
        })
        .unwrap();
        let stream = prefixed_body_stream(
            Bytes::from_static(b"data: synthetic-"),
            byte_stream(vec![Ok(b"marker\n\n")]),
        );
        let filter_invocations = Arc::new(AtomicUsize::new(0));
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let sink: ResponseFilterEventSink = Arc::new({
            let recorded = recorded.clone();
            move |event| recorded.lock().unwrap().push(event)
        });
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
        let response = stream_response_from_byte_stream_with_filter_events(
            StatusCode::OK,
            headers,
            Box::pin(stream.inspect({
                let filter_invocations = filter_invocations.clone();
                move |_| {
                    filter_invocations.fetch_add(1, Ordering::Relaxed);
                }
            })),
            Arc::new(RwLock::new(filter)),
            None,
            true,
            Some(ResponseFilterEventOptions {
                context: ResponseFilterEventContext {
                    request_id: "req_prefixed".to_string(),
                    channel_id: "test".to_string(),
                    public_model: "gpt-test".to_string(),
                },
                sink,
            }),
        );

        let body = response_bytes(response).await.unwrap();

        assert_eq!(body, Bytes::from_static(b"data: [removed]\n\n"));
        assert_eq!(filter_invocations.load(Ordering::Relaxed), 2);
        let events = recorded.lock().unwrap().clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].request_id, "req_prefixed");
        assert_eq!(events[0].channel_id, "test");
        assert_eq!(events[0].public_model, "gpt-test");
        assert_eq!(events[0].rule_id, "synthetic-marker");
        assert_eq!(events[0].action, "redact");
        assert_eq!(events[0].content_kind, "sse");
        assert_eq!(events[0].reason_code, "rule_matched");
        assert_eq!(events[0].outcome, "redacted");
        assert!(events[0].body_committed);
    }

    #[tokio::test]
    async fn response_filter_records_safe_event_metadata_for_redaction() {
        let filter = ResponseFilterPolicy::compile(ResponseFilterSpec {
            enabled: true,
            replacement: "[removed]".to_string(),
            rules: vec![ResponseFilterRuleSpec {
                id: "synthetic-rule".to_string(),
                kind: ResponseFilterRuleKind::Literal {
                    value: "synthetic-marker".to_string(),
                    case_sensitive: true,
                },
                action: ResponseFilterAction::Redact,
            }],
        })
        .unwrap();
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let sink: ResponseFilterEventSink = Arc::new({
            let recorded = recorded.clone();
            move |event| recorded.lock().unwrap().push(event)
        });
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        let response = stream_response_from_byte_stream_with_filter_events(
            StatusCode::OK,
            headers,
            byte_stream(vec![Ok(br#"{"content":"synthetic-marker"}"#)]),
            Arc::new(RwLock::new(filter)),
            None,
            false,
            Some(ResponseFilterEventOptions {
                context: ResponseFilterEventContext {
                    request_id: "req_test".to_string(),
                    channel_id: "test".to_string(),
                    public_model: "gpt-test".to_string(),
                },
                sink,
            }),
        );

        let body = response_bytes(response).await.unwrap();
        assert_eq!(body, Bytes::from_static(br#"{"content":"[removed]"}"#));
        let events = recorded.lock().unwrap().clone();
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.request_id, "req_test");
        assert_eq!(event.channel_id, "test");
        assert_eq!(event.public_model, "gpt-test");
        assert_eq!(event.rule_id, "synthetic-rule");
        assert_eq!(event.action, "redact");
        assert_eq!(event.content_kind, "json");
        assert_eq!(event.reason_code, "rule_matched");
        assert_eq!(event.outcome, "redacted");
        assert!(event.body_committed);
        let serialized = serde_json::to_string(event).unwrap();
        for forbidden in [
            "synthetic-marker",
            "request_body",
            "response_body",
            "raw_chunk",
            "upstream_key",
            "client_token",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }

    #[tokio::test]
    async fn non_utf8_response_filter_passes_through_without_event() {
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let sink: ResponseFilterEventSink = Arc::new({
            let recorded = recorded.clone();
            move |event| recorded.lock().unwrap().push(event)
        });
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            "application/octet-stream".parse().unwrap(),
        );
        let response = stream_response_from_byte_stream_with_filter_events(
            StatusCode::OK,
            headers,
            byte_stream(vec![Ok(b"\xff\xfe\xfd")]),
            reject_filter(),
            None,
            false,
            Some(ResponseFilterEventOptions {
                context: ResponseFilterEventContext {
                    request_id: "req_test".to_string(),
                    channel_id: "test".to_string(),
                    public_model: "gpt-test".to_string(),
                },
                sink,
            }),
        );

        let body = response_bytes(response).await.unwrap();
        assert_eq!(body, Bytes::from_static(b"\xff\xfe\xfd"));
        assert!(recorded.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn sse_response_filter_rejection_emits_valid_error_event() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
        headers.insert(header::CONTENT_LENGTH, "64".parse().unwrap());
        let response = stream_response_from_byte_stream(
            StatusCode::OK,
            headers,
            byte_stream(vec![Ok(b"data: synthetic-marker\n\n")]),
            reject_filter(),
            None,
            false,
        );

        assert!(response.headers().get(header::CONTENT_LENGTH).is_none());
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );
        let body = response_bytes(response).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.starts_with("event: error\ndata: "));
        assert!(text.ends_with("\n\n"));
        assert!(text.contains(r#""type":"response_filter_rejected""#));
    }

    #[tokio::test]
    async fn json_response_filter_rejection_uses_local_json_content_type() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            "application/vnd.test+json".parse().unwrap(),
        );
        headers.insert(header::CONTENT_LENGTH, "64".parse().unwrap());
        let response = stream_response_from_byte_stream(
            StatusCode::OK,
            headers,
            byte_stream(vec![Ok(br#"{"content":"synthetic-marker"}"#)]),
            reject_filter(),
            None,
            false,
        );

        assert!(response.headers().get(header::CONTENT_LENGTH).is_none());
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let body = response_bytes(response).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"]["type"], "response_filter_rejected");
    }
}

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, RwLock},
};

use axum::{
    body::Body,
    http::{HeaderMap, StatusCode},
    response::Response,
};
use bytes::{Bytes, BytesMut};
use futures_util::{stream, Stream, StreamExt};

use crate::{
    auth::json_error,
    provider::HeaderForwardPolicy,
    response_filter::{ResponseFilterDecision, ResponseFilterPolicy},
};

const MAX_FILTER_PENDING_BYTES: usize = 8192;
const FILTER_OVERLAP_BYTES: usize = 1024;

pub type BodyStreamFailureObserver =
    Arc<dyn Fn(bool) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync + 'static>;

pub fn stream_response(
    status: StatusCode,
    headers: HeaderMap,
    upstream_resp: reqwest::Response,
    response_filter: Arc<RwLock<ResponseFilterPolicy>>,
    body_failure_observer: Option<BodyStreamFailureObserver>,
) -> Response {
    let stream = upstream_resp
        .bytes_stream()
        .map(|item| item.map_err(std::io::Error::other));
    let effective_filter = response_filter
        .read()
        .expect("response filter lock poisoned")
        .is_effective();
    let stream: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> =
        if effective_filter {
            let response_filter = response_filter
                .read()
                .expect("response filter lock poisoned")
                .clone();
            Box::pin(filter_response_stream(stream, response_filter))
        } else {
            Box::pin(stream)
        };
    let stream = observe_body_stream_failures(stream, body_failure_observer);
    response_with_headers(status, headers, Body::from_stream(stream))
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
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static
where
    S: futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static,
{
    stream::unfold(
        FilterStreamState {
            stream: Box::pin(stream),
            response_filter,
            pending: BytesMut::new(),
            finished: false,
        },
        |mut state| async move {
            loop {
                if let Some(event_end) = sse_event_end(&state.pending) {
                    let event = state.pending.split_to(event_end);
                    let filtered = filter_response_bytes(event.freeze(), &state.response_filter);
                    return Some((filtered, state));
                }
                if state.pending.len() > MAX_FILTER_PENDING_BYTES {
                    let release_len = state.pending.len().saturating_sub(FILTER_OVERLAP_BYTES);
                    let prefix = state.pending.split_to(release_len);
                    let filtered = filter_response_bytes(prefix.freeze(), &state.response_filter);
                    return Some((filtered, state));
                }
                if state.finished {
                    if state.pending.is_empty() {
                        return None;
                    }
                    let remaining = state.pending.split().freeze();
                    let filtered = filter_response_bytes(remaining, &state.response_filter);
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
    pending: BytesMut,
    finished: bool,
}

fn sse_event_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| position + 2)
}

fn filter_response_bytes(
    chunk: Bytes,
    response_filter: &ResponseFilterPolicy,
) -> Result<Bytes, std::io::Error> {
    let Ok(text) = std::str::from_utf8(&chunk) else {
        return Ok(chunk);
    };
    match response_filter.inspect_text(text) {
        ResponseFilterDecision::Unchanged => Ok(chunk),
        ResponseFilterDecision::Redacted { text, .. } => Ok(Bytes::from(text)),
        ResponseFilterDecision::Rejected { .. } => Ok(Bytes::from_static(
            br#"{"error":{"type":"response_filter_rejected","message":"upstream response content was blocked by response filter"}}"#,
        )),
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

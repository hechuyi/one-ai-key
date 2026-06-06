use serde_json::Value;

#[derive(Debug, Clone)]
pub(crate) struct RuntimeReloadSummary {
    pub active_registry_generation: Option<u64>,
    pub active_registry_version: Option<u64>,
    pub staged_registry_version: Option<u64>,
    pub runtime_reload_required: Option<bool>,
    pub reload_diff_status: &'static str,
    pub reload_diff_reason_code: &'static str,
    pub reload_diff_next_action: Value,
}

pub(crate) async fn fetch_runtime_reload_projection(
    client: &crate::operator_client::OperatorClient,
) -> Result<Option<Value>, crate::operator_client::OperatorClientError> {
    match client
        .get_json(crate::operator_client::ReadOnlyEndpoint::RuntimeReloadDiff)
        .await
    {
        Ok(projection) => Ok(Some(projection)),
        Err(error) if error.reason_code() == "management_not_found" => {
            match client
                .get_json(crate::operator_client::ReadOnlyEndpoint::ExplainRuntime)
                .await
            {
                Ok(projection) => Ok(Some(projection)),
                Err(fallback) if fallback.reason_code() == "management_not_found" => Ok(None),
                Err(fallback) => Err(fallback),
            }
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn attach_runtime_reload_projection(target: &mut Value, projection: Option<Value>) {
    let Some(projection) = projection else {
        return;
    };
    if let Some(target) = target.as_object_mut() {
        target.insert("runtime_reload_projection".to_string(), projection);
    }
}

pub(crate) fn summarize_runtime_reload_projection(
    projection: Option<&Value>,
) -> RuntimeReloadSummary {
    let active_registry_generation = projection
        .and_then(|projection| projection.get("active_registry_generation"))
        .and_then(Value::as_u64);
    let active_registry_version = projection
        .and_then(|projection| projection.get("active_registry_version"))
        .and_then(Value::as_u64);
    let staged_registry_version = projection
        .and_then(|projection| projection.get("staged_registry_version"))
        .and_then(Value::as_u64);
    let runtime_reload_required = projection
        .and_then(|projection| {
            projection
                .get("runtime_reload_required")
                .or_else(|| projection.get("reload_required"))
        })
        .and_then(Value::as_bool);
    let reload_diff_status = projection
        .and_then(|projection| projection.get("status"))
        .and_then(Value::as_str)
        .and_then(safe_reload_diff_status)
        .unwrap_or("unknown");
    let reload_diff_reason_code = projection
        .and_then(|projection| projection.get("reason_code"))
        .and_then(Value::as_str)
        .and_then(safe_reload_diff_reason_code)
        .unwrap_or("unknown");

    RuntimeReloadSummary {
        active_registry_generation,
        active_registry_version,
        staged_registry_version,
        runtime_reload_required,
        reload_diff_status,
        reload_diff_reason_code,
        reload_diff_next_action: reload_diff_next_action(
            runtime_reload_required,
            reload_diff_reason_code,
        ),
    }
}

pub(crate) fn append_runtime_reload_table_fields(output: &mut String, report: &Value) {
    for field in [
        "active_registry_generation",
        "active_registry_version",
        "staged_registry_version",
        "runtime_reload_required",
        "reload_diff_status",
        "reload_diff_reason_code",
    ] {
        crate::cli_report::push_table_field(output, field, report.get(field));
    }
    if let Some(action) = report.get("reload_diff_next_action") {
        crate::cli_report::push_table_field(
            output,
            "reload_diff_next_action",
            action.get("summary"),
        );
        crate::cli_report::push_table_field(
            output,
            "reload_diff_next_action.template_id",
            action.get("template_id"),
        );
        crate::cli_report::push_table_field(
            output,
            "reload_diff_next_action.side_effect_class",
            action.get("side_effect_class"),
        );
        crate::cli_report::push_table_field(
            output,
            "reload_diff_next_action.requires_confirmation",
            action.get("requires_confirmation"),
        );
        append_reload_diff_safe_argv(output, action.get("safe_argv"));
    }
}

fn append_reload_diff_safe_argv(output: &mut String, safe_argv: Option<&Value>) {
    let Some(argv) = safe_argv.and_then(Value::as_array) else {
        return;
    };
    if argv.is_empty() {
        output.push_str("reload_diff_next_action.safe_argv: []\n");
        return;
    }
    for (index, arg) in argv.iter().enumerate() {
        crate::cli_report::push_table_field(
            output,
            &format!("reload_diff_next_action.safe_argv[{index}]"),
            Some(arg),
        );
    }
}

fn safe_reload_diff_status(status: &str) -> Option<&'static str> {
    match status {
        "ok" => Some("ok"),
        "unavailable" => Some("unavailable"),
        _ => None,
    }
}

fn safe_reload_diff_reason_code(reason_code: &str) -> Option<&'static str> {
    match reason_code {
        "reload_diff_available" => Some("reload_diff_available"),
        "reload_diff_truncated" => Some("reload_diff_truncated"),
        "reload_diff_empty" => Some("reload_diff_empty"),
        "unavailable_without_staged_projection" => Some("unavailable_without_staged_projection"),
        _ => None,
    }
}

fn reload_diff_next_action(
    runtime_reload_required: Option<bool>,
    reload_diff_reason_code: &str,
) -> Value {
    match reload_diff_reason_code {
        "reload_diff_available" | "reload_diff_truncated" | "reload_diff_empty" => {
            serde_json::json!({
                "summary": "Inspect the read-only reload diff projection before deciding whether to apply a runtime reload.",
                "template_id": "reload_diff_available",
                "safe_argv": ["one-ai-key", "reload", "diff", "--management-url", "<url>", "--management-token-env", "<env>"],
                "side_effect_class": "runtime_readonly",
                "requires_confirmation": false,
            })
        }
        "unavailable_without_staged_projection" => serde_json::json!({
            "summary": "Reload diff has no staged projection to compare; inspect reload status or prepare supported staged changes.",
            "template_id": "reload_diff_unavailable",
            "safe_argv": [],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
        _ if runtime_reload_required == Some(true) => serde_json::json!({
            "summary": "Runtime reports pending reload state; inspect reload diff if the server supports that read-only projection.",
            "template_id": "reload_diff",
            "safe_argv": ["one-ai-key", "reload", "diff", "--management-url", "<url>", "--management-token-env", "<env>"],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
        _ => serde_json::json!({
            "summary": "Runtime reload diff projection is not available in this explanation response.",
            "template_id": "reload_diff_unknown",
            "safe_argv": [],
            "side_effect_class": "runtime_readonly",
            "requires_confirmation": false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use axum::{http::StatusCode, routing::get, Json, Router};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    async fn spawn_management_fixture(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{addr}")
    }

    fn test_client(
        management_url: String,
    ) -> Result<crate::operator_client::OperatorClient, crate::operator_client::OperatorClientError>
    {
        crate::operator_client::OperatorClient::new(crate::operator_client::OperatorClientConfig {
            base_url: reqwest::Url::parse(&management_url).unwrap(),
            bearer_token: crate::operator_client::ManagementToken::from_stdin_bytes(
                b"opaque-management-fixture",
            )
            .unwrap(),
            timeout_seconds: 10,
        })
    }

    #[tokio::test]
    async fn fetch_runtime_reload_projection_falls_back_only_on_not_found() {
        let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
        let diff_seen = Arc::clone(&seen_paths);
        let explain_seen = Arc::clone(&seen_paths);
        let router = Router::new()
            .route(
                "/management/runtime/reload-diff",
                get(move || {
                    let diff_seen = Arc::clone(&diff_seen);
                    async move {
                        diff_seen
                            .lock()
                            .unwrap()
                            .push("/management/runtime/reload-diff".to_string());
                        StatusCode::NOT_FOUND
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
                            "active_registry_generation": 44,
                            "runtime_reload_required": true
                        }))
                    }
                }),
            );
        let client = test_client(spawn_management_fixture(router).await).unwrap();

        let projection = super::fetch_runtime_reload_projection(&client)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(projection["active_registry_generation"], 44);
        assert_eq!(
            *seen_paths.lock().unwrap(),
            vec![
                "/management/runtime/reload-diff".to_string(),
                "/management/explain/runtime".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn fetch_runtime_reload_projection_propagates_non_not_found_errors() {
        for (status, reason_code) in [
            (StatusCode::UNAUTHORIZED, "management_unauthorized"),
            (StatusCode::FORBIDDEN, "management_forbidden"),
            (StatusCode::BAD_GATEWAY, "management_non_json_error"),
        ] {
            let seen_paths = Arc::new(Mutex::new(Vec::<String>::new()));
            let diff_seen = Arc::clone(&seen_paths);
            let explain_seen = Arc::clone(&seen_paths);
            let router = Router::new()
                .route(
                    "/management/runtime/reload-diff",
                    get(move || {
                        let diff_seen = Arc::clone(&diff_seen);
                        async move {
                            diff_seen
                                .lock()
                                .unwrap()
                                .push("/management/runtime/reload-diff".to_string());
                            status
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
                            Json(json!({"runtime_reload_required": false}))
                        }
                    }),
                );
            let client = test_client(spawn_management_fixture(router).await).unwrap();

            let error = super::fetch_runtime_reload_projection(&client)
                .await
                .unwrap_err();

            assert_eq!(error.reason_code(), reason_code);
            assert_eq!(
                *seen_paths.lock().unwrap(),
                vec!["/management/runtime/reload-diff".to_string()]
            );
        }
    }
}

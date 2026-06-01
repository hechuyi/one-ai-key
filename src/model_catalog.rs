use std::collections::BTreeMap;

use axum::{
    response::{IntoResponse, Response},
    Json,
};
use serde_json::Value;

use crate::state::{
    ChannelHealth, PublicModelCatalogEntry, PublicModelCatalogTarget, RuntimeCatalogs,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedModelCatalog {
    pub model_ids: Vec<String>,
}

pub fn openai_model_catalog_from_body(body: &[u8]) -> Option<ParsedModelCatalog> {
    let value = serde_json::from_slice::<Value>(body).ok()?;
    if value.get("object").and_then(Value::as_str) != Some("list") {
        return None;
    }
    let items = value.get("data").and_then(Value::as_array)?;
    let mut ids = BTreeMap::new();
    for item in items {
        if item.get("object").and_then(Value::as_str) != Some("model") {
            return None;
        }
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        ids.entry(id.to_string()).or_insert(());
    }
    Some(ParsedModelCatalog {
        model_ids: ids.into_keys().collect(),
    })
}

pub fn compiled_openai_models(
    public_catalog: Vec<PublicModelCatalogEntry>,
    runtime_catalogs: &RuntimeCatalogs,
    allowed_model_groups: &[String],
    allowed_channels: &[String],
) -> Response {
    let mut models = BTreeMap::new();
    let public_routes = public_route_catalog(
        public_catalog,
        runtime_catalogs,
        allowed_model_groups,
        allowed_channels,
    );
    for route in &public_routes {
        models.insert(route.public_model.clone(), ());
    }

    Json(openai_model_catalog_response(models.into_keys())).into_response()
}

fn openai_model_catalog_response<I, S>(model_ids: I) -> Value
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    serde_json::json!({
        "object": "list",
        "data": model_ids
            .into_iter()
            .map(|id| {
                serde_json::json!({
                    "id": id.as_ref(),
                    "object": "model",
                    "owned_by": "key-pool-router"
                })
            })
            .collect::<Vec<_>>()
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicRouteCatalogEntry {
    public_model: String,
}

fn public_route_catalog(
    public_catalog: Vec<PublicModelCatalogEntry>,
    runtime_catalogs: &RuntimeCatalogs,
    allowed_model_groups: &[String],
    allowed_channels: &[String],
) -> Vec<PublicRouteCatalogEntry> {
    let mut entries = Vec::new();
    for route in public_catalog {
        if !runtime_catalogs.client_model_allowed(allowed_model_groups, &route.public_model) {
            continue;
        }
        if any_public_route_target_runtime_visible(&route.targets, allowed_channels) {
            entries.push(PublicRouteCatalogEntry {
                public_model: route.public_model,
            });
        }
    }
    entries
}

fn any_public_route_target_runtime_visible(
    targets: &[PublicModelCatalogTarget],
    allowed_channels: &[String],
) -> bool {
    targets
        .iter()
        .any(|target| public_route_target_runtime_visible(target, allowed_channels))
}

fn public_route_target_runtime_visible(
    target: &PublicModelCatalogTarget,
    allowed_channels: &[String],
) -> bool {
    if !allowed_channels.is_empty()
        && !allowed_channels
            .iter()
            .any(|allowed| allowed == &target.channel_id)
    {
        return false;
    };

    !matches!(
        *target.health.lock().expect("channel health mutex poisoned"),
        ChannelHealth::Disabled { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_catalog_parser_returns_explicit_catalog_type() {
        assert_eq!(
            openai_model_catalog_from_body(
                br#"{
                    "object": "list",
                    "data": [
                        {"id": " gpt-4.1 ", "object": "model"},
                        {"id": "gpt-4.1", "object": "model"},
                        {"id": "claude-sonnet-4", "object": "model"}
                    ]
                }"#
            ),
            Some(ParsedModelCatalog {
                model_ids: vec!["claude-sonnet-4".to_string(), "gpt-4.1".to_string()]
            })
        );
    }

    #[test]
    fn openai_catalog_parser_rejects_structured_non_model_data() {
        assert_eq!(
            openai_model_catalog_from_body(
                br#"{
                    "data": [
                        {"id": "not-a-model"}
                    ]
                }"#
            ),
            None
        );
        assert_eq!(
            openai_model_catalog_from_body(
                br#"{
                    "object": "list",
                    "data": [
                        {"id": "not-a-model", "object": "chat.completion"}
                    ]
                }"#
            ),
            None
        );
    }

    #[test]
    fn openai_model_catalog_response_uses_openai_models_shape() {
        assert_eq!(
            openai_model_catalog_response(["gpt-4.1", "claude-sonnet-4"]),
            serde_json::json!({
                "object": "list",
                "data": [
                    {
                        "id": "gpt-4.1",
                        "object": "model",
                        "owned_by": "key-pool-router"
                    },
                    {
                        "id": "claude-sonnet-4",
                        "object": "model",
                        "owned_by": "key-pool-router"
                    }
                ]
            })
        );
    }
}

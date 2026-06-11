use axum::{
    extract::{ConnectInfo, MatchedPath, Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};

use crate::{
    auth::{self, authorize_management},
    config::ManagementRole,
    state::AppState,
};
use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ManagementRouteSpec {
    pub(crate) method: &'static str,
    pub(crate) path: &'static str,
    pub(crate) minimum_role: ManagementRole,
}

const MANAGEMENT_ROUTE_SPECS: &[ManagementRouteSpec] = &[
    ManagementRouteSpec {
        method: "GET",
        path: "/management/pools",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/channels",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/client-tokens",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/client-tokens",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "PATCH",
        path: "/management/client-tokens/:id",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/client-tokens/:id/disable",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/client-tokens/:id/enable",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/providers",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "PUT",
        path: "/management/registry/providers/:id",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/registry/providers/:id/disable",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/registry/providers/:id/enable",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/accounts",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "PUT",
        path: "/management/registry/accounts/:id",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/registry/accounts/:id/disable",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/registry/accounts/:id/enable",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "PUT",
        path: "/management/registry/channels/:id",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/registry/channels/:id/disable",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/registry/channels/:id/enable",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/credential-sets",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/credential-sets/:id/operations",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/credential-sets/:id/credentials",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/credential-sets/:id/imports",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/credential-sets/:id/imports/:batch_id",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/credential-sets/:id/credentials/:credential_id",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "PUT",
        path: "/management/credential-sets/:id/credentials/:credential_id/metadata",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/credential-sets/:id/credentials/:credential_id/history",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/credential-sets/:id/credentials/:credential_id/probe",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/credential-sets/:id/credentials/:credential_id/probes",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/credential-sets/:id/credentials/:credential_id/apply-latest-probe/plan",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/credential-sets/:id/credentials/:credential_id/apply-latest-probe",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/credential-sets/:id/credentials/import",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/credential-sets/:id/credentials/:credential_id/expire",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/credential-sets/:id/credentials/:credential_id/restore",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/credential-sets/:id/credentials/:credential_id/quota-exhaust",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/credential-sets/:id/credentials/:credential_id/disable",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/credential-sets/:id/credentials/:credential_id/enable",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/credential-sets/:id/credentials/:credential_id/reset-cooldown",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/model-routes",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/model-availability",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "PUT",
        path: "/management/registry/model-routes/*model",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/model-discovery/sync-plan",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/model-discovery/sync-apply",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/policy-profiles",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "PUT",
        path: "/management/registry/policy-profiles/:id",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/policy-profiles/:id",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/routing-profiles",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "PUT",
        path: "/management/registry/routing-profiles/:id",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/routing-profiles/:id",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/routing/preview",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/runtime",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/explain/runtime",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/runtime/reload-diff",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/health/serving",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/health/resilience",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/runtime/reload",
        minimum_role: ManagementRole::Admin,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/alerts",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/channels/:id",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/channels/:id/model-discovery",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/channels/:id/reset-health",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/channels/:id/disable",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/channels/:id/enable",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/channels/:id/error-rules",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/channels/:id/credentials",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/channels/:id/credentials/:credential_id/expire",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/channels/:id/credentials/:credential_id/restore",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/channels/:id/credentials/:credential_id/quota-exhaust",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/channels/:id/credentials/:credential_id/disable",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/channels/:id/credentials/:credential_id/enable",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "POST",
        path: "/management/channels/:id/credentials/:credential_id/reset-cooldown",
        minimum_role: ManagementRole::Operator,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/events",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/response-filter-events",
        minimum_role: ManagementRole::Readonly,
    },
    ManagementRouteSpec {
        method: "GET",
        path: "/management/routing-telemetry",
        minimum_role: ManagementRole::Readonly,
    },
];

#[cfg(test)]
pub(crate) fn management_route_specs() -> &'static [ManagementRouteSpec] {
    MANAGEMENT_ROUTE_SPECS
}

fn management_route_spec(method: &str, path: &str) -> Option<&'static ManagementRouteSpec> {
    MANAGEMENT_ROUTE_SPECS
        .iter()
        .find(|spec| spec.method == method && spec.path == path)
}

pub(crate) fn is_management_path(path: &str) -> bool {
    path == "/management" || path.starts_with("/management/")
}

pub(super) async fn management_role_gate(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if !is_management_path(request.uri().path()) {
        return next.run(request).await;
    }

    let peer = match request.extensions().get::<ConnectInfo<SocketAddr>>() {
        Some(ConnectInfo(peer)) => peer,
        None => {
            return auth::json_error(
                StatusCode::FORBIDDEN,
                "management peer address is unavailable",
            );
        }
    };
    let peer_allowed = state
        .management_ip_allowlist
        .read()
        .expect("management IP allowlist lock poisoned")
        .allows_ip(peer.ip());
    if !peer_allowed {
        return auth::json_error(
            StatusCode::FORBIDDEN,
            "management peer address is not allowed",
        );
    }

    let principal = match authorize_management(&state, request.headers()) {
        Ok(principal) => principal,
        Err(response) => return *response,
    };
    let matched_path = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or_else(|| request.uri().path());
    let Some(spec) = management_route_spec(request.method().as_str(), matched_path) else {
        return auth::json_error(StatusCode::FORBIDDEN, "management route is not registered");
    };
    if !principal.role.allows(spec.minimum_role) {
        return auth::json_error(
            StatusCode::FORBIDDEN,
            "management principal role is not allowed for this route",
        );
    }

    next.run(request).await
}

pub(super) async fn unregistered_management_route() -> Response {
    auth::json_error(StatusCode::FORBIDDEN, "management route is not registered")
}

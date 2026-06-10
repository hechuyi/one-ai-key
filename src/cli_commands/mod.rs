pub mod client_tokens;
pub mod doctor;
pub mod failures;
pub mod keys;
pub(crate) mod model_availability_projection;
pub mod models;
pub mod models_onboard;
pub mod reload;
pub mod route;
pub(crate) mod runtime_reload_projection;

use crate::{
    cli::OperatorConnectionOptions,
    operator_client::{
        ManagementTokenSource, OperatorClient, OperatorClientConfig, OperatorClientError,
        OperatorClientOptions,
    },
};

pub fn operator_client_from_connection(
    connection: &OperatorConnectionOptions,
) -> Result<OperatorClient, OperatorClientError> {
    let token_source = if let Some(env_name) = connection.management_token_env.clone() {
        ManagementTokenSource::Env(env_name)
    } else if connection.management_token_stdin {
        ManagementTokenSource::Stdin
    } else {
        return Err(OperatorClientError::new(
            "management_token_source_missing",
            "operator commands require --management-token-env or --management-token-stdin",
        ));
    };
    let config = OperatorClientConfig::from_options(
        OperatorClientOptions {
            management_url: connection.management_url.clone(),
            deprecated_base_url: connection.deprecated_base_url.clone(),
            token_source,
            timeout_seconds: connection.timeout_seconds,
        },
        |name| std::env::var(name).ok(),
    )?;
    OperatorClient::new(config)
}
